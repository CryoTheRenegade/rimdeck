//! rimtui-gui — RimWorld mod manager GUI (GPL-3.0-only), built on the shared rimtui core.
//!
//! Inspired by RimSort (https://github.com/RimSort/RimSort).

mod manage;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use iced::keyboard::{self, key::Named, Key};
use iced::widget::operation::{self, RelativeOffset};
use iced::widget::{
    button, column, container, image, mouse_area, pick_list, row, rule, scrollable, space, text,
    text_input,
};
use iced::{Color, Element, Length, Subscription, Task, Theme};

use rimtui_core::issues::Severity;
use rimtui_core::model::{Mod, ModSource};
use rimtui_core::session::{self, Session};
use rimtui_core::settings::{Settings, SettingsField, SortMode};

const ERROR_COLOR: Color = Color::from_rgb(0.95, 0.35, 0.35);
const WARN_COLOR: Color = Color::from_rgb(0.95, 0.8, 0.3);
const DIM_COLOR: Color = Color::from_rgb(0.6, 0.6, 0.6);
const INACTIVE_LIST: &str = "inactive-list";
const ACTIVE_LIST: &str = "active-list";

fn main() -> iced::Result {
    session::init_tracing();
    iced::application(Gui::boot, Gui::update, Gui::view)
        .title(Gui::title)
        .subscription(Gui::subscription)
        .theme(|_: &Gui| Theme::Dark)
        .window_size((1400.0, 860.0))
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Mods,
    Workshop,
    Github,
    Git,
    Settings,
}

impl View {
    const TABS: [(View, &'static str); 5] = [
        (View::Mods, "Mods"),
        (View::Workshop, "Workshop"),
        (View::Github, "GitHub"),
        (View::Git, "Git"),
        (View::Settings, "Settings"),
    ];
}

struct Gui {
    session: Session,
    view: View,
    selected: Option<PathBuf>,
    inactive_search: String,
    active_search: String,
    /// Text being edited on the settings page, one per `SettingsField`.
    settings_draft: HashMap<SettingsField, String>,
    previews: HashMap<PathBuf, Option<image::Handle>>,
    /// Refresh was pressed once with unsaved edits; the next press discards them.
    confirm_refresh: bool,
    /// Folder armed for deletion; a second Delete press on the same row removes it.
    confirm_delete: Option<PathBuf>,
    workshop_query: String,
    github_query: String,
    git_query: String,
    /// packageId typed into the rule editor in the info panel.
    rule_input: String,
}

/// Keyboard shortcuts on the Mods tab.
#[derive(Debug, Clone, Copy)]
enum KeyCmd {
    /// Up/Down: select the previous/next mod in the selected mod's list.
    Step(isize),
    /// Ctrl+Up/Down: move the selected active mod.
    Move(isize),
    /// Enter: activate or deactivate the selected mod.
    Toggle,
    /// Ctrl+S: save ModsConfig.xml.
    Save,
}

fn key_cmd(event: keyboard::Event) -> Option<KeyCmd> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    let ctrl = modifiers.command();
    match key.as_ref() {
        Key::Named(Named::ArrowUp) if ctrl => Some(KeyCmd::Move(-1)),
        Key::Named(Named::ArrowDown) if ctrl => Some(KeyCmd::Move(1)),
        Key::Named(Named::ArrowUp) => Some(KeyCmd::Step(-1)),
        Key::Named(Named::ArrowDown) => Some(KeyCmd::Step(1)),
        Key::Named(Named::Enter) => Some(KeyCmd::Toggle),
        Key::Character("s") if ctrl => Some(KeyCmd::Save),
        _ => None,
    }
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Key(KeyCmd),
    /// Enter in a mod search box: select the first match (`true` = active list).
    SearchSubmit(bool),
    BuildDb,
    Textures { clean: bool },
    Select(PathBuf),
    Toggle(PathBuf),
    InactiveSearch(String),
    ActiveSearch(String),
    MoveSelected(isize),
    Sort,
    SortMode(SortMode),
    Save,
    Refresh,
    Clear,
    Launch,
    FetchDbs,
    UpdateSelected,
    ToggleLoadTop,
    ToggleLoadBottom,
    RuleInput(String),
    RuleAdd { before: bool },
    RuleRemove { other: String, before: bool },
    Nav(View),
    SettingsEdit(SettingsField, String),
    SaveSettings,
    ImportList,
    ExportList,
    ImportPicked(Option<PathBuf>),
    ExportPicked(Option<PathBuf>),
    WorkshopQuery(String),
    WorkshopSearch,
    WorkshopCollection,
    WorkshopDownload {
        id: String,
        title: String,
        activate: bool,
    },
    GithubQuery(String),
    GithubInstall,
    GithubCheck,
    GithubUpdate(PathBuf),
    GithubToggleAuto(PathBuf),
    GithubDelete(PathBuf),
    GitQuery(String),
    GitClone,
    GitCheck,
    GitPullBehind,
    GitPull(PathBuf),
    GitDelete(PathBuf),
}

impl Gui {
    fn boot() -> (Self, Task<Message>) {
        let settings = Settings::load().unwrap_or_else(|e| {
            tracing::warn!("settings load failed: {e}");
            Settings::default().with_autodetect()
        });
        let mut session = Session::new(settings);
        let view = if session.start() {
            View::Mods
        } else {
            View::Settings
        };
        let mut gui = Self {
            session,
            view: View::Mods,
            selected: None,
            inactive_search: String::new(),
            active_search: String::new(),
            settings_draft: HashMap::new(),
            previews: HashMap::new(),
            confirm_refresh: false,
            confirm_delete: None,
            workshop_query: String::new(),
            github_query: String::new(),
            git_query: String::new(),
            rule_input: String::new(),
        };
        if view == View::Settings {
            gui.open_settings();
        }
        (gui, Task::none())
    }

    fn title(&self) -> String {
        let dirty = if self.session.dirty { " *" } else { "" };
        format!("rimtui{dirty}")
    }

    fn subscription(&self) -> Subscription<Message> {
        // Only keys no widget used (a focused text box keeps its own Enter/arrows).
        let keys = keyboard::listen().filter_map(|e| key_cmd(e).map(Message::Key));
        if self.session.is_busy() {
            Subscription::batch([
                keys,
                iced::time::every(Duration::from_millis(100)).map(|_| Message::Tick),
            ])
        } else {
            keys
        }
    }

    fn open_settings(&mut self) {
        let s = &self.session.settings;
        self.settings_draft = SettingsField::ALL
            .iter()
            .filter(|f| !f.is_cycle())
            .map(|&f| (f, s.field_edit_value(f)))
            .collect();
        self.view = View::Settings;
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ImportList => Task::perform(
                rfd::AsyncFileDialog::new()
                    .add_filter("Mod list", &["xml"])
                    .set_directory(Settings::default_lists_dir())
                    .pick_file(),
                |f| Message::ImportPicked(f.map(|f| f.path().to_path_buf())),
            ),
            Message::ExportList => Task::perform(
                rfd::AsyncFileDialog::new()
                    .add_filter("Mod list", &["xml"])
                    .set_directory(Settings::default_lists_dir())
                    .set_file_name("modlist.xml")
                    .save_file(),
                |f| Message::ExportPicked(f.map(|f| f.path().to_path_buf())),
            ),
            Message::Key(cmd) => {
                if self.view != View::Mods {
                    return Task::none();
                }
                match cmd {
                    KeyCmd::Step(delta) => self.step(delta),
                    KeyCmd::Move(delta) => self.apply(Message::MoveSelected(delta)),
                    KeyCmd::Toggle => {
                        if let Some(p) = self.selected.clone() {
                            self.apply(Message::Toggle(p));
                        }
                    }
                    KeyCmd::Save => self.apply(Message::Save),
                }
                self.reveal()
            }
            Message::SearchSubmit(active) => {
                let first = self.visible(active).into_iter().next();
                if let Some(p) = first {
                    self.select(p);
                }
                self.reveal()
            }
            other => {
                let reveal = matches!(other, Message::MoveSelected(_));
                self.apply(other);
                if reveal {
                    self.reveal()
                } else {
                    Task::none()
                }
            }
        }
    }

    /// The filtered inactive (`false`) or active (`true`) list as shown.
    fn visible(&self, active: bool) -> Vec<PathBuf> {
        let s = &self.session;
        if active {
            s.filtered_active(&self.active_search)
        } else {
            s.filtered_inactive(&self.inactive_search)
        }
    }

    /// Select the mod `delta` rows away in the selected mod's list (first active mod if none).
    fn step(&mut self, delta: isize) {
        let current = self.selected.as_ref().and_then(|p| {
            [false, true].into_iter().find_map(|active| {
                let list = self.visible(active);
                let i = list.iter().position(|x| x == p)?;
                Some((list, i))
            })
        });
        let next = match current {
            Some((list, i)) => {
                let j = (i as isize + delta).clamp(0, list.len() as isize - 1) as usize;
                list.into_iter().nth(j)
            }
            None => self
                .visible(true)
                .into_iter()
                .next()
                .or_else(|| self.visible(false).into_iter().next()),
        };
        if let Some(p) = next {
            self.select(p);
        }
    }

    /// Scroll the list holding the selected mod so the selection is in view.
    fn reveal(&self) -> Task<Message> {
        let Some(p) = &self.selected else {
            return Task::none();
        };
        for (active, id) in [(false, INACTIVE_LIST), (true, ACTIVE_LIST)] {
            let list = self.visible(active);
            if let Some(i) = list.iter().position(|x| x == p) {
                // Snapping to index/(len-1) keeps that row inside the viewport for any row height.
                let y = i as f32 / list.len().saturating_sub(1).max(1) as f32;
                return operation::snap_to(id, RelativeOffset { x: None, y: Some(y) });
            }
        }
        Task::none()
    }

    fn apply(&mut self, message: Message) {
        if !matches!(message, Message::Refresh | Message::Tick) {
            self.confirm_refresh = false;
        }
        let s = &mut self.session;
        match message {
            Message::Key(_) | Message::SearchSubmit(_) => {} // handled in `update`
            Message::BuildDb => s.build_db(),
            Message::Textures { clean } => s.optimize_textures(clean),
            Message::Tick => {
                for ev in s.poll() {
                    match ev {
                        session::Event::Reloaded => {
                            self.previews.clear();
                            if let Some(p) = self.selected.take() {
                                if self.session.mod_ref(&p).is_some() {
                                    self.select(p);
                                }
                            }
                        }
                        session::Event::GithubInstalled => self.github_query.clear(),
                        session::Event::GitCloned => self.git_query.clear(),
                        session::Event::WorkshopResults => {}
                    }
                }
            }
            Message::ImportList | Message::ExportList => {} // handled in `update`
            Message::ImportPicked(Some(p)) => {
                s.load_list(&p);
            }
            Message::ExportPicked(Some(p)) => s.export_list(&p),
            Message::ImportPicked(None) | Message::ExportPicked(None) => {}
            Message::Select(p) => self.select(p),
            Message::Toggle(p) => {
                s.toggle(&p);
                self.select(p);
            }
            Message::InactiveSearch(q) => self.inactive_search = q,
            Message::ActiveSearch(q) => self.active_search = q,
            Message::MoveSelected(delta) => {
                if let Some(p) = &self.selected {
                    s.move_active(p, delta);
                }
            }
            Message::Sort => s.sort(),
            Message::SortMode(mode) => s.set_sort_mode(mode),
            Message::Save => s.save_mods_config(),
            Message::Refresh => {
                if s.dirty && !self.confirm_refresh {
                    self.confirm_refresh = true;
                    s.status = "Unsaved changes: press Refresh again to discard them".into();
                } else {
                    self.confirm_refresh = false;
                    s.reload();
                }
            }
            Message::Clear => s.clear_active(),
            Message::Launch => s.launch(),
            Message::FetchDbs => s.start_fetch(false),
            Message::UpdateSelected => {
                if let Some(p) = &self.selected {
                    s.update_mod(p);
                }
            }
            Message::ToggleLoadTop | Message::ToggleLoadBottom => {
                let Some(pid) = self
                    .selected
                    .as_ref()
                    .and_then(|p| s.mod_ref(p))
                    .map(|m| m.package_id.clone())
                else {
                    return;
                };
                if matches!(message, Message::ToggleLoadTop) {
                    s.toggle_user_load_top(&pid);
                } else {
                    s.toggle_user_load_bottom(&pid);
                }
            }
            Message::RuleInput(v) => self.rule_input = v,
            Message::RuleAdd { before } => {
                if let Some(pid) = self.selected_package_id() {
                    let other = std::mem::take(&mut self.rule_input);
                    self.session.set_user_order_rule(&pid, &other, before, true);
                }
            }
            Message::RuleRemove { other, before } => {
                if let Some(pid) = self.selected_package_id() {
                    self.session.set_user_order_rule(&pid, &other, before, false);
                }
            }
            Message::Nav(View::Settings) => self.open_settings(),
            Message::Nav(view) => {
                self.confirm_delete = None;
                self.view = view;
                match view {
                    View::Github => s.reload_github_cache(),
                    View::Git => s.refresh_git_items(),
                    _ => {}
                }
            }
            Message::SettingsEdit(field, value) => {
                self.settings_draft.insert(field, value);
            }
            Message::SaveSettings => {
                // Only edited fields, decided before any setter runs: setting Game install fills a
                // blank Local mods, which an unchanged blank draft must not then clear.
                let edited: Vec<(SettingsField, String)> = self
                    .settings_draft
                    .iter()
                    .filter(|(&f, v)| **v != s.settings.field_edit_value(f))
                    .map(|(&f, v)| (f, v.clone()))
                    .collect();
                // Switch instance first so path edits in the same save apply to the new one.
                let (instance, rest): (Vec<_>, Vec<_>) = edited
                    .into_iter()
                    .partition(|(f, _)| *f == SettingsField::Instance);
                for (field, value) in instance.into_iter().chain(rest) {
                    s.settings.set_text_field(field, &value);
                }
                if s.save_settings() {
                    self.view = View::Mods;
                }
            }
            other => self.apply_manage(other),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let tabs = View::TABS.iter().map(|&(view, label)| {
            button(label)
                .on_press(Message::Nav(view))
                .style(if view == self.view {
                    button::primary
                } else {
                    button::text
                })
                .into()
        });
        let body = match self.view {
            View::Mods => self.main_view(),
            View::Workshop => self.workshop_view(),
            View::Github => self.github_view(),
            View::Git => self.git_view(),
            View::Settings => self.settings_view(),
        };
        column![
            row(tabs).spacing(4).padding([4, 8]),
            rule::horizontal(1),
            body,
            self.status_bar()
        ]
        .into()
    }

    // ---- main window ------------------------------------------------------

    fn main_view(&self) -> Element<'_, Message> {
        let s = &self.session;
        let toolbar = row![
            button(if self.confirm_refresh {
                "Discard & refresh"
            } else {
                "Refresh"
            })
            .on_press(Message::Refresh)
            .style(if self.confirm_refresh {
                button::danger
            } else {
                button::primary
            }),
            button("Sort").on_press(Message::Sort),
            pick_list(&SortMode::ALL[..], Some(s.settings.sort_mode), Message::SortMode),
            button("Save").on_press(Message::Save).style(if s.dirty {
                button::success
            } else {
                button::primary
            }),
            button("Clear").on_press(Message::Clear).style(button::secondary),
            button("Import list…")
                .on_press(Message::ImportList)
                .style(button::secondary),
            button("Export list…")
                .on_press(Message::ExportList)
                .style(button::secondary),
            space::horizontal(),
            button("Download DBs")
                .on_press(Message::FetchDbs)
                .style(button::secondary),
            button("Build DB")
                .on_press(Message::BuildDb)
                .style(button::secondary),
            button("Optimize textures")
                .on_press(Message::Textures { clean: false })
                .style(button::secondary),
            button("Delete DDS")
                .on_press(Message::Textures { clean: true })
                .style(button::secondary),
            button("Run game").on_press(Message::Launch),
        ]
        .spacing(8)
        .padding(8);

        let inactive = s.filtered_inactive(&self.inactive_search);
        let active = s.filtered_active(&self.active_search);
        let lists = row![
            container(self.info_panel()).width(Length::FillPortion(3)),
            self.mod_list(
                format!("Inactive ({})", s.inactive_paths.len()),
                &self.inactive_search,
                Message::InactiveSearch,
                &inactive,
                false,
            ),
            self.mod_list(
                format!("Active ({})", s.active_paths.len()),
                &self.active_search,
                Message::ActiveSearch,
                &active,
                true,
            ),
        ]
        .spacing(8)
        .padding([0, 8])
        .height(Length::FillPortion(4));

        column![
            toolbar,
            lists,
            rule::horizontal(1),
            self.issues_panel().height(Length::FillPortion(1)),
        ]
        .height(Length::Fill)
        .into()
    }

    fn mod_list<'a>(
        &'a self,
        title: String,
        query: &'a str,
        on_search: fn(String) -> Message,
        paths: &[PathBuf],
        active: bool,
    ) -> Element<'a, Message> {
        let rows = paths.iter().filter_map(|p| {
            let m = self.session.mod_ref(p)?;
            Some(self.mod_row(m, active))
        });
        let mut header = row![text(title).size(16), space::horizontal()].spacing(4);
        if active {
            header = header
                .push(button("▲").on_press(Message::MoveSelected(-1)).padding([2, 8]))
                .push(button("▼").on_press(Message::MoveSelected(1)).padding([2, 8]));
        }
        column![
            header,
            text_input("Search name, packageId, author…", query)
                .on_input(on_search)
                .on_submit(Message::SearchSubmit(active))
                .padding(6),
            container(
                scrollable(column(rows).spacing(1))
                    .id(if active { ACTIVE_LIST } else { INACTIVE_LIST })
                    .height(Length::Fill)
            )
                .style(container::bordered_box)
                .padding(2)
                .height(Length::Fill),
        ]
        .spacing(6)
        .width(Length::FillPortion(3))
        .into()
    }

    fn mod_row<'a>(&'a self, m: &'a Mod, active: bool) -> Element<'a, Message> {
        let selected = self.selected.as_deref() == Some(m.path.as_path());
        let severity = active
            .then(|| self.worst_issue(&m.package_id))
            .flatten();
        let name_color = match severity {
            Some(Severity::Error) => Some(ERROR_COLOR),
            Some(Severity::Warning) => Some(WARN_COLOR),
            None => None,
        };
        let label = row![
            text(source_tag(m.source)).size(11).color(DIM_COLOR).width(28),
            text(&m.name).size(14).color_maybe(name_color),
        ]
        .spacing(6);
        // Not a button: a button captures the press, so MouseArea would never see the double-click.
        let row_box = container(label)
            .width(Length::Fill)
            .padding([3, 6])
            .style(if selected {
                container::primary
            } else {
                container::transparent
            });
        mouse_area(row_box)
            .interaction(iced::mouse::Interaction::Pointer)
            .on_press(Message::Select(m.path.clone()))
            .on_double_click(Message::Toggle(m.path.clone()))
            .into()
    }

    fn worst_issue(&self, package_id: &str) -> Option<Severity> {
        let mut worst = None;
        for issue in self.session.issues.iter().filter(|i| i.package_id == package_id) {
            if issue.severity == Severity::Error {
                return Some(Severity::Error);
            }
            worst = Some(issue.severity);
        }
        worst
    }

    fn info_panel(&self) -> Element<'_, Message> {
        let s = &self.session;
        let Some(m) = self.selected.as_ref().and_then(|p| s.mod_ref(p)) else {
            return container(text("Select a mod. Double-click to activate or deactivate.").color(DIM_COLOR))
                .padding(12)
                .style(container::bordered_box)
                .height(Length::Fill)
                .width(Length::Fill)
                .into();
        };
        let mut col = column![].spacing(6).padding(12);
        if let Some(handle) = self.preview(m) {
            col = col.push(image(handle).width(Length::Fill).height(200));
        }
        col = col
            .push(text(&m.name).size(20))
            .push(field("Author", m.author_line()))
            .push(field("Package ID", m.package_id.clone()))
            .push(field("Source", m.source.label().to_string()))
            .push(field("Path", m.path.display().to_string()));
        if !m.mod_version.is_empty() {
            col = col.push(field("Version", m.mod_version.clone()));
        }
        if !m.supported_versions.is_empty() {
            col = col.push(field("Supports", m.supported_versions.join(", ")));
        }
        if let Some(pfid) = &m.published_file_id {
            col = col.push(field("Workshop ID", pfid.clone()));
        }
        if !m.dependencies.is_empty() {
            let deps = m
                .dependencies
                .iter()
                .map(|d| d.display_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            col = col.push(field("Requires", deps));
        }
        let mut flags = Vec::new();
        if m.load_first {
            flags.push("loadTop");
        }
        if m.load_last {
            flags.push("loadBottom");
        }
        if !flags.is_empty() {
            col = col.push(field("Rules", flags.join(", ")));
        }
        let is_active = s.is_active(&m.path);
        col = col.push(
            row![
                button(if is_active { "Deactivate" } else { "Activate" })
                    .on_press(Message::Toggle(m.path.clone())),
                button("Update")
                    .on_press(Message::UpdateSelected)
                    .style(button::secondary),
                button(if s.user_rules.load_top(&m.package_id) {
                    "Unset loadTop"
                } else {
                    "loadTop"
                })
                .on_press(Message::ToggleLoadTop)
                .style(button::secondary),
                button(if s.user_rules.load_bottom(&m.package_id) {
                    "Unset loadBottom"
                } else {
                    "loadBottom"
                })
                .on_press(Message::ToggleLoadBottom)
                .style(button::secondary),
            ]
            .spacing(6)
            .wrap(),
        );
        col = col.push(self.rules_editor(m));
        col = col.push(rule::horizontal(1)).push(text(&m.description).size(13));
        container(scrollable(col))
            .style(container::bordered_box)
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
    }

    fn selected_package_id(&self) -> Option<String> {
        let p = self.selected.as_ref()?;
        self.session.mod_ref(p).map(|m| m.package_id.clone())
    }

    /// Community rules (read-only) and user rules (removable) plus an add row.
    fn rules_editor<'a>(&'a self, m: &'a Mod) -> Element<'a, Message> {
        let s = &self.session;
        let (c_after, c_before) = s.community_rules.order_rules(&m.package_id);
        let (u_after, u_before) = s.user_rules.order_rules(&m.package_id);
        let mut col = column![text("Load order rules").size(15)].spacing(4);
        for (ids, before) in [(c_after, false), (c_before, true)] {
            for id in ids {
                let kind = if before { "before" } else { "after" };
                col = col.push(text(format!("community: load {kind} {id}")).size(12).color(DIM_COLOR));
            }
        }
        for (ids, before) in [(u_after, false), (u_before, true)] {
            for id in ids {
                let kind = if before { "before" } else { "after" };
                col = col.push(
                    row![
                        text(format!("yours: load {kind} {id}")).size(13),
                        button("✕")
                            .on_press(Message::RuleRemove { other: id, before })
                            .padding([0, 6])
                            .style(button::text),
                    ]
                    .spacing(6)
                    .align_y(iced::Alignment::Center),
                );
            }
        }
        col.push(
            row![
                text_input("another mod's packageId", &self.rule_input)
                    .on_input(Message::RuleInput)
                    .on_submit(Message::RuleAdd { before: false })
                    .padding(4)
                    .size(13),
                button("Load after")
                    .on_press(Message::RuleAdd { before: false })
                    .style(button::secondary),
                button("Load before")
                    .on_press(Message::RuleAdd { before: true })
                    .style(button::secondary),
            ]
            .spacing(6),
        )
        .into()
    }

    fn preview(&self, m: &Mod) -> Option<image::Handle> {
        self.previews.get(&m.path).cloned().flatten()
    }

    /// Select a mod and cache its preview handle (stable handles avoid re-decoding).
    fn select(&mut self, p: PathBuf) {
        if let Some(m) = self.session.mod_ref(&p) {
            let handle = m.preview_path().map(image::Handle::from_path);
            self.previews.entry(p.clone()).or_insert(handle);
        }
        self.selected = Some(p);
    }

    fn issues_panel(&self) -> iced::widget::Container<'_, Message> {
        let s = &self.session;
        let errors = s.issues.iter().filter(|i| i.severity == Severity::Error).count();
        let warnings = s.issues.len() - errors;
        let lines = s.issues.iter().map(|i| {
            let (tag, color) = match i.severity {
                Severity::Error => ("error", ERROR_COLOR),
                Severity::Warning => ("warn", WARN_COLOR),
            };
            row![
                text(tag).color(color).size(13).width(44),
                text(&i.message).size(13),
            ]
            .spacing(6)
            .into()
        });
        container(
            column![
                text(format!("Issues — {errors} error(s), {warnings} warning(s)")).size(15),
                scrollable(column(lines).spacing(2)).height(Length::Fill),
            ]
            .spacing(4),
        )
        .padding([4, 8])
        .width(Length::Fill)
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let s = &self.session;
        let busy = if s.is_busy() { "⏳ " } else { "" };
        let dirty = if s.dirty { "  •  unsaved changes" } else { "" };
        container(text(format!("{busy}{}{dirty}", s.status)).size(13))
            .padding([4, 8])
            .width(Length::Fill)
            .style(container::dark)
            .into()
    }

    // ---- settings ---------------------------------------------------------

    fn settings_view(&self) -> Element<'_, Message> {
        let s = &self.session;
        let mut form = column![
            text("Settings").size(22),
            text("Instance: a new name creates one, an existing name switches, -name deletes")
                .size(12)
                .color(DIM_COLOR),
        ]
        .spacing(10)
        .padding(16);
        if !s.settings.game_install_ok() {
            form = form.push(
                text("Game install does not look like a RimWorld folder yet.").color(WARN_COLOR),
            );
        }
        // Sort mode is on the main toolbar and saves immediately, so it is not a draft field here.
        for &f in SettingsField::ALL.iter().filter(|f| !f.is_cycle()) {
            let value = self.settings_draft.get(&f).map(String::as_str).unwrap_or("");
            let mut input = text_input(f.label(), value)
                .on_input(move |v| Message::SettingsEdit(f, v))
                .on_submit(Message::SaveSettings)
                .padding(6);
            if matches!(f, SettingsField::SteamApiKey | SettingsField::GithubToken) {
                input = input.secure(true);
            }
            form = form.push(
                row![text(f.label()).width(140), input]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
            );
        }
        let mut actions = row![button("Save").on_press(Message::SaveSettings)].spacing(8);
        if s.settings.game_install_ok() {
            actions = actions.push(
                button("Cancel")
                    .on_press(Message::Nav(View::Mods))
                    .style(button::secondary),
            );
        }
        form = form.push(actions);
        if let Some(p) = Settings::config_file_path() {
            form = form.push(text(format!("Stored in {}", p.display())).size(12).color(DIM_COLOR));
        }
        scrollable(form).height(Length::Fill).into()
    }
}

fn field<'a>(label: &'a str, value: String) -> Element<'a, Message> {
    row![text(label).size(13).color(DIM_COLOR).width(90), text(value).size(13)]
        .spacing(6)
        .into()
}

fn source_tag(source: ModSource) -> &'static str {
    match source {
        ModSource::Dlc => "DLC",
        ModSource::Local => "L",
        ModSource::Workshop => "W",
    }
}
