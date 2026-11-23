# rimtui

A RimWorld mod manager written in Rust, with a desktop GUI ([iced](https://iced.rs/)) and a terminal UI ([ratatui](https://ratatui.rs/)) built on one shared core.

rimtui is a clone of [RimSort](https://github.com/RimSort/RimSort): scan local, DLC, and Steam Workshop mods, sort load order, surface dependency issues, and write `ModsConfig.xml`. The goal is full RimSort feature parity, with the TUI as usable as the GUI.

This project is licensed under the GNU General Public License v3.0 only, the same license as RimSort. Sort tiers, load-order graphs, and metadata shapes follow RimSort’s implementation (rewritten in Rust, not vendored Python).

## Status

Core + TUI: usable sorter plus community/user rules, Steam DB, HTTP fetch of databases, list import/export, game launch, SteamCMD install/update, Steam WebAPI workshop search, GitHub release mods, and a git HEAD updater (clone/pull from GitLab, GitGud, Forgejo, GitHub, …).

GUI (early): RimSort-style main window (mod info with preview, inactive/active lists with search, double-click to toggle, reorder, sort + sort mode, save, clear, refresh, run game, DB download, user loadTop/loadBottom, issues panel) and a settings page. Workshop/GitHub/git views and list import/export are TUI-only for now.

## Layout

| Crate | Path | What |
| --- | --- | --- |
| `rimtui-core` | `crates/core` | All domain logic plus `session::Session`: mod lists, rules, DBs, and background workers. No UI code. |
| `rimtui` | `crates/tui` | Terminal frontend (ratatui). |
| `rimtui-gui` | `crates/gui` | Desktop frontend (iced). |

Both frontends own a `Session`, call `poll()` on a timer, and pass explicit targets (paths, package IDs). Selection and widgets stay in the frontend. Settings and databases are shared, so either UI can be used on the same setup.

## Build

```bash
cargo run --release -p rimtui-gui   # desktop GUI
cargo run --release -p rimtui       # terminal UI
cargo test --workspace
```

## GUI

Click a mod to see its info; double-click (or Activate/Deactivate) to move it between lists. ▲/▼ move the selected active mod. Names in the active list turn red/yellow when they have errors/warnings.

## TUI keybindings

| Key | Action |
| --- | --- |
| `Tab` | Switch inactive / active list |
| `j` / `k` or arrows | Move selection |
| `Enter` | Activate or deactivate mod |
| `Ctrl+j` / `Ctrl+k` | Reorder active list |
| `/` | Search |
| `s` | Sort using the current mode (default: topological) |
| `S` | Cycle sort mode (topological / alphabetical) |
| `w` | Write `ModsConfig.xml` |
| `l` | Load a ModsConfig-shaped XML list (file explorer) |
| `e` | Export a ModsConfig-shaped XML list |
| `g` | Launch RimWorld (`RimWorldWin64.exe`, else `steam://run/294100`) |
| `d` | Download/update community rules and Steam DB from configured URLs |
| `n` | Workshop search (Steam WebAPI; key in settings or `STEAM_WEB_API_KEY`) |
| `h` | GitHub release mods (install from URL, check/update, auto-update) |
| `G` | Git updater (clone/pull HEAD from any git host; needs `git` on PATH) |
| `u` | SteamCMD, GitHub release, or git pull for the selected mod |
| `t` / `b` | Toggle user `loadTop` / `loadBottom` for the selected mod |
| `r` | Rescan |
| `p` | Path, database, SteamCMD, Steam API key, and GitHub token settings |
| `?` | Help |
| `q` | Quit |

Databases are **not** shipped in this repo. Press `d` to fetch RimSort's community rules and Steam Workshop DB into `%APPDATA%\rimtui\config\` (or the platform config dir). Missing files are also fetched on startup. Default URLs are GitHub zip archives; change them in path settings.

User rules are written to `userRules.json` in the same folder. Alphabetical sort is RimPy-style (name order with dependencies inserted) and is optional; topological remains the default.

Workshop search (`n`) uses the Steam WebAPI. Put a key in settings or the `STEAM_WEB_API_KEY` / `RIMSORT_STEAM_API_KEY` environment variable (never commit it). Searching by published file ID works without a key. Enter queues a SteamCMD download and activates the mod after rescan; Space downloads only.

SteamCMD (`u`, or from workshop results) uses `+workshop_download_item 294100 <id> validate` like RimSort. The executable is auto-detected, overridable in settings, and downloaded into `%APPDATA%\rimtui\config\steamcmd\` on first use if missing.

GitHub mods (`h`) install release zips into the local Mods folder. Paste `owner/repo` or a `github.com` URL (optional `/releases/tag/v1.2.3`). Tracking is stored in `github_mods.json`; release metadata is cached in `github_releases.json`. A GitHub token in settings (or `GITHUB_TOKEN` / `RIMSORT_GITHUB_TOKEN`) is optional and never written to logs. Enter/u updates the selected tracked mod; `c` checks for updates; `a` toggles auto-update (also runs on startup); `X` deletes the mod files and tracking. `u` on the main list updates a GitHub-tracked mod from GitHub instead of SteamCMD.

Git mods (`G`) clone and `git pull --ff-only` HEAD, matching RimSort’s “Add from Git” / Git Mod Updater. Any host works (GitHub, GitLab, GitGud, Codeberg/Forgejo, …). GitHub *release zip* tracking stays on `h` and is excluded from this list. Needs `git` on PATH. Overlay: Enter clones+enables; `c` fetches and counts commits behind; Enter/`u` pulls the selection; `A` pulls every repo that is behind; `X` deletes the clone. `u` on the main list pulls a local git clone that is not GitHub-release-tracked.

## Paths

Settings are stored in the platform config directory (on Windows: `%APPDATA%\rimtui\config\settings.toml`). Game, workshop, and config folders are auto-detected from Steam when possible.

## Credits

- [RimSort](https://github.com/RimSort/RimSort) — original mod manager this TUI clones
- Ludeon Studios — RimWorld
