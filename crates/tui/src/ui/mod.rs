use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, InputMode, Pane, PathPromptKind, Screen};
use rimtui_core::issues::Severity;
use rimtui_core::settings::SettingsField;

pub fn draw(frame: &mut Frame, app: &mut App) {
    match app.screen {
        Screen::Main => draw_main(frame, app),
        Screen::Settings => {
            draw_main(frame, app);
            draw_settings(frame, app);
        }
        Screen::Help => {
            draw_main(frame, app);
            draw_help(frame);
        }
        Screen::Workshop => {
            draw_main(frame, app);
            draw_workshop(frame, app);
        }
        Screen::Github => {
            draw_main(frame, app);
            draw_github(frame, app);
        }
        Screen::Git => {
            draw_main(frame, app);
            draw_git(frame, app);
        }
        Screen::ImageWarning => {
            draw_main(frame, app);
            draw_image_warning(frame, app);
        }
    }
    if app.input_mode == InputMode::PathPrompt {
        draw_path_prompt(frame, app);
    }
    if app.input_mode == InputMode::FileExplorer {
        draw_file_explorer(frame, app);
    }
}

fn draw_main(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let status = status_text(app);
    let inner_width = area.width.saturating_sub(2).max(1);
    let status_height = status_bar_height(&status, inner_width);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(7),
            Constraint::Length(status_height),
        ])
        .split(area);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(35),
            Constraint::Percentage(35),
        ])
        .split(chunks[0]);

    draw_info(frame, app, main[0]);
    draw_list(frame, app, main[1], Pane::Inactive);
    draw_list(frame, app, main[2], Pane::Active);
    draw_warnings(frame, app, chunks[1]);
    draw_status(frame, chunks[2], status_line(app));
    app.info_area = main[0];
    app.inactive_area = main[1];
    app.active_area = main[2];
    app.issues_area = chunks[1];
}

fn draw_info(frame: &mut Frame, app: &mut App, area: Rect) {
    app.ensure_preview();

    let mut lines = Vec::new();
    let selected_path = app.selected_mod().map(|m| m.path.clone());
    let github_line = selected_path.as_ref().and_then(|path| {
        let (owner_repo, installed, auto) = {
            let gh = app.github_entry_for(path)?;
            (
                gh.owner_repo.clone(),
                gh.installed_version.clone(),
                gh.auto_update,
            )
        };
        let latest = app.github_latest_tag(&owner_repo);
        let mut line = format!("github: {owner_repo} @ {installed}");
        if let Some(latest) = latest {
            if latest != installed {
                line.push_str(&format!(" (update: {latest})"));
            }
        }
        if auto {
            line.push_str(" auto");
        }
        Some(line)
    });
    let git_line = selected_path.as_ref().and_then(|path| {
        if app.github_entry_for(path).is_some() {
            return None;
        }
        if let Some(info) = app.git_items.iter().find(|i| &i.path == path) {
            let mut line = format!("git: {}", info.origin);
            if !info.branch.is_empty() {
                line.push_str(&format!(" @ {}", info.branch));
            }
            if !info.short_sha.is_empty() {
                line.push_str(&format!(" {}", info.short_sha));
            }
            if let Some(n) = info.behind {
                if n > 0 {
                    line.push_str(&format!(" ({n} behind)"));
                }
            }
            return Some(line);
        }
        rimtui_core::gitmod::is_git_repo(path).then(|| "git: cloned repo".to_string())
    });
    if let Some(m) = app.selected_mod() {
        lines.push(Line::from(Span::styled(
            m.name.clone(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(format!("id: {}", m.package_id)));
        lines.push(Line::from(format!("author: {}", m.author_line())));
        lines.push(Line::from(format!("source: {}", m.source.label())));
        if !m.mod_version.is_empty() {
            lines.push(Line::from(format!("version: {}", m.mod_version)));
        }
        if !m.supported_versions.is_empty() {
            lines.push(Line::from(format!(
                "game: {}",
                m.supported_versions.join(", ")
            )));
        }
        lines.push(Line::from(format!("path: {}", m.path.display())));
        if !m.url.is_empty() {
            lines.push(Line::from(format!("url: {}", m.url)));
        }
        if let Some(line) = github_line {
            lines.push(Line::from(Span::styled(
                line,
                Style::default().fg(Color::Cyan),
            )));
        }
        if let Some(line) = git_line {
            lines.push(Line::from(Span::styled(
                line,
                Style::default().fg(Color::Cyan),
            )));
        }
        if !m.dependencies.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "dependencies",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for d in &m.dependencies {
                let name = if d.display_name.is_empty() {
                    d.package_id.clone()
                } else {
                    format!("{} ({})", d.display_name, d.package_id)
                };
                lines.push(Line::from(format!("  {name}")));
            }
        }
        if !m.load_after.is_empty() {
            lines.push(Line::from(format!(
                "loadAfter: {}",
                m.load_after.join(", ")
            )));
        }
        if !m.load_before.is_empty() {
            lines.push(Line::from(format!(
                "loadBefore: {}",
                m.load_before.join(", ")
            )));
        }
        if !m.incompatible_with.is_empty() {
            lines.push(Line::from(format!(
                "incompatible: {}",
                m.incompatible_with.join(", ")
            )));
        }
        if m.load_first {
            lines.push(Line::from("loadTop: yes (tier 1)"));
        }
        if m.load_last {
            lines.push(Line::from("loadBottom: yes (tier 3)"));
        }
        if !m.description.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(m.description.clone()));
        }
    } else {
        lines.push(Line::from("No mod selected."));
    }

    let block = Block::default().title(" Info ").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let has_preview = app
        .current_preview
        .as_ref()
        .is_some_and(|p| app.preview_cache.contains_key(p));
    let (image_area, text_area) = if has_preview && inner.height > 8 {
        let img_h = inner.height.saturating_div(2).clamp(6, 14);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(img_h), Constraint::Min(3)])
            .split(inner);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, inner)
    };

    if let Some(image_area) = image_area {
        if let Some(path) = app.current_preview.clone() {
            if let Some(state) = app.preview_cache.get_mut(&path) {
                let widget = ratatui_image::StatefulImage::new(None)
                    .resize(ratatui_image::Resize::Fit(None));
                frame.render_stateful_widget(widget, image_area, state);
            }
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.info_scroll, 0)),
        text_area,
    );
}

fn draw_list(frame: &mut Frame, app: &mut App, area: Rect, pane: Pane) {
    let focused = app.pane == pane && app.screen == Screen::Main;
    let (title, paths, selected) = match pane {
        Pane::Inactive => (
            format!(" Inactive ({}) ", app.inactive_paths.len()),
            app.filtered_inactive(),
            app.selected_inactive,
        ),
        Pane::Active => (
            format!(" Active ({}) ", app.active_paths.len()),
            app.filtered_active(),
            app.selected_active,
        ),
    };

    let items: Vec<ListItem> = paths
        .iter()
        .map(|p| {
            let m = app.mod_ref(p);
            let label = match m {
                Some(m) => format!("[{}] {}", source_tag(m.source), m.name),
                None => p.display().to_string(),
            };
            ListItem::new(label)
        })
        .collect();

    let border = if focused { Color::Yellow } else { Color::Gray };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let state = match pane {
        Pane::Inactive => {
            app.inactive_state.select(if paths.is_empty() {
                None
            } else {
                Some(selected.min(paths.len().saturating_sub(1)))
            });
            &mut app.inactive_state
        }
        Pane::Active => {
            app.active_state.select(if paths.is_empty() {
                None
            } else {
                Some(selected.min(paths.len().saturating_sub(1)))
            });
            &mut app.active_state
        }
    };
    frame.render_stateful_widget(list, area, state);
}

fn source_tag(source: rimtui_core::model::ModSource) -> &'static str {
    match source {
        rimtui_core::model::ModSource::Dlc => "DLC",
        rimtui_core::model::ModSource::Local => "L",
        rimtui_core::model::ModSource::Workshop => "W",
    }
}

fn draw_warnings(frame: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line> = if app.issues.is_empty() {
        vec![Line::from(Span::styled(
            "No issues.",
            Style::default().fg(Color::Green),
        ))]
    } else {
        app.issues
            .iter()
            .map(|issue| {
                let (prefix, color) = match issue.severity {
                    Severity::Error => ("ERR", Color::Red),
                    Severity::Warning => ("WARN", Color::Yellow),
                };
                Line::from(vec![
                    Span::styled(format!("{prefix} "), Style::default().fg(color)),
                    Span::raw(issue.message.clone()),
                ])
            })
            .collect()
    };
    let title = format!(" Issues ({}) ", app.issues.len());
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(title).borders(Borders::ALL))
            .scroll((app.warning_scroll, 0)),
        area,
    );
}

fn draw_status(frame: &mut Frame, area: Rect, line: Line<'_>) {
    frame.render_widget(
        Paragraph::new(line)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL)),
        area,
    );
}

const KEY_STYLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const LABEL_STYLE: Style = Style::new().fg(Color::Gray);

const HELP_KEYS: &[(&str, &str)] = &[
    ("q", "quit"),
    ("Tab", "pane"),
    ("Enter", "on/off"),
    ("Ctrl+j/k", "reorder"),
    ("t/b", "top/bottom"),
    ("R", "rule"),
    ("s", "sort"),
    ("S", "mode"),
    ("w", "save"),
    ("l", "load"),
    ("e", "export"),
    ("g", "launch"),
    ("d", "fetch"),
    ("D", "build db"),
    ("o/O", "textures"),
    ("n", "workshop"),
    ("h", "github"),
    ("G", "git"),
    ("u", "steamcmd"),
    ("r", "rescan"),
    ("/", "search"),
    ("p", "paths"),
    ("?", "help"),
];

fn help_plain() -> String {
    HELP_KEYS
        .iter()
        .map(|(key, label)| format!("{key} {label}"))
        .collect::<Vec<_>>()
        .join("  ")
}

fn help_spans() -> Vec<Span<'static>> {
    let mut spans = Vec::with_capacity(HELP_KEYS.len() * 3);
    for (i, (key, label)) in HELP_KEYS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(*key, KEY_STYLE));
        spans.push(Span::styled(format!(" {label}"), LABEL_STYLE));
    }
    spans
}

fn status_text(app: &App) -> String {
    let dirty = if app.dirty { " *unsaved" } else { "" };
    let spin = if app.is_busy() {
        format!(" {} ", app.spinner_glyph())
    } else {
        String::new()
    };
    let search = if app.input_mode == InputMode::Search || !app.search.is_empty() {
        format!("  /{}", app.search)
    } else {
        String::new()
    };
    format!(
        " rimtui{dirty}{spin}  {}  {}{search}",
        app.status,
        help_plain()
    )
}

fn status_line(app: &App) -> Line<'_> {
    let mut spans = vec![Span::styled(
        " rimtui",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )];
    if app.dirty {
        spans.push(Span::styled(
            " *unsaved",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if app.is_busy() {
        spans.push(Span::styled(
            format!(" {} ", app.spinner_glyph()),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::raw("  "));
    }
    if !app.status.is_empty() {
        let status_style = if app.is_busy() {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        spans.push(Span::styled(app.status.as_str(), status_style));
        spans.push(Span::raw("  "));
    }
    spans.extend(help_spans());
    if app.input_mode == InputMode::Search || !app.search.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled("/", KEY_STYLE));
        spans.push(Span::raw(app.search.as_str()));
    }
    Line::from(spans)
}

/// Border (2) + wrapped content, capped so the lists still have room.
fn status_bar_height(text: &str, inner_width: u16) -> u16 {
    let lines = wrap_line_count(text, inner_width as usize) as u16;
    lines.saturating_add(2).clamp(3, 7)
}

fn wrap_line_count(text: &str, width: usize) -> usize {
    let width = width.max(1);
    let mut total = 0;
    for line in text.split('\n') {
        total += wrap_one_line(line, width);
    }
    total.max(1)
}

fn wrap_one_line(line: &str, width: usize) -> usize {
    if line.is_empty() {
        return 1;
    }
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut lines = 0;
    while i < chars.len() {
        let remaining = chars.len() - i;
        if remaining <= width {
            lines += 1;
            break;
        }
        let window = &chars[i..i + width];
        let brk = window
            .iter()
            .rposition(|c| c.is_whitespace())
            .filter(|&rel| rel > 0)
            .map(|rel| i + rel + 1)
            .unwrap_or(i + width);
        i = brk;
        lines += 1;
    }
    lines.max(1)
}

const INSTANCE_HINT: &str = "Instance: a new name creates one, an existing name switches, -name deletes";

fn draw_settings(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 84, 28);
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(Span::styled(
            "Settings",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("Up/Down select  Enter edit/cycle  s save  Esc back"),
        Line::from(INSTANCE_HINT),
        Line::from(""),
    ];
    for field in SettingsField::ALL {
        let selected = field == app.settings_field;
        let marker = if selected { ">" } else { " " };
        let value = if selected && app.input_mode == InputMode::SettingsEdit {
            format!("{}█", app.edit_buffer)
        } else {
            app.settings.field_display(field)
        };
        let style = if selected {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker} {:14} {value}", field.label()),
            style,
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(" Settings ").borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_image_warning(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 72, 12);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(Span::styled(
            "Preview warning",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!(
            "Mod images may not be fully supported in {}.",
            app.terminal_name
        )),
        Line::from("Previews are shown as colored blocks and can look coarse."),
        Line::from("Kitty, WezTerm, iTerm2, and Ghostty render them more accurately."),
        Line::from(""),
        Line::from("Press Enter or Esc to continue."),
    ];
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(" Warning ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        area,
    );
}

fn draw_file_explorer(frame: &mut Frame, app: &mut App) {
    let height = (frame.area().height.saturating_mul(80) / 100).clamp(14, frame.area().height);
    let area = centered(frame.area(), 80, height);
    frame.render_widget(Clear, area);

    let location = if app.file_browser.showing_drives {
        "Drives".to_string()
    } else {
        app.file_browser.cwd.display().to_string()
    };
    let outer = Block::default()
        .title(" Load mod list ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(inner);

    let header = vec![
        Line::from(Span::styled(
            location,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("j/k move  Enter open  Backspace up  click select  Esc cancel"),
    ];
    frame.render_widget(Paragraph::new(header).wrap(Wrap { trim: false }), chunks[0]);

    let items: Vec<ListItem> = app
        .file_browser
        .entries
        .iter()
        .map(|e| {
            let (label, style) = if e.is_parent {
                ("../".to_string(), Style::default().fg(Color::Cyan))
            } else if e.is_dir {
                (format!("{}/", e.name), Style::default().fg(Color::Cyan))
            } else {
                (e.name.clone(), Style::default().fg(Color::White))
            };
            ListItem::new(label).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" XML files "))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    app.file_browser
        .state
        .select(if app.file_browser.entries.is_empty() {
            None
        } else {
            Some(app.file_browser.selected)
        });
    app.explorer_area = chunks[1];
    frame.render_stateful_widget(list, chunks[1], &mut app.file_browser.state);
}

fn draw_workshop(frame: &mut Frame, app: &mut App) {
    let height = (frame.area().height.saturating_mul(80) / 100).clamp(16, frame.area().height);
    let area = centered(frame.area(), 84, height);
    frame.render_widget(Clear, area);

    let editing = app.input_mode == InputMode::WorkshopQuery;
    let query = if editing {
        format!("{}█", app.workshop_query)
    } else if app.workshop_query.is_empty() {
        "(empty — popular mods)".into()
    } else {
        app.workshop_query.clone()
    };
    let outer = Block::default()
        .title(" Workshop search ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5)])
        .split(inner);

    let header = vec![
        Line::from(vec![
            Span::styled("Query: ", Style::default().fg(Color::Gray)),
            Span::styled(
                query,
                if editing {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                },
            ),
        ]),
        Line::from(if editing {
            "Type to search  Enter run  Esc results/close"
        } else {
            "j/k move  Enter download+enable  Space download  c import query as collection  / query  Esc close"
        }),
        Line::from("Needs a Steam WebAPI key (p) except numeric published file IDs."),
    ];
    frame.render_widget(Paragraph::new(header).wrap(Wrap { trim: false }), chunks[0]);

    let items: Vec<ListItem> = app
        .workshop_items
        .iter()
        .map(|item| {
            let installed = if app.workshop_installed(&item.published_file_id) {
                " [have]"
            } else {
                ""
            };
            let tags = if item.tags.is_empty() {
                String::new()
            } else {
                format!(
                    "  {}",
                    item.tags.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
                )
            };
            let label = format!(
                "{} {}  ({}){installed}{tags}",
                item.published_file_id,
                item.title,
                rimtui_core::workshop::format_subs(item.subscriptions),
            );
            ListItem::new(label)
        })
        .collect();

    let title = if app.workshop_items.is_empty() {
        " Results ".to_string()
    } else {
        format!(" Results ({}) ", app.workshop_items.len())
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    app.workshop_state.select(if app.workshop_items.is_empty() {
        None
    } else {
        Some(
            app.workshop_selected
                .min(app.workshop_items.len().saturating_sub(1)),
        )
    });
    app.workshop_area = chunks[1];
    frame.render_stateful_widget(list, chunks[1], &mut app.workshop_state);
}

fn draw_github(frame: &mut Frame, app: &mut App) {
    let height = (frame.area().height.saturating_mul(80) / 100).clamp(16, frame.area().height);
    let area = centered(frame.area(), 84, height);
    frame.render_widget(Clear, area);

    let editing = app.input_mode == InputMode::GithubQuery;
    let query = if editing {
        format!("{}█", app.github_query)
    } else if app.github_query.is_empty() {
        "(owner/repo or github.com URL)".into()
    } else {
        app.github_query.clone()
    };
    let outer = Block::default()
        .title(" GitHub mods ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5)])
        .split(inner);

    let header = vec![
        Line::from(vec![
            Span::styled("Install: ", Style::default().fg(Color::Gray)),
            Span::styled(
                query,
                if editing {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                },
            ),
        ]),
        Line::from(if editing {
            "Type owner/repo  Enter install+enable  Esc list/close"
        } else {
            "j/k move  Enter/u update  c check  a auto  X delete  / install  Esc close"
        }),
        Line::from("Token optional in path settings (p); raises the unauthenticated rate limit."),
    ];
    frame.render_widget(Paragraph::new(header).wrap(Wrap { trim: false }), chunks[0]);

    let items: Vec<ListItem> = {
        let tracked = &app.github_mods.mods;
        let mods = &app.mods;
        let cache = &app.github_cache;
        tracked
            .iter()
            .map(|entry| {
                let name = mods
                    .iter()
                    .find(|m| m.path == entry.mod_path)
                    .map(|m| m.name.as_str())
                    .unwrap_or("(missing)");
                let releases = cache.releases_for(&entry.owner_repo);
                let latest = rimtui_core::github::latest_stable_release(&releases)
                    .map(|r| r.tag.as_str())
                    .unwrap_or("—");
                let has_update = rimtui_core::github::update_for_entry(entry, &releases).is_some();
                let auto = if entry.auto_update { "  auto" } else { "" };
                let label = format!(
                    "{name}  {}  {} → {latest}{auto}",
                    entry.owner_repo, entry.installed_version
                );
                let style = if has_update {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                ListItem::new(label).style(style)
            })
            .collect()
    };

    let title = if app.github_mods.mods.is_empty() {
        " Tracked ".to_string()
    } else {
        format!(" Tracked ({}) ", app.github_mods.mods.len())
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    app.github_state.select(if app.github_mods.mods.is_empty() {
        None
    } else {
        Some(
            app.github_selected
                .min(app.github_mods.mods.len().saturating_sub(1)),
        )
    });
    app.github_area = chunks[1];
    frame.render_stateful_widget(list, chunks[1], &mut app.github_state);
}

fn draw_git(frame: &mut Frame, app: &mut App) {
    let height = (frame.area().height.saturating_mul(80) / 100).clamp(16, frame.area().height);
    let area = centered(frame.area(), 84, height);
    frame.render_widget(Clear, area);

    let editing = app.input_mode == InputMode::GitQuery;
    let query = if editing {
        format!("{}█", app.git_query)
    } else if app.git_query.is_empty() {
        "(https:// or git@ clone URL)".into()
    } else {
        app.git_query.clone()
    };
    let outer = Block::default()
        .title(" Git mods ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5)])
        .split(inner);

    let header = vec![
        Line::from(vec![
            Span::styled("Clone: ", Style::default().fg(Color::Gray)),
            Span::styled(
                query,
                if editing {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                },
            ),
        ]),
        Line::from(if editing {
            "Paste clone URL  Enter clone+enable  Esc list/close"
        } else {
            "j/k move  Enter/u pull  c check  A pull updates  X delete  / clone  Esc close"
        }),
        Line::from("HEAD clones via git. GitHub release zips stay on h. Needs git on PATH."),
    ];
    frame.render_widget(Paragraph::new(header).wrap(Wrap { trim: false }), chunks[0]);

    let items: Vec<ListItem> = app
        .git_items
        .iter()
        .map(|item| {
            let behind = match item.behind {
                _ if item.is_detached() => "detached".to_string(),
                Some(0) => "up to date".to_string(),
                Some(n) => format!("{n} behind"),
                None => "—".into(),
            };
            let err = item
                .error
                .as_deref()
                .map(|e| format!("  ({e})"))
                .unwrap_or_default();
            let label = format!(
                "{}  {}  {} {}  {behind}{err}",
                item.name, item.origin, item.branch, item.short_sha
            );
            let style = if item.has_update() {
                Style::default().fg(Color::Yellow)
            } else if item.error.is_some() {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();

    let title = if app.git_items.is_empty() {
        " Cloned ".to_string()
    } else {
        format!(" Cloned ({}) ", app.git_items.len())
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    app.git_state.select(if app.git_items.is_empty() {
        None
    } else {
        Some(app.git_selected.min(app.git_items.len().saturating_sub(1)))
    });
    app.git_area = chunks[1];
    frame.render_stateful_widget(list, chunks[1], &mut app.git_state);
}

fn draw_path_prompt(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 80, 10);
    frame.render_widget(Clear, area);
    let rule_hint;
    let (title, action, label) = match app.path_prompt {
        PathPromptKind::Export => ("Export mod list", "Enter write  Esc cancel", "Path"),
        PathPromptKind::Rule => {
            let pid = app.selected_mod().map(|m| m.package_id.as_str()).unwrap_or("");
            let (after, before) = app.user_rules.order_rules(pid);
            rule_hint = format!(
                "a <id> load after · b <id> load before · -a / -b <id> remove · Esc cancel  ·  yours: after [{}]  before [{}]",
                after.join(", "),
                before.join(", ")
            );
            ("User load-order rule for the selected mod", rule_hint.as_str(), "Rule")
        }
    };
    let lines = vec![
        Line::from(Span::styled(
            title,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(action),
        Line::from(""),
        Line::from(format!("{label}: {}█", app.edit_buffer)),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(" File ").borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_help(frame: &mut Frame) {
    let area = centered(frame.area(), 76, 33);
    frame.render_widget(Clear, area);
    let text = "\
rimtui — RimSort-style RimWorld mod manager

  Tab             switch inactive / active
  j k / arrows    move selection
  Enter           activate or deactivate
  Ctrl+j / Ctrl+k reorder active list
  /               search (Esc to clear)
  s               sort (current mode)
  S               cycle topological / alphabetical
  w               write ModsConfig.xml
  l               load a mod list XML (file explorer)
  e               export a mod list XML
  g               launch RimWorld (exe, else Steam)
  d               download community rules + Steam DB
  D               add installed Workshop mods to steamDB.json (build-db)
  o / O           optimize active mods' textures to DDS / delete those DDS
  n               workshop search (Steam WebAPI)
  h               GitHub release mods (track, update)
  G               git clone / pull HEAD (GitLab, GitGud, Forgejo, …)
  u               SteamCMD, GitHub, or git pull selected mod
  t / b           user loadTop / loadBottom (saved)
  R               user loadAfter / loadBefore rule for selected mod
  r               rescan mods
  p               path / database / SteamCMD / API keys
  mouse wheel     slow for 1–2 clicks, faster if you keep scrolling
  click           select a mod
  [ ]             scroll issues
  ?               this help
  q               quit

Esc to close.
";
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Left)
            .block(Block::default().title(" Help ").borders(Borders::ALL)),
        area,
    );
}

fn centered(area: Rect, percent_x: u16, height: u16) -> Rect {
    let width = (area.width.saturating_mul(percent_x) / 100).max(10);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_count_fits_on_one_line() {
        assert_eq!(wrap_line_count("short", 20), 1);
    }

    #[test]
    fn wrap_count_breaks_on_spaces() {
        let text = "q quit  s sort  w save  l load  e export";
        assert!(wrap_line_count(text, 16) > 1);
        assert!(wrap_line_count(text, 16) < wrap_line_count(text, 8));
    }

    #[test]
    fn status_bar_grows_then_caps() {
        let help = help_plain();
        // One line wide enough for the whole bar: 1 text row + 2 border rows.
        assert_eq!(status_bar_height(&help, help.chars().count() as u16), 3);
        let tall = status_bar_height(&help, 20);
        assert!(tall > 3);
        assert!(tall <= 7);
    }

    #[test]
    fn help_plain_lists_enter_and_reorder() {
        let help = help_plain();
        assert!(help.contains("Enter on/off"));
        assert!(help.contains("Ctrl+j/k reorder"));
        assert!(help.contains("t/b top/bottom"));
        assert!(help.contains("n workshop"));
        assert!(help.contains("h github"));
        assert!(help.contains("G git"));
        assert!(help.contains("u steamcmd"));
    }
}
