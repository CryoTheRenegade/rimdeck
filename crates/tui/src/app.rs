use std::collections::{HashMap, HashSet};
use std::io::{self, stdout};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use ratatui::Terminal;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use rimtui_core::github::GithubModEntry;
use rimtui_core::gitmod::{self, GitModInfo};
use rimtui_core::model::Mod;
use rimtui_core::session::{self, Session};
use rimtui_core::settings::{Settings, SettingsField};
use rimtui_core::workshop::WorkshopItem;

use crate::fs_browser::FileBrowser;
use crate::preview;
use crate::ui;

/// Pause longer than this between notches and the next click is slow again.
const WHEEL_RESET: Duration = Duration::from_millis(200);
const SPINNER_TICK: Duration = Duration::from_millis(80);
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Main,
    Settings,
    Help,
    ImageWarning,
    Workshop,
    Github,
    Git,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Inactive,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
    SettingsEdit,
    PathPrompt,
    FileExplorer,
    WorkshopQuery,
    GithubQuery,
    GitQuery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathPromptKind {
    Export,
}

/// Terminal frontend state. Domain state lives in `session`; `Deref` lets the drawing
/// code read session fields (`app.mods`, `app.status`, ...) without a prefix.
pub struct App {
    pub session: Session,
    pub screen: Screen,
    pub pane: Pane,
    pub input_mode: InputMode,
    pub settings_field: SettingsField,
    pub edit_buffer: String,
    pub selected_inactive: usize,
    pub selected_active: usize,
    pub inactive_state: ListState,
    pub active_state: ListState,
    pub search: String,
    pub should_quit: bool,
    /// `r` was pressed once with unsaved edits; the next `r` discards them.
    confirm_refresh: bool,
    pub warning_scroll: u16,
    pub info_scroll: u16,
    pub info_area: Rect,
    pub inactive_area: Rect,
    pub active_area: Rect,
    pub issues_area: Rect,
    pub explorer_area: Rect,
    pub picker: Option<Picker>,
    pub preview_cache: HashMap<PathBuf, StatefulProtocol>,
    pub preview_failed: HashSet<PathBuf>,
    pub current_preview: Option<PathBuf>,
    pub terminal_name: String,
    pub after_warning: Screen,
    pub path_prompt: PathPromptKind,
    pub file_browser: FileBrowser,
    pub last_browse_dir: Option<PathBuf>,
    pub workshop_query: String,
    pub workshop_selected: usize,
    pub workshop_state: ListState,
    pub workshop_area: Rect,
    pub github_query: String,
    pub github_selected: usize,
    pub github_state: ListState,
    pub github_area: Rect,
    pub git_query: String,
    pub git_selected: usize,
    pub git_state: ListState,
    pub git_area: Rect,
    spinner_frame: usize,
    last_spin: Instant,
    scroll_streak: u32,
    scroll_dir: isize,
    last_scroll: Instant,
}

impl Deref for App {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.session
    }
}

impl DerefMut for App {
    fn deref_mut(&mut self) -> &mut Session {
        &mut self.session
    }
}

/// Clamp `sel` to `len` items and mirror it into the ratatui list state.
fn clamp_list(sel: &mut usize, state: &mut ListState, len: usize) {
    *sel = (*sel).min(len.saturating_sub(1));
    state.select((len > 0).then_some(*sel));
}

impl App {
    pub fn new(settings: Settings) -> Self {
        let mut session = Session::new(settings);
        let first_run = !session.start();
        let after_warning = if first_run {
            Screen::Settings
        } else {
            Screen::Main
        };
        if first_run {
            session.status = "Set RimWorld paths, then press s to save".into();
        }
        Self {
            session,
            screen: Screen::ImageWarning,
            pane: Pane::Inactive,
            input_mode: InputMode::Normal,
            settings_field: SettingsField::GameInstall,
            edit_buffer: String::new(),
            selected_inactive: 0,
            selected_active: 0,
            inactive_state: ListState::default(),
            active_state: ListState::default(),
            search: String::new(),
            should_quit: false,
            confirm_refresh: false,
            warning_scroll: 0,
            info_scroll: 0,
            info_area: Rect::default(),
            inactive_area: Rect::default(),
            active_area: Rect::default(),
            issues_area: Rect::default(),
            explorer_area: Rect::default(),
            picker: None,
            preview_cache: HashMap::new(),
            preview_failed: HashSet::new(),
            current_preview: None,
            terminal_name: preview::terminal_emulator_name(),
            after_warning,
            path_prompt: PathPromptKind::Export,
            file_browser: FileBrowser::default(),
            last_browse_dir: None,
            workshop_query: String::new(),
            workshop_selected: 0,
            workshop_state: ListState::default(),
            workshop_area: Rect::default(),
            github_query: String::new(),
            github_selected: 0,
            github_state: ListState::default(),
            github_area: Rect::default(),
            git_query: String::new(),
            git_selected: 0,
            git_state: ListState::default(),
            git_area: Rect::default(),
            spinner_frame: 0,
            last_spin: Instant::now(),
            scroll_streak: 0,
            scroll_dir: 0,
            last_scroll: Instant::now(),
        }
    }

    pub fn spinner_glyph(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()]
    }

    pub fn tick_spinner(&mut self) {
        if !self.is_busy() {
            return;
        }
        let now = Instant::now();
        if now.saturating_duration_since(self.last_spin) >= SPINNER_TICK {
            self.spinner_frame = self.spinner_frame.wrapping_add(1);
            self.last_spin = now;
        }
    }

    /// Drain session workers and fix up UI state they invalidate.
    pub fn poll(&mut self) {
        for ev in self.session.poll() {
            match ev {
                session::Event::Reloaded => {
                    self.preview_cache.clear();
                    self.preview_failed.clear();
                    self.current_preview = None;
                    self.clamp_selection();
                    let len = self.session.git_items.len();
                    clamp_list(&mut self.git_selected, &mut self.git_state, len);
                }
                session::Event::WorkshopResults => {
                    self.workshop_selected = 0;
                    let len = self.session.workshop_items.len();
                    clamp_list(&mut self.workshop_selected, &mut self.workshop_state, len);
                    self.input_mode = InputMode::Normal;
                }
                session::Event::GithubInstalled | session::Event::GitCloned => {
                    self.input_mode = InputMode::Normal;
                }
            }
        }
    }

    pub fn filtered_inactive(&self) -> Vec<PathBuf> {
        self.session.filtered_inactive(&self.search)
    }

    pub fn filtered_active(&self) -> Vec<PathBuf> {
        self.session.filtered_active(&self.search)
    }

    fn selected_path(&self) -> Option<PathBuf> {
        let (paths, idx) = match self.pane {
            Pane::Inactive => (self.filtered_inactive(), self.selected_inactive),
            Pane::Active => (self.filtered_active(), self.selected_active),
        };
        paths.get(idx).cloned()
    }

    pub fn selected_mod(&self) -> Option<&Mod> {
        self.selected_path().and_then(|p| self.session.mod_ref(&p))
    }

    pub fn ensure_preview(&mut self) {
        let Some(path) = self.selected_mod().and_then(Mod::preview_path) else {
            self.current_preview = None;
            return;
        };
        if self.preview_failed.contains(&path) {
            self.current_preview = None;
            return;
        }
        if !self.preview_cache.contains_key(&path) {
            let loaded = self
                .picker
                .as_mut()
                .map(|picker| preview::load_protocol(picker, &path));
            match loaded {
                Some(Ok(protocol)) => {
                    if self.preview_cache.len() >= 12 {
                        if let Some(evict) =
                            self.preview_cache.keys().find(|k| *k != &path).cloned()
                        {
                            self.preview_cache.remove(&evict);
                        }
                    }
                    self.preview_cache.insert(path.clone(), protocol);
                }
                Some(Err(e)) => {
                    tracing::debug!("preview {}: {e}", path.display());
                    self.preview_failed.insert(path);
                    self.current_preview = None;
                    return;
                }
                None => {
                    self.current_preview = None;
                    return;
                }
            }
        }
        self.current_preview = Some(path);
    }

    fn clamp_selection(&mut self) {
        let inact = self.filtered_inactive().len();
        let act = self.filtered_active().len();
        clamp_list(&mut self.selected_inactive, &mut self.inactive_state, inact);
        clamp_list(&mut self.selected_active, &mut self.active_state, act);
    }

    fn move_sel(&mut self, delta: isize) {
        let len = match self.pane {
            Pane::Inactive => self.filtered_inactive().len(),
            Pane::Active => self.filtered_active().len(),
        };
        if len == 0 {
            return;
        }
        let sel = match self.pane {
            Pane::Inactive => &mut self.selected_inactive,
            Pane::Active => &mut self.selected_active,
        };
        *sel = (*sel as isize + delta).rem_euclid(len as isize) as usize;
        self.clamp_selection();
    }

    fn scroll_sel(&mut self, delta: isize) {
        let len = match self.pane {
            Pane::Inactive => self.filtered_inactive().len(),
            Pane::Active => self.filtered_active().len(),
        };
        if len == 0 {
            return;
        }
        let sel = match self.pane {
            Pane::Inactive => &mut self.selected_inactive,
            Pane::Active => &mut self.selected_active,
        };
        *sel = (*sel as isize + delta).clamp(0, len as isize - 1) as usize;
        self.clamp_selection();
    }

    fn accelerate_wheel(&mut self, dir: isize) -> isize {
        let now = Instant::now();
        let same_dir = self.scroll_dir == dir;
        let recent = now.saturating_duration_since(self.last_scroll) < WHEEL_RESET;
        if same_dir && recent {
            self.scroll_streak = self.scroll_streak.saturating_add(1);
        } else {
            self.scroll_streak = 1;
        }
        self.scroll_dir = dir;
        self.last_scroll = now;
        wheel_list_lines(self.scroll_streak)
    }

    fn toggle_selected(&mut self) {
        if let Some(path) = self.selected_path() {
            self.session.toggle(&path);
            self.clamp_selection();
        }
    }

    fn reorder_active(&mut self, delta: isize) {
        let Some(path) = self.filtered_active().get(self.selected_active).cloned() else {
            return;
        };
        self.session.move_active(&path, delta);
        if let Some(new_sel) = self.filtered_active().iter().position(|p| p == &path) {
            self.selected_active = new_sel;
        }
        self.clamp_selection();
    }

    fn sort_list(&mut self) {
        self.session.sort();
        self.clamp_selection();
    }

    fn cycle_sort_mode(&mut self) {
        let mode = self.session.settings.sort_mode.toggle();
        self.session.set_sort_mode(mode);
        self.session.status.push_str(" (press s to sort)");
    }

    fn start_export(&mut self) {
        self.path_prompt = PathPromptKind::Export;
        self.edit_buffer = Settings::default_list_path().display().to_string();
        self.input_mode = InputMode::PathPrompt;
        self.session.status = "Export list: enter a path".into();
    }

    fn start_import(&mut self) {
        let start = self
            .last_browse_dir
            .clone()
            .unwrap_or_else(Settings::default_lists_dir);
        if !start.is_dir() {
            let _ = std::fs::create_dir_all(&start);
        }
        self.file_browser = FileBrowser::open(start);
        self.input_mode = InputMode::FileExplorer;
        self.session.status = "Load list: pick an XML file".into();
    }

    fn close_explorer(&mut self) {
        self.input_mode = InputMode::Normal;
        self.session.status = "Cancelled".into();
    }

    fn confirm_explorer(&mut self) {
        if let Some(path) = self.file_browser.activate() {
            self.load_list_from(&path);
        }
    }

    fn load_list_from(&mut self, path: &Path) {
        if self.session.load_list(path) {
            self.input_mode = InputMode::Normal;
            self.last_browse_dir = path.parent().map(Path::to_path_buf);
            self.clamp_selection();
        }
    }

    fn finish_path_prompt(&mut self) {
        let raw = std::mem::take(&mut self.edit_buffer).trim().to_string();
        self.input_mode = InputMode::Normal;
        if raw.is_empty() {
            self.session.status = "Cancelled (empty path)".into();
            return;
        }
        match self.path_prompt {
            PathPromptKind::Export => self.session.export_list(Path::new(&raw)),
        }
    }

    fn launch_rimworld(&mut self) {
        self.session.launch();
    }

    fn toggle_user_load_top(&mut self) {
        if let Some(pid) = self.selected_mod().map(|m| m.package_id.clone()) {
            self.session.toggle_user_load_top(&pid);
        }
    }

    fn toggle_user_load_bottom(&mut self) {
        if let Some(pid) = self.selected_mod().map(|m| m.package_id.clone()) {
            self.session.toggle_user_load_bottom(&pid);
        }
    }

    fn update_selected_mod(&mut self) {
        match self.selected_path() {
            Some(path) => self.session.update_mod(&path),
            None => self.session.status = "No mod selected".into(),
        }
    }

    // ---- workshop overlay -------------------------------------------------

    fn open_workshop_search(&mut self) {
        self.screen = Screen::Workshop;
        self.input_mode = InputMode::WorkshopQuery;
        self.session.status = "Workshop search: type a query or published file ID".into();
    }

    fn close_workshop(&mut self) {
        self.screen = Screen::Main;
        self.input_mode = InputMode::Normal;
        self.session.status = "Closed workshop search".into();
    }

    pub fn selected_workshop_item(&self) -> Option<&WorkshopItem> {
        self.session.workshop_items.get(self.workshop_selected)
    }

    fn start_workshop_search(&mut self) {
        self.session.search_workshop(&self.workshop_query);
    }

    fn queue_workshop_download(&mut self, activate: bool) {
        let Some(item) = self.selected_workshop_item().cloned() else {
            self.session.status = "No workshop result selected".into();
            return;
        };
        self.session
            .download_workshop(&item.published_file_id, &item.title, activate);
    }

    // ---- GitHub overlay ---------------------------------------------------

    pub fn github_entry_for(&self, path: &Path) -> Option<&GithubModEntry> {
        self.session.github_mods.by_path(path)
    }

    pub fn selected_github_entry(&self) -> Option<&GithubModEntry> {
        self.session.github_mods.mods.get(self.github_selected)
    }

    fn selected_github_path(&self) -> Option<PathBuf> {
        self.selected_github_entry().map(|e| e.mod_path.clone())
    }

    fn open_github(&mut self) {
        self.screen = Screen::Github;
        self.session.reload_github_cache();
        let len = self.session.github_mods.mods.len();
        if len == 0 {
            self.input_mode = InputMode::GithubQuery;
            self.session.status = "GitHub: paste owner/repo or a github.com URL".into();
        } else {
            self.input_mode = InputMode::Normal;
            clamp_list(&mut self.github_selected, &mut self.github_state, len);
            self.session.status = format!("GitHub: {len} tracked mod(s)");
        }
    }

    fn close_github(&mut self) {
        self.screen = Screen::Main;
        self.input_mode = InputMode::Normal;
        self.session.status = "Closed GitHub mods".into();
    }

    fn start_github_install(&mut self) {
        self.session.github_install(&self.github_query);
    }

    fn start_github_check(&mut self, force: bool) {
        self.session.github_check(force);
    }

    fn start_github_update_selected(&mut self) {
        match self.selected_github_path() {
            Some(p) => self.session.github_update(&p),
            None => self.session.status = "GitHub: no tracked mod selected".into(),
        }
    }

    fn toggle_github_auto_update(&mut self) {
        match self.selected_github_path() {
            Some(p) => self.session.github_toggle_auto_update(&p),
            None => self.session.status = "GitHub: no tracked mod selected".into(),
        }
    }

    fn uninstall_github_selected(&mut self) {
        let Some(p) = self.selected_github_path() else {
            self.session.status = "GitHub: no tracked mod selected".into();
            return;
        };
        self.session.github_uninstall(&p);
        let len = self.session.github_mods.mods.len();
        clamp_list(&mut self.github_selected, &mut self.github_state, len);
    }

    // ---- git overlay ------------------------------------------------------

    pub fn selected_git_item(&self) -> Option<&GitModInfo> {
        self.session.git_items.get(self.git_selected)
    }

    fn open_git(&mut self) {
        self.screen = Screen::Git;
        self.session.refresh_git_items();
        let len = self.session.git_items.len();
        clamp_list(&mut self.git_selected, &mut self.git_state, len);
        if !gitmod::git_available() {
            self.input_mode = InputMode::Normal;
            self.session.status = "Git: git is not on PATH".into();
        } else if len == 0 {
            self.input_mode = InputMode::GitQuery;
            self.session.status =
                "Git: paste a clone URL (GitHub, GitLab, GitGud, Forgejo, …)".into();
        } else {
            self.input_mode = InputMode::Normal;
            self.session.status = format!("Git: {len} cloned mod(s)");
        }
    }

    fn close_git(&mut self) {
        self.screen = Screen::Main;
        self.input_mode = InputMode::Normal;
        self.session.status = "Closed git updater".into();
    }

    fn start_git_clone(&mut self) {
        self.session.git_clone(&self.git_query);
    }

    fn start_git_check(&mut self) {
        self.session.git_check();
    }

    fn start_git_pull_selected(&mut self) {
        match self.selected_git_item().map(|i| i.path.clone()) {
            Some(p) => self.session.git_pull(&p),
            None => self.session.status = "Git: no cloned mod selected".into(),
        }
    }

    fn start_git_pull_updates(&mut self) {
        self.session.git_pull_updates();
    }

    fn uninstall_git_selected(&mut self) {
        let Some(p) = self.selected_git_item().map(|i| i.path.clone()) else {
            self.session.status = "Git: no cloned mod selected".into();
            return;
        };
        self.session.git_uninstall(&p);
        let len = self.session.git_items.len();
        clamp_list(&mut self.git_selected, &mut self.git_state, len);
    }

    fn save_settings(&mut self) {
        if self.session.save_settings() {
            self.screen = Screen::Main;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Repeat {
            let explorer = self.input_mode == InputMode::FileExplorer;
            let workshop =
                self.screen == Screen::Workshop && self.input_mode == InputMode::Normal;
            let github = self.screen == Screen::Github && self.input_mode == InputMode::Normal;
            let git = self.screen == Screen::Git && self.input_mode == InputMode::Normal;
            if !explorer
                && !workshop
                && !github
                && !git
                && (self.input_mode != InputMode::Normal || self.screen != Screen::Main)
            {
                return;
            }
            match key.code {
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Char('j')
                | KeyCode::Char('k')
                | KeyCode::Char('[')
                | KeyCode::Char(']') => {}
                _ => return,
            }
        } else if key.kind != KeyEventKind::Press {
            return;
        }
        match self.input_mode {
            InputMode::Search => self.handle_search_key(key),
            InputMode::SettingsEdit => self.handle_settings_edit_key(key),
            InputMode::PathPrompt => self.handle_path_prompt_key(key),
            InputMode::FileExplorer => self.handle_file_explorer_key(key),
            InputMode::WorkshopQuery => self.handle_workshop_query_key(key),
            InputMode::GithubQuery => self.handle_github_query_key(key),
            InputMode::GitQuery => self.handle_git_query_key(key),
            InputMode::Normal => match self.screen {
                Screen::ImageWarning => {
                    if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ')) {
                        self.dismiss_image_warning();
                    }
                }
                Screen::Help => {
                    if matches!(
                        key.code,
                        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
                    ) {
                        self.screen = Screen::Main;
                    }
                }
                Screen::Settings => self.handle_settings_key(key),
                Screen::Workshop => self.handle_workshop_list_key(key),
                Screen::Github => self.handle_github_list_key(key),
                Screen::Git => self.handle_git_list_key(key),
                Screen::Main => self.handle_main_key(key),
            },
        }
    }

    fn dismiss_image_warning(&mut self) {
        self.screen = self.after_warning;
        if self.status.is_empty() {
            self.status = format!("Previews may look blocky in {}", self.terminal_name);
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.screen == Screen::ImageWarning {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.dismiss_image_warning();
            }
            return;
        }
        if self.input_mode == InputMode::FileExplorer {
            self.handle_explorer_mouse(mouse);
            return;
        }
        if self.screen == Screen::Workshop {
            self.handle_workshop_mouse(mouse);
            return;
        }
        if self.screen == Screen::Github {
            self.handle_github_mouse(mouse);
            return;
        }
        if self.screen == Screen::Git {
            self.handle_git_mouse(mouse);
            return;
        }
        if self.screen != Screen::Main
            || matches!(
                self.input_mode,
                InputMode::PathPrompt | InputMode::SettingsEdit
            )
        {
            return;
        }
        let col = mouse.column;
        let row = mouse.row;
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let dir: isize = if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                    -1
                } else {
                    1
                };
                let step = self.accelerate_wheel(dir);
                if point_in(self.issues_area, col, row) {
                    let lines = wheel_pane_lines(self.scroll_streak);
                    if dir < 0 {
                        self.warning_scroll = self.warning_scroll.saturating_sub(lines);
                    } else {
                        self.warning_scroll = self.warning_scroll.saturating_add(lines);
                    }
                } else if point_in(self.info_area, col, row) {
                    let lines = wheel_pane_lines(self.scroll_streak);
                    if dir < 0 {
                        self.info_scroll = self.info_scroll.saturating_sub(lines);
                    } else {
                        self.info_scroll = self.info_scroll.saturating_add(lines);
                    }
                } else if point_in(self.inactive_area, col, row) {
                    self.pane = Pane::Inactive;
                    self.scroll_sel(dir * step);
                } else if point_in(self.active_area, col, row) {
                    self.pane = Pane::Active;
                    self.scroll_sel(dir * step);
                } else {
                    self.scroll_sel(dir * step);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if point_in(self.inactive_area, col, row) {
                    self.pane = Pane::Inactive;
                    let len = self.filtered_inactive().len();
                    if let Some(i) =
                        list_index_at(self.inactive_area, self.inactive_state.offset(), row, len)
                    {
                        self.selected_inactive = i;
                        self.clamp_selection();
                    }
                } else if point_in(self.active_area, col, row) {
                    self.pane = Pane::Active;
                    let len = self.filtered_active().len();
                    if let Some(i) =
                        list_index_at(self.active_area, self.active_state.offset(), row, len)
                    {
                        self.selected_active = i;
                        self.clamp_selection();
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.search.clear();
                self.input_mode = InputMode::Normal;
                self.clamp_selection();
            }
            KeyCode::Enter => {
                self.input_mode = InputMode::Normal;
                self.toggle_selected();
            }
            KeyCode::Backspace => {
                self.search.pop();
                self.selected_inactive = 0;
                self.selected_active = 0;
                self.clamp_selection();
            }
            KeyCode::Char(c) => {
                self.search.push(c);
                self.selected_inactive = 0;
                self.selected_active = 0;
                self.clamp_selection();
            }
            _ => {}
        }
    }

    fn handle_file_explorer_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_explorer(),
            KeyCode::Enter | KeyCode::Right => self.confirm_explorer(),
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => self.file_browser.go_up(),
            KeyCode::Down | KeyCode::Char('j') => self.file_browser.move_sel(1),
            KeyCode::Up | KeyCode::Char('k') => self.file_browser.move_sel(-1),
            KeyCode::PageDown => self.file_browser.move_sel(10),
            KeyCode::PageUp => self.file_browser.move_sel(-10),
            _ => {}
        }
    }

    fn handle_explorer_mouse(&mut self, mouse: MouseEvent) {
        let col = mouse.column;
        let row = mouse.row;
        if !point_in(self.explorer_area, col, row) {
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let dir: isize = if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                    -1
                } else {
                    1
                };
                let step = self.accelerate_wheel(dir);
                self.file_browser.scroll_sel(dir * step);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let len = self.file_browser.entries.len();
                if let Some(i) = list_index_at(
                    self.explorer_area,
                    self.file_browser.state.offset(),
                    row,
                    len,
                ) {
                    self.file_browser.select_index(i);
                }
            }
            _ => {}
        }
    }

    fn handle_path_prompt_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.edit_buffer.clear();
                self.status = "Cancelled".into();
            }
            KeyCode::Enter => self.finish_path_prompt(),
            KeyCode::Backspace => {
                self.edit_buffer.pop();
            }
            KeyCode::Char(c) => self.edit_buffer.push(c),
            _ => {}
        }
    }

    fn handle_settings_edit_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.edit_buffer.clear();
            }
            KeyCode::Enter => {
                self.session.settings
                    .set_text_field(self.settings_field, self.edit_buffer.trim());
                self.input_mode = InputMode::Normal;
                self.edit_buffer.clear();
            }
            KeyCode::Backspace => {
                self.edit_buffer.pop();
            }
            KeyCode::Char(c) => self.edit_buffer.push(c),
            _ => {}
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.screen = Screen::Main;
            }
            KeyCode::Char('s') => self.save_settings(),
            KeyCode::Up | KeyCode::Char('k') => {
                self.settings_field = self.settings_field.prev();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.settings_field = self.settings_field.next();
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                if self.settings_field.is_cycle() {
                    self.session.settings.cycle_field(self.settings_field);
                } else {
                    self.edit_buffer = self.settings.field_edit_value(self.settings_field);
                    self.input_mode = InputMode::SettingsEdit;
                }
            }
            _ => {}
        }
    }

    fn handle_main_key(&mut self, key: KeyEvent) {
        let refresh_armed = std::mem::take(&mut self.confirm_refresh);
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('j') => self.reorder_active(1),
                KeyCode::Char('k') => self.reorder_active(-1),
                KeyCode::Char('c') => self.should_quit = true,
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Tab => {
                self.pane = match self.pane {
                    Pane::Inactive => Pane::Active,
                    Pane::Active => Pane::Inactive,
                };
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(-1),
            KeyCode::PageDown => self.move_sel(10),
            KeyCode::PageUp => self.move_sel(-10),
            KeyCode::Enter => self.toggle_selected(),
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Search;
            }
            KeyCode::Char('s') => self.sort_list(),
            KeyCode::Char('S') => self.cycle_sort_mode(),
            KeyCode::Char('w') => self.save_mods_config(),
            KeyCode::Char('l') | KeyCode::Char('i') => self.start_import(),
            KeyCode::Char('e') => self.start_export(),
            KeyCode::Char('g') => self.launch_rimworld(),
            KeyCode::Char('t') => self.toggle_user_load_top(),
            KeyCode::Char('b') => self.toggle_user_load_bottom(),
            KeyCode::Char('d') => self.start_fetch(false),
            KeyCode::Char('n') => self.open_workshop_search(),
            KeyCode::Char('h') => self.open_github(),
            KeyCode::Char('G') => self.open_git(),
            KeyCode::Char('u') => self.update_selected_mod(),
            KeyCode::Char('r') if self.session.dirty && !refresh_armed => {
                self.confirm_refresh = true;
                self.session.status = "Unsaved changes: press r again to discard them".into();
            }
            KeyCode::Char('r') => self.reload(),
            KeyCode::Char('p') => {
                self.screen = Screen::Settings;
            }
            KeyCode::Char('?') => {
                self.screen = Screen::Help;
            }
            KeyCode::Char('[') => {
                self.warning_scroll = self.warning_scroll.saturating_sub(1);
            }
            KeyCode::Char(']') => {
                self.warning_scroll = self.warning_scroll.saturating_add(1);
            }
            _ => {}
        }
    }

    fn handle_workshop_query_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                if self.workshop_items.is_empty() {
                    self.close_workshop();
                } else {
                    self.input_mode = InputMode::Normal;
                }
            }
            KeyCode::Enter => self.start_workshop_search(),
            KeyCode::Backspace => {
                self.workshop_query.pop();
            }
            KeyCode::Char(c) => self.workshop_query.push(c),
            _ => {}
        }
    }

    fn handle_workshop_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_workshop(),
            KeyCode::Char('/') | KeyCode::Char('n') => {
                self.input_mode = InputMode::WorkshopQuery;
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_workshop_sel(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_workshop_sel(-1),
            KeyCode::PageDown => self.move_workshop_sel(10),
            KeyCode::PageUp => self.move_workshop_sel(-10),
            KeyCode::Enter => self.queue_workshop_download(true),
            KeyCode::Char(' ') => self.queue_workshop_download(false),
            KeyCode::Char('u') => self.queue_workshop_download(false),
            _ => {}
        }
    }

    fn move_workshop_sel(&mut self, delta: isize) {
        let len = self.workshop_items.len();
        if len == 0 {
            return;
        }
        let next = (self.workshop_selected as isize + delta).rem_euclid(len as isize) as usize;
        self.workshop_selected = next;
        self.workshop_state.select(Some(next));
    }

    fn handle_workshop_mouse(&mut self, mouse: MouseEvent) {
        if self.input_mode == InputMode::WorkshopQuery {
            return;
        }
        let col = mouse.column;
        let row = mouse.row;
        if !point_in(self.workshop_area, col, row) {
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let dir: isize = if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                    -1
                } else {
                    1
                };
                let step = self.accelerate_wheel(dir);
                self.scroll_workshop_sel(dir * step);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let len = self.workshop_items.len();
                if let Some(i) = list_index_at(
                    self.workshop_area,
                    self.workshop_state.offset(),
                    row,
                    len,
                ) {
                    self.workshop_selected = i;
                    self.workshop_state.select(Some(i));
                }
            }
            _ => {}
        }
    }

    fn scroll_workshop_sel(&mut self, delta: isize) {
        let len = self.workshop_items.len();
        if len == 0 {
            return;
        }
        let next = (self.workshop_selected as isize + delta).clamp(0, len as isize - 1) as usize;
        self.workshop_selected = next;
        self.workshop_state.select(Some(next));
    }

    fn handle_github_query_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                if self.github_mods.mods.is_empty() {
                    self.close_github();
                } else {
                    self.input_mode = InputMode::Normal;
                }
            }
            KeyCode::Enter => self.start_github_install(),
            KeyCode::Backspace => {
                self.github_query.pop();
            }
            KeyCode::Char(c) => self.github_query.push(c),
            _ => {}
        }
    }

    fn handle_github_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_github(),
            KeyCode::Char('/') | KeyCode::Char('h') => {
                self.input_mode = InputMode::GithubQuery;
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_github_sel(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_github_sel(-1),
            KeyCode::PageDown => self.move_github_sel(10),
            KeyCode::PageUp => self.move_github_sel(-10),
            KeyCode::Enter | KeyCode::Char('u') => self.start_github_update_selected(),
            KeyCode::Char('c') => self.start_github_check(true),
            KeyCode::Char('a') => self.toggle_github_auto_update(),
            KeyCode::Char('X') => self.uninstall_github_selected(),
            _ => {}
        }
    }

    fn move_github_sel(&mut self, delta: isize) {
        let len = self.github_mods.mods.len();
        if len == 0 {
            return;
        }
        let next = (self.github_selected as isize + delta).rem_euclid(len as isize) as usize;
        self.github_selected = next;
        self.github_state.select(Some(next));
    }

    fn handle_github_mouse(&mut self, mouse: MouseEvent) {
        if self.input_mode == InputMode::GithubQuery {
            return;
        }
        let col = mouse.column;
        let row = mouse.row;
        if !point_in(self.github_area, col, row) {
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let dir: isize = if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                    -1
                } else {
                    1
                };
                let step = self.accelerate_wheel(dir);
                self.scroll_github_sel(dir * step);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let len = self.github_mods.mods.len();
                if let Some(i) = list_index_at(
                    self.github_area,
                    self.github_state.offset(),
                    row,
                    len,
                ) {
                    self.github_selected = i;
                    self.github_state.select(Some(i));
                }
            }
            _ => {}
        }
    }

    fn scroll_github_sel(&mut self, delta: isize) {
        let len = self.github_mods.mods.len();
        if len == 0 {
            return;
        }
        let next = (self.github_selected as isize + delta).clamp(0, len as isize - 1) as usize;
        self.github_selected = next;
        self.github_state.select(Some(next));
    }

    fn handle_git_query_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                if self.git_items.is_empty() {
                    self.close_git();
                } else {
                    self.input_mode = InputMode::Normal;
                }
            }
            KeyCode::Enter => self.start_git_clone(),
            KeyCode::Backspace => {
                self.git_query.pop();
            }
            KeyCode::Char(c) => self.git_query.push(c),
            _ => {}
        }
    }

    fn handle_git_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_git(),
            KeyCode::Char('/') | KeyCode::Char('G') => {
                self.input_mode = InputMode::GitQuery;
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_git_sel(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_git_sel(-1),
            KeyCode::PageDown => self.move_git_sel(10),
            KeyCode::PageUp => self.move_git_sel(-10),
            KeyCode::Enter | KeyCode::Char('u') => self.start_git_pull_selected(),
            KeyCode::Char('c') => self.start_git_check(),
            KeyCode::Char('A') => self.start_git_pull_updates(),
            KeyCode::Char('X') => self.uninstall_git_selected(),
            _ => {}
        }
    }

    fn move_git_sel(&mut self, delta: isize) {
        let len = self.git_items.len();
        if len == 0 {
            return;
        }
        let next = (self.git_selected as isize + delta).rem_euclid(len as isize) as usize;
        self.git_selected = next;
        self.git_state.select(Some(next));
    }

    fn handle_git_mouse(&mut self, mouse: MouseEvent) {
        if self.input_mode == InputMode::GitQuery {
            return;
        }
        let col = mouse.column;
        let row = mouse.row;
        if !point_in(self.git_area, col, row) {
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let dir: isize = if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                    -1
                } else {
                    1
                };
                let step = self.accelerate_wheel(dir);
                self.scroll_git_sel(dir * step);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let len = self.git_items.len();
                if let Some(i) = list_index_at(self.git_area, self.git_state.offset(), row, len) {
                    self.git_selected = i;
                    self.git_state.select(Some(i));
                }
            }
            _ => {}
        }
    }

    fn scroll_git_sel(&mut self, delta: isize) {
        let len = self.git_items.len();
        if len == 0 {
            return;
        }
        let next = (self.git_selected as isize + delta).clamp(0, len as isize - 1) as usize;
        self.git_selected = next;
        self.git_state.select(Some(next));
    }
}

pub fn run() -> anyhow::Result<()> {
    session::init_tracing();
    let settings = Settings::load().unwrap_or_else(|e| {
        tracing::warn!("settings load failed: {e}");
        Settings::default().with_autodetect()
    });
    let mut app = App::new(settings);
    app.picker = Some(preview::init_picker());

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    loop {
        app.poll();
        app.tick_spinner();
        terminal
            .draw(|frame| ui::draw(frame, app))
            .context("draw")?;
        if event::poll(Duration::from_millis(50))? {
            loop {
                match event::read()? {
                    Event::Key(key) => app.handle_key(key),
                    Event::Mouse(mouse) => app.handle_mouse(mouse),
                    Event::Resize(_, _) => {}
                    _ => {}
                }
                if app.should_quit || !event::poll(Duration::from_millis(0))? {
                    break;
                }
            }
        }
        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn point_in(area: Rect, col: u16, row: u16) -> bool {
    col >= area.x
        && col < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn list_index_at(area: Rect, offset: usize, row: u16, len: usize) -> Option<usize> {
    if row <= area.y || row + 1 >= area.y.saturating_add(area.height) {
        return None;
    }
    let inner = usize::from(row.saturating_sub(area.y.saturating_add(1)));
    let idx = offset.saturating_add(inner);
    (idx < len).then_some(idx)
}

/// Precise for the first two notches, then ramps up while the wheel keeps moving.
fn wheel_list_lines(streak: u32) -> isize {
    match streak {
        0..=2 => 1,
        3 | 4 => 3,
        5 | 6 => 8,
        7 | 8 => 16,
        _ => 28,
    }
}

fn wheel_pane_lines(streak: u32) -> u16 {
    match streak {
        0..=2 => 1,
        3 | 4 => 2,
        5 | 6 => 4,
        _ => 8,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_cycles_through_frames() {
        assert_eq!(SPINNER_FRAMES[0], "⠋");
        assert_eq!(SPINNER_FRAMES.len(), 10);
        assert_eq!(SPINNER_FRAMES[10 % SPINNER_FRAMES.len()], "⠋");
        assert_ne!(SPINNER_FRAMES[1], SPINNER_FRAMES[0]);
    }

    #[test]
    fn first_wheel_notches_are_precise() {
        assert_eq!(wheel_list_lines(1), 1);
        assert_eq!(wheel_list_lines(2), 1);
        assert_eq!(wheel_pane_lines(1), 1);
        assert_eq!(wheel_pane_lines(2), 1);
    }

    #[test]
    fn continued_wheeling_accelerates() {
        assert!(wheel_list_lines(4) > wheel_list_lines(2));
        assert!(wheel_list_lines(8) > wheel_list_lines(4));
        assert!(wheel_list_lines(12) > wheel_list_lines(8));
        assert!(wheel_pane_lines(8) > wheel_pane_lines(2));
    }
}
