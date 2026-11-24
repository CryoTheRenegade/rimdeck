//! Steam integration that needs no Steam SDK in this process: Steam's install record for
//! Workshop items, update checks against Steam's public web API, `steam://` links, and
//! driving the `rimdeck-steam` helper (the only binary that links Steamworks).

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

use crate::constants::RIMWORLD_STEAM_APP_ID;
use crate::workshop;

pub const HELPER_NAME: &str = if cfg!(windows) {
    "rimdeck-steam.exe"
} else {
    "rimdeck-steam"
};

/// A Workshop mod whose copy on disk is older than the one on Steam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkshopUpdate {
    pub published_file_id: String,
    pub name: String,
    pub path: PathBuf,
    /// Unix seconds: when the installed copy was last updated.
    pub local: u64,
    /// Unix seconds: when the item was last updated on Steam.
    pub remote: u64,
}

/// `steamapps/workshop/appworkshop_294100.acf` for a `.../steamapps/workshop/content/294100` folder.
pub fn acf_path(workshop: &Path) -> Option<PathBuf> {
    let dir = workshop.parent()?.parent()?;
    Some(dir.join(format!("appworkshop_{RIMWORLD_STEAM_APP_ID}.acf")))
}

/// Where Steam puts this game's Workshop items: the same library as the game
/// (`<lib>/steamapps/common/RimWorld` -> `<lib>/steamapps/workshop/content/294100`).
pub fn workshop_dir_for_game(game_install: &Path) -> Option<PathBuf> {
    let common = game_install.parent()?;
    let steamapps = common.parent()?;
    let named = |p: &Path, n: &str| p.file_name().is_some_and(|f| f.eq_ignore_ascii_case(n));
    (named(common, "common") && named(steamapps, "steamapps")).then(|| {
        steamapps
            .join("workshop")
            .join("content")
            .join(RIMWORLD_STEAM_APP_ID)
    })
}

/// Same folder, also when one spelling differs (separators, case) and both exist.
pub fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// `pfid -> timeupdated` for every item under `WorkshopItemsInstalled` in an acf file.
pub fn installed_times(acf: &str) -> HashMap<String, u64> {
    let tokens = tokenize(acf);
    let mut out = HashMap::new();
    // Walk the tree with a key stack; only `WorkshopItemsInstalled/<id>/timeupdated` matters.
    let mut stack: Vec<String> = Vec::new();
    let mut pending_key: Option<String> = None;
    for token in tokens {
        match token {
            Token::Open => stack.push(pending_key.take().unwrap_or_default()),
            Token::Close => {
                stack.pop();
            }
            Token::Str(s) => match pending_key.take() {
                None => pending_key = Some(s),
                Some(key) => {
                    let in_installed =
                        stack.len() == 3 && stack[1].eq_ignore_ascii_case("WorkshopItemsInstalled");
                    if in_installed && key.eq_ignore_ascii_case("timeupdated") {
                        if let Ok(t) = s.parse() {
                            out.insert(stack[2].clone(), t);
                        }
                    }
                }
            },
        }
    }
    out
}

enum Token {
    Open,
    Close,
    Str(String),
}

/// Valve KeyValues: quoted strings (with `\"` / `\\` escapes) and braces.
fn tokenize(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '{' => out.push(Token::Open),
            '}' => out.push(Token::Close),
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => s.extend(chars.next()),
                        c => s.push(c),
                    }
                }
                out.push(Token::Str(s));
            }
            _ => {}
        }
    }
    out
}

/// Compare installed copies against Steam. `mods` is `(pfid, name, folder)` for Workshop mods.
/// Needs the network; run it off the UI thread.
pub fn check_updates(
    workshop_dir: &Path,
    mods: &[(String, String, PathBuf)],
) -> crate::error::Result<Vec<WorkshopUpdate>> {
    let installed = acf_path(workshop_dir)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| installed_times(&t))
        .unwrap_or_default();
    let pfids: Vec<String> = mods.iter().map(|(id, _, _)| id.clone()).collect();
    let remote = workshop::time_updated(&pfids)?;
    Ok(find_outdated(mods, &installed, &remote))
}

fn find_outdated(
    mods: &[(String, String, PathBuf)],
    installed: &HashMap<String, u64>,
    remote: &HashMap<String, u64>,
) -> Vec<WorkshopUpdate> {
    let mut out: Vec<WorkshopUpdate> = mods
        .iter()
        .filter_map(|(id, name, path)| {
            let remote = *remote.get(id)?;
            // shortcut: folder mtime stands in when Steam has no record (e.g. copied in by hand);
            // it can over-report, which only costs a needless re-download.
            let local = installed.get(id).copied().or_else(|| mtime_secs(path))?;
            (remote > local).then(|| WorkshopUpdate {
                published_file_id: id.clone(),
                name: name.clone(),
                path: path.clone(),
                local,
                remote,
            })
        })
        .collect();
    out.sort_by_key(|u| u.name.to_lowercase());
    out
}

fn mtime_secs(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs(),
    )
}

/// `YYYY-MM-DD` (UTC) for unix seconds.
pub fn ymd(secs: u64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn workshop_page_url(published_file_id: &str) -> String {
    format!("steam://url/CommunityFilePage/{published_file_id}")
}

/// What the helper asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperAction {
    /// Subscribe; Steam downloads the item. Waits until it is installed.
    Subscribe,
    /// Unsubscribe; Steam removes the item.
    Unsubscribe,
    /// Make Steam re-download a subscribed item now. Waits until it is installed.
    Download,
}

impl HelperAction {
    pub fn arg(self) -> &'static str {
        match self {
            Self::Subscribe => "subscribe",
            Self::Unsubscribe => "unsubscribe",
            Self::Download => "download",
        }
    }
}

/// Item IDs that succeeded and `(id, reason)` for failures; `Err` when Steam could not be used.
pub type HelperOutcome = std::result::Result<(Vec<String>, Vec<(String, String)>), String>;

#[derive(Debug, Clone)]
pub enum HelperMsg {
    Progress(String),
    Done(HelperOutcome),
}

/// One stdout line of the helper: `progress <text>`, `ok <id>`, `fail <id> <reason>`,
/// or `error <reason>` (Steam unusable; nothing was done).
#[derive(Debug, PartialEq, Eq)]
pub enum HelperLine {
    Progress(String),
    Ok(String),
    Fail(String, String),
    Error(String),
}

pub fn parse_helper_line(line: &str) -> Option<HelperLine> {
    let (kind, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
    Some(match kind {
        "progress" => HelperLine::Progress(rest.to_string()),
        "ok" => HelperLine::Ok(rest.to_string()),
        "fail" => {
            let (id, reason) = rest.split_once(' ').unwrap_or((rest, "failed"));
            HelperLine::Fail(id.to_string(), reason.to_string())
        }
        "error" => HelperLine::Error(rest.to_string()),
        _ => return None,
    })
}

/// The helper lives next to the running frontend.
pub fn helper_path() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(HELPER_NAME))
}

/// Run the helper for `ids` on a worker thread and report through `tx`.
pub fn spawn_helper(action: HelperAction, ids: Vec<String>, tx: Sender<HelperMsg>) {
    let spawned = std::thread::Builder::new()
        .name("rimdeck-steamworks".into())
        .spawn({
            let tx = tx.clone();
            move || {
                let result = run_helper(action, &ids, &tx);
                let _ = tx.send(HelperMsg::Done(result));
            }
        });
    if let Err(e) = spawned {
        let _ = tx.send(HelperMsg::Done(Err(format!("could not start thread: {e}"))));
    }
}

fn run_helper(action: HelperAction, ids: &[String], tx: &Sender<HelperMsg>) -> HelperOutcome {
    let exe = helper_path().filter(|p| p.is_file()).ok_or_else(|| {
        format!("{HELPER_NAME} was not found next to rimdeck; build it with `cargo build --release` (whole workspace)")
    })?;
    let mut cmd = Command::new(&exe);
    cmd.arg(action.arg())
        .args(ids)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not run {HELPER_NAME}: {e}"))?;
    let (mut ok, mut failed, mut error) = (Vec::new(), Vec::new(), None);
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
            match parse_helper_line(&line) {
                Some(HelperLine::Progress(p)) => {
                    let _ = tx.send(HelperMsg::Progress(format!("Steam: {p}")));
                }
                Some(HelperLine::Ok(id)) => ok.push(id),
                Some(HelperLine::Fail(id, reason)) => failed.push((id, reason)),
                Some(HelperLine::Error(reason)) => error = Some(reason),
                None => {}
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    match error {
        Some(reason) => Err(reason),
        None if !status.success() && ok.is_empty() && failed.is_empty() => {
            Err(format!("{HELPER_NAME} exited with {status}"))
        }
        None => Ok((ok, failed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACF: &str = r#"
"AppWorkshop"
{
	"appid"		"294100"
	"WorkshopItemsInstalled"
	{
		"111"
		{
			"size"		"10"
			"timeupdated"		"1000"
			"manifest"		"5"
		}
		"222"
		{
			"timeupdated"		"3000"
		}
	}
	"WorkshopItemDetails"
	{
		"111"
		{
			"timeupdated"		"9999"
		}
	}
}"#;

    #[test]
    fn reads_installed_times_only() {
        let t = installed_times(ACF);
        assert_eq!(
            t.get("111"),
            Some(&1000),
            "WorkshopItemDetails must not override"
        );
        assert_eq!(t.get("222"), Some(&3000));
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn outdated_when_steam_is_newer() {
        let mods = vec![
            (
                "111".to_string(),
                "Old".to_string(),
                PathBuf::from("/nope/111"),
            ),
            (
                "222".to_string(),
                "Fresh".to_string(),
                PathBuf::from("/nope/222"),
            ),
            (
                "333".to_string(),
                "Gone".to_string(),
                PathBuf::from("/nope/333"),
            ),
        ];
        let installed = installed_times(ACF);
        let remote = HashMap::from([("111".to_string(), 2000), ("222".to_string(), 3000)]);
        let out = find_outdated(&mods, &installed, &remote);
        assert_eq!(out.len(), 1);
        assert_eq!(
            (out[0].name.as_str(), out[0].local, out[0].remote),
            ("Old", 1000, 2000)
        );
    }

    #[test]
    fn acf_sits_beside_the_content_folder() {
        let ws = Path::new("D:/Steam/steamapps/workshop/content/294100");
        assert_eq!(
            acf_path(ws).unwrap(),
            Path::new("D:/Steam/steamapps/workshop/appworkshop_294100.acf")
        );
    }

    #[test]
    fn workshop_folder_follows_the_game_library() {
        assert_eq!(
            workshop_dir_for_game(Path::new("D:/Steam/steamapps/common/RimWorld")).unwrap(),
            Path::new("D:/Steam/steamapps/workshop/content/294100")
        );
        assert_eq!(workshop_dir_for_game(Path::new("C:/Games/RimWorld")), None);
    }

    #[test]
    fn formats_dates() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(1_700_000_000), "2023-11-14");
        assert_eq!(ymd(951_782_400), "2000-02-29");
    }

    #[test]
    fn parses_helper_lines() {
        assert_eq!(
            parse_helper_line("ok 123"),
            Some(HelperLine::Ok("123".into()))
        );
        assert_eq!(
            parse_helper_line("fail 9 not subscribed"),
            Some(HelperLine::Fail("9".into(), "not subscribed".into()))
        );
        assert_eq!(
            parse_helper_line("error Steam is not running"),
            Some(HelperLine::Error("Steam is not running".into()))
        );
        assert_eq!(
            parse_helper_line("progress 1/2"),
            Some(HelperLine::Progress("1/2".into()))
        );
        assert_eq!(parse_helper_line("garbage"), None);
    }
}
