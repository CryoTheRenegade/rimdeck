use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, InputMode, Pane, PathPromptKind, Screen};
use rimdeck_core::issues::Severity;
use rimdeck_core::settings::SettingsField;

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
        Screen::Updates => {
            draw_main(frame, app);
            draw_updates(frame, app);
        }
        Screen::Packs => {
            draw_main(frame, app);
            draw_packs(frame, app);
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

// Same tokens as the GUI (crates/gui/src/theme.rs). Body text keeps the terminal's own color so
// light and dark terminal themes both stay readable; tokens carry structure and meaning.
const PARCHMENT: Color = Color::Rgb(0xE8, 0xE1, 0xCE);
const DUST: Color = Color::Rgb(0x9A, 0xA0, 0xA3);
const RIVET: Color = Color::Rgb(0x4A, 0x53, 0x5A);
const PLATE: Color = Color::Rgb(0x30, 0x36, 0x3B);
const VERDIGRIS: Color = Color::Rgb(0x6F, 0xA8, 0xA0);
const OCHRE: Color = Color::Rgb(0xD8, 0xA2, 0x3E);
const BRICK: Color = Color::Rgb(0xCF, 0x5A, 0x4A);
/// Verdigris at about a third over Plate: the selected row.
const SELECTED: Color = Color::Rgb(0x44, 0x5A, 0x5B);

const DIM: Style = Style::new().fg(DUST);

/// A pane: thin Rivet frame, or Verdigris when it has focus.
fn pane<'a>(title: Line<'a>, focused: bool) -> Block<'a> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::new().fg(if focused { VERDIGRIS } else { RIVET }))
}

/// " Name  detail " with the name bold and the detail dim.
fn title<'a>(name: &'a str, detail: String) -> Line<'a> {
    Line::from(vec![
        // Reset: the block's border color would otherwise tint the title.
        Span::styled(
            format!(" {name} "),
            Style::new().fg(Color::Reset).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{detail} "), DIM),
    ])
}

/// Shape and color together, so severity never depends on color alone.
fn severity_glyph(severity: Option<Severity>) -> Span<'static> {
    match severity {
        Some(Severity::Error) => Span::styled("●", Style::new().fg(BRICK)),
        Some(Severity::Warning) => Span::styled("▲", Style::new().fg(OCHRE)),
        None => Span::raw(" "),
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn draw_main(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let (status_rows, help_rows) = footer_rows(
        &format!("  ⠋ /{}  Unsaved changes (w saves)  {}", app.search, app.status),
        &help_plain(),
        area.width,
    );
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(7),
            Constraint::Length(status_rows + help_rows),
        ])
        .split(area);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(28),
            Constraint::Percentage(40),
            Constraint::Percentage(32),
        ])
        .split(chunks[0]);

    draw_list(frame, app, main[0], Pane::Inactive);
    draw_list(frame, app, main[1], Pane::Active);
    draw_info(frame, app, main[2]);
    draw_warnings(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2], status_rows);
    app.inactive_area = main[0];
    app.active_area = main[1];
    app.info_area = main[2];
    app.issues_area = chunks[1];
}

fn field_line<'a>(label: &'a str, value: String) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label:<14}"), DIM),
        Span::raw(value),
    ])
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
        let mut line = format!("{owner_repo} at {installed}");
        if let Some(latest) = latest {
            if latest != installed {
                line.push_str(&format!(", update {latest} available"));
            }
        }
        if auto {
            line.push_str(", auto-update on");
        }
        Some(line)
    });
    let git_line = selected_path.as_ref().and_then(|path| {
        if app.github_entry_for(path).is_some() {
            return None;
        }
        if let Some(info) = app.git_items.iter().find(|i| &i.path == path) {
            let mut line = info.origin.clone();
            if !info.branch.is_empty() {
                line.push_str(&format!(" on {}", info.branch));
            }
            if !info.short_sha.is_empty() {
                line.push_str(&format!(" at {}", info.short_sha));
            }
            if let Some(n) = info.behind {
                if n > 0 {
                    line.push_str(&format!(", {n} behind"));
                }
            }
            return Some(line);
        }
        rimdeck_core::gitmod::is_git_repo(path).then(|| "cloned repository".to_string())
    });
    if let Some(m) = app.selected_mod() {
        lines.push(Line::from(Span::styled(
            m.name.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(m.author_line(), DIM)));
        for issue in app.issues.iter().filter(|i| i.package_id == m.package_id) {
            lines.push(Line::from(vec![
                severity_glyph(Some(issue.severity)),
                Span::raw(format!(" {}", issue.message)),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(field_line("Package ID", m.package_id.clone()));
        lines.push(field_line("Source", m.source.label().to_string()));
        if !m.mod_version.is_empty() {
            lines.push(field_line("Version", m.mod_version.clone()));
        }
        if !m.supported_versions.is_empty() {
            lines.push(field_line("Game versions", m.supported_versions.join(", ")));
        }
        lines.push(field_line("Folder", m.path.display().to_string()));
        if !m.url.is_empty() {
            lines.push(field_line("URL", m.url.clone()));
        }
        if let Some(line) = github_line {
            lines.push(field_line("GitHub", line));
        }
        if let Some(line) = git_line {
            lines.push(field_line("Git", line));
        }
        if !m.dependencies.is_empty() {
            let deps: Vec<String> = m
                .dependencies
                .iter()
                .map(|d| {
                    if d.display_name.is_empty() {
                        d.package_id.clone()
                    } else {
                        d.display_name.clone()
                    }
                })
                .collect();
            lines.push(field_line("Requires", deps.join(", ")));
        }
        if !m.load_after.is_empty() {
            lines.push(field_line("Loads after", m.load_after.join(", ")));
        }
        if !m.load_before.is_empty() {
            lines.push(field_line("Loads before", m.load_before.join(", ")));
        }
        if !m.incompatible_with.is_empty() {
            lines.push(field_line("Incompatible", m.incompatible_with.join(", ")));
        }
        if m.load_first {
            lines.push(field_line("Pinned", "always loads at the top".into()));
        }
        if m.load_last {
            lines.push(field_line("Pinned", "always loads at the bottom".into()));
        }
        if !m.description.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(m.description.clone()));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "Select a mod to see its details. Enter activates or deactivates it.",
            DIM,
        )));
    }

    let block = pane(title("Details", String::new()), false);
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

fn draw_list(frame: &mut Frame, app: &mut App, area: Rect, pane_kind: Pane) {
    let focused = app.pane == pane_kind && app.screen == Screen::Main;
    let active = pane_kind == Pane::Active;
    let (heading, paths, selected) = if active {
        (
            title("Load order", format!("{} active", app.active_paths.len())),
            app.filtered_active(),
            app.selected_active,
        )
    } else {
        (
            title("Inactive", app.inactive_paths.len().to_string()),
            app.filtered_inactive(),
            app.selected_inactive,
        )
    };

    let positions: std::collections::HashMap<&std::path::PathBuf, usize> = if active {
        app.active_paths
            .iter()
            .enumerate()
            .map(|(i, p)| (p, i + 1))
            .collect()
    } else {
        Default::default()
    };
    let mut worst: std::collections::HashMap<&str, Severity> = Default::default();
    if active {
        for issue in &app.issues {
            worst
                .entry(issue.package_id.as_str())
                .and_modify(|w| {
                    if issue.severity == Severity::Error {
                        *w = Severity::Error;
                    }
                })
                .or_insert(issue.severity);
        }
    }
    let number_width = app.active_paths.len().to_string().len();

    let selected_row = selected.min(paths.len().saturating_sub(1));
    let items: Vec<ListItem> = paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let Some(m) = app.mod_ref(p) else {
                return ListItem::new(p.display().to_string());
            };
            let (dim, body) = if i == selected_row {
                (Style::new().fg(PARCHMENT), Style::new().fg(PARCHMENT))
            } else {
                (DIM, Style::new())
            };
            let mut spans = Vec::with_capacity(5);
            if let Some(n) = positions.get(p) {
                spans.push(Span::styled(format!("{n:>number_width$} "), dim));
                let mut glyph = severity_glyph(worst.get(m.package_id.as_str()).copied());
                if i == selected_row {
                    // Brick is too faint on the selection; the glyph's shape still tells severity.
                    glyph.style = glyph.style.fg(PARCHMENT);
                }
                spans.push(glyph);
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(format!("{:<4}", source_tag(m.source)), dim));
            spans.push(Span::styled(m.name.clone(), body));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .block(pane(heading, focused))
        // Full selection color only where the keys go; the other list keeps a quiet marker.
        .highlight_style(if focused {
            Style::new().bg(SELECTED)
        } else {
            Style::new().bg(PLATE)
        })
        .highlight_symbol("▌");

    let state = match pane_kind {
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

fn source_tag(source: rimdeck_core::model::ModSource) -> &'static str {
    match source {
        rimdeck_core::model::ModSource::Dlc => "DLC",
        rimdeck_core::model::ModSource::Local => "L",
        rimdeck_core::model::ModSource::Workshop => "W",
    }
}

fn draw_warnings(frame: &mut Frame, app: &App, area: Rect) {
    let errors = app
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let heading = if app.issues.is_empty() {
        title("No problems found", String::new())
    } else {
        title(
            "Problems",
            format!(
                "{} and {}",
                plural(errors, "error"),
                plural(app.issues.len() - errors, "warning")
            ),
        )
    };
    let lines: Vec<Line> = app
        .issues
        .iter()
        .map(|issue| {
            Line::from(vec![
                severity_glyph(Some(issue.severity)),
                Span::raw(format!(" {}", issue.message)),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane(heading, false))
            .scroll((app.warning_scroll, 0)),
        area,
    );
}

/// Status on one line, key help under it, on a Plate band without a frame.
fn draw_footer(frame: &mut Frame, app: &App, area: Rect, status_rows: u16) {
    let mut status = vec![Span::raw(" ")];
    // The query goes first so a long status message can never push it out of view.
    if app.input_mode == InputMode::Search || !app.search.is_empty() {
        status.push(Span::styled("/", KEY_STYLE));
        status.push(Span::raw(format!("{}  ", app.search)));
    }
    if app.is_busy() {
        status.push(Span::styled(
            format!("{} ", app.spinner_glyph()),
            Style::new().fg(VERDIGRIS),
        ));
    }
    if app.dirty {
        status.push(Span::styled(
            "Unsaved changes (w saves)  ",
            Style::new().fg(OCHRE),
        ));
    }
    status.push(Span::raw(app.status.as_str()));
    let mut help = vec![Span::raw(" ")];
    help.extend(help_spans());
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(status_rows), Constraint::Min(1)])
        .split(area);
    let band = Style::new().bg(PLATE).fg(PARCHMENT);
    for (line, rect) in [(Line::from(status), rows[0]), (Line::from(help), rows[1])] {
        frame.render_widget(
            Paragraph::new(line).wrap(Wrap { trim: false }).style(band),
            rect,
        );
    }
}

const KEY_STYLE: Style = Style::new().fg(VERDIGRIS).add_modifier(Modifier::BOLD);
const LABEL_STYLE: Style = Style::new().fg(DUST);

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
    ("U", "updates"),
    ("P", "packs"),
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

/// Status line(s) plus the wrapped key help, capped so the lists keep room.
fn footer_rows(status: &str, help: &str, width: u16) -> (u16, u16) {
    let width = width.saturating_sub(1).max(1) as usize;
    (
        (wrap_line_count(status, width) as u16).clamp(1, 2),
        (wrap_line_count(help, width) as u16).clamp(1, 4),
    )
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

const INSTANCE_HINT: &str =
    "Instance: a new name creates one, an existing name switches, -name deletes";

fn draw_settings(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 84, 28);
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(Span::styled(
            "Settings",
            Style::default().fg(VERDIGRIS).add_modifier(Modifier::BOLD),
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
            Style::default().fg(VERDIGRIS)
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
            Style::default().fg(VERDIGRIS).add_modifier(Modifier::BOLD),
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
                .border_style(Style::default().fg(VERDIGRIS)),
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
        .border_style(Style::default().fg(VERDIGRIS));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(inner);

    let header = vec![
        Line::from(Span::styled(
            location,
            Style::default().fg(VERDIGRIS).add_modifier(Modifier::BOLD),
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
                ("../".to_string(), Style::default().fg(VERDIGRIS))
            } else if e.is_dir {
                (format!("{}/", e.name), Style::default().fg(VERDIGRIS))
            } else {
                (e.name.clone(), Style::default())
            };
            ListItem::new(label).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" XML files "))
        .highlight_style(
            Style::default()
                .bg(SELECTED)
                .fg(PARCHMENT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌");

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
        .border_style(Style::default().fg(VERDIGRIS));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5)])
        .split(inner);

    let header = vec![
        Line::from(vec![
            Span::styled("Query: ", Style::default().fg(DUST)),
            Span::styled(
                query,
                if editing {
                    Style::default().fg(VERDIGRIS)
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
                    item.tags
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let label = format!(
                "{} {}  ({}){installed}{tags}",
                item.published_file_id,
                item.title,
                rimdeck_core::workshop::format_subs(item.subscriptions),
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
                .bg(SELECTED)
                .fg(PARCHMENT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌");

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

fn draw_updates(frame: &mut Frame, app: &mut App) {
    let height = (frame.area().height.saturating_mul(80) / 100).clamp(14, frame.area().height);
    let area = centered(frame.area(), 84, height);
    frame.render_widget(Clear, area);
    let outer = pane(title("Workshop updates", String::new()), true);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(inner);

    let backend = app.settings.steam_backend;
    let summary = match (app.updates_checked, app.workshop_updates.len()) {
        (false, _) => "Not checked yet".to_string(),
        (true, 0) => "All Workshop mods are up to date".to_string(),
        (true, n) => format!("{} have updates", plural(n, "mod")),
    };
    let header = vec![
        Line::from(vec![
            Span::styled("Workshop via ", DIM),
            Span::styled(backend.label(), Style::new().add_modifier(Modifier::BOLD)),
            Span::styled("  (b switches)   ", DIM),
            Span::raw(summary),
        ]),
        Line::from(vec![
            Span::styled("c", KEY_STYLE),
            Span::styled(" check  ", LABEL_STYLE),
            Span::styled("Enter/u", KEY_STYLE),
            Span::styled(" update  ", LABEL_STYLE),
            Span::styled("A", KEY_STYLE),
            Span::styled(" update all  ", LABEL_STYLE),
            Span::styled("o", KEY_STYLE),
            Span::styled(" open in Steam  ", LABEL_STYLE),
            Span::styled("X X", KEY_STYLE),
            Span::styled(" unsubscribe  ", LABEL_STYLE),
            Span::styled("Esc", KEY_STYLE),
            Span::styled(" close", LABEL_STYLE),
        ]),
    ];
    frame.render_widget(Paragraph::new(header).wrap(Wrap { trim: false }), chunks[0]);

    let items: Vec<ListItem> = app
        .workshop_updates
        .iter()
        .map(|u| {
            ListItem::new(Line::from(vec![
                Span::raw(u.name.clone()),
                Span::styled(
                    format!(
                        "  Steam {}  yours {}",
                        rimdeck_core::steam::ymd(u.remote),
                        rimdeck_core::steam::ymd(u.local)
                    ),
                    DIM,
                ),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(Style::new().bg(SELECTED).fg(PARCHMENT))
        .highlight_symbol("▌");
    let len = app.workshop_updates.len();
    app.updates_selected = app.updates_selected.min(len.saturating_sub(1));
    app.updates_state
        .select((len > 0).then_some(app.updates_selected));
    frame.render_stateful_widget(list, chunks[1], &mut app.updates_state);
}

fn draw_packs(frame: &mut Frame, app: &mut App) {
    let height = (frame.area().height.saturating_mul(80) / 100).clamp(14, frame.area().height);
    let area = centered(frame.area(), 90, height);
    frame.render_widget(Clear, area);
    let outer = pane(title("Packs", "tracked Workshop collections".into()), true);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(inner);
    let keys = [
        ("t", "track"),
        ("c", "check"),
        ("Enter/s", "sync"),
        ("A", "sync all"),
        ("o", "open in Steam"),
        ("X", "stop tracking"),
        ("Esc", "close"),
    ];
    let mut spans = Vec::new();
    for (key, label) in keys {
        spans.push(Span::styled(key, KEY_STYLE));
        spans.push(Span::styled(format!(" {label}  "), LABEL_STYLE));
    }
    let hint = if app.settings.packs.is_empty() {
        "Press t and paste a modpack's Workshop collection URL to keep it complete."
    } else {
        "Sync adds and activates mods the author added; removed ones are only listed."
    };
    frame.render_widget(
        Paragraph::new(vec![Line::from(spans), Line::from(Span::styled(hint, DIM))]),
        chunks[0],
    );

    let name_of = |id: &String| {
        app.mods
            .iter()
            .find(|m| m.published_file_id.as_deref() == Some(id.as_str()))
            .map(|m| m.name.clone())
            .unwrap_or_else(|| id.clone())
    };
    let items: Vec<ListItem> = app
        .settings
        .packs
        .iter()
        .map(|pack| {
            let status = app.pack_status.iter().find(|p| p.id == pack.id);
            let (line, warn) = match status {
                None => (format!("{} mods at the last sync", pack.items.len()), false),
                Some(st) if st.error.is_some() => (
                    format!("couldn't check: {}", st.error.as_deref().unwrap_or_default()),
                    true,
                ),
                Some(st) if st.added.is_empty() && st.removed.is_empty() => {
                    (format!("{} mods, in sync", st.items.len()), false)
                }
                Some(st) => (
                    format!(
                        "{} mods: {} to add ({} to download), {} removed by the author",
                        st.items.len(),
                        st.added.len(),
                        st.missing.len(),
                        st.removed.len()
                    ),
                    true,
                ),
            };
            let mut lines = vec![
                Line::from(pack.title.clone()),
                Line::from(Span::styled(
                    format!("  {line}"),
                    if warn { Style::new().fg(OCHRE) } else { DIM },
                )),
            ];
            if let Some(st) = status.filter(|st| !st.removed.is_empty()) {
                let names: Vec<String> = st.removed.iter().map(name_of).collect();
                lines.push(Line::from(Span::styled(
                    format!("  removed: {}", names.join(", ")),
                    DIM,
                )));
            }
            ListItem::new(lines)
        })
        .collect();
    let list = List::new(items)
        .highlight_style(Style::new().bg(SELECTED))
        .highlight_symbol("▌");
    let len = app.settings.packs.len();
    app.packs_selected = app.packs_selected.min(len.saturating_sub(1));
    app.packs_state.select((len > 0).then_some(app.packs_selected));
    frame.render_stateful_widget(list, chunks[1], &mut app.packs_state);
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
        .border_style(Style::default().fg(VERDIGRIS));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5)])
        .split(inner);

    let header = vec![
        Line::from(vec![
            Span::styled("Install: ", Style::default().fg(DUST)),
            Span::styled(
                query,
                if editing {
                    Style::default().fg(VERDIGRIS)
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
                let latest = rimdeck_core::github::latest_stable_release(&releases)
                    .map(|r| r.tag.as_str())
                    .unwrap_or("—");
                let has_update = rimdeck_core::github::update_for_entry(entry, &releases).is_some();
                let auto = if entry.auto_update { "  auto" } else { "" };
                let label = format!(
                    "{name}  {}  {} → {latest}{auto}",
                    entry.owner_repo, entry.installed_version
                );
                let style = if has_update {
                    Style::default().fg(OCHRE)
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
                .bg(SELECTED)
                .fg(PARCHMENT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌");

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
        .border_style(Style::default().fg(VERDIGRIS));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5)])
        .split(inner);

    let header = vec![
        Line::from(vec![
            Span::styled("Clone: ", Style::default().fg(DUST)),
            Span::styled(
                query,
                if editing {
                    Style::default().fg(VERDIGRIS)
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
                Style::default().fg(OCHRE)
            } else if item.error.is_some() {
                Style::default().fg(BRICK)
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
                .bg(SELECTED)
                .fg(PARCHMENT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌");

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
        PathPromptKind::Pack => (
            "Track a pack",
            "Paste one or more collection URLs or IDs (space-separated)  Enter track and sync  Esc cancel",
            "Collection",
        ),
        PathPromptKind::Rule => {
            let pid = app
                .selected_mod()
                .map(|m| m.package_id.as_str())
                .unwrap_or("");
            let (after, before) = app.user_rules.order_rules(pid);
            rule_hint = format!(
                "a <id> load after · b <id> load before · -a / -b <id> remove · Esc cancel  ·  yours: after [{}]  before [{}]",
                after.join(", "),
                before.join(", ")
            );
            (
                "User load-order rule for the selected mod",
                rule_hint.as_str(),
                "Rule",
            )
        }
    };
    let lines = vec![
        Line::from(Span::styled(
            title,
            Style::default().fg(VERDIGRIS).add_modifier(Modifier::BOLD),
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
rimdeck — RimSort-style RimWorld mod manager

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
  U               Workshop updates: check, update, open in Steam, switch backend
  W               open the selected mod's Workshop page in Steam
  P               packs: track Workshop collections (modpacks) and sync added mods
  X X             unsubscribe from the selected mod (Steam app backend)
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
    fn footer_grows_then_caps() {
        let help = help_plain();
        let wide = help.chars().count() as u16 + 10;
        assert_eq!(footer_rows("ok", &help, wide), (1, 1));
        let (status, help_rows) = footer_rows("ok", &help, 20);
        assert_eq!(status, 1);
        assert!(help_rows > 1 && help_rows <= 4);
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
