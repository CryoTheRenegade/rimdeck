//! Workshop, GitHub, and git tabs: thin views over the shared `Session` jobs.

use std::path::{Path, PathBuf};

use iced::widget::{
    button, column, container, pick_list, row, scrollable, space, text, text_input, toggler,
};
use iced::{Alignment, Element, Length};

use rimdeck_core::settings::SteamBackend;
use rimdeck_core::steam::ymd;
use rimdeck_core::workshop::format_subs;

use crate::theme::{self, DUST as DIM_COLOR, OCHRE as WARN_COLOR};
use crate::{Gui, Message};

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
            Message::SteamBackend(backend) => s.set_steam_backend(backend),
            Message::CheckUpdates => s.check_workshop_updates(),
            Message::PackQuery(q) => self.pack_query = q,
            Message::TrackPack => {
                s.track_pack(&self.pack_query);
                self.pack_query.clear();
            }
            Message::CheckPacks => s.check_packs(),
            Message::SyncPack(id) => s.sync_packs(vec![id]),
            Message::SyncAllPacks => s.sync_all_packs(),
            Message::UntrackPack(id) => s.untrack_pack(&id),
            Message::UpdateAllOutdated => s.update_all_outdated(),
            Message::UpdateWorkshop(id) => s.update_workshop_items(vec![id]),
            Message::OpenInSteam(id) => s.open_in_steam(&id),
            Message::Unsubscribe { path, .. } if self.confirm_delete.as_ref() != Some(&path) => {
                self.confirm_delete = Some(path);
            }
            Message::Unsubscribe { id, .. } => {
                self.confirm_delete = None;
                s.unsubscribe_workshop(vec![id]);
            }
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
                s.status = format!("Press Delete again to remove {} from disk", p.display());
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
            button("Search")
                .style(theme::primary)
                .on_press_maybe((!s.workshop_searching()).then_some(Message::WorkshopSearch)),
            button("Import as collection")
                .on_press_maybe((!s.workshop_searching()).then_some(Message::WorkshopCollection))
                .style(theme::quiet),
        ]
        .spacing(8);
        let steamworks = s.settings.steam_backend == SteamBackend::Steamworks;
        let backend = row![
            text("Workshop via").size(14).color(DIM_COLOR),
            pick_list(
                &SteamBackend::ALL[..],
                Some(s.settings.steam_backend),
                Message::SteamBackend
            ),
            text(if steamworks {
                "Uses your Steam app and account (Steam must be running). Can subscribe and unsubscribe."
            } else {
                "Works without the Steam app. Downloads into your Workshop folder; can't subscribe."
            })
            .size(13)
            .color(DIM_COLOR),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let mut page = column![backend, search]
            .spacing(10)
            .padding(12);
        if s.settings.resolved_steam_api_key().is_empty() {
            page = page.push(
                text("Searching by name needs a Steam WebAPI key (add one in Settings). Published file IDs and collection URLs work without one.")
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
            let (get, get_and_activate) = if steamworks {
                ("Subscribe", "Subscribe and activate")
            } else {
                ("Download", "Download and activate")
            };
            let mut actions = row![].spacing(6);
            if installed {
                actions = actions.push(
                    button("Redownload")
                        .on_press(Message::UpdateWorkshop(item.published_file_id.clone()))
                        .style(theme::quiet),
                );
            } else {
                actions = actions
                    .push(button(get_and_activate).on_press(download(true)).style(theme::primary))
                    .push(button(get).on_press(download(false)).style(theme::quiet));
            }
            item_row(
                column![
                    text(&item.title).size(15),
                    text(format!(
                        "Workshop ID {}, {} subscribers{}",
                        item.published_file_id,
                        format_subs(item.subscriptions),
                        if installed { ", installed" } else { "" }
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
            button("Install")
                .style(theme::primary)
                .on_press(Message::GithubInstall),
            button("Check for updates")
                .on_press(Message::GithubCheck)
                .style(theme::quiet),
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
                button("Update")
                    .on_press(Message::GithubUpdate(entry.mod_path.clone()))
                    .style(theme::quiet),
                self.delete_button(&entry.mod_path, Message::GithubDelete),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            let info = text(format!(
                "Installed {}, latest {latest}. {}",
                if entry.installed_version.is_empty() {
                    "?"
                } else {
                    &entry.installed_version
                },
                entry.mod_path.display()
            ))
            .size(12)
            .color(if update.is_some() {
                WARN_COLOR
            } else {
                DIM_COLOR
            });
            item_row(column![text(&entry.owner_repo).size(15), info], actions)
        });
        column![
            install,
            list(
                rows,
                "No GitHub mods tracked yet.",
                s.github_mods.mods.is_empty()
            ),
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
            button("Clone")
                .style(theme::primary)
                .on_press(Message::GitClone),
            button("Check all")
                .on_press(Message::GitCheck)
                .style(theme::quiet),
            button("Pull all behind")
                .on_press(Message::GitPullBehind)
                .style(theme::quiet),
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
                "{}, {} at {}: {status}",
                item.origin, item.branch, item.short_sha
            ))
            .size(12)
            .color(if item.has_update() || item.error.is_some() {
                WARN_COLOR
            } else {
                DIM_COLOR
            });
            let actions = row![
                button("Pull")
                    .on_press(Message::GitPull(item.path.clone()))
                    .style(theme::quiet),
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
            list(
                rows,
                "No cloned mods found in the local Mods folder.",
                s.git_items.is_empty()
            ),
        ]
        .spacing(8)
        .padding(12)
        .height(Length::Fill)
        .into()
    }

    /// Packs (tracked collections) on top, then mods with a newer version on Steam.
    pub(crate) fn updates_view(&self) -> Element<'_, Message> {
        scrollable(
            column![self.packs_section(), self.updates_section()]
                .spacing(24)
                .padding(12),
        )
        .height(Length::Fill)
        .into()
    }

    fn packs_section(&self) -> Element<'_, Message> {
        let s = &self.session;
        let pending = |f: fn(&rimdeck_core::session::PackStatus) -> usize| -> usize {
            s.pack_status.iter().map(f).sum()
        };
        let (added, removed) = (pending(|p| p.added.len()), pending(|p| p.removed.len()));
        let summary = match (s.settings.packs.len(), s.pack_status.is_empty(), added + removed) {
            (0, _, _) => "Track a modpack's Workshop collection to keep it complete".to_string(),
            (_, true, _) => "Not checked yet".to_string(),
            (_, false, 0) => "All packs are in sync".to_string(),
            _ => format!("{added} mod(s) to add, {removed} removed by their authors"),
        };
        let mut header = row![
            text("Packs").size(17).font(theme::SEMIBOLD),
            text(summary).size(13).color(DIM_COLOR),
            space::horizontal(),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if !s.settings.packs.is_empty() {
            header = header.push(
                button("Check packs")
                    .on_press(Message::CheckPacks)
                    .style(theme::quiet),
            );
        }
        if added + removed > 0 {
            header = header.push(
                button("Sync all")
                    .on_press(Message::SyncAllPacks)
                    .style(theme::primary),
            );
        }
        let track = row![
            text_input(
                "Track a Workshop collection: paste its URL or ID",
                &self.pack_query
            )
            .on_input(Message::PackQuery)
            .on_submit(Message::TrackPack)
            .padding(6),
            button("Track").on_press(Message::TrackPack).style(theme::quiet),
        ]
        .spacing(8);
        let name_of = |id: &String| {
            s.mods
                .iter()
                .find(|m| m.published_file_id.as_deref() == Some(id.as_str()))
                .map(|m| m.name.clone())
                .unwrap_or_else(|| id.clone())
        };
        let rows = s.settings.packs.iter().map(|pack| {
            let status = s.pack_status.iter().find(|p| p.id == pack.id);
            let mut info = column![text(&pack.title).size(15)].spacing(2);
            let line = match status {
                None => format!("{} mods at the last sync", pack.items.len()),
                Some(st) if st.error.is_some() => format!(
                    "Couldn't check: {}",
                    st.error.as_deref().unwrap_or_default()
                ),
                Some(st) if st.added.is_empty() && st.removed.is_empty() => {
                    format!("{} mods, in sync", st.items.len())
                }
                Some(st) => format!(
                    "{} mods: {} to add ({} to download), {} removed by the author",
                    st.items.len(),
                    st.added.len(),
                    st.missing.len(),
                    st.removed.len()
                ),
            };
            let attention = status.is_some_and(|st| {
                st.error.is_some() || !st.added.is_empty() || !st.removed.is_empty()
            });
            info = info.push(text(line).size(12).color(if attention {
                WARN_COLOR
            } else {
                DIM_COLOR
            }));
            if let Some(st) = status.filter(|st| !st.removed.is_empty()) {
                let names: Vec<String> = st.removed.iter().map(name_of).collect();
                info = info.push(
                    text(format!("Removed by the author: {}", names.join(", ")))
                        .size(12)
                        .color(DIM_COLOR),
                );
            }
            let mut actions = row![].spacing(6);
            if status.is_some_and(|st| {
                st.error.is_none() && (!st.added.is_empty() || !st.removed.is_empty())
            }) {
                actions = actions.push(
                    button("Sync")
                        .on_press(Message::SyncPack(pack.id.clone()))
                        .style(theme::quiet),
                );
            }
            actions = actions
                .push(
                    button("Open in Steam")
                        .on_press(Message::OpenInSteam(pack.id.clone()))
                        .style(theme::subtle),
                )
                .push(
                    button("Stop tracking")
                        .on_press(Message::UntrackPack(pack.id.clone()))
                        .style(theme::subtle),
                );
            item_row(info, actions)
        });
        column![header, track, column(rows).spacing(6)]
            .spacing(8)
            .into()
    }

    /// Update check results: Workshop mods with a newer version on Steam.
    fn updates_section(&self) -> Element<'_, Message> {
        let s = &self.session;
        let summary = match (s.updates_checked, s.workshop_updates.len()) {
            (false, _) => "Not checked yet".to_string(),
            (true, 0) => "All Workshop mods are up to date".to_string(),
            (true, 1) => "1 mod has an update".to_string(),
            (true, n) => format!("{n} mods have updates"),
        };
        let mut header = row![
            text("Updates").size(17).font(theme::SEMIBOLD),
            text(summary).size(13).color(DIM_COLOR),
            space::horizontal(),
            button("Check for updates")
                .on_press(Message::CheckUpdates)
                .style(theme::quiet),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if !s.workshop_updates.is_empty() {
            header = header.push(
                button("Update all")
                    .on_press(Message::UpdateAllOutdated)
                    .style(theme::primary),
            );
        }
        if s.workshop_updates.is_empty() {
            return header.into();
        }
        let steamworks = s.settings.steam_backend == SteamBackend::Steamworks;
        let rows = s.workshop_updates.iter().map(|u| {
            let mut actions = row![
                button("Update")
                    .on_press(Message::UpdateWorkshop(u.published_file_id.clone()))
                    .style(theme::quiet),
                button("Open in Steam")
                    .on_press(Message::OpenInSteam(u.published_file_id.clone()))
                    .style(theme::subtle),
            ]
            .spacing(6);
            if steamworks {
                actions = actions.push(self.unsubscribe_button(&u.published_file_id, &u.path));
            }
            item_row(
                column![
                    text(&u.name).size(15),
                    text(format!(
                        "Updated on Steam {}, your copy is from {}",
                        ymd(u.remote),
                        ymd(u.local)
                    ))
                    .size(12)
                    .color(DIM_COLOR),
                ],
                actions,
            )
        });
        column![
            header,
            container(scrollable(column(rows).spacing(6))).max_height(260),
        ]
        .spacing(8)
        .into()
    }

    pub(crate) fn unsubscribe_button<'a>(&self, id: &str, path: &Path) -> Element<'a, Message> {
        let armed = self.confirm_delete.as_deref() == Some(path);
        button(if armed { "Confirm unsubscribe" } else { "Unsubscribe" })
            .on_press(Message::Unsubscribe {
                id: id.to_string(),
                path: path.to_path_buf(),
            })
            .style(theme::danger)
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
            .style(theme::danger)
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
    .style(theme::panel)
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
    scrollable(column(rows).spacing(6))
        .height(Length::Fill)
        .into()
}
