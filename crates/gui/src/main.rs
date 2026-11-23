//! rimdeck — RimWorld mod manager GUI (GPL-3.0-only), built on the shared rimdeck core.
//!
//! Inspired by RimSort (https://github.com/RimSort/RimSort).

mod manage;
mod theme;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use iced::keyboard::{self, key::Named, Key};
use iced::widget::operation::{self, RelativeOffset};
use iced::widget::{
    button, column, container, image, mouse_area, pick_list, row, rule, scrollable, space, text,
    text_input, toggler,
};
use iced::{Alignment, Element, Length, Subscription, Task};

use rimdeck_core::issues::Severity;
use rimdeck_core::model::{Mod, ModSource};
use rimdeck_core::session::{self, Session};
use rimdeck_core::settings::{Settings, SettingsField, SortMode, SteamBackend};

const INACTIVE_LIST: &str = "inactive-list";
const ACTIVE_LIST: &str = "active-list";

fn main() -> iced::Result {
    session::init_tracing();
    iced::application(Gui::boot, Gui::update, Gui::view)
        .title(Gui::title)
        .subscription(Gui::subscription)
        .theme(|_: &Gui| theme::theme())
        .font(theme::FONT_FILES[0])
        .font(theme::FONT_FILES[1])
        .font(theme::FONT_FILES[2])
        .default_font(theme::FONT)
        // Below this the toolbar and the three panes start squeezing each other.
        .window(iced::window::Settings {
            size: iced::Size::new(1400.0, 860.0),
            min_size: Some(iced::Size::new(1000.0, 540.0)),
            ..Default::default()
        })
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
    /// The second toolbar row with occasional tools is showing.
    tools_open: bool,
    /// "Deactivate all mods" was pressed once; the next press clears the load order.
    confirm_clear: bool,
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
    /// Clicked issue: select the mod with this packageId.
    SelectPackage(String),
    BuildDb,
    Tool(Tool),
    ToggleTools,
    Textures {
        clean: bool,
    },
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
    RuleAdd {
        before: bool,
    },
    RuleRemove {
        other: String,
        before: bool,
    },
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
    SteamBackend(SteamBackend),
    CheckUpdates,
    UpdateAllOutdated,
    UpdateWorkshop(String),
    OpenInSteam(String),
    /// First press arms (keyed by the mod folder), second unsubscribes.
    Unsubscribe {
        id: String,
        path: PathBuf,
    },
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

/// Occasional actions, behind "More tools" so the toolbar fits and the bulk clear sits apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    UpdateDatabases,
    BuildSteamDb,
    OptimizeTextures,
    RemoveDds,
    DeactivateAll,
}

impl Tool {
    const ALL: [Tool; 5] = [
        Tool::UpdateDatabases,
        Tool::BuildSteamDb,
        Tool::OptimizeTextures,
        Tool::RemoveDds,
        Tool::DeactivateAll,
    ];

    fn message(self) -> Message {
        match self {
            Tool::UpdateDatabases => Message::FetchDbs,
            Tool::BuildSteamDb => Message::BuildDb,
            Tool::OptimizeTextures => Message::Textures { clean: false },
            Tool::RemoveDds => Message::Textures { clean: true },
            Tool::DeactivateAll => Message::Clear,
        }
    }
}

impl std::fmt::Display for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Tool::UpdateDatabases => "Update databases",
            Tool::BuildSteamDb => "Build Steam DB",
            Tool::OptimizeTextures => "Optimize textures",
            Tool::RemoveDds => "Remove DDS",
            Tool::DeactivateAll => "Deactivate all mods",
        })
    }
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
            tools_open: false,
            confirm_clear: false,
        };
        if view == View::Settings {
            gui.open_settings();
        }
        (gui, Task::none())
    }

    fn title(&self) -> String {
        let dirty = if self.session.dirty { " *" } else { "" };
        format!("rimdeck{dirty}")
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
        // Any other action disarms the two-press confirmations.
        if !matches!(message, Message::Refresh | Message::Tick) {
            self.confirm_refresh = false;
        }
        if !matches!(message, Message::Tool(Tool::DeactivateAll) | Message::Tick) {
            self.confirm_clear = false;
        }
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
            Message::SelectPackage(pid) => {
                let Some(p) = self.find_package(&pid) else {
                    self.session.status =
                        format!("{pid} isn't installed, so there is nothing to show");
                    return Task::none();
                };
                // A filter that hides the target would leave the details changed but the row unseen.
                let active = self.session.is_active(&p);
                if !self.visible(active).contains(&p) {
                    if active {
                        self.active_search.clear();
                    } else {
                        self.inactive_search.clear();
                    }
                }
                self.select(p);
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
                return operation::snap_to(
                    id,
                    RelativeOffset {
                        x: None,
                        y: Some(y),
                    },
                );
            }
        }
        Task::none()
    }

    fn apply(&mut self, message: Message) {
        let s = &mut self.session;
        match message {
            Message::Key(_) | Message::SearchSubmit(_) | Message::SelectPackage(_) => {} // handled in `update`
            Message::BuildDb => s.build_db(),
            Message::ToggleTools => self.tools_open = !self.tools_open,
            // The armed button's own label asks for confirmation; no status text to go stale.
            Message::Tool(Tool::DeactivateAll) if !self.confirm_clear => self.confirm_clear = true,
            Message::Tool(tool) => {
                self.tools_open = false;
                self.confirm_clear = false;
                self.apply(tool.message());
            }
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
                    self.session
                        .set_user_order_rule(&pid, &other, before, false);
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
            let open = view == self.view;
            button(
                text(label)
                    .size(15)
                    .font(if open { theme::SEMIBOLD } else { theme::FONT }),
            )
            .on_press(Message::Nav(view))
            .padding([6, 14])
            .style(move |t, status| theme::tab(t, status, open))
            .into()
        });
        let header = row(tabs)
            .push(space::horizontal())
            .push(
                text(format!(
                    "Instance: {}",
                    self.session.settings.instance_name()
                ))
                .size(13)
                .color(theme::DUST),
            )
            .spacing(2)
            .padding(iced::Padding {
                top: 6.0,
                right: 14.0,
                bottom: 0.0,
                left: 10.0,
            })
            .align_y(Alignment::Center);
        let body = match self.view {
            View::Mods => self.main_view(),
            View::Workshop => self.workshop_view(),
            View::Github => self.github_view(),
            View::Git => self.git_view(),
            View::Settings => self.settings_view(),
        };
        column![header, body, self.status_bar()].into()
    }

    // ---- main window ------------------------------------------------------

    fn main_view(&self) -> Element<'_, Message> {
        let s = &self.session;
        // Fixed widths: a label that grows with state would shift its neighbours under the pointer.
        let save = fixed_button(if s.dirty { "Save changes" } else { "Save" }, 140.0)
            .on_press(Message::Save)
            .style(if s.dirty {
                theme::attention
            } else {
                theme::quiet
            });
        let reload = fixed_button(
            if self.confirm_refresh {
                "Discard changes?"
            } else {
                "Reload"
            },
            160.0,
        )
        .on_press(Message::Refresh)
        .style(if self.confirm_refresh {
            theme::danger
        } else {
            theme::quiet
        });
        let tools = [
            ("Import list…", Message::ImportList),
            ("Export list…", Message::ExportList),
        ]
        .map(|(label, msg)| button(label).on_press(msg).style(theme::subtle).into());
        // A button, not a pick_list: pick_list fires an option on Ctrl+scroll while closed.
        // Same label open or closed, so the toolbar never shifts (see fixed_button).
        let more = button("More tools")
            .on_press(Message::ToggleTools)
            .style(if self.tools_open {
                theme::quiet
            } else {
                theme::subtle
            });
        let toolbar = row![
            button("Sort").on_press(Message::Sort).style(theme::primary),
            pick_list(
                &SortMode::ALL[..],
                Some(s.settings.sort_mode),
                Message::SortMode
            ),
            save,
            reload,
            space::horizontal(),
            row(tools).spacing(6),
            more,
            button("Run game")
                .on_press(Message::Launch)
                .style(theme::primary),
        ]
        .spacing(8)
        .padding([10, 12])
        .align_y(Alignment::Center);

        let lists = row![
            self.mod_list(false),
            self.mod_list(true),
            container(self.info_panel()).width(Length::FillPortion(3)),
        ]
        .spacing(10)
        .padding([0, 12])
        // 3:1 leaves the problems panel a summary plus rows even at the minimum window height.
        .height(Length::FillPortion(3));

        let mut page = column![toolbar];
        if self.tools_open {
            let mut tools = row![].spacing(8).padding(iced::Padding {
                bottom: 10.0,
                ..iced::Padding::from([0, 12])
            });
            for tool in Tool::ALL {
                if tool == Tool::DeactivateAll {
                    let label = if self.confirm_clear {
                        "Confirm: deactivate all".to_string()
                    } else {
                        tool.to_string()
                    };
                    tools = tools.push(space::horizontal()).push(
                        button(text(label))
                            .on_press(Message::Tool(tool))
                            .style(theme::danger),
                    );
                } else {
                    tools = tools.push(
                        button(text(tool.to_string()))
                            .on_press(Message::Tool(tool))
                            .style(theme::subtle),
                    );
                }
            }
            page = page.push(tools);
        } else {
            page = page.push(space::vertical().height(0));
        }
        page.push(
            column![
                lists,
                container(self.issues_panel())
                    .padding([10, 12])
                    .height(Length::FillPortion(1)),
            ]
            .height(Length::Fill),
        )
        .height(Length::Fill)
        .into()
    }

    /// The inactive list, or the load order (numbered by its real position, also while filtered).
    fn mod_list(&self, active: bool) -> Element<'_, Message> {
        let s = &self.session;
        let paths = self.visible(active);
        let positions: HashMap<&PathBuf, usize> = if active {
            s.active_paths
                .iter()
                .enumerate()
                .map(|(i, p)| (p, i + 1))
                .collect()
        } else {
            HashMap::new()
        };
        let worst = self.worst_issues();
        let rows = paths.iter().filter_map(|p| {
            let m = s.mod_ref(p)?;
            let severity = if active {
                worst.get(m.package_id.as_str()).copied()
            } else {
                None
            };
            Some(self.mod_row(m, positions.get(p).copied(), severity))
        });
        let (title, count, query, on_search): (_, _, _, fn(String) -> Message) = if active {
            (
                "Load order",
                format!("{} active", s.active_paths.len()),
                &self.active_search,
                Message::ActiveSearch,
            )
        } else {
            (
                "Inactive",
                s.inactive_paths.len().to_string(),
                &self.inactive_search,
                Message::InactiveSearch,
            )
        };
        let mut header = row![
            text(title).size(17).font(theme::SEMIBOLD),
            text(count).size(13).color(theme::DUST),
            space::horizontal(),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if active {
            header = header
                .push(small_button("Move up", Message::MoveSelected(-1)))
                .push(small_button("Move down", Message::MoveSelected(1)));
        }
        column![
            header,
            text_input("Filter by name, packageId or author", query)
                .on_input(on_search)
                .on_submit(Message::SearchSubmit(active))
                .padding(6)
                .size(14),
            container(
                scrollable(column(rows))
                    .id(if active { ACTIVE_LIST } else { INACTIVE_LIST })
                    .height(Length::Fill)
            )
            .style(theme::panel)
            .padding(4)
            .height(Length::Fill),
        ]
        .spacing(6)
        .width(Length::FillPortion(if active { 4 } else { 3 }))
        .into()
    }

    fn mod_row<'a>(
        &self,
        m: &'a Mod,
        position: Option<usize>,
        severity: Option<Severity>,
    ) -> Element<'a, Message> {
        let selected = self.selected.as_deref() == Some(m.path.as_path());
        let mut line = row![].spacing(8).align_y(Alignment::Center);
        if let Some(n) = position {
            let (glyph, color) = severity_glyph(severity);
            line = line
                .push(
                    text(n.to_string())
                        .size(12)
                        .color(theme::DUST)
                        .width(34)
                        .align_x(iced::alignment::Horizontal::Right),
                )
                .push(text(glyph).size(11).color(color).width(10));
        }
        line = line
            .push(
                text(source_tag(m.source))
                    .size(11)
                    .color(theme::DUST)
                    .width(24),
            )
            .push(text(&m.name).size(14));
        // Not a button: a button captures the press, so MouseArea would never see the double-click.
        let row_box = container(line)
            .width(Length::Fill)
            .padding([3, 6])
            .style(if selected {
                theme::selected_row
            } else {
                container::transparent
            });
        mouse_area(row_box)
            .interaction(iced::mouse::Interaction::Pointer)
            .on_press(Message::Select(m.path.clone()))
            .on_double_click(Message::Toggle(m.path.clone()))
            .into()
    }

    /// Worst issue per packageId, built once per frame instead of once per row.
    fn worst_issues(&self) -> HashMap<&str, Severity> {
        let mut worst = HashMap::new();
        for issue in &self.session.issues {
            worst
                .entry(issue.package_id.as_str())
                .and_modify(|w| {
                    if issue.severity == Severity::Error {
                        *w = Severity::Error;
                    }
                })
                .or_insert(issue.severity);
        }
        worst
    }

    /// Active copy first, so an issue jumps to the mod that is actually loaded.
    fn find_package(&self, package_id: &str) -> Option<PathBuf> {
        let s = &self.session;
        s.active_paths
            .iter()
            .chain(&s.inactive_paths)
            .find(|p| {
                s.mod_ref(p)
                    .is_some_and(|m| m.package_id.eq_ignore_ascii_case(package_id))
            })
            .cloned()
    }

    fn info_panel(&self) -> Element<'_, Message> {
        let s = &self.session;
        let Some(m) = self.selected.as_ref().and_then(|p| s.mod_ref(p)) else {
            return container(
                text("Select a mod to see its details. Double-click it or press Enter to activate or deactivate it.")
                    .size(14)
                    .color(theme::DUST),
            )
            .padding(16)
            .style(theme::panel)
            .height(Length::Fill)
            .width(Length::Fill)
            .into();
        };
        let mut col = column![].spacing(8).padding(16);
        if let Some(handle) = self.preview(m) {
            col = col.push(image(handle).width(Length::Fill).height(180));
        }
        col = col.push(
            column![
                text(&m.name).size(22).font(theme::SEMIBOLD),
                text(m.author_line()).size(14).color(theme::DUST),
            ]
            .spacing(2),
        );
        for issue in s.issues.iter().filter(|i| i.package_id == m.package_id) {
            let (glyph, color) = severity_glyph(Some(issue.severity));
            col = col.push(
                row![
                    text(glyph).size(11).color(color),
                    text(&issue.message).size(13)
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        col = col
            .push(field("Package ID", m.package_id.clone()))
            .push(field("Source", m.source.label().to_string()));
        if !m.mod_version.is_empty() {
            col = col.push(field("Version", m.mod_version.clone()));
        }
        if !m.supported_versions.is_empty() {
            col = col.push(field("Game versions", m.supported_versions.join(", ")));
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
        col = col.push(field("Folder", m.path.display().to_string()));
        let is_active = s.is_active(&m.path);
        col = col.push(
            row![
                button(if is_active { "Deactivate" } else { "Activate" })
                    .on_press(Message::Toggle(m.path.clone()))
                    .style(theme::primary),
                button("Update")
                    .on_press(Message::UpdateSelected)
                    .style(theme::quiet),
            ]
            .spacing(8),
        );
        if let Some(pfid) = &m.published_file_id {
            let mut steam_row = row![button("Open in Steam")
                .on_press(Message::OpenInSteam(pfid.clone()))
                .style(theme::subtle)]
            .spacing(8);
            if s.settings.steam_backend == SteamBackend::Steamworks {
                steam_row = steam_row.push(self.unsubscribe_button(pfid, &m.path));
            }
            col = col.push(steam_row);
        }
        col = col.push(self.rules_editor(m));
        if !m.description.is_empty() {
            col = col
                .push(rule::horizontal(1))
                .push(text(&m.description).size(14));
        }
        container(scrollable(col))
            .style(theme::panel)
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
        let mut col = column![
            text("Load order rules").size(16).font(theme::MEDIUM),
            toggler(s.user_rules.load_top(&m.package_id))
                .label("Always load at the top")
                .text_size(14)
                .on_toggle(|_| Message::ToggleLoadTop),
            toggler(s.user_rules.load_bottom(&m.package_id))
                .label("Always load at the bottom")
                .text_size(14)
                .on_toggle(|_| Message::ToggleLoadBottom),
        ]
        .spacing(6)
        .padding(iced::Padding {
            top: 8.0,
            ..iced::Padding::ZERO
        });
        for (pinned, place) in [
            (s.community_rules.load_top(&m.package_id), "top"),
            (s.community_rules.load_bottom(&m.package_id), "bottom"),
        ] {
            if pinned {
                col = col.push(
                    text(format!("Always loads at the {place} (community rule)"))
                        .size(13)
                        .color(theme::DUST),
                );
            }
        }
        for (ids, before) in [(c_after, false), (c_before, true)] {
            for id in ids {
                let kind = if before { "before" } else { "after" };
                col = col.push(
                    text(format!("Loads {kind} {id} (community rule)"))
                        .size(13)
                        .color(theme::DUST),
                );
            }
        }
        for (ids, before) in [(u_after, false), (u_before, true)] {
            for id in ids {
                let kind = if before { "before" } else { "after" };
                col = col.push(
                    row![
                        text(format!("Loads {kind} {id}")).size(13),
                        button(text("✕").size(12))
                            .on_press(Message::RuleRemove { other: id, before })
                            .padding([0, 6])
                            .style(theme::bare(theme::DUST)),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                );
            }
        }
        col.push(
            row![
                text_input("Another mod's packageId", &self.rule_input)
                    .on_input(Message::RuleInput)
                    .on_submit(Message::RuleAdd { before: false })
                    .padding(5)
                    .size(13),
                small_button("Load after it", Message::RuleAdd { before: false }),
                small_button("Load before it", Message::RuleAdd { before: true }),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
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

    fn issues_panel(&self) -> Element<'_, Message> {
        let s = &self.session;
        let errors = s
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count();
        let warnings = s.issues.len() - errors;
        let summary = if s.issues.is_empty() {
            "No problems found in the load order.".to_string()
        } else {
            format!(
                "{} and {}. Click one to find the mod.",
                plural(errors, "error"),
                plural(warnings, "warning")
            )
        };
        let lines = s.issues.iter().map(|i| {
            let (glyph, color) = severity_glyph(Some(i.severity));
            mouse_area(
                row![
                    text(glyph).size(11).color(color).width(14),
                    text(&i.message).size(14)
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .interaction(iced::mouse::Interaction::Pointer)
            .on_press(Message::SelectPackage(i.package_id.clone()))
            .into()
        });
        container(
            column![
                text(summary).size(15).font(theme::MEDIUM),
                scrollable(column(lines).spacing(3).width(Length::Fill))
                    .width(Length::Fill)
                    .height(Length::Fill),
            ]
            .spacing(6),
        )
        .padding([10, 14])
        .style(theme::panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let s = &self.session;
        let mut line = row![].spacing(8).align_y(Alignment::Center);
        if s.is_busy() {
            line = line.push(text("●").size(10).color(theme::VERDIGRIS));
        }
        line = line.push(text(&s.status).size(13));
        container(line)
            .padding([5, 14])
            .width(Length::Fill)
            .style(theme::status_bar)
            .into()
    }

    // ---- settings ---------------------------------------------------------

    fn settings_view(&self) -> Element<'_, Message> {
        let s = &self.session;
        let mut form = column![
            text("Settings").size(24).font(theme::SEMIBOLD),
            text("Instance: a new name creates one, an existing name switches to it, and -name deletes it.")
                .size(13)
                .color(theme::DUST),
        ]
        .spacing(12)
        .padding(20)
        .max_width(820);
        if !s.settings.game_install_ok() {
            form = form.push(
                text("Game install doesn't look like a RimWorld folder yet. Point it at the folder that holds RimWorldWin64.exe.")
                    .color(theme::OCHRE),
            );
        }
        // Sort mode is on the main toolbar and saves immediately, so it is not a draft field here.
        for &f in SettingsField::ALL.iter().filter(|f| !f.is_cycle()) {
            let value = self
                .settings_draft
                .get(&f)
                .map(String::as_str)
                .unwrap_or("");
            let mut input = text_input(f.label(), value)
                .on_input(move |v| Message::SettingsEdit(f, v))
                .on_submit(Message::SaveSettings)
                .padding(6)
                .size(14);
            if matches!(f, SettingsField::SteamApiKey | SettingsField::GithubToken) {
                input = input.secure(true);
            }
            form = form.push(
                row![
                    text(f.label()).size(14).color(theme::DUST).width(150),
                    input
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        form = form.push(
            row![
                text("Workshop via").size(14).color(theme::DUST).width(150),
                pick_list(
                    &SteamBackend::ALL[..],
                    Some(s.settings.steam_backend),
                    Message::SteamBackend
                ),
                text("Applies right away").size(12).color(theme::DUST),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );
        let mut actions = row![button("Save settings")
            .on_press(Message::SaveSettings)
            .style(theme::primary)]
        .spacing(8);
        if s.settings.game_install_ok() {
            actions = actions.push(
                button("Cancel")
                    .on_press(Message::Nav(View::Mods))
                    .style(theme::quiet),
            );
        }
        form = form.push(actions);
        if let Some(p) = Settings::config_file_path() {
            form = form.push(
                text(format!("Settings file: {}", p.display()))
                    .size(12)
                    .color(theme::DUST),
            );
        }
        scrollable(form).height(Length::Fill).into()
    }
}

fn field<'a>(label: &'a str, value: String) -> Element<'a, Message> {
    row![
        text(label).size(13).color(theme::DUST).width(100),
        // Folder paths have no spaces to break at.
        text(value)
            .size(13)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
    ]
    .spacing(8)
    .into()
}

fn fixed_button(label: &str, width: f32) -> iced::widget::Button<'_, Message> {
    button(
        text(label)
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center),
    )
    .width(width)
}

fn small_button(label: &str, msg: Message) -> Element<'_, Message> {
    button(text(label).size(13))
        .on_press(msg)
        .padding([3, 8])
        .style(theme::quiet)
        .into()
}

/// Shape and color together, so severity never depends on color alone.
fn severity_glyph(severity: Option<Severity>) -> (&'static str, iced::Color) {
    match severity {
        Some(Severity::Error) => ("●", theme::BRICK),
        Some(Severity::Warning) => ("▲", theme::OCHRE),
        None => ("", theme::DUST),
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn source_tag(source: ModSource) -> &'static str {
    match source {
        ModSource::Dlc => "DLC",
        ModSource::Local => "L",
        ModSource::Workshop => "W",
    }
}
