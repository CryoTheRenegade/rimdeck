# rimdeck

A RimWorld mod manager written in Rust, with a desktop GUI ([iced](https://iced.rs/)) and a terminal UI ([ratatui](https://ratatui.rs/)) built on one shared core.

rimdeck is a clone of [RimSort](https://github.com/RimSort/RimSort): scan local, DLC, and Steam Workshop mods, sort load order, surface dependency issues, and write `ModsConfig.xml`. The goal is full RimSort feature parity, with the TUI as usable as the GUI.

This project is licensed under the GNU General Public License v3.0 only, the same license as RimSort. Sort tiers, load-order graphs, and metadata shapes follow RimSort’s implementation (rewritten in Rust, not vendored Python).

## Status

Core + TUI: usable sorter plus community/user rules, Steam DB, HTTP fetch of databases, list import/export, game launch, SteamCMD install/update, Steam WebAPI workshop search, GitHub release mods, and a git HEAD updater (clone/pull from GitLab, GitGud, Forgejo, GitHub, …).

GUI: RimSort-style main window (mod info with preview, inactive/active lists with search, double-click to toggle, reorder, sort + sort mode, save, clear, refresh, import/export lists, run game, DB download, issues panel) plus Workshop, GitHub, Git, and Settings tabs.

Both frontends also have: a per-mod rule editor (user loadAfter/loadBefore/loadTop/loadBottom in `userRules.json`), duplicate-install and game-version warnings, Steam Workshop collection import, and instances (named sets of game/mods/workshop/config paths, picked with the Instance setting: a new name creates one, an existing name switches, `-name` deletes).

Texture optimization (RimSort's todds feature) uses [ddsify](https://crates.io/crates/ddsify): `Textures` PNGs in active non-DLC mods become BC1/BC7 DDS with mipmaps, re-encoded only when the PNG is newer. "Remove DDS" (`O` in the TUI) removes DDS files that sit next to a PNG.

`rimdeck-tui build-db [--out PATH] [--no-deps]` merges installed mods into `steamDB.json` (with a Steam WebAPI key it also fetches Workshop "required items" as dependencies).

## Layout

| Crate | Path | What |
| --- | --- | --- |
| `rimdeck-core` | `crates/core` | All domain logic plus `session::Session`: mod lists, rules, DBs, and background workers. No UI code. |
| `rimdeck-tui` | `crates/tui` | Terminal frontend (ratatui). |
| `rimdeck` | `crates/gui` | Desktop frontend (iced). |

Both frontends own a `Session`, call `poll()` on a timer, and pass explicit targets (paths, package IDs). Selection and widgets stay in the frontend. Settings and databases are shared, so either UI can be used on the same setup.

## Look

Both frontends share one palette (`crates/gui/src/theme.rs`, mirrored at the top of `crates/tui/src/ui/mod.rs`): steel panels, parchment text, verdigris for selection and primary actions, ochre for warnings and unsaved changes, brick for errors. The GUI uses [Lexend](https://www.lexend.com/) (SIL OFL, bundled in `crates/gui/fonts`).

## Build

```bash
cargo build --release            # everything, including the rimdeck-steam helper
cargo run --release -p rimdeck   # desktop GUI
cargo run --release -p rimdeck-tui  # terminal UI
cargo test --workspace
```

`rimdeck-steam` (`crates/steam-helper`) is the only binary that links Steamworks; the build copies Valve's `steam_api64.dll` (or `libsteam_api.so`/`.dylib`) next to it. The GUI and TUI never load that library, so they start fine without it; they look for the helper next to their own executable.

## GUI

Click a mod to see its details; double-click it (or use Activate/Deactivate) to move it between lists. The load order is numbered by position; a brick ● marks a mod with an error and an ochre ▲ one with a warning, in both frontends. Click a problem in the bottom panel to jump to its mod.

Keyboard: ↑/↓ select, Ctrl+↑/↓ move the selected active mod, Enter activates/deactivates, Ctrl+S saves. Enter in a search box selects the first match. Build Steam DB adds installed Workshop mods to `steamDB.json`.

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
| `D` | Build DB: merge installed Workshop mods into `steamDB.json` |
| `o` / `O` | Optimize active mods' textures to DDS / delete those DDS files |
| `n` | Workshop search (Steam WebAPI; key in settings or `STEAM_WEB_API_KEY`) |
| `h` | GitHub release mods (install from URL, check/update, auto-update) |
| `G` | Git updater (clone/pull HEAD from any git host; needs `git` on PATH) |
| `U` | Workshop updates: check, update one/all, open in Steam, switch backend (`b`) |
| `W` | Open the selected mod's Workshop page in Steam |
| `P` | Packs: track Workshop collections (modpacks), check, sync added mods |
| `X` `X` | Unsubscribe from the selected mod (Steam app backend) |
| `u` | SteamCMD, GitHub release, or git pull for the selected mod |
| `t` / `b` | Toggle user `loadTop` / `loadBottom` for the selected mod |
| `R` | User `loadAfter` / `loadBefore` rule for the selected mod (`a <id>`, `b <id>`, `-a`/`-b` to remove) |
| `r` | Rescan (press twice to discard unsaved changes) |
| `p` | Path, database, SteamCMD, Steam API key, and GitHub token settings |
| `?` | Help |
| `q` | Quit |

Databases are **not** shipped in this repo. Press `d` to fetch RimSort's community rules and Steam Workshop DB into `%APPDATA%\rimdeck\rimdeck\config\` (or the platform config dir). Missing files are also fetched on startup. Default URLs are GitHub zip archives; change them in path settings.

User rules are written to `userRules.json` in the same folder. Alphabetical sort is RimPy-style (name order with dependencies inserted) and is optional; topological remains the default.

Workshop search (`n`) uses the Steam WebAPI. Put a key in settings or the `STEAM_WEB_API_KEY` / `RIMSORT_STEAM_API_KEY` environment variable (never commit it). Searching by published file ID works without a key. Enter queues a SteamCMD download and activates the mod after rescan; Space downloads only.

**Workshop via** (Settings, or the Workshop tab / `b` on the TUI Updates screen) picks how Workshop mods are downloaded and updated:

- **SteamCMD** (default): works without the Steam app; downloads into your Workshop folder; can't subscribe.
- **Steam app**: uses your running Steam client through the `rimdeck-steam` helper (Steamworks, as RimWorld 294100). Download becomes Subscribe, Update asks Steam to re-download now, and Unsubscribe is available (Steam removes the files). Steam shows you as playing RimWorld only while the helper runs.

**Packs** (Updates tab, `P` in the TUI) keep modpacks distributed as Workshop collections complete. Track any number of collections by URL or ID (paste several at once, separated by spaces or commas); nested collections are expanded, so a pack whose main collection links its other parts needs only that one. Check compares the collection with its contents at your last sync: Sync activates installed mods the author added (in pack order) and downloads the rest with the Workshop via backend; mods the author removed are listed, never deactivated. The first sync of a newly tracked pack treats every item as added, like a collection import. Updates to the pack's mods themselves come from Updates below.

**Updates** (Updates tab, `U` in the TUI) compares each Workshop mod with Steam's public API (no key needed): the installed time comes from Steam's `appworkshop_294100.acf`, falling back to the folder date. Update one, update all, or open the item in Steam (`W` on the main screen). `X` twice unsubscribes the selected mod with the Steam app backend.

SteamCMD (`u`, or from workshop results) uses `+workshop_download_item 294100 <id> validate` like RimSort. The executable is auto-detected, overridable in settings, and downloaded into `%APPDATA%\rimdeck\rimdeck\config\steamcmd\` on first use if missing.

GitHub mods (`h`) install release zips into the local Mods folder. Paste `owner/repo` or a `github.com` URL (optional `/releases/tag/v1.2.3`). Tracking is stored in `github_mods.json`; release metadata is cached in `github_releases.json`. A GitHub token in settings (or `GITHUB_TOKEN` / `RIMSORT_GITHUB_TOKEN`) is optional and never written to logs. Enter/u updates the selected tracked mod; `c` checks for updates; `a` toggles auto-update (also runs on startup); `X` deletes the mod files and tracking. `u` on the main list updates a GitHub-tracked mod from GitHub instead of SteamCMD.

Git mods (`G`) clone and `git pull --ff-only` HEAD, matching RimSort’s “Add from Git” / Git Mod Updater. Any host works (GitHub, GitLab, GitGud, Codeberg/Forgejo, …). GitHub *release zip* tracking stays on `h` and is excluded from this list. Needs `git` on PATH. Overlay: Enter clones+enables; `c` fetches and counts commits behind; Enter/`u` pulls the selection; `A` pulls every repo that is behind; `X` deletes the clone. `u` on the main list pulls a local git clone that is not GitHub-release-tracked.

## Paths

Settings are stored in the platform config directory (on Windows: `%APPDATA%\rimdeck\rimdeck\config\settings.toml`). Game, workshop, and config folders are auto-detected from Steam when possible.

## Credits

- [RimSort](https://github.com/RimSort/RimSort) — original mod manager this TUI clones
- Ludeon Studios — RimWorld
