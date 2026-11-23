//! Workshop, GitHub, and git tabs: thin views over the shared `Session` jobs.

use std::path::{Path, PathBuf};

use iced::widget::{button, column, container, row, scrollable, space, text, text_input, toggler};
use iced::{Alignment, Element, Length};

use rimdeck_core::workshop::format_subs;

use crate::{Gui, Message, DIM_COLOR, WARN_COLOR};

impl Gui {
    pub(crate) fn apply_manage(&mut self, message: Message) {
        let s = &mut self.session;
        match message {
            Message::WorkshopQuery(q) => self.workshop_query = q,
            Message::WorkshopSearch => s.search_workshop(&self.workshop_query),
            Message::WorkshopCollection => s.import_collection(&self.workshop_query),
            Message::WorkshopDownload {
                id,
                title,
                activate,
            } => s.download_workshop(&id, &title, activate),
            Message::GithubQuery(q) => self.github_query = q,
            Message::GithubInstall => s.github_install(&self.github_query),
            Message::GithubCheck => s.github_check(true),
            Message::GithubUpdate(p) => s.github_update(&p),
            Message::GithubToggleAuto(p) => s.github_toggle_auto_update(&p),
            Message::GitQuery(q) => self.git_query = q,
            Message::GitClone => s.git_clone(&self.git_query),
            Message::GitCheck => s.git_check(),
            Message::GitPullBehind => s.git_pull_updates(),
            Message::GitPull(p) => s.git_pull(&p),
            Message::GithubDelete(p) | Message::GitDelete(p)
                if self.confirm_delete.as_ref() != Some(&p) =>
            {
                s.status = format!(
                    "Press Delete again to remove {} from disk",
                    p.display()
                );
                self.confirm_delete = Some(p);
            }
            Message::GithubDelete(p) => {
                self.confirm_delete = None;
                s.github_uninstall(&p);
            }
            Message::GitDelete(p) => {
                self.confirm_delete = None;
                s.git_uninstall(&p);
            }
            other => tracing::debug!("unhandled message {other:?}"),
        }
    }

    pub(crate) fn workshop_view(&self) -> Element<'_, Message> {
        let s = &self.session;
        let search = row![
            text_input(
                "Search the Steam Workshop, or paste a published file ID / collection URL",
                &self.workshop_query
            )
            .on_input(Message::WorkshopQuery)
            .on_submit(Message::WorkshopSearch)
            .padding(6),
            button("Search").on_press_maybe(
                (!s.workshop_searching()).then_some(Message::WorkshopSearch)
            ),
            button("Import as collection")
                .on_press_maybe((!s.workshop_searching()).then_some(Message::WorkshopCollection))
                .style(button::secondary),
        ]
        .spacing(8);
        let mut page = column![search].spacing(8).padding(12);
        if s.settings.resolved_steam_api_key().is_empty() {
            page = page.push(
                text("No Steam WebAPI key: only published file IDs work. Add a key in Settings.")
                    .color(WARN_COLOR)
                    .size(13),
            );
        }
        let rows = s.workshop_items.iter().map(|item| {
            let installed = s.workshop_installed(&item.published_file_id);
            let download = |activate| Message::WorkshopDownload {
                id: item.published_file_id.clone(),
                title: item.title.clone(),
                activate,
            };
            let mut actions = row![].spacing(6);
            if !installed {
                actions = actions.push(button("Download & activate").on_press(download(true)));
            }
            actions = actions.push(
                button(if installed { "Redownload" } else { "Download" })
                    .on_press(download(false))
                    .style(button::secondary),
            );
            item_row(
                column![
                    text(&item.title).size(15),
                    text(format!(
                        "{} · {} subscribers{}",
                        item.published_file_id,
                        format_subs(item.subscriptions),
                        if installed { " · installed" } else { "" }
                    ))
                    .size(12)
                    .color(DIM_COLOR),
                ],
                actions,
            )
        });
        page.push(list(rows, "No results yet.", s.workshop_items.is_empty()))
            .height(Length::Fill)
            .into()
    }

    pub(crate) fn github_view(&self) -> Element<'_, Message> {
        let s = &self.session;
        let install = row![
            text_input(
                "owner/repo or a github.com URL (optional /releases/tag/TAG or @tag)",
                &self.github_query
            )
            .on_input(Message::GithubQuery)
            .on_submit(Message::GithubInstall)
            .padding(6),
            button("Install").on_press(Message::GithubInstall),
            button("Check for updates")
                .on_press(Message::GithubCheck)
                .style(button::secondary),
        ]
        .spacing(8);
        let rows = s.github_mods.mods.iter().map(|entry| {
            let update = s.github_update_tag(entry);
            let latest = update
                .clone()
                .or_else(|| s.github_latest_tag(&entry.owner_repo))
                .unwrap_or_else(|| "unchecked".into());
            let p = entry.mod_path.clone();
            let actions = row![
                toggler(entry.auto_update)
                    .label("Auto-update")
                    .on_toggle(move |_| Message::GithubToggleAuto(p.clone())),
                button("Update").on_press(Message::GithubUpdate(entry.mod_path.clone())),
                self.delete_button(&entry.mod_path, Message::GithubDelete),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            let info = text(format!(
                "installed {} · latest {latest} · {}",
                if entry.installed_version.is_empty() {
                    "?"
                } else {
                    &entry.installed_version
                },
                entry.mod_path.display()
            ))
            .size(12)
            .color(if update.is_some() { WARN_COLOR } else { DIM_COLOR });
            item_row(column![text(&entry.owner_repo).size(15), info], actions)
        });
        column![
            install,
            list(rows, "No GitHub mods tracked yet.", s.github_mods.mods.is_empty()),
        ]
        .spacing(8)
        .padding(12)
        .height(Length::Fill)
        .into()
    }

    pub(crate) fn git_view(&self) -> Element<'_, Message> {
        let s = &self.session;
        let clone = row![
            text_input(
                "Clone URL (https:// or git@; GitHub, GitLab, GitGud, Forgejo, …)",
                &self.git_query
            )
            .on_input(Message::GitQuery)
            .on_submit(Message::GitClone)
            .padding(6),
            button("Clone").on_press(Message::GitClone),
            button("Check all").on_press(Message::GitCheck).style(button::secondary),
            button("Pull all behind")
                .on_press(Message::GitPullBehind)
                .style(button::secondary),
        ]
        .spacing(8);
        let rows = s.git_items.iter().map(|item| {
            let status = match (&item.error, item.behind) {
                _ if item.is_detached() => "detached".into(),
                (Some(e), _) => e.clone(),
                (None, Some(0)) => "up to date".into(),
                (None, Some(n)) => format!("{n} commit(s) behind"),
                (None, None) => "unchecked".into(),
            };
            let info = text(format!(
                "{} · {} @ {} · {status}",
                item.origin, item.branch, item.short_sha
            ))
            .size(12)
            .color(if item.has_update() || item.error.is_some() {
                WARN_COLOR
            } else {
                DIM_COLOR
            });
            let actions = row![
                button("Pull").on_press(Message::GitPull(item.path.clone())),
                self.delete_button(&item.path, Message::GitDelete),
            ]
            .spacing(8);
            item_row(column![text(&item.name).size(15), info], actions)
        });
        column![
            clone,
            text("Needs git on PATH. Pulls are fast-forward only.")
                .size(12)
                .color(DIM_COLOR),
            list(rows, "No cloned mods found in the local Mods folder.", s.git_items.is_empty()),
        ]
        .spacing(8)
        .padding(12)
        .height(Length::Fill)
        .into()
    }

    fn delete_button<'a>(
        &self,
        path: &Path,
        on_press: fn(PathBuf) -> Message,
    ) -> Element<'a, Message> {
        let armed = self.confirm_delete.as_deref() == Some(path);
        button(if armed { "Confirm delete" } else { "Delete" })
            .on_press(on_press(path.to_path_buf()))
            .style(button::danger)
            .into()
    }
}

fn item_row<'a>(
    info: impl Into<Element<'a, Message>>,
    actions: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(
        row![container(info).width(Length::Fill), actions.into()]
            .spacing(12)
            .align_y(Alignment::Center),
    )
    .padding([6, 8])
    .style(container::rounded_box)
    .into()
}

fn list<'a>(
    rows: impl Iterator<Item = Element<'a, Message>>,
    empty: &'a str,
    is_empty: bool,
) -> Element<'a, Message> {
    if is_empty {
        return column![space::vertical().height(12), text(empty).color(DIM_COLOR)].into();
    }
    scrollable(column(rows).spacing(4))
        .height(Length::Fill)
        .into()
}
