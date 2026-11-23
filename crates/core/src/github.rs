use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::error::{Error, Result};
use crate::settings::Settings;

const USER_AGENT: &str = concat!("rimdeck/", env!("CARGO_PKG_VERSION"));
const TIMEOUT_SECS: u64 = 120;
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
const API_ACCEPT: &str = "application/vnd.github+json";
const API_VERSION: &str = "2022-11-28";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRef {
    pub owner: String,
    pub repo: String,
    pub tag: Option<String>,
}

impl GithubRef {
    pub fn owner_repo(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag: String,
    pub name: String,
    pub published_at: String,
    pub prerelease: bool,
    pub zipball_url: String,
    pub assets: Vec<ReleaseAsset>,
    pub body: String,
}

impl ReleaseInfo {
    pub fn custom_zip_assets(&self) -> Vec<&ReleaseAsset> {
        self.assets
            .iter()
            .filter(|a| {
                let name = a.name.to_ascii_lowercase();
                name.ends_with(".zip") && !name.starts_with("source code")
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubModEntry {
    pub owner_repo: String,
    pub mod_path: PathBuf,
    #[serde(default)]
    pub installed_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_asset_name: Option<String>,
    #[serde(default)]
    pub auto_update: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GithubModsDb {
    #[serde(default)]
    pub mods: Vec<GithubModEntry>,
}

impl GithubModsDb {
    pub fn load() -> Self {
        let Some(path) = Settings::github_mods_path() else {
            return Self::default();
        };
        Self::load_path(&path)
    }

    pub fn load_path(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Settings::github_mods_path()
            .ok_or_else(|| Error::Github("could not determine config directory".into()))?;
        self.save_path(&path)?;
        Ok(path)
    }

    pub fn save_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text =
            serde_json::to_string_pretty(self).map_err(|e| Error::Github(e.to_string()))?;
        atomic_write(path, text.as_bytes())?;
        Ok(())
    }

    pub fn by_path(&self, path: &Path) -> Option<&GithubModEntry> {
        self.mods.iter().find(|m| m.mod_path == path)
    }

    pub fn by_path_mut(&mut self, path: &Path) -> Option<&mut GithubModEntry> {
        self.mods.iter_mut().find(|m| m.mod_path == path)
    }

    pub fn upsert(&mut self, entry: GithubModEntry) {
        if let Some(existing) = self
            .mods
            .iter_mut()
            .find(|m| m.mod_path == entry.mod_path)
        {
            let auto_update = existing.auto_update;
            *existing = entry;
            existing.auto_update = auto_update;
            return;
        }
        self.mods.push(entry);
    }

    pub fn remove_path(&mut self, path: &Path) -> Option<GithubModEntry> {
        let idx = self.mods.iter().position(|m| m.mod_path == path)?;
        Some(self.mods.remove(idx))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReleaseCache {
    #[serde(flatten)]
    pub entries: HashMap<String, CachedReleases>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedReleases {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    #[serde(default)]
    pub last_checked_unix: u64,
    #[serde(default)]
    pub releases: Vec<CachedRelease>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedRelease {
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub published_at: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub zipball_url: String,
    #[serde(default)]
    pub assets: Vec<CachedAsset>,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedAsset {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub download_url: String,
    #[serde(default)]
    pub size: u64,
}

impl ReleaseCache {
    pub fn load() -> Self {
        let Some(path) = Settings::github_cache_path() else {
            return Self::default();
        };
        Self::load_path(&path)
    }

    pub fn load_path(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text =
            serde_json::to_string_pretty(self).map_err(|e| Error::Github(e.to_string()))?;
        atomic_write(path, text.as_bytes())?;
        Ok(())
    }

    pub fn releases_for(&self, owner_repo: &str) -> Vec<ReleaseInfo> {
        self.entries
            .get(owner_repo)
            .map(|c| c.releases.iter().map(ReleaseInfo::from).collect())
            .unwrap_or_default()
    }

    pub fn latest_stable(&self, owner_repo: &str) -> Option<ReleaseInfo> {
        let releases = self.releases_for(owner_repo);
        latest_stable_release(&releases).cloned()
    }
}

impl From<&CachedRelease> for ReleaseInfo {
    fn from(r: &CachedRelease) -> Self {
        Self {
            tag: r.tag.clone(),
            name: r.name.clone(),
            published_at: r.published_at.clone(),
            prerelease: r.prerelease,
            zipball_url: r.zipball_url.clone(),
            assets: r
                .assets
                .iter()
                .map(|a| ReleaseAsset {
                    name: a.name.clone(),
                    download_url: a.download_url.clone(),
                    size: a.size,
                })
                .collect(),
            body: r.body.clone(),
        }
    }
}

impl From<&ReleaseInfo> for CachedRelease {
    fn from(r: &ReleaseInfo) -> Self {
        Self {
            tag: r.tag.clone(),
            name: r.name.clone(),
            published_at: r.published_at.clone(),
            prerelease: r.prerelease,
            zipball_url: r.zipball_url.clone(),
            assets: r
                .assets
                .iter()
                .map(|a| CachedAsset {
                    name: a.name.clone(),
                    download_url: a.download_url.clone(),
                    size: a.size,
                })
                .collect(),
            body: r.body.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAvailable {
    pub owner_repo: String,
    pub mod_path: PathBuf,
    pub installed_version: String,
    pub latest_version: String,
    pub auto_update: bool,
}

#[derive(Debug, Clone)]
pub struct GithubUpdateItem {
    pub owner_repo: String,
    pub mod_path: PathBuf,
    pub tag: Option<String>,
}

#[derive(Debug, Clone)]
pub enum GithubJob {
    Install {
        owner_repo: String,
        tag: Option<String>,
        dest: PathBuf,
        activate: bool,
    },
    Update {
        items: Vec<GithubUpdateItem>,
    },
    Check {
        entries: Vec<GithubModEntry>,
        force: bool,
    },
}

#[derive(Debug, Clone)]
pub struct GithubRequest {
    pub token: String,
    pub cache_path: PathBuf,
    pub job: GithubJob,
}

#[derive(Debug, Clone)]
pub enum GithubMsg {
    Progress(String),
    Done(GithubReport),
}

#[derive(Debug, Clone)]
pub enum GithubReport {
    Installed {
        owner_repo: String,
        version: String,
        asset_name: String,
        path: PathBuf,
        activate: bool,
        warning: Option<String>,
    },
    Updated {
        succeeded: Vec<UpdatedMod>,
        failed: Vec<(PathBuf, String)>,
    },
    Checked {
        updates: Vec<UpdateAvailable>,
        errors: Vec<String>,
    },
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct UpdatedMod {
    pub path: PathBuf,
    pub owner_repo: String,
    pub version: String,
    pub asset_name: String,
}

impl GithubReport {
    pub fn status_line(&self) -> String {
        match self {
            Self::Installed {
                owner_repo,
                version,
                warning,
                ..
            } => {
                let warn = warning
                    .as_deref()
                    .map(|w| format!(" ({w})"))
                    .unwrap_or_default();
                format!("GitHub: installed {owner_repo} @{version}{warn}")
            }
            Self::Updated { succeeded, failed } => {
                let ok = succeeded.len();
                let bad = failed.len();
                if bad == 0 {
                    format!("GitHub: updated {ok} mod(s)")
                } else if ok == 0 {
                    let detail = failed
                        .first()
                        .map(|(_, m)| m.as_str())
                        .unwrap_or("failed");
                    format!("GitHub: 0 ok, {bad} failed ({detail})")
                } else {
                    format!("GitHub: {ok} ok, {bad} failed")
                }
            }
            Self::Checked { updates, errors } => {
                if updates.is_empty() && errors.is_empty() {
                    "GitHub: all tracked mods are up to date".into()
                } else if updates.is_empty() {
                    format!("GitHub: check finished with {} error(s)", errors.len())
                } else {
                    format!("GitHub: {} update(s) available", updates.len())
                }
            }
            Self::Failed(e) => format!("GitHub: {e}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnwrapResult {
    Unwrapped,
    AlreadyCorrect,
    NoAboutXml,
}

pub fn spawn(req: GithubRequest, tx: Sender<GithubMsg>) {
    let tx_thread = tx.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("rimdeck-github".into())
        .spawn(move || {
            let report = run_job(&req, &tx_thread);
            let _ = tx_thread.send(GithubMsg::Done(report));
        })
    {
        let _ = tx.send(GithubMsg::Done(GithubReport::Failed(format!(
            "could not start GitHub thread: {e}"
        ))));
    }
}

fn run_job(req: &GithubRequest, tx: &Sender<GithubMsg>) -> GithubReport {
    let client = match blocking_client() {
        Ok(c) => c,
        Err(e) => return GithubReport::Failed(redact(&e.to_string(), &req.token)),
    };
    let mut cache = ReleaseCache::load_path(&req.cache_path);
    let report = match &req.job {
        GithubJob::Install {
            owner_repo,
            tag,
            dest,
            activate,
        } => install_one(
            &client,
            req,
            &mut cache,
            owner_repo,
            tag.as_deref(),
            dest,
            *activate,
            tx,
        ),
        GithubJob::Update { items } => update_many(&client, req, &mut cache, items, tx),
        GithubJob::Check { entries, force } => {
            check_updates(&client, req, &mut cache, entries, *force, tx)
        }
    };
    if let Err(e) = cache.save_path(&req.cache_path) {
        tracing::warn!("github release cache: {e}");
    }
    report
}

#[allow(clippy::too_many_arguments)]
fn install_one(
    client: &reqwest::blocking::Client,
    req: &GithubRequest,
    cache: &mut ReleaseCache,
    owner_repo: &str,
    tag: Option<&str>,
    dest: &Path,
    activate: bool,
    tx: &Sender<GithubMsg>,
) -> GithubReport {
    let _ = tx.send(GithubMsg::Progress(format!(
        "GitHub: fetching releases for {owner_repo}..."
    )));
    let releases = match get_releases(client, req, cache, owner_repo, true) {
        Ok(r) => r,
        Err(e) => return GithubReport::Failed(redact(&e.to_string(), &req.token)),
    };
    let release = match pick_release(&releases, tag) {
        Ok(r) => r,
        Err(e) => return GithubReport::Failed(e.to_string()),
    };
    let (asset_name, url) = match pick_download(release, owner_repo) {
        Ok(v) => v,
        Err(e) => return GithubReport::Failed(e.to_string()),
    };
    let _ = tx.send(GithubMsg::Progress(format!(
        "GitHub: downloading {asset_name}..."
    )));
    let warning = match download_extract_replace(client, req, &url, dest, false) {
        Ok(w) => w,
        Err(e) => return GithubReport::Failed(redact(&e.to_string(), &req.token)),
    };
    GithubReport::Installed {
        owner_repo: owner_repo.to_string(),
        version: release.tag.clone(),
        asset_name,
        path: dest.to_path_buf(),
        activate,
        warning,
    }
}

fn update_many(
    client: &reqwest::blocking::Client,
    req: &GithubRequest,
    cache: &mut ReleaseCache,
    items: &[GithubUpdateItem],
    tx: &Sender<GithubMsg>,
) -> GithubReport {
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    for item in items {
        let _ = tx.send(GithubMsg::Progress(format!(
            "GitHub: updating {}...",
            item.owner_repo
        )));
        match update_one(client, req, cache, item) {
            Ok(updated) => succeeded.push(updated),
            Err(e) => failed.push((
                item.mod_path.clone(),
                redact(&e.to_string(), &req.token),
            )),
        }
    }
    GithubReport::Updated { succeeded, failed }
}

fn update_one(
    client: &reqwest::blocking::Client,
    req: &GithubRequest,
    cache: &mut ReleaseCache,
    item: &GithubUpdateItem,
) -> Result<UpdatedMod> {
    let releases = get_releases(client, req, cache, &item.owner_repo, true)?;
    let release = pick_release(&releases, item.tag.as_deref())?;
    let (asset_name, url) = pick_download(release, &item.owner_repo)?;
    download_extract_replace(client, req, &url, &item.mod_path, true)?;
    Ok(UpdatedMod {
        path: item.mod_path.clone(),
        owner_repo: item.owner_repo.clone(),
        version: release.tag.clone(),
        asset_name,
    })
}

fn check_updates(
    client: &reqwest::blocking::Client,
    req: &GithubRequest,
    cache: &mut ReleaseCache,
    entries: &[GithubModEntry],
    force: bool,
    tx: &Sender<GithubMsg>,
) -> GithubReport {
    let mut errors = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, entry) in entries.iter().enumerate() {
        if !seen.insert(entry.owner_repo.clone()) {
            continue;
        }
        let _ = tx.send(GithubMsg::Progress(format!(
            "GitHub: checking {} ({}/{})...",
            entry.owner_repo,
            i + 1,
            entries.len()
        )));
        if let Err(e) = get_releases(client, req, cache, &entry.owner_repo, force) {
            errors.push(format!(
                "{}: {}",
                entry.owner_repo,
                redact(&e.to_string(), &req.token)
            ));
        }
    }
    let mut updates = Vec::new();
    for entry in entries {
        let releases = cache.releases_for(&entry.owner_repo);
        if let Some(update) = update_for_entry(entry, &releases) {
            updates.push(update);
        }
    }
    GithubReport::Checked { updates, errors }
}

/// Compare an installed tag against cached releases (RimSort publish-date check).
pub fn update_for_entry(entry: &GithubModEntry, releases: &[ReleaseInfo]) -> Option<UpdateAvailable> {
    let latest = latest_stable_release(releases)?;
    if entry.installed_version == "HEAD" {
        return Some(make_update(entry, latest));
    }
    if entry.installed_version == latest.tag {
        return None;
    }
    let Some(installed) = find_release_by_tag(releases, &entry.installed_version) else {
        return Some(make_update(entry, latest));
    };
    if published_key(&latest.published_at) > published_key(&installed.published_at) {
        Some(make_update(entry, latest))
    } else {
        None
    }
}

fn make_update(entry: &GithubModEntry, latest: &ReleaseInfo) -> UpdateAvailable {
    UpdateAvailable {
        owner_repo: entry.owner_repo.clone(),
        mod_path: entry.mod_path.clone(),
        installed_version: entry.installed_version.clone(),
        latest_version: latest.tag.clone(),
        auto_update: entry.auto_update,
    }
}

pub fn latest_stable_release(releases: &[ReleaseInfo]) -> Option<&ReleaseInfo> {
    let stable: Vec<&ReleaseInfo> = releases.iter().filter(|r| !r.prerelease).collect();
    if stable.is_empty() {
        return releases.first();
    }
    stable.into_iter().max_by_key(|r| published_key(&r.published_at))
}

fn find_release_by_tag<'a>(releases: &'a [ReleaseInfo], tag: &str) -> Option<&'a ReleaseInfo> {
    releases.iter().find(|r| r.tag == tag)
}

fn pick_release<'a>(releases: &'a [ReleaseInfo], tag: Option<&str>) -> Result<&'a ReleaseInfo> {
    if releases.is_empty() {
        return Err(Error::Github("no releases found".into()));
    }
    if let Some(tag) = tag {
        return find_release_by_tag(releases, tag)
            .ok_or_else(|| Error::Github(format!("release tag {tag} not found")));
    }
    latest_stable_release(releases)
        .ok_or_else(|| Error::Github("no releases found".into()))
}

fn pick_download(release: &ReleaseInfo, owner_repo: &str) -> Result<(String, String)> {
    let custom = release.custom_zip_assets();
    if let Some(asset) = pick_custom_asset(&custom, owner_repo) {
        if asset.download_url.trim().is_empty() {
            return Err(Error::Github(format!(
                "release {} has a zip asset with no download URL",
                release.tag
            )));
        }
        return Ok((asset.name.clone(), asset.download_url.clone()));
    }
    if !release.zipball_url.trim().is_empty() {
        return Ok((format!("{}-source.zip", release.tag), release.zipball_url.clone()));
    }
    Err(Error::Github(format!(
        "release {} has no zip asset or source archive",
        release.tag
    )))
}

fn pick_custom_asset<'a>(assets: &[&'a ReleaseAsset], owner_repo: &str) -> Option<&'a ReleaseAsset> {
    if assets.is_empty() {
        return None;
    }
    let repo = owner_repo
        .split('/')
        .next_back()
        .unwrap_or(owner_repo)
        .to_ascii_lowercase();
    assets
        .iter()
        .copied()
        .find(|a| a.name.to_ascii_lowercase().contains(&repo))
        .or_else(|| assets.first().copied())
}

pub fn install_dest(local_mods: &Path, owner_repo: &str) -> Result<PathBuf> {
    if local_mods.as_os_str().is_empty() {
        return Err(Error::Github(
            "local mods path is not set (press p)".into(),
        ));
    }
    let repo = owner_repo
        .split('/')
        .next_back()
        .unwrap_or(owner_repo);
    let name = sanitize_folder_name(repo);
    if name.is_empty() {
        return Err(Error::Github("could not derive a folder name".into()));
    }
    Ok(local_mods.join(name))
}

fn sanitize_folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    cleaned.trim_matches([' ', '.']).to_string()
}

/// Extract `(owner, repo, optional tag)` from a GitHub URL or `owner/repo[@tag]`.
pub fn parse_github_ref(input: &str) -> Option<GithubRef> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        return parse_owner_repo_path(rest.trim_end_matches('/'));
    }
    if let Some(without_scheme) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .or_else(|| trimmed.strip_prefix("ssh://git@"))
    {
        let host_path = without_scheme.trim_start_matches("git@");
        let (host, path) = host_path.split_once('/')?;
        let host = host.trim_end_matches(':');
        if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com")
        {
            return None;
        }
        return parse_owner_repo_path(path);
    }
    if trimmed.contains("://") {
        return None;
    }
    parse_owner_repo_path(trimmed)
}

fn parse_owner_repo_path(path: &str) -> Option<GithubRef> {
    let path = path.trim().trim_start_matches('/');
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next()?.trim();
    let repo_raw = parts.next()?.trim();
    let (repo, at_tag) = match repo_raw.split_once('@') {
        Some((r, t)) if !r.is_empty() && !t.is_empty() => (r.trim(), Some(t.to_string())),
        _ => (repo_raw, None),
    };
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    if owner.starts_with('.') || repo.starts_with('.') {
        return None;
    }
    let rest: Vec<&str> = parts.collect();
    let tag = tag_from_extra_path(&rest).or(at_tag);
    Some(GithubRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        tag,
    })
}

fn tag_from_extra_path(parts: &[&str]) -> Option<String> {
    if parts.len() >= 3 && parts[0] == "releases" && parts[1] == "tag" && !parts[2].is_empty() {
        return Some(parts[2].to_string());
    }
    if parts.len() >= 3 && parts[0] == "releases" && parts[1] == "download" && !parts[2].is_empty() {
        return Some(parts[2].to_string());
    }
    if parts.len() == 1 && parts[0].starts_with('@') {
        let tag = parts[0].trim_start_matches('@');
        if !tag.is_empty() {
            return Some(tag.to_string());
        }
    }
    None
}

pub fn parse_releases_json(text: &str) -> Result<Vec<ReleaseInfo>> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| Error::Github(format!("invalid JSON: {e}")))?;
    if let Some(msg) = value.get("message").and_then(serde_json::Value::as_str) {
        let lower = msg.to_ascii_lowercase();
        if lower.contains("rate limit") {
            return Err(Error::Github(
                "GitHub API rate limit reached. Set a GitHub token in path settings (p).".into(),
            ));
        }
        if lower.contains("not found") {
            return Ok(Vec::new());
        }
        return Err(Error::Github(msg.to_string()));
    }
    let arr = value
        .as_array()
        .or_else(|| value.get("releases").and_then(serde_json::Value::as_array))
        .ok_or_else(|| Error::Github("releases response is not an array".into()))?;
    let mut out = Vec::new();
    for entry in arr {
        if let Some(rel) = parse_release(entry) {
            out.push(rel);
        }
    }
    Ok(out)
}

fn parse_release(value: &serde_json::Value) -> Option<ReleaseInfo> {
    if value.get("draft").and_then(serde_json::Value::as_bool) == Some(true) {
        return None;
    }
    let tag = json_string(value.get("tag_name"))
        .or_else(|| json_string(value.get("tag")))
        .filter(|s| !s.is_empty())?;
    let name = json_string(value.get("name")).filter(|s| !s.is_empty()).unwrap_or_else(|| tag.clone());
    let published_at = json_string(value.get("published_at")).unwrap_or_default();
    let prerelease = value
        .get("prerelease")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let zipball_url = json_string(value.get("zipball_url")).unwrap_or_default();
    let body = json_string(value.get("body")).unwrap_or_default();
    let assets = value
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_asset).collect())
        .unwrap_or_default();
    Some(ReleaseInfo {
        tag,
        name,
        published_at,
        prerelease,
        zipball_url,
        assets,
        body,
    })
}

fn parse_asset(value: &serde_json::Value) -> Option<ReleaseAsset> {
    let name = json_string(value.get("name")).filter(|s| !s.is_empty())?;
    let download_url = json_string(value.get("browser_download_url"))
        .or_else(|| json_string(value.get("download_url")))
        .unwrap_or_default();
    let size = value
        .get("size")
        .and_then(|v| v.as_u64().or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok())))
        .or_else(|| json_string(value.get("size")).and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    Some(ReleaseAsset {
        name,
        download_url,
        size,
    })
}

fn json_string(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn published_key(s: &str) -> String {
    s.chars().take(19).collect()
}

fn blocking_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|e| Error::Github(e.to_string()))
}

fn get_releases(
    client: &reqwest::blocking::Client,
    req: &GithubRequest,
    cache: &mut ReleaseCache,
    owner_repo: &str,
    force: bool,
) -> Result<Vec<ReleaseInfo>> {
    if !force {
        if let Some(cached) = cache.entries.get(owner_repo) {
            let now = unix_now();
            if cached.last_checked_unix > 0
                && now.saturating_sub(cached.last_checked_unix) < CHECK_INTERVAL_SECS
                && !cached.releases.is_empty()
            {
                return Ok(cached.releases.iter().map(ReleaseInfo::from).collect());
            }
        }
    }
    let url = format!("https://api.github.com/repos/{owner_repo}/releases?per_page=100");
    let mut builder = client
        .get(&url)
        .header(reqwest::header::ACCEPT, API_ACCEPT)
        .header("X-GitHub-Api-Version", API_VERSION);
    if !req.token.trim().is_empty() {
        builder = builder.bearer_auth(req.token.trim());
    }
    if let Some(etag) = cache.entries.get(owner_repo).map(|c| c.etag.clone()) {
        if !etag.is_empty() {
            builder = builder.header(reqwest::header::IF_NONE_MATCH, etag);
        }
    }
    let resp = builder.send().map_err(|e| Error::Github(e.to_string()))?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        if let Some(entry) = cache.entries.get_mut(owner_repo) {
            entry.last_checked_unix = unix_now();
            return Ok(entry.releases.iter().map(ReleaseInfo::from).collect());
        }
        return Ok(Vec::new());
    }
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.text().map_err(|e| Error::Github(e.to_string()))?;
    if status.as_u16() == 403 || status.as_u16() == 429 {
        return Err(Error::Github(
            "GitHub API rate limit reached. Set a GitHub token in path settings (p).".into(),
        ));
    }
    if status.as_u16() == 404 {
        cache.entries.insert(
            owner_repo.to_string(),
            CachedReleases {
                etag,
                last_checked_unix: unix_now(),
                releases: Vec::new(),
            },
        );
        return Ok(Vec::new());
    }
    if !status.is_success() {
        return Err(Error::Github(format!(
            "releases API returned HTTP {status}"
        )));
    }
    let releases = parse_releases_json(&body)?;
    cache.entries.insert(
        owner_repo.to_string(),
        CachedReleases {
            etag,
            last_checked_unix: unix_now(),
            releases: releases.iter().map(CachedRelease::from).collect(),
        },
    );
    Ok(releases)
}

fn download_extract_replace(
    client: &reqwest::blocking::Client,
    req: &GithubRequest,
    url: &str,
    dest: &Path,
    backup: bool,
) -> Result<Option<String>> {
    let bytes = download_bytes(client, req, url)?;
    if !looks_like_zip(&bytes) {
        return Err(Error::Github(
            "download is not a zip archive".into(),
        ));
    }
    let backup_path = if backup && dest.exists() {
        Some(backup_mod(dest)?)
    } else {
        if dest.exists() {
            fs::remove_dir_all(dest).map_err(|e| Error::Github(e.to_string()))?;
        }
        None
    };
    fs::create_dir_all(dest).map_err(|e| Error::Github(e.to_string()))?;
    let unwrap = match extract_zip(&bytes, dest) {
        Ok(u) => u,
        Err(e) => {
            if let Some(backup_path) = backup_path.as_ref() {
                let _ = restore_backup(backup_path, dest);
            }
            return Err(e);
        }
    };
    if let Some(backup_path) = backup_path {
        delete_backup(&backup_path);
    }
    let warning = match unwrap {
        UnwrapResult::NoAboutXml => Some("extracted zip has no About/About.xml".into()),
        UnwrapResult::Unwrapped | UnwrapResult::AlreadyCorrect => None,
    };
    Ok(warning)
}

fn download_bytes(
    client: &reqwest::blocking::Client,
    req: &GithubRequest,
    url: &str,
) -> Result<Vec<u8>> {
    let mut builder = client.get(url);
    if url.contains("api.github.com") {
        builder = builder
            .header(reqwest::header::ACCEPT, "application/octet-stream")
            .header("X-GitHub-Api-Version", API_VERSION);
    }
    if !req.token.trim().is_empty() {
        builder = builder.bearer_auth(req.token.trim());
    }
    let resp = builder.send().map_err(|e| Error::Github(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(Error::Github(format!("download returned HTTP {status}")));
    }
    let bytes = resp.bytes().map_err(|e| Error::Github(e.to_string()))?;
    Ok(bytes.to_vec())
}

pub fn extract_zip(bytes: &[u8], dest: &Path) -> Result<UnwrapResult> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Github(format!("zip: {e}")))?;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| Error::Github(format!("zip: {e}")))?;
        let name = file.name().replace('\\', "/");
        if name.split('/').any(|p| p == "__MACOSX") {
            continue;
        }
        let Some(out_path) = sanitized_zip_path(dest, &name) else {
            tracing::warn!("zip slip skipped: {name}");
            continue;
        };
        if file.is_dir() || name.ends_with('/') {
            fs::create_dir_all(&out_path).map_err(|e| Error::Github(e.to_string()))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::Github(e.to_string()))?;
        }
        let mut out = fs::File::create(&out_path).map_err(|e| Error::Github(e.to_string()))?;
        std::io::copy(&mut file, &mut out).map_err(|e| Error::Github(e.to_string()))?;
    }
    unwrap_extracted_mod(dest)
}

pub fn sanitized_zip_path(target: &Path, name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    let mut dest = target.to_path_buf();
    for part in name.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            return None;
        }
        dest.push(part);
    }
    Some(dest)
}

pub fn unwrap_extracted_mod(target_dir: &Path) -> Result<UnwrapResult> {
    if has_about_xml(target_dir) {
        return Ok(UnwrapResult::AlreadyCorrect);
    }
    let top = unwrap_candidate_dirs(target_dir)?;
    if top.len() == 1 && has_about_xml(&top[0]) {
        let wrapper = &top[0];
        let temp = target_dir.parent().map_or_else(
            || target_dir.with_file_name(format!("{}.unwrap_tmp", file_name(target_dir))),
            |p| p.join(format!("{}.unwrap_tmp", file_name(target_dir))),
        );
        if temp.exists() {
            fs::remove_dir_all(&temp).map_err(|e| Error::Github(e.to_string()))?;
        }
        fs::rename(wrapper, &temp).map_err(|e| Error::Github(e.to_string()))?;
        for entry in fs::read_dir(&temp).map_err(|e| Error::Github(e.to_string()))? {
            let entry = entry.map_err(|e| Error::Github(e.to_string()))?;
            let dest = target_dir.join(entry.file_name());
            fs::rename(entry.path(), dest).map_err(|e| Error::Github(e.to_string()))?;
        }
        let _ = fs::remove_dir_all(&temp);
        return Ok(UnwrapResult::Unwrapped);
    }
    Ok(UnwrapResult::NoAboutXml)
}

fn unwrap_candidate_dirs(target_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let iter = match fs::read_dir(target_dir) {
        Ok(i) => i,
        Err(e) => return Err(Error::Github(e.to_string())),
    };
    for entry in iter {
        let entry = entry.map_err(|e| Error::Github(e.to_string()))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "__MACOSX" || name.starts_with('.') {
            continue;
        }
        out.push(path);
    }
    Ok(out)
}

fn has_about_xml(directory: &Path) -> bool {
    let exact = directory.join("About").join("About.xml");
    if exact.is_file() {
        return true;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !entry.file_name().eq_ignore_ascii_case("about") {
            continue;
        }
        if let Ok(files) = fs::read_dir(&path) {
            if files
                .flatten()
                .any(|f| f.file_name().eq_ignore_ascii_case("about.xml") && f.path().is_file())
            {
                return true;
            }
        }
    }
    false
}

fn backup_mod(mod_path: &Path) -> Result<PathBuf> {
    let backup_path = backup_path_for(mod_path);
    if backup_path.exists() {
        fs::remove_dir_all(&backup_path).map_err(|e| Error::Github(e.to_string()))?;
    }
    fs::rename(mod_path, &backup_path).map_err(|e| Error::Github(e.to_string()))?;
    Ok(backup_path)
}

fn restore_backup(backup_path: &Path, mod_path: &Path) -> Result<()> {
    if mod_path.exists() {
        fs::remove_dir_all(mod_path).map_err(|e| Error::Github(e.to_string()))?;
    }
    fs::rename(backup_path, mod_path).map_err(|e| Error::Github(e.to_string()))?;
    Ok(())
}

fn delete_backup(backup_path: &Path) {
    if backup_path.exists() {
        let _ = fs::remove_dir_all(backup_path);
    }
}

fn backup_path_for(mod_path: &Path) -> PathBuf {
    let name = file_name(mod_path);
    mod_path
        .parent()
        .map(|p| p.join(format!("{name}.rimdeck_backup")))
        .unwrap_or_else(|| PathBuf::from(format!("{name}.rimdeck_backup")))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mod".into())
}

fn looks_like_zip(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0] == b'P' && bytes[1] == b'K'
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn redact(text: &str, secret: &str) -> String {
    let secret = secret.trim();
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "***")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::sync::atomic::{AtomicU64, Ordering};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    const RELEASES_FIXTURE: &str = r#"[
      {
        "tag_name": "v2.0.0-beta",
        "name": "Beta",
        "published_at": "2026-02-01T00:00:00Z",
        "prerelease": true,
        "draft": false,
        "zipball_url": "https://api.github.com/repos/o/r/zipball/v2.0.0-beta",
        "assets": []
      },
      {
        "tag_name": "v1.2.0",
        "name": "1.2.0",
        "published_at": "2026-01-15T12:00:00Z",
        "prerelease": false,
        "zipball_url": "https://api.github.com/repos/o/r/zipball/v1.2.0",
        "body": "newer",
        "assets": [
          {
            "name": "Source code (zip)",
            "browser_download_url": "https://example.invalid/source.zip",
            "size": 10
          },
          {
            "name": "CoolMod.zip",
            "browser_download_url": "https://example.invalid/CoolMod.zip",
            "size": 99
          }
        ]
      },
      {
        "tag_name": "v1.0.0",
        "published_at": "2025-06-01T00:00:00Z",
        "prerelease": false,
        "draft": true,
        "assets": []
      },
      {
        "tag_name": "v1.1.0",
        "published_at": "2025-12-01T00:00:00Z",
        "prerelease": false,
        "zipball_url": "https://api.github.com/repos/o/r/zipball/v1.1.0",
        "assets": [
          {"name": "other.zip", "download_url": "https://example.invalid/other.zip", "size": "12"}
        ]
      }
    ]"#;

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            for (name, payload) in entries {
                zip.start_file(*name, SimpleFileOptions::default()).unwrap();
                zip.write_all(payload).unwrap();
            }
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rimdeck-gh-{}-{}-{}",
            label,
            std::process::id(),
            TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_github_urls() {
        let a = parse_github_ref("https://github.com/UnlimitedHugs/HugsLib").unwrap();
        assert_eq!(a.owner, "UnlimitedHugs");
        assert_eq!(a.repo, "HugsLib");
        assert_eq!(a.tag, None);

        let b = parse_github_ref("https://www.github.com/o/r.git").unwrap();
        assert_eq!(b.owner_repo(), "o/r");

        let c = parse_github_ref("owner/repo@v1.2.3").unwrap();
        assert_eq!(c.owner_repo(), "owner/repo");
        assert_eq!(c.tag.as_deref(), Some("v1.2.3"));
        let d = parse_github_ref("owner/repo").unwrap();
        assert_eq!(d.owner_repo(), "owner/repo");

        let e = parse_github_ref(
            "https://github.com/UnlimitedHugs/HugsLib/releases/tag/v10.0.1",
        )
        .unwrap();
        assert_eq!(e.tag.as_deref(), Some("v10.0.1"));

        let f = parse_github_ref("git@github.com:o/r.git").unwrap();
        assert_eq!(f.owner_repo(), "o/r");
    }

    #[test]
    fn rejects_non_github_hosts() {
        assert!(parse_github_ref("https://gitlab.com/o/r").is_none());
        assert!(parse_github_ref("").is_none());
        assert!(parse_github_ref("onlyone").is_none());
    }

    #[test]
    fn parses_releases_leniently() {
        let releases = parse_releases_json(RELEASES_FIXTURE).unwrap();
        assert_eq!(releases.len(), 3);
        assert_eq!(releases[1].tag, "v1.2.0");
        assert_eq!(releases[1].assets.len(), 2);
        assert_eq!(releases[2].assets[0].download_url, "https://example.invalid/other.zip");
        assert_eq!(releases[2].assets[0].size, 12);
    }

    #[test]
    fn skips_drafts_and_picks_latest_stable() {
        let releases = parse_releases_json(RELEASES_FIXTURE).unwrap();
        let latest = latest_stable_release(&releases).unwrap();
        assert_eq!(latest.tag, "v1.2.0");
        let custom = latest.custom_zip_assets();
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0].name, "CoolMod.zip");
    }

    #[test]
    fn rate_limit_json_is_an_error() {
        let err = parse_releases_json(r#"{"message":"API rate limit exceeded"}"#).unwrap_err();
        assert!(err.to_string().to_ascii_lowercase().contains("rate limit"));
    }

    #[test]
    fn not_found_json_is_empty() {
        let releases = parse_releases_json(r#"{"message":"Not Found"}"#).unwrap();
        assert!(releases.is_empty());
    }

    #[test]
    fn update_uses_publish_dates() {
        let releases = parse_releases_json(RELEASES_FIXTURE).unwrap();
        let entry = GithubModEntry {
            owner_repo: "o/CoolMod".into(),
            mod_path: PathBuf::from("/mods/CoolMod"),
            installed_version: "v1.1.0".into(),
            installed_asset_name: None,
            auto_update: false,
        };
        let update = update_for_entry(&entry, &releases).unwrap();
        assert_eq!(update.latest_version, "v1.2.0");

        let current = GithubModEntry {
            installed_version: "v1.2.0".into(),
            ..entry.clone()
        };
        assert!(update_for_entry(&current, &releases).is_none());

        let head = GithubModEntry {
            installed_version: "HEAD".into(),
            ..entry
        };
        assert_eq!(
            update_for_entry(&head, &releases).unwrap().latest_version,
            "v1.2.0"
        );
    }

    #[test]
    fn missing_installed_tag_counts_as_update() {
        let releases = parse_releases_json(RELEASES_FIXTURE).unwrap();
        let entry = GithubModEntry {
            owner_repo: "o/r".into(),
            mod_path: PathBuf::from("/m"),
            installed_version: "unknown".into(),
            installed_asset_name: None,
            auto_update: false,
        };
        assert_eq!(
            update_for_entry(&entry, &releases).unwrap().latest_version,
            "v1.2.0"
        );
    }

    #[test]
    fn unwraps_single_wrapper_directory() {
        let dir = temp_dir("unwrap");
        let wrapper = dir.join("CoolMod-1.2.3");
        fs::create_dir_all(wrapper.join("About")).unwrap();
        fs::write(wrapper.join("About").join("About.xml"), b"<ModMetaData/>").unwrap();
        assert_eq!(unwrap_extracted_mod(&dir).unwrap(), UnwrapResult::Unwrapped);
        assert!(dir.join("About").join("About.xml").is_file());
        assert!(!wrapper.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn already_correct_mod_layout() {
        let dir = temp_dir("flat");
        fs::create_dir_all(dir.join("About")).unwrap();
        fs::write(dir.join("About").join("About.xml"), b"<ModMetaData/>").unwrap();
        assert_eq!(
            unwrap_extracted_mod(&dir).unwrap(),
            UnwrapResult::AlreadyCorrect
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_skips_zip_slip_and_unwraps() {
        let dir = temp_dir("slip");
        let zip = zip_with(&[
            ("CoolMod/About/About.xml", b"<ModMetaData/>".as_slice()),
            ("../evil.txt", b"nope"),
        ]);
        let result = extract_zip(&zip, &dir).unwrap();
        assert_eq!(result, UnwrapResult::Unwrapped);
        assert!(dir.join("About").join("About.xml").is_file());
        assert!(!dir.join("evil.txt").exists());
        assert!(!dir
            .parent()
            .unwrap()
            .join("evil.txt")
            .is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitized_zip_path_rejects_parent_and_drive() {
        let target = PathBuf::from("/mods/out");
        assert!(sanitized_zip_path(&target, "../x").is_none());
        assert!(sanitized_zip_path(&target, "C:/Windows/x").is_none());
        let ok = sanitized_zip_path(&target, "About/About.xml").unwrap();
        assert!(ok.ends_with(Path::new("About/About.xml")) || ok.ends_with("About\\About.xml"));
    }

    #[test]
    fn tracking_upsert_preserves_auto_update() {
        let mut db = GithubModsDb::default();
        db.upsert(GithubModEntry {
            owner_repo: "o/r".into(),
            mod_path: PathBuf::from("/mods/r"),
            installed_version: "v1".into(),
            installed_asset_name: Some("r.zip".into()),
            auto_update: true,
        });
        db.upsert(GithubModEntry {
            owner_repo: "o/r".into(),
            mod_path: PathBuf::from("/mods/r"),
            installed_version: "v2".into(),
            installed_asset_name: Some("r.zip".into()),
            auto_update: false,
        });
        assert_eq!(db.mods.len(), 1);
        assert_eq!(db.mods[0].installed_version, "v2");
        assert!(db.mods[0].auto_update);
    }

    #[test]
    fn tracking_round_trip_json() {
        let dir = temp_dir("db");
        let path = dir.join("github_mods.json");
        let mut db = GithubModsDb::default();
        db.mods.push(GithubModEntry {
            owner_repo: "o/r".into(),
            mod_path: PathBuf::from("/mods/r"),
            installed_version: "v1".into(),
            installed_asset_name: None,
            auto_update: true,
        });
        db.save_path(&path).unwrap();
        let loaded = GithubModsDb::load_path(&path);
        assert_eq!(loaded.mods.len(), 1);
        assert!(loaded.mods[0].auto_update);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_tracking_is_empty() {
        let dir = temp_dir("bad");
        let path = dir.join("github_mods.json");
        fs::write(&path, "not json").unwrap();
        let loaded = GithubModsDb::load_path(&path);
        assert!(loaded.mods.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn redacts_token() {
        let token = "ghp_supersecret123";
        let msg = format!("Authorization: Bearer {token}");
        let redacted = redact(&msg, token);
        assert!(!redacted.contains(token));
        assert!(redacted.contains("***"));
    }

    #[test]
    fn install_dest_uses_repo_name() {
        let dest = install_dest(Path::new("C:/RimWorld/Mods"), "UnlimitedHugs/HugsLib").unwrap();
        assert!(dest.ends_with("HugsLib"));
    }

    #[test]
    fn pick_download_prefers_custom_zip() {
        let releases = parse_releases_json(RELEASES_FIXTURE).unwrap();
        let latest = latest_stable_release(&releases).unwrap();
        let (name, url) = pick_download(latest, "o/CoolMod").unwrap();
        assert_eq!(name, "CoolMod.zip");
        assert!(url.ends_with("CoolMod.zip"));
    }
}
