//! Frontend-neutral app state: mod lists, rules, databases, and background workers.
//!
//! Both the TUI and the GUI own one `Session`, call `poll()` every frame/tick, and pass
//! explicit targets (paths, package IDs, queries). Selection, focus, and widgets stay in
//! the frontend.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use tracing::{error, info};

use crate::constants::is_ludeon_package;
use crate::fetch::{self, FetchMsg, FetchRequest};
use crate::github::{
    self, GithubJob, GithubModEntry, GithubModsDb, GithubMsg, GithubReport, GithubRequest,
    GithubUpdateItem, ReleaseCache,
};
use crate::gitmod::{self, GitJob, GitModInfo, GitMsg, GitReport};
use crate::issues::{check_issues, Issue};
use crate::launch::{self, LaunchKind};
use crate::lists;
use crate::model::{resolve_mod, Mod, ModSource};
use crate::rules::{self, RulesDb};
use crate::scan::scan_mods;
use crate::settings::{Settings, SortMode};
use crate::sort::{sort_active, sort_active_alphabetical};
use crate::steamcmd::{self, SteamcmdItem, SteamcmdMsg, SteamcmdRequest};
use crate::steamdb::{self, SteamDb};
use crate::workshop::{self, WorkshopItem, WorkshopMsg};
use crate::xml::mods_config::{read_mods_config, write_mods_config, ModsConfig};

/// Things a frontend may want to react to after `Session::poll`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Mods were rescanned; lists, issues, and git items changed.
    Reloaded,
    /// `workshop_items` has fresh results.
    WorkshopResults,
    /// A GitHub install finished (the query field can close).
    GithubInstalled,
    /// A git clone finished (the query field can close).
    GitCloned,
}

pub struct Session {
    pub settings: Settings,
    pub mods: Vec<Mod>,
    pub mod_index: HashMap<PathBuf, usize>,
    pub inactive_paths: Vec<PathBuf>,
    pub active_paths: Vec<PathBuf>,
    pub missing_from_disk: Vec<String>,
    pub mods_config: ModsConfig,
    pub issues: Vec<Issue>,
    pub dirty: bool,
    pub status: String,
    pub community_rules: RulesDb,
    pub user_rules: RulesDb,
    pub steam_db: SteamDb,
    pub workshop_items: Vec<WorkshopItem>,
    pub github_mods: GithubModsDb,
    pub github_cache: ReleaseCache,
    pub git_items: Vec<GitModInfo>,
    fetch_rx: Option<Receiver<FetchMsg>>,
    scan_rx: Option<Receiver<Result<ReloadData, String>>>,
    /// The running scan keeps unsaved list edits (background rescan) instead of re-reading disk.
    scan_keeps_edits: bool,
    /// A scan queued behind the running one; `Some(true)` = keep edits.
    pending_reload: Option<bool>,
    /// Job result to repeat in the status line once the follow-up scan replaces it.
    scan_note: Option<String>,
    /// Some SteamCMD batch in the current chain succeeded (rescan when the queue drains).
    steamcmd_succeeded: bool,
    workshop_rx: Option<Receiver<WorkshopMsg>>,
    steamcmd_rx: Option<Receiver<SteamcmdMsg>>,
    pending_steamcmd: Vec<SteamcmdItem>,
    pending_activate: HashSet<String>,
    pending_path_activate: HashSet<PathBuf>,
    github_rx: Option<Receiver<GithubMsg>>,
    pending_github: Vec<GithubJob>,
    github_auto_after_check: bool,
    git_rx: Option<Receiver<GitMsg>>,
    /// Paths the current mod list was scanned from; Save is refused if settings moved away.
    loaded_paths: Option<ScanPaths>,
    pending_git: Vec<GitJob>,
}

type ScanPaths = [PathBuf; 4];

fn scan_paths(s: &Settings) -> ScanPaths {
    [
        s.game_install.clone(),
        s.local_mods.clone(),
        s.workshop.clone(),
        s.config_dir.clone(),
    ]
}

struct ReloadData {
    paths: ScanPaths,
    mods: Vec<Mod>,
    community_rules: RulesDb,
    user_rules: RulesDb,
    steam_db: SteamDb,
    rules_note: String,
}

fn collect_reload(settings: &Settings) -> Result<ReloadData, String> {
    let mut mods = scan_mods(settings).map_err(|e| e.to_string())?;
    let (community_rules, user_rules, steam_db, rules_note) = load_external_metadata();
    rules::apply_rules(&mut mods, &community_rules, &user_rules);
    steamdb::apply_steam_db(&mut mods, &steam_db);
    Ok(ReloadData {
        paths: scan_paths(settings),
        mods,
        community_rules,
        user_rules,
        steam_db,
        rules_note,
    })
}

fn load_rules(path: Option<PathBuf>, label: &str, note: &mut String) -> RulesDb {
    let Some(p) = path else {
        return RulesDb::default();
    };
    match RulesDb::load_path(&p) {
        Ok(db) => {
            if !db.rules.is_empty() {
                note.push_str(&format!("; {} {label} rules", db.rules.len()));
            }
            db
        }
        Err(e) => {
            tracing::warn!("{label} rules: {e}");
            note.push_str(&format!("; {label} rules error: {e}"));
            RulesDb::default()
        }
    }
}

fn load_external_metadata() -> (RulesDb, RulesDb, SteamDb, String) {
    let mut note = String::new();
    let community_rules = load_rules(Settings::community_rules_path(), "community", &mut note);
    let user_rules = load_rules(Settings::user_rules_path(), "user", &mut note);
    let steam_db = match Settings::steam_db_path() {
        Some(p) => {
            let db = SteamDb::load_path(&p);
            if !db.database.is_empty() {
                note.push_str(&format!("; {} steam DB entries", db.database.len()));
            }
            db
        }
        None => SteamDb::default(),
    };
    (community_rules, user_rules, steam_db, note)
}

/// `Some(Ok)` = message, `Some(Err)` = worker hung up, `None` = nothing yet.
fn try_take<T>(rx: &Option<Receiver<T>>) -> Option<Result<T, ()>> {
    match rx.as_ref()?.try_recv() {
        Ok(msg) => Some(Ok(msg)),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err(())),
    }
}

impl Session {
    /// Loads nothing yet; call `start` once the frontend is ready.
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            mods: Vec::new(),
            mod_index: HashMap::new(),
            inactive_paths: Vec::new(),
            active_paths: Vec::new(),
            missing_from_disk: Vec::new(),
            mods_config: ModsConfig::default(),
            issues: Vec::new(),
            dirty: false,
            status: String::new(),
            community_rules: RulesDb::default(),
            user_rules: RulesDb::default(),
            steam_db: SteamDb::default(),
            workshop_items: Vec::new(),
            github_mods: GithubModsDb::load(),
            github_cache: ReleaseCache::load(),
            git_items: Vec::new(),
            fetch_rx: None,
            scan_rx: None,
            scan_keeps_edits: false,
            pending_reload: None,
            scan_note: None,
            steamcmd_succeeded: false,
            workshop_rx: None,
            steamcmd_rx: None,
            pending_steamcmd: Vec::new(),
            pending_activate: HashSet::new(),
            pending_path_activate: HashSet::new(),
            github_rx: None,
            pending_github: Vec::new(),
            github_auto_after_check: false,
            git_rx: None,
            loaded_paths: None,
            pending_git: Vec::new(),
        }
    }

    /// Scan, fetch missing DBs, and run GitHub auto-updates. Returns false on first run
    /// (game path not set), in which case the frontend should show settings.
    pub fn start(&mut self) -> bool {
        if !self.settings.game_install_ok() {
            self.status = "Set RimWorld paths, then save settings".into();
            return false;
        }
        self.reload();
        self.maybe_auto_fetch();
        self.maybe_auto_github();
        true
    }

    pub fn is_busy(&self) -> bool {
        self.fetch_rx.is_some()
            || self.scan_rx.is_some()
            || self.workshop_rx.is_some()
            || self.steamcmd_rx.is_some()
            || self.github_rx.is_some()
            || self.git_rx.is_some()
    }

    pub fn steamcmd_running(&self) -> bool {
        self.steamcmd_rx.is_some()
    }

    pub fn workshop_searching(&self) -> bool {
        self.workshop_rx.is_some()
    }

    /// Drain every worker once. Cheap when idle.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        self.poll_fetch();
        self.poll_scan(&mut events);
        self.poll_workshop(&mut events);
        self.poll_steamcmd();
        self.poll_github(&mut events);
        self.poll_git(&mut events);
        events
    }

    // ---- scan -------------------------------------------------------------

    /// Rescan and reload the mod list from ModsConfig.xml (explicit refresh, path changes).
    pub fn reload(&mut self) {
        self.start_scan(false);
    }

    /// Rescan after mods changed on disk (downloads, updates, DB refresh); keeps unsaved edits.
    fn rescan(&mut self) {
        self.start_scan(true);
    }

    /// Rescan and keep `note` visible: now, and again after the scan's "Loaded ..." line.
    fn rescan_then(&mut self, note: String) {
        self.rescan();
        self.status = note.clone();
        self.scan_note = Some(match self.scan_note.take() {
            Some(prev) => format!("{prev} · {note}"),
            None => note,
        });
    }

    fn start_scan(&mut self, keep_edits: bool) {
        if self.scan_rx.is_some() {
            // One queued scan; it keeps edits only if every request asked for that.
            self.pending_reload = Some(self.pending_reload.unwrap_or(true) && keep_edits);
            return;
        }
        self.scan_keeps_edits = keep_edits;
        let (tx, rx) = mpsc::channel();
        self.status = "Scanning mods...".into();
        let settings = self.settings.clone();
        match std::thread::Builder::new()
            .name("rimtui-scan".into())
            .spawn(move || {
                let _ = tx.send(collect_reload(&settings));
            }) {
            Ok(_) => self.scan_rx = Some(rx),
            Err(e) => self.status = format!("Could not start scan thread: {e}"),
        }
    }

    fn poll_scan(&mut self, events: &mut Vec<Event>) {
        let Some(next) = try_take(&self.scan_rx) else {
            return;
        };
        self.scan_rx = None;
        match next {
            Ok(Ok(data)) => {
                self.apply_reload(data);
                events.push(Event::Reloaded);
            }
            Ok(Err(e)) => {
                error!("scan failed: {e}");
                self.status = format!("Scan failed: {e}");
            }
            Err(()) => {
                self.status = "Scan ended unexpectedly".into();
                return;
            }
        }
        if let Some(keep_edits) = self.pending_reload.take() {
            self.start_scan(keep_edits);
        }
    }

    fn apply_reload(&mut self, data: ReloadData) {
        // Unsaved edits survive a background rescan; mods gone from disk drop out.
        let kept_active = (self.scan_keeps_edits && self.dirty)
            .then(|| std::mem::take(&mut self.active_paths));
        self.mods = data.mods;
        self.community_rules = data.community_rules;
        self.user_rules = data.user_rules;
        self.steam_db = data.steam_db;
        self.loaded_paths = Some(data.paths.clone());
        self.mod_index = self
            .mods
            .iter()
            .enumerate()
            .map(|(i, m)| (m.path.clone(), i))
            .collect();
        // Read here, not in the worker: a Save during the scan must not be undone by a stale read.
        let config_path = crate::xml::mods_config::mods_config_path(&data.paths[3]);
        let config_note = match read_mods_config(&config_path) {
            Ok(cfg) => {
                self.mods_config = cfg;
                String::new()
            }
            Err(e) => {
                self.mods_config = ModsConfig::default();
                format!("; failed to read ModsConfig.xml: {e}")
            }
        };
        match kept_active {
            Some(active) => {
                let active = active
                    .into_iter()
                    .filter(|p| self.mod_index.contains_key(p))
                    .collect();
                self.set_active_paths(active);
            }
            None => {
                self.apply_mods_config();
                self.dirty = false;
            }
        }
        let activated = self.activate_pending();
        self.refresh_issues();
        let extra = if activated > 0 {
            format!("; activated {activated} downloaded")
        } else {
            String::new()
        };
        let note = self
            .scan_note
            .take()
            .map(|n| format!("{n} · "))
            .unwrap_or_default();
        self.status = format!(
            "{note}Loaded {} mods ({} active){}{extra}{config_note}",
            self.mods.len(),
            self.active_paths.len(),
            data.rules_note
        );
        info!(
            mods = self.mods.len(),
            active = self.active_paths.len(),
            "reload complete"
        );
        self.refresh_git_items();
    }

    fn apply_mods_config(&mut self) {
        let mut used = HashSet::new();
        let mut active = Vec::new();
        self.missing_from_disk.clear();
        for id in &self.mods_config.active_mods {
            if let Some(m) = resolve_mod(&self.mods, id) {
                if used.insert(m.path.clone()) {
                    active.push(m.path.clone());
                }
            } else {
                self.missing_from_disk.push(id.clone());
            }
        }
        self.set_active_paths(active);
    }

    /// Set the active order; every other scanned mod becomes inactive.
    fn set_active_paths(&mut self, active: Vec<PathBuf>) {
        let used: HashSet<&PathBuf> = active.iter().collect();
        self.inactive_paths = self
            .mods
            .iter()
            .filter(|m| !used.contains(&m.path))
            .map(|m| m.path.clone())
            .collect();
        self.active_paths = active;
        self.sort_inactive();
    }

    fn sort_inactive(&mut self) {
        let mut paths = std::mem::take(&mut self.inactive_paths);
        paths.sort_by_cached_key(|p| self.mod_ref(p).map(|m| m.name.to_ascii_lowercase()));
        self.inactive_paths = paths;
    }

    // ---- queries ----------------------------------------------------------

    pub fn mod_ref(&self, path: &Path) -> Option<&Mod> {
        self.mod_index.get(path).and_then(|&i| self.mods.get(i))
    }

    pub fn is_active(&self, path: &Path) -> bool {
        self.active_paths.iter().any(|p| p == path)
    }

    pub fn filtered_inactive(&self, query: &str) -> Vec<PathBuf> {
        self.filter(&self.inactive_paths, query)
    }

    pub fn filtered_active(&self, query: &str) -> Vec<PathBuf> {
        self.filter(&self.active_paths, query)
    }

    fn filter(&self, paths: &[PathBuf], query: &str) -> Vec<PathBuf> {
        paths
            .iter()
            .filter(|p| self.mod_ref(p).is_some_and(|m| m.matches_query(query)))
            .cloned()
            .collect()
    }

    pub fn refresh_issues(&mut self) {
        self.issues = check_issues(&self.mods, &self.active_paths, &self.missing_from_disk);
    }

    pub fn workshop_installed(&self, pfid: &str) -> bool {
        self.mods
            .iter()
            .any(|m| m.published_file_id.as_deref() == Some(pfid))
    }

    pub fn published_file_id(&self, path: &Path) -> Option<String> {
        let m = self.mod_ref(path)?;
        if let Some(id) = &m.published_file_id {
            return Some(id.clone());
        }
        self.steam_db
            .pfid_for_package(&m.package_id)
            .map(str::to_string)
    }

    // ---- list edits -------------------------------------------------------

    fn mark_changed(&mut self, status: &str) {
        self.dirty = true;
        self.refresh_issues();
        self.status = status.into();
    }

    /// Move a mod between inactive and active. Activation appends to the end.
    pub fn set_active(&mut self, path: &Path, active: bool) {
        if self.is_active(path) == active || self.mod_ref(path).is_none() {
            return;
        }
        let path = path.to_path_buf();
        if active {
            self.inactive_paths.retain(|p| p != &path);
            self.active_paths.push(path);
        } else {
            self.active_paths.retain(|p| p != &path);
            self.inactive_paths.push(path);
            self.sort_inactive();
        }
        self.mark_changed("List changed (unsaved)");
    }

    pub fn toggle(&mut self, path: &Path) {
        let active = self.is_active(path);
        self.set_active(path, !active);
    }

    /// Deactivate everything except Core and DLC, like RimSort's "Clear".
    pub fn clear_active(&mut self) {
        let keep: Vec<PathBuf> = self
            .active_paths
            .iter()
            .filter(|p| self.mod_ref(p).is_some_and(|m| m.source == ModSource::Dlc))
            .cloned()
            .collect();
        let dropped: Vec<PathBuf> = self
            .active_paths
            .iter()
            .filter(|p| !keep.contains(p))
            .cloned()
            .collect();
        self.active_paths = keep;
        self.inactive_paths.extend(dropped);
        self.sort_inactive();
        self.mark_changed("Cleared active list (unsaved)");
    }

    /// Shift an active mod by `delta` positions (clamped to the list).
    pub fn move_active(&mut self, path: &Path, delta: isize) {
        let Some(idx) = self.active_paths.iter().position(|p| p == path) else {
            return;
        };
        let last = self.active_paths.len() as isize - 1;
        let to = (idx as isize + delta).clamp(0, last) as usize;
        if to == idx {
            return;
        }
        let item = self.active_paths.remove(idx);
        self.active_paths.insert(to, item);
        self.mark_changed("Reordered (unsaved)");
    }

    pub fn sort(&mut self) {
        let result = match self.settings.sort_mode {
            SortMode::Topological => sort_active(&self.mods, &self.active_paths),
            SortMode::Alphabetical => Ok(sort_active_alphabetical(&self.mods, &self.active_paths)),
        };
        match result {
            Ok(sorted) => {
                self.active_paths = sorted;
                let msg = format!("Sorted ({})", self.settings.sort_mode.label());
                self.mark_changed(&msg);
            }
            Err(e) => {
                self.status = format!("Sort failed: {e}");
                self.refresh_issues();
            }
        }
    }

    pub fn set_sort_mode(&mut self, mode: SortMode) {
        self.settings.sort_mode = mode;
        self.status = match self.settings.save() {
            Ok(_) => format!("Sort mode: {}", mode.label()),
            Err(e) => format!("Sort mode: {} (could not save settings: {e})", mode.label()),
        };
    }

    pub fn save_mods_config(&mut self) {
        // Until a scan of the current paths lands, the list is empty or from another install;
        // writing it would wipe that install's ModsConfig.xml.
        if self.mods.is_empty() || self.loaded_paths != Some(scan_paths(&self.settings)) {
            self.status = "Not saved: mod list is not loaded for the current paths (rescan)".into();
            return;
        }
        self.mods_config = self.current_list_config();
        let path = self.settings.mods_config_path();
        match write_mods_config(&path, &self.mods_config) {
            Ok(_) => {
                self.dirty = false;
                // The written file only has resolvable IDs, so the "missing" warnings are gone.
                self.missing_from_disk.clear();
                self.refresh_issues();
                self.status = format!("Wrote {}", path.display());
                info!("wrote {}", path.display());
            }
            Err(e) => {
                self.status = format!("Save failed: {e}");
                error!("save failed: {e}");
            }
        }
    }

    pub fn current_list_config(&self) -> ModsConfig {
        let active_mods = self
            .active_paths
            .iter()
            .filter_map(|p| self.mod_ref(p).map(|m| m.package_id.clone()))
            .collect();
        let known_expansions = self
            .mods
            .iter()
            .filter(|m| m.source == ModSource::Dlc && is_ludeon_package(&m.package_id))
            .filter(|m| m.package_id != "ludeon.rimworld")
            .map(|m| m.package_id.clone())
            .collect();
        ModsConfig {
            version: self.mods_config.version.clone(),
            active_mods,
            known_expansions,
        }
    }

    /// Replace the active list from a ModsConfig-shaped XML file. Returns true on success.
    pub fn load_list(&mut self, path: &Path) -> bool {
        match lists::import_list(path) {
            Ok(cfg) => {
                if let Some(version) = cfg.version {
                    self.mods_config.version = Some(version);
                }
                self.mods_config.active_mods = cfg.active_mods;
                self.apply_mods_config();
                let msg = format!(
                    "Loaded {} active mods from {} (unsaved)",
                    self.active_paths.len(),
                    path.display()
                );
                self.mark_changed(&msg);
                true
            }
            Err(e) => {
                self.status = format!("Load failed: {e}");
                false
            }
        }
    }

    pub fn export_list(&mut self, path: &Path) {
        self.status = match lists::export_list(path, &self.current_list_config()) {
            Ok(written) => format!("Exported {}", written.display()),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    pub fn launch(&mut self) {
        let dirty_note = if self.dirty {
            " (mod list not saved)"
        } else {
            ""
        };
        self.status = match launch::launch_game(&self.settings.game_install) {
            Ok(LaunchKind::Executable(exe)) => format!("Launched {}{dirty_note}", exe.display()),
            Ok(LaunchKind::SteamProtocol) => {
                format!("Launched {}{dirty_note}", launch::STEAM_RUN_URL)
            }
            Err(e) => format!("Launch failed: {e}"),
        };
    }

    /// Save settings to disk and rescan. Returns true on success.
    pub fn save_settings(&mut self) -> bool {
        match self.settings.save() {
            Ok(path) => {
                self.reload();
                self.maybe_auto_fetch();
                self.maybe_auto_github();
                self.status = format!("Saved settings to {}", path.display());
                true
            }
            Err(e) => {
                self.status = format!("Could not save settings: {e}");
                false
            }
        }
    }

    // ---- user rules -------------------------------------------------------

    fn persist_user_rules(&mut self) -> bool {
        let Some(path) = Settings::user_rules_path() else {
            self.status = "Could not determine config directory for userRules.json".into();
            return false;
        };
        match self.user_rules.save_path(&path) {
            Ok(()) => true,
            Err(e) => {
                self.status = format!("Could not save userRules.json: {e}");
                false
            }
        }
    }

    fn refresh_load_flags(&mut self) {
        for m in &mut self.mods {
            let pid = m.package_id.as_str();
            m.load_first = self.community_rules.load_top(pid) || self.user_rules.load_top(pid);
            m.load_last = self.community_rules.load_bottom(pid) || self.user_rules.load_bottom(pid);
        }
    }

    pub fn toggle_user_load_top(&mut self, package_id: &str) {
        let next = !self.user_rules.load_top(package_id);
        self.user_rules.set_load_top(package_id, next);
        if !self.persist_user_rules() {
            return;
        }
        self.refresh_load_flags();
        let community = if self.community_rules.load_top(package_id) {
            " (community loadTop still on)"
        } else {
            ""
        };
        self.status = format!(
            "user loadTop {} for {package_id}{community}",
            if next { "on" } else { "off" }
        );
    }

    pub fn toggle_user_load_bottom(&mut self, package_id: &str) {
        let next = !self.user_rules.load_bottom(package_id);
        self.user_rules.set_load_bottom(package_id, next);
        if !self.persist_user_rules() {
            return;
        }
        self.refresh_load_flags();
        let community = if self.community_rules.load_bottom(package_id) {
            " (community loadBottom still on)"
        } else {
            ""
        };
        self.status = format!(
            "user loadBottom {} for {package_id}{community}",
            if next { "on" } else { "off" }
        );
    }

    // ---- database fetch ---------------------------------------------------

    fn maybe_auto_fetch(&mut self) {
        let community_missing = Settings::community_rules_path().is_none_or(|p| !p.exists());
        let steam_missing = Settings::steam_db_path().is_none_or(|p| !p.exists());
        if community_missing || steam_missing {
            self.start_fetch(true);
        }
    }

    pub fn start_fetch(&mut self, only_missing: bool) {
        if self.fetch_rx.is_some() {
            self.status = "Already downloading databases".into();
            return;
        }
        let (Some(community_dest), Some(steam_dest), Some(cache_path)) = (
            Settings::community_rules_path(),
            Settings::steam_db_path(),
            Settings::http_cache_path(),
        ) else {
            self.status = "Could not determine config directory for databases".into();
            return;
        };
        if self.settings.community_rules_url.trim().is_empty()
            && self.settings.steam_db_url.trim().is_empty()
        {
            self.status = "No database URLs configured (see settings)".into();
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.fetch_rx = Some(rx);
        self.status = if only_missing {
            "Downloading missing databases...".into()
        } else {
            "Downloading databases...".into()
        };
        fetch::spawn(
            FetchRequest {
                community_url: self.settings.community_rules_url.clone(),
                steam_url: self.settings.steam_db_url.clone(),
                community_dest,
                steam_dest,
                cache_path,
                only_missing,
            },
            tx,
        );
    }

    fn poll_fetch(&mut self) {
        match try_take(&self.fetch_rx) {
            Some(Ok(FetchMsg::Progress(msg))) => self.status = msg,
            Some(Ok(FetchMsg::Done(report))) => {
                self.fetch_rx = None;
                if report.any_updated() {
                    self.rescan_then(report.status_line());
                } else {
                    self.status = report.status_line();
                }
            }
            Some(Err(())) => {
                self.fetch_rx = None;
                self.status = "Database download ended unexpectedly".into();
            }
            None => {}
        }
    }

    // ---- workshop + SteamCMD ----------------------------------------------

    pub fn search_workshop(&mut self, query: &str) {
        if self.workshop_rx.is_some() {
            self.status = "Already searching the workshop".into();
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.workshop_rx = Some(rx);
        self.status = "Searching Steam Workshop...".into();
        workshop::spawn(
            workshop::SearchRequest {
                api_key: self.settings.resolved_steam_api_key(),
                query: query.to_string(),
            },
            tx,
        );
    }

    fn poll_workshop(&mut self, events: &mut Vec<Event>) {
        let Some(next) = try_take(&self.workshop_rx) else {
            return;
        };
        self.workshop_rx = None;
        match next {
            Ok(WorkshopMsg::Done(Ok(items))) => {
                self.workshop_items = items;
                self.status = if self.workshop_items.is_empty() {
                    "No workshop results".into()
                } else {
                    format!("{} workshop result(s)", self.workshop_items.len())
                };
                events.push(Event::WorkshopResults);
            }
            Ok(WorkshopMsg::Done(Err(e))) => self.status = format!("Workshop search failed: {e}"),
            Err(()) => self.status = "Workshop search ended unexpectedly".into(),
        }
    }

    /// Queue a SteamCMD download; `activate` enables the mod after the next rescan.
    pub fn download_workshop(&mut self, published_file_id: &str, title: &str, activate: bool) {
        let already = self.steamcmd_running();
        self.enqueue_steamcmd(vec![SteamcmdItem {
            published_file_id: published_file_id.to_string(),
            activate,
        }]);
        if already {
            self.status = if activate {
                format!("Queued {title} (will activate after download)")
            } else {
                format!("Queued {title}")
            };
        }
    }

    /// Update one mod from wherever it came from: GitHub release, git clone, or SteamCMD.
    pub fn update_mod(&mut self, path: &Path) {
        if let Some(entry) = self.github_mods.by_path(path).cloned() {
            self.enqueue_github(GithubJob::Update {
                items: vec![GithubUpdateItem {
                    owner_repo: entry.owner_repo,
                    mod_path: entry.mod_path,
                    tag: None,
                }],
            });
            return;
        }
        if self
            .mod_ref(path)
            .is_some_and(|m| m.source == ModSource::Local && gitmod::is_git_repo(&m.path))
        {
            self.git_pull(path);
            return;
        }
        let Some(pfid) = self.published_file_id(path) else {
            self.status =
                "Selected mod has no published file ID, GitHub tracking, or git clone".into();
            return;
        };
        let name = self
            .mod_ref(path)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| pfid.clone());
        let already = self.steamcmd_running();
        self.enqueue_steamcmd(vec![SteamcmdItem {
            published_file_id: pfid,
            activate: false,
        }]);
        if already {
            self.status = format!("Queued SteamCMD update for {name}");
        }
    }

    fn enqueue_steamcmd(&mut self, items: Vec<SteamcmdItem>) {
        for item in items.iter().filter(|i| i.activate) {
            self.pending_activate.insert(item.published_file_id.clone());
        }
        if self.steamcmd_running() {
            self.pending_steamcmd.extend(items);
        } else {
            self.start_steamcmd(items);
        }
    }

    fn start_steamcmd(&mut self, items: Vec<SteamcmdItem>) {
        if items.is_empty() {
            return;
        }
        let Some(install_dir) = Settings::steamcmd_dir() else {
            self.status = "Could not determine config directory for SteamCMD".into();
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.steamcmd_rx = Some(rx);
        self.status = format!("SteamCMD: downloading {} mod(s)...", items.len());
        steamcmd::spawn(
            SteamcmdRequest {
                configured_exe: self.settings.steamcmd_path.clone(),
                workshop: self.settings.workshop.clone(),
                install_dir,
                items,
            },
            tx,
        );
    }

    fn poll_steamcmd(&mut self) {
        match try_take(&self.steamcmd_rx) {
            Some(Ok(SteamcmdMsg::Progress(msg))) => self.status = msg,
            Some(Ok(SteamcmdMsg::Done(report))) => {
                self.steamcmd_rx = None;
                for (id, _) in &report.failed {
                    let retry_queued = self
                        .pending_steamcmd
                        .iter()
                        .any(|i| &i.published_file_id == id);
                    if !retry_queued {
                        self.pending_activate.remove(id);
                    }
                }
                self.steamcmd_succeeded |= report.any_success();
                self.status = report.status_line();
                if !self.pending_steamcmd.is_empty() {
                    let more = std::mem::take(&mut self.pending_steamcmd);
                    self.start_steamcmd(more);
                } else if std::mem::take(&mut self.steamcmd_succeeded) {
                    self.rescan_then(report.status_line());
                }
            }
            Some(Err(())) => {
                self.steamcmd_rx = None;
                self.status = "SteamCMD ended unexpectedly".into();
            }
            None => {}
        }
    }

    /// Move freshly downloaded/cloned mods to the active list once a scan finds them.
    fn activate_pending(&mut self) -> usize {
        let mut targets = Vec::new();
        let pfids = std::mem::take(&mut self.pending_activate);
        for id in pfids {
            match self
                .mods
                .iter()
                .find(|m| m.published_file_id.as_deref() == Some(id.as_str()))
            {
                Some(m) => targets.push(m.path.clone()),
                None => {
                    self.pending_activate.insert(id);
                }
            }
        }
        let paths = std::mem::take(&mut self.pending_path_activate);
        for path in paths {
            if self.mod_index.contains_key(&path) {
                targets.push(path);
            } else {
                self.pending_path_activate.insert(path);
            }
        }
        let mut activated = 0;
        for path in targets {
            if self.is_active(&path) {
                continue;
            }
            self.inactive_paths.retain(|p| p != &path);
            self.active_paths.push(path);
            activated += 1;
        }
        if activated > 0 {
            self.dirty = true;
        }
        activated
    }

    // ---- GitHub release mods ----------------------------------------------

    pub fn github_latest_tag(&self, owner_repo: &str) -> Option<String> {
        self.github_cache.latest_stable(owner_repo).map(|r| r.tag)
    }

    pub fn github_update_tag(&self, entry: &GithubModEntry) -> Option<String> {
        let releases = self.github_cache.releases_for(&entry.owner_repo);
        github::update_for_entry(entry, &releases).map(|u| u.latest_version)
    }

    fn persist_github_mods(&mut self) -> bool {
        match self.github_mods.save() {
            Ok(_) => true,
            Err(e) => {
                self.status = format!("Could not save github_mods.json: {e}");
                false
            }
        }
    }

    fn maybe_auto_github(&mut self) {
        if self.github_mods.mods.iter().any(|m| m.auto_update) {
            self.github_auto_after_check = true;
            self.github_check(false);
        }
    }

    pub fn reload_github_cache(&mut self) {
        self.github_cache = ReleaseCache::load();
    }

    /// Install `owner/repo` (or a github.com URL) into the local Mods folder and activate it.
    pub fn github_install(&mut self, query: &str) {
        let Some(parsed) = github::parse_github_ref(query) else {
            self.status = "GitHub: enter owner/repo or a github.com URL".into();
            return;
        };
        let owner_repo = parsed.owner_repo();
        match github::install_dest(&self.settings.local_mods, &owner_repo) {
            Ok(dest) => self.enqueue_github(GithubJob::Install {
                owner_repo,
                tag: parsed.tag,
                dest,
                activate: true,
            }),
            Err(e) => self.status = format!("GitHub: {e}"),
        }
    }

    pub fn github_check(&mut self, force: bool) {
        if self.github_mods.mods.is_empty() {
            self.status = "GitHub: no tracked mods".into();
            self.github_auto_after_check = false;
            return;
        }
        self.enqueue_github(GithubJob::Check {
            entries: self.github_mods.mods.clone(),
            force,
        });
    }

    pub fn github_update(&mut self, mod_path: &Path) {
        let Some(entry) = self.github_mods.by_path(mod_path).cloned() else {
            self.status = "GitHub: no tracked mod selected".into();
            return;
        };
        self.enqueue_github(GithubJob::Update {
            items: vec![GithubUpdateItem {
                owner_repo: entry.owner_repo,
                mod_path: entry.mod_path,
                tag: None,
            }],
        });
    }

    pub fn github_toggle_auto_update(&mut self, mod_path: &Path) {
        let Some(entry) = self.github_mods.by_path_mut(mod_path) else {
            self.status = "GitHub: no tracked mod selected".into();
            return;
        };
        entry.auto_update = !entry.auto_update;
        let on = entry.auto_update;
        let repo = entry.owner_repo.clone();
        if self.persist_github_mods() {
            self.status = format!(
                "GitHub: auto-update {} for {repo}",
                if on { "on" } else { "off" }
            );
        }
    }

    /// Delete the mod folder and stop tracking it.
    pub fn github_uninstall(&mut self, mod_path: &Path) {
        let Some(entry) = self.github_mods.by_path(mod_path).cloned() else {
            self.status = "GitHub: no tracked mod selected".into();
            return;
        };
        if entry.mod_path.exists() {
            if let Err(e) = std::fs::remove_dir_all(&entry.mod_path) {
                self.status = format!("GitHub: could not delete {}: {e}", entry.mod_path.display());
                return;
            }
        }
        self.github_mods.remove_path(&entry.mod_path);
        if self.persist_github_mods() {
            self.rescan_then(format!("GitHub: deleted {}", entry.owner_repo));
        }
    }

    fn enqueue_github(&mut self, job: GithubJob) {
        if self.github_rx.is_some() {
            self.pending_github.push(job);
            self.status = "GitHub: queued".into();
        } else {
            self.start_github(job);
        }
    }

    fn start_github(&mut self, job: GithubJob) {
        let Some(cache_path) = Settings::github_cache_path() else {
            self.status = "Could not determine config directory for GitHub cache".into();
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.github_rx = Some(rx);
        self.status = match &job {
            GithubJob::Install { owner_repo, .. } => format!("GitHub: installing {owner_repo}..."),
            GithubJob::Update { items } => format!("GitHub: updating {} mod(s)...", items.len()),
            GithubJob::Check { .. } => "GitHub: checking for updates...".into(),
        };
        github::spawn(
            GithubRequest {
                token: self.settings.resolved_github_token(),
                cache_path,
                job,
            },
            tx,
        );
    }

    fn poll_github(&mut self, events: &mut Vec<Event>) {
        match try_take(&self.github_rx) {
            Some(Ok(GithubMsg::Progress(msg))) => self.status = msg,
            Some(Ok(GithubMsg::Done(report))) => {
                self.github_rx = None;
                self.apply_github_report(report, events);
                // The report may have started an auto-update; don't replace its receiver.
                if self.github_rx.is_none() && !self.pending_github.is_empty() {
                    let more = self.pending_github.remove(0);
                    self.start_github(more);
                }
            }
            Some(Err(())) => {
                self.github_rx = None;
                self.github_auto_after_check = false;
                self.status = "GitHub job ended unexpectedly".into();
            }
            None => {}
        }
    }

    fn apply_github_report(&mut self, report: GithubReport, events: &mut Vec<Event>) {
        self.github_cache = ReleaseCache::load();
        match report {
            GithubReport::Installed {
                owner_repo,
                version,
                asset_name,
                path,
                activate,
                warning,
            } => {
                self.github_mods.upsert(GithubModEntry {
                    owner_repo: owner_repo.clone(),
                    mod_path: path.clone(),
                    installed_version: version.clone(),
                    installed_asset_name: Some(asset_name),
                    auto_update: false,
                });
                let _ = self.persist_github_mods();
                if activate {
                    self.pending_path_activate.insert(path);
                }
                let warn = warning.map(|w| format!(" ({w})")).unwrap_or_default();
                self.rescan_then(format!("GitHub: installed {owner_repo} @{version}{warn}"));
                events.push(Event::GithubInstalled);
            }
            GithubReport::Updated { succeeded, failed } => {
                for item in &succeeded {
                    if let Some(entry) = self.github_mods.by_path_mut(&item.path) {
                        entry.installed_version = item.version.clone();
                        entry.installed_asset_name = Some(item.asset_name.clone());
                    }
                }
                let _ = self.persist_github_mods();
                let any_success = !succeeded.is_empty();
                let line = GithubReport::Updated { succeeded, failed }.status_line();
                if any_success {
                    self.rescan_then(line);
                } else {
                    self.status = line;
                }
            }
            GithubReport::Checked { updates, errors } => {
                let auto: Vec<GithubUpdateItem> = updates
                    .iter()
                    .filter(|u| u.auto_update)
                    .map(|u| GithubUpdateItem {
                        owner_repo: u.owner_repo.clone(),
                        mod_path: u.mod_path.clone(),
                        tag: Some(u.latest_version.clone()),
                    })
                    .collect();
                self.status = if !errors.is_empty() && !updates.is_empty() {
                    format!(
                        "GitHub: {} update(s) available, {} error(s)",
                        updates.len(),
                        errors.len()
                    )
                } else {
                    GithubReport::Checked { updates, errors }.status_line()
                };
                if std::mem::take(&mut self.github_auto_after_check) && !auto.is_empty() {
                    self.enqueue_github(GithubJob::Update { items: auto });
                }
            }
            GithubReport::Failed(e) => {
                self.github_auto_after_check = false;
                self.status = format!("GitHub: {e}");
            }
        }
    }

    // ---- git clones -------------------------------------------------------

    pub fn refresh_git_items(&mut self) {
        let tracked: Vec<PathBuf> = self
            .github_mods
            .mods
            .iter()
            .map(|e| e.mod_path.clone())
            .collect();
        let previous: HashMap<PathBuf, GitModInfo> = self
            .git_items
            .drain(..)
            .map(|i| (i.path.clone(), i))
            .collect();
        self.git_items = gitmod::discover_git_mods(&self.mods, &tracked);
        for item in &mut self.git_items {
            if let Some(old) = previous.get(&item.path) {
                if old.short_sha == item.short_sha {
                    item.behind = old.behind;
                    item.error = old.error.clone();
                }
            }
        }
    }

    pub fn git_clone(&mut self, query: &str) {
        let Some(parsed) = gitmod::parse_git_url(query) else {
            self.status = "Git: enter an https:// or git@ clone URL".into();
            return;
        };
        match gitmod::clone_dest(&self.settings.local_mods, &parsed) {
            Ok(dest) => self.enqueue_git(GitJob::Clone {
                clone_url: parsed.clone_url,
                branch: parsed.branch,
                dest,
                activate: true,
            }),
            Err(e) => self.status = format!("Git: {e}"),
        }
    }

    pub fn git_check(&mut self) {
        // Detached checkouts have no branch to compare against; skip them.
        let paths: Vec<PathBuf> = self
            .git_items
            .iter()
            .filter(|i| !i.is_detached())
            .map(|i| i.path.clone())
            .collect();
        if paths.is_empty() {
            self.status = "Git: no cloned mods on a branch to check".into();
            return;
        }
        self.enqueue_git(GitJob::Check { paths });
    }

    pub fn git_pull(&mut self, path: &Path) {
        if let Some(item) = self.git_items.iter().find(|i| i.path == path && i.is_detached()) {
            self.status = format!("Git: {} is detached (not on a branch); not pulled", item.name);
            return;
        }
        self.enqueue_git(GitJob::Pull {
            paths: vec![path.to_path_buf()],
        });
    }

    pub fn git_pull_updates(&mut self) {
        let paths: Vec<PathBuf> = self
            .git_items
            .iter()
            .filter(|i| i.has_update())
            .map(|i| i.path.clone())
            .collect();
        if paths.is_empty() {
            self.status = "Git: nothing with updates (check first)".into();
            return;
        }
        self.enqueue_git(GitJob::Pull { paths });
    }

    /// Delete a cloned mod folder.
    pub fn git_uninstall(&mut self, path: &Path) {
        let Some(item) = self.git_items.iter().find(|i| i.path == path).cloned() else {
            self.status = "Git: no cloned mod selected".into();
            return;
        };
        if item.path.exists() {
            if let Err(e) = std::fs::remove_dir_all(&item.path) {
                self.status = format!("Git: could not delete {}: {e}", item.path.display());
                return;
            }
        }
        self.git_items.retain(|i| i.path != item.path);
        self.rescan_then(format!("Git: deleted {}", item.name));
    }

    fn enqueue_git(&mut self, job: GitJob) {
        if self.git_rx.is_some() {
            self.pending_git.push(job);
            self.status = "Git: queued".into();
        } else {
            self.start_git(job);
        }
    }

    fn start_git(&mut self, job: GitJob) {
        let (tx, rx) = mpsc::channel();
        self.git_rx = Some(rx);
        self.status = match &job {
            GitJob::Clone { clone_url, .. } => format!("Git: cloning {clone_url}..."),
            GitJob::Check { paths } => format!("Git: checking {} repo(s)...", paths.len()),
            GitJob::Pull { paths } => format!("Git: pulling {} repo(s)...", paths.len()),
        };
        gitmod::spawn(job, tx);
    }

    fn poll_git(&mut self, events: &mut Vec<Event>) {
        match try_take(&self.git_rx) {
            Some(Ok(GitMsg::Progress(msg))) => self.status = msg,
            Some(Ok(GitMsg::Done(report))) => {
                self.git_rx = None;
                self.apply_git_report(report, events);
                if !self.pending_git.is_empty() {
                    let more = self.pending_git.remove(0);
                    self.start_git(more);
                }
            }
            Some(Err(())) => {
                self.git_rx = None;
                self.status = "Git job ended unexpectedly".into();
            }
            None => {}
        }
    }

    fn apply_git_report(&mut self, report: GitReport, events: &mut Vec<Event>) {
        match report {
            GitReport::Cloned {
                dest,
                origin,
                short_sha,
                activate,
                warning,
            } => {
                if activate {
                    self.pending_path_activate.insert(dest);
                }
                let warn = warning.map(|w| format!(" ({w})")).unwrap_or_default();
                self.rescan_then(format!("Git: cloned {origin} @{short_sha}{warn}"));
                events.push(Event::GitCloned);
            }
            GitReport::Checked { items } => {
                for result in &items {
                    if let Some(existing) = self.git_items.iter_mut().find(|i| i.path == result.path)
                    {
                        existing.origin = result.origin.clone();
                        existing.branch = result.branch.clone();
                        existing.short_sha = result.short_sha.clone();
                        existing.behind = result.behind;
                        existing.error = result.error.clone();
                    }
                }
                self.status = GitReport::Checked { items }.status_line();
            }
            GitReport::Pulled { succeeded, failed } => {
                let any = !succeeded.is_empty();
                let line = GitReport::Pulled { succeeded, failed }.status_line();
                if any {
                    self.rescan_then(line);
                } else {
                    self.status = line;
                }
            }
            GitReport::Failed(e) => self.status = format!("Git: {e}"),
        }
    }
}

/// Log to `<config>/rimtui.log` (or nowhere). Frontends call this once at startup.
pub fn init_tracing() {
    let env = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    if let Some(path) = Settings::log_file_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(file) = std::fs::File::create(&path) {
            tracing_subscriber::fmt()
                .with_writer(std::sync::Mutex::new(file))
                .with_env_filter(env)
                .with_ansi(false)
                .init();
            return;
        }
    }
    tracing_subscriber::fmt()
        .with_writer(std::io::sink)
        .with_env_filter(env)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_session() -> Session {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let settings = Settings {
            game_install: root.join("game"),
            local_mods: root.join("game/Mods"),
            workshop: root.join("workshop"),
            config_dir: root.join("game/Config"),
            ..Settings::default()
        };
        let mut s = Session::new(settings);
        let data = collect_reload(&s.settings).unwrap();
        s.apply_reload(data);
        s
    }

    #[test]
    fn toggle_and_move_keep_lists_disjoint() {
        let mut s = fixture_session();
        let total = s.mods.len();
        assert_eq!(s.active_paths.len() + s.inactive_paths.len(), total);
        let p = s.inactive_paths[0].clone();
        s.toggle(&p);
        assert!(s.is_active(&p));
        assert_eq!(s.active_paths.last(), Some(&p));
        assert!(s.dirty);
        s.move_active(&p, -100);
        assert_eq!(s.active_paths.first(), Some(&p));
        s.toggle(&p);
        assert!(!s.is_active(&p));
        assert_eq!(s.active_paths.len() + s.inactive_paths.len(), total);
    }

    #[test]
    fn save_before_scan_keeps_existing_mods_config() {
        let dir = std::env::temp_dir().join(format!("rimtui-save-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = Settings {
            config_dir: dir.clone(),
            ..Settings::default()
        };
        let path = settings.mods_config_path();
        std::fs::write(&path, "original").unwrap();
        let mut s = Session::new(settings);
        s.save_mods_config();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_refused_after_paths_change_until_rescan() {
        let mut s = fixture_session();
        let dir = std::env::temp_dir().join(format!("rimtui-path-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        s.settings.config_dir = dir.clone();
        let path = s.settings.mods_config_path();
        std::fs::write(&path, "original").unwrap();
        s.save_mods_config();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_result_uses_mods_config_saved_during_scan() {
        let mut s = fixture_session();
        let dir = std::env::temp_dir().join(format!("rimtui-scan-race-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        s.settings.config_dir = dir.clone();
        let path = s.settings.mods_config_path();
        let write = |ids: &[&str]| {
            let cfg = ModsConfig {
                active_mods: ids.iter().map(|i| i.to_string()).collect(),
                ..ModsConfig::default()
            };
            write_mods_config(&path, &cfg).unwrap();
        };
        write(&["ludeon.rimworld", "brrainz.harmony"]);
        let data = collect_reload(&s.settings).unwrap();
        // Save lands while the scan is still in flight.
        write(&["ludeon.rimworld"]);
        s.apply_reload(data);
        assert_eq!(s.active_paths.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn background_rescan_keeps_unsaved_edits_but_reload_does_not() {
        let mut s = fixture_session();
        let from_disk = s.active_paths.clone();
        let p = s.inactive_paths[0].clone();
        s.toggle(&p);

        s.scan_keeps_edits = true;
        s.apply_reload(collect_reload(&s.settings).unwrap());
        assert!(s.is_active(&p));
        assert!(s.dirty);
        assert_eq!(s.active_paths.len() + s.inactive_paths.len(), s.mods.len());

        s.scan_keeps_edits = false;
        s.apply_reload(collect_reload(&s.settings).unwrap());
        assert_eq!(s.active_paths, from_disk);
        assert!(!s.dirty);
    }

    #[test]
    fn detached_git_mod_is_not_pulled() {
        let mut s = Session::new(Settings::default());
        let path = PathBuf::from("Mods/detached");
        s.git_items.push(GitModInfo {
            path: path.clone(),
            name: "Detached".into(),
            origin: String::new(),
            branch: "HEAD".into(),
            short_sha: "abc1234".into(),
            behind: None,
            error: None,
        });
        s.git_pull(&path);
        assert!(!s.is_busy());
        assert!(s.status.contains("detached"));
    }

    #[test]
    fn job_note_survives_the_follow_up_scan() {
        let mut s = fixture_session();
        s.scan_note = Some("DBs: community updated · steam failed (timeout)".into());
        s.apply_reload(collect_reload(&s.settings).unwrap());
        assert!(s.status.starts_with("DBs: community updated · steam failed (timeout) · Loaded"));
        assert!(s.scan_note.is_none());
    }

    #[test]
    fn save_clears_missing_id_warnings() {
        let mut s = fixture_session();
        let dir = std::env::temp_dir().join(format!("rimtui-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        s.settings.config_dir = dir.clone();
        let cfg = ModsConfig {
            active_mods: vec!["ludeon.rimworld".into(), "ghost.notinstalled".into()],
            ..ModsConfig::default()
        };
        write_mods_config(&s.settings.mods_config_path(), &cfg).unwrap();
        s.apply_reload(collect_reload(&s.settings).unwrap());
        assert_eq!(s.missing_from_disk, vec!["ghost.notinstalled".to_string()]);
        s.save_mods_config();
        assert!(s.missing_from_disk.is_empty());
        assert!(!s.issues.iter().any(|i| i.message.contains("ghost.notinstalled")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_active_keeps_dlc_only() {
        let mut s = fixture_session();
        for p in s.inactive_paths.clone() {
            s.set_active(&p, true);
        }
        s.clear_active();
        assert!(s
            .active_paths
            .iter()
            .all(|p| s.mod_ref(p).unwrap().source == ModSource::Dlc));
        assert_eq!(s.active_paths.len() + s.inactive_paths.len(), s.mods.len());
    }
}
