use std::fs;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Duration;

use zip::ZipArchive;

use crate::constants::RIMWORLD_STEAM_APP_ID;
use crate::error::{Error, Result};
use crate::settings::Settings;

/// SteamCMD's vprof profiler overflows if too many workshop items are queued
/// in one process (RimSort `STEAMCMD_BATCH_SIZE`).
pub const BATCH_SIZE: usize = 25;

const USER_AGENT: &str = concat!("rimtui/", env!("CARGO_PKG_VERSION"));
const STEAMCMD_ZIP_URL: &str = "https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip";
#[cfg(not(windows))]
const STEAMCMD_TGZ_URL: &str =
    "https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamcmdItem {
    pub published_file_id: String,
    pub activate: bool,
}

#[derive(Debug, Clone)]
pub struct SteamcmdRequest {
    pub configured_exe: PathBuf,
    pub workshop: PathBuf,
    pub install_dir: PathBuf,
    pub items: Vec<SteamcmdItem>,
}

#[derive(Debug, Clone)]
pub enum SteamcmdMsg {
    Progress(String),
    Done(SteamcmdReport),
}

#[derive(Debug, Clone, Default)]
pub struct SteamcmdReport {
    pub succeeded: Vec<String>,
    pub failed: Vec<(String, String)>,
}

impl SteamcmdReport {
    pub fn status_line(&self) -> String {
        let ok = self.succeeded.len();
        let bad = self.failed.len();
        if bad == 0 {
            format!("SteamCMD: downloaded {ok} mod(s)")
        } else if ok == 0 {
            let detail = self
                .failed
                .first()
                .map(|(_, m)| m.as_str())
                .unwrap_or("failed");
            format!("SteamCMD: 0 ok, {bad} failed ({detail})")
        } else {
            format!("SteamCMD: {ok} ok, {bad} failed")
        }
    }

    pub fn any_success(&self) -> bool {
        !self.succeeded.is_empty()
    }
}

/// Locate an existing SteamCMD executable (configured path is checked by the caller).
pub fn detect() -> Option<PathBuf> {
    detect_from(Settings::steamcmd_dir(), std::env::var_os("PATH"))
}

fn detect_from(managed_dir: Option<PathBuf>, path_var: Option<std::ffi::OsString>) -> Option<PathBuf> {
    steamcmd_candidates(managed_dir, path_var)
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn steamcmd_exe_name() -> &'static str {
    if cfg!(windows) {
        "steamcmd.exe"
    } else {
        "steamcmd.sh"
    }
}

fn steamcmd_candidates(
    managed_dir: Option<PathBuf>,
    path_var: Option<std::ffi::OsString>,
) -> Vec<PathBuf> {
    let exe = steamcmd_exe_name();
    let mut out = Vec::new();
    if let Some(dir) = managed_dir {
        out.push(dir.join(exe));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(profile);
        out.push(home.join("steamcmd").join(exe));
        out.push(
            home.join("AppData")
                .join("Local")
                .join("RimSort")
                .join("instances")
                .join("Default")
                .join("steamcmd")
                .join(exe),
        );
    }
    #[cfg(windows)]
    {
        out.push(PathBuf::from(r"C:\steamcmd").join(exe));
        out.push(PathBuf::from(r"C:\Program Files (x86)\SteamCMD").join(exe));
        out.push(PathBuf::from(r"C:\Program Files\SteamCMD").join(exe));
    }
    #[cfg(not(windows))]
    {
        out.push(PathBuf::from("/usr/games").join("steamcmd"));
        out.push(PathBuf::from("/usr/bin").join("steamcmd"));
    }
    if let Some(path) = path_var {
        for dir in std::env::split_paths(&path) {
            out.push(dir.join(exe));
            if !cfg!(windows) {
                out.push(dir.join("steamcmd"));
            }
        }
    }
    out
}

/// SteamCMD `force_install_dir` so workshop items land under `.../steamapps/workshop/content/294100`.
pub fn force_install_dir(workshop: &Path) -> Option<PathBuf> {
    let id = workshop.file_name()?;
    if id != RIMWORLD_STEAM_APP_ID {
        return None;
    }
    let content = workshop.parent()?;
    if content.file_name()? != "content" {
        return None;
    }
    let workshop_dir = content.parent()?;
    if workshop_dir.file_name()? != "workshop" {
        return None;
    }
    let steamapps = workshop_dir.parent()?;
    if steamapps.file_name()? != "steamapps" {
        return None;
    }
    steamapps.parent().map(Path::to_path_buf)
}

pub fn workshop_item_dir(workshop: &Path, published_file_id: &str) -> PathBuf {
    workshop.join(published_file_id)
}

pub fn downloaded_item_dir(install_dir: &Path, published_file_id: &str) -> PathBuf {
    install_dir
        .join("steamapps")
        .join("workshop")
        .join("content")
        .join(RIMWORLD_STEAM_APP_ID)
        .join(published_file_id)
}

pub fn spawn(req: SteamcmdRequest, tx: Sender<SteamcmdMsg>) {
    let tx_thread = tx.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("rimtui-steamcmd".into())
        .spawn(move || {
            let report = run_jobs(&req, &tx_thread);
            let _ = tx_thread.send(SteamcmdMsg::Done(report));
        })
    {
        let _ = tx.send(SteamcmdMsg::Done(SteamcmdReport {
            succeeded: Vec::new(),
            failed: vec![(
                String::new(),
                format!("could not start SteamCMD thread: {e}"),
            )],
        }));
    }
}

fn run_jobs(req: &SteamcmdRequest, tx: &Sender<SteamcmdMsg>) -> SteamcmdReport {
    let exe = match resolve_exe(&req.configured_exe, &req.install_dir, tx) {
        Ok(p) => p,
        Err(e) => {
            return SteamcmdReport {
                succeeded: Vec::new(),
                failed: req
                    .items
                    .iter()
                    .map(|i| (i.published_file_id.clone(), e.to_string()))
                    .collect(),
            };
        }
    };
    if req.workshop.as_os_str().is_empty() {
        return SteamcmdReport {
            succeeded: Vec::new(),
            failed: req
                .items
                .iter()
                .map(|i| {
                    (
                        i.published_file_id.clone(),
                        "workshop path is not set (press p)".into(),
                    )
                })
                .collect(),
        };
    }
    let install_dir = force_install_dir(&req.workshop).unwrap_or_else(|| {
        req.install_dir.join("steam")
    });
    if let Err(e) = fs::create_dir_all(&install_dir) {
        return SteamcmdReport {
            succeeded: Vec::new(),
            failed: vec![(String::new(), format!("could not create install dir: {e}"))],
        };
    }
    if let Err(e) = fs::create_dir_all(&req.workshop) {
        return SteamcmdReport {
            succeeded: Vec::new(),
            failed: vec![(String::new(), format!("could not create workshop dir: {e}"))],
        };
    }

    let mut report = SteamcmdReport::default();
    for (batch_idx, chunk) in req.items.chunks(BATCH_SIZE).enumerate() {
        let batch_num = batch_idx + 1;
        let batches = req.items.len().div_ceil(BATCH_SIZE).max(1);
        let _ = tx.send(SteamcmdMsg::Progress(format!(
            "SteamCMD batch {batch_num}/{batches}: {} mod(s)...",
            chunk.len()
        )));
        let ids: Vec<String> = chunk.iter().map(|i| i.published_file_id.clone()).collect();
        match download_batch(&exe, &install_dir, &ids, tx) {
            Ok(output) => merge_batch(&mut report, chunk, &output, &install_dir, &req.workshop),
            Err(e) => {
                for item in chunk {
                    report
                        .failed
                        .push((item.published_file_id.clone(), e.to_string()));
                }
            }
        }
    }
    report
}

fn resolve_exe(
    configured: &Path,
    managed_dir: &Path,
    tx: &Sender<SteamcmdMsg>,
) -> Result<PathBuf> {
    if configured.is_file() {
        return Ok(configured.to_path_buf());
    }
    if let Some(found) = detect() {
        return Ok(found);
    }
    let _ = tx.send(SteamcmdMsg::Progress(
        "SteamCMD not found; downloading official zip...".into(),
    ));
    install_steamcmd(managed_dir)
}

fn install_steamcmd(dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dir).map_err(|e| Error::Steamcmd(e.to_string()))?;
    #[cfg(windows)]
    {
        let bytes = download_bytes(STEAMCMD_ZIP_URL)?;
        extract_zip_exe(&bytes, dir, "steamcmd.exe")?;
        let exe = dir.join("steamcmd.exe");
        if exe.is_file() {
            return Ok(exe);
        }
        Err(Error::Steamcmd(
            "steamcmd.exe missing after extracting the official zip".into(),
        ))
    }
    #[cfg(not(windows))]
    {
        let _ = (STEAMCMD_ZIP_URL, STEAMCMD_TGZ_URL);
        Err(Error::Steamcmd(
            "auto-install of SteamCMD is Windows-only for now; set the SteamCMD path in settings"
                .into(),
        ))
    }
}

fn download_bytes(url: &str) -> Result<Vec<u8>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| Error::Steamcmd(e.to_string()))?;
    let resp = client
        .get(url)
        .send()
        .map_err(|e| Error::Steamcmd(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(Error::Steamcmd(format!(
            "SteamCMD download returned HTTP {}",
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .map_err(|e| Error::Steamcmd(e.to_string()))?;
    Ok(bytes.to_vec())
}

fn extract_zip_exe(bytes: &[u8], dest: &Path, filename: &str) -> Result<()> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Steamcmd(format!("zip: {e}")))?;
    let mut found = None;
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| Error::Steamcmd(format!("zip: {e}")))?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().replace('\\', "/");
        let base = name.rsplit('/').next().unwrap_or(&name);
        if base.eq_ignore_ascii_case(filename) {
            found = Some(i);
            break;
        }
    }
    let Some(idx) = found else {
        return Err(Error::Steamcmd(format!("{filename} not found in zip")));
    };
    let mut file = archive
        .by_index(idx)
        .map_err(|e| Error::Steamcmd(format!("zip: {e}")))?;
    let mut out = Vec::new();
    file.read_to_end(&mut out)
        .map_err(|e| Error::Steamcmd(e.to_string()))?;
    let path = dest.join(filename);
    let mut f = fs::File::create(&path).map_err(|e| Error::Steamcmd(e.to_string()))?;
    f.write_all(&out)
        .map_err(|e| Error::Steamcmd(e.to_string()))?;
    Ok(())
}

fn download_batch(
    exe: &Path,
    install_dir: &Path,
    ids: &[String],
    tx: &Sender<SteamcmdMsg>,
) -> Result<String> {
    let mut last_err = None;
    for attempt in 0..2 {
        let output = run_steamcmd(exe, install_dir, ids, tx)?;
        if attempt == 0 && looks_like_self_update(&output) && parse_output(&output).is_empty() {
            let _ = tx.send(SteamcmdMsg::Progress(
                "SteamCMD updated itself; retrying download...".into(),
            ));
            last_err = Some(output);
            continue;
        }
        return Ok(output);
    }
    Ok(last_err.unwrap_or_default())
}

fn run_steamcmd(
    exe: &Path,
    install_dir: &Path,
    ids: &[String],
    tx: &Sender<SteamcmdMsg>,
) -> Result<String> {
    let cwd = exe.parent().unwrap_or(Path::new("."));
    let mut cmd = Command::new(exe);
    cmd.current_dir(cwd)
        .arg("+force_install_dir")
        .arg(install_dir)
        .arg("+login")
        .arg("anonymous");
    for id in ids {
        cmd.arg("+workshop_download_item")
            .arg(RIMWORLD_STEAM_APP_ID)
            .arg(id)
            .arg("validate");
    }
    cmd.arg("+quit")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Steamcmd(format!("could not start SteamCMD: {e}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Steamcmd("SteamCMD stdout missing".into()))?;
    let stderr = child.stderr.take();
    let stderr_handle = stderr.map(|err| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let reader = BufReader::new(err);
            for line in reader.lines().map_while(std::result::Result::ok) {
                buf.push_str(&line);
                buf.push('\n');
            }
            buf
        })
    });
    let mut combined = String::new();
    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(std::result::Result::ok) {
        if is_progress_line(&line) {
            let _ = tx.send(SteamcmdMsg::Progress(truncate_status(&line)));
        }
        combined.push_str(&line);
        combined.push('\n');
    }
    if let Some(handle) = stderr_handle {
        if let Ok(err) = handle.join() {
            combined.push_str(&err);
        }
    }
    let status = child
        .wait()
        .map_err(|e| Error::Steamcmd(format!("SteamCMD wait failed: {e}")))?;
    if combined.trim().is_empty() && !status.success() {
        return Err(Error::Steamcmd(format!(
            "SteamCMD exited with {}",
            status.code().unwrap_or(-1)
        )));
    }
    Ok(combined)
}

fn is_progress_line(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    l.contains("downloading item")
        || l.contains("success. downloaded")
        || l.contains("error!")
        || l.contains("update complete")
        || l.contains("login")
}

fn truncate_status(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() <= 120 {
        trimmed.to_string()
    } else {
        trimmed.chars().take(117).collect::<String>() + "..."
    }
}

fn merge_batch(
    report: &mut SteamcmdReport,
    chunk: &[SteamcmdItem],
    output: &str,
    install_dir: &Path,
    workshop: &Path,
) {
    let parsed = parse_output(output);
    for item in chunk {
        let parsed_one = parsed
            .iter()
            .find(|r| r.published_file_id == item.published_file_id);
        let src = downloaded_item_dir(install_dir, &item.published_file_id);
        let dest = workshop_item_dir(workshop, &item.published_file_id);
        match parsed_one {
            Some(r) if r.ok => match place_download(&src, &dest) {
                Ok(()) => report.succeeded.push(item.published_file_id.clone()),
                Err(e) => report
                    .failed
                    .push((item.published_file_id.clone(), e.to_string())),
            },
            Some(r) => report
                .failed
                .push((item.published_file_id.clone(), r.message.clone())),
            None if src.is_dir() => match place_download(&src, &dest) {
                Ok(()) => report.succeeded.push(item.published_file_id.clone()),
                Err(e) => report
                    .failed
                    .push((item.published_file_id.clone(), e.to_string())),
            },
            None => {
                let hint = if looks_like_self_update(output) {
                    "SteamCMD updated itself; try again".to_string()
                } else {
                    "no Success/ERROR line in SteamCMD output".to_string()
                };
                report
                    .failed
                    .push((item.published_file_id.clone(), hint));
            }
        }
    }
}

fn place_download(src: &Path, dest: &Path) -> Result<()> {
    if !src.is_dir() {
        return Err(Error::Steamcmd(format!(
            "download folder missing: {}",
            src.display()
        )));
    }
    if same_dir(src, dest) {
        return Ok(());
    }
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|e| Error::Steamcmd(e.to_string()))?;
    }
    copy_dir(src, dest).map_err(|e| Error::Steamcmd(e.to_string()))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(aa), Ok(bb)) => aa == bb,
        _ => false,
    }
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedItem {
    pub published_file_id: String,
    pub ok: bool,
    pub message: String,
}

/// Parse SteamCMD stdout for `workshop_download_item` success/error lines (RimSort).
pub fn parse_output(text: &str) -> Vec<ParsedItem> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(id) = after_token(line, "Success. Downloaded item ") {
            let id = id
                .split(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .find(|s| !s.is_empty())
                .unwrap_or(id)
                .to_string();
            if is_pfid(&id) {
                out.push(ParsedItem {
                    published_file_id: id,
                    ok: true,
                    message: "downloaded".into(),
                });
            }
            continue;
        }
        if let Some(rest) = after_token(line, "ERROR! Download item ") {
            let mut parts = rest.splitn(2, char::is_whitespace);
            let id = parts.next().unwrap_or("").to_string();
            let msg = parts
                .next()
                .unwrap_or("failed")
                .trim()
                .trim_start_matches("failed")
                .trim()
                .trim_start_matches('(')
                .trim_end_matches(')')
                .trim();
            if is_pfid(&id) {
                out.push(ParsedItem {
                    published_file_id: id,
                    ok: false,
                    message: if msg.is_empty() {
                        "failed".into()
                    } else {
                        msg.to_string()
                    },
                });
            }
        }
    }
    out
}

pub fn looks_like_self_update(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("update complete") || t.contains("installing update")
}

fn after_token<'a>(line: &'a str, token: &str) -> Option<&'a str> {
    let idx = line.find(token)?;
    Some(line[idx + token.len()..].trim())
}

pub fn is_pfid(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t != "0"
        && t.len() <= 20
        && t.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_success_and_error_from_rimsort_log() {
        let output = r#"
workshop_download_item 294100 3250762483 validate
Downloading item 3250762483 ...
ERROR! Download item 3250762483 failed (No Connection).
workshop_download_item 294100 3396308787 validate
Downloading item 3396308787 ...
Success. Downloaded item 3396308787 to "c:\users\me\steamapps\workshop\content\294100\3396308787" (1101355 bytes)
"#;
        let parsed = parse_output(output);
        assert_eq!(parsed.len(), 2);
        assert!(!parsed[0].ok);
        assert_eq!(parsed[0].published_file_id, "3250762483");
        assert!(parsed[0].message.to_ascii_lowercase().contains("connection"));
        assert!(parsed[1].ok);
        assert_eq!(parsed[1].published_file_id, "3396308787");
    }

    #[test]
    fn force_install_dir_walks_steam_workshop_layout() {
        let workshop = PathBuf::from("SteamLibrary")
            .join("steamapps")
            .join("workshop")
            .join("content")
            .join("294100");
        assert_eq!(
            force_install_dir(&workshop),
            Some(PathBuf::from("SteamLibrary"))
        );
        assert!(force_install_dir(Path::new("mods")).is_none());
    }

    #[test]
    fn downloaded_item_dir_matches_steamcmd_layout() {
        let dir = downloaded_item_dir(Path::new(r"D:\SteamLibrary"), "12345");
        assert!(dir.ends_with(Path::new(r"steamapps\workshop\content\294100\12345"))
            || dir.ends_with("steamapps/workshop/content/294100/12345"));
    }

    #[test]
    fn detects_self_update_banner() {
        assert!(looks_like_self_update(
            "Update complete, exiting...\nSteam Console Client"
        ));
        assert!(!looks_like_self_update("Success. Downloaded item 1"));
    }

    #[test]
    fn pfid_rejects_junk() {
        assert!(is_pfid("3396308787"));
        assert!(!is_pfid(""));
        assert!(!is_pfid("0"));
        assert!(!is_pfid("harmony"));
    }

    #[test]
    fn candidate_list_includes_managed_dir() {
        let managed = PathBuf::from(r"C:\Users\me\AppData\Roaming\rimtui\config\steamcmd");
        let cands = steamcmd_candidates(Some(managed.clone()), None);
        assert!(cands.iter().any(|p| p.starts_with(&managed)));
    }

    #[test]
    fn extract_zip_finds_steamcmd_exe() {
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            zip.start_file("steamcmd.exe", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"fake-exe").unwrap();
            zip.finish().unwrap();
        }
        let bytes = cursor.into_inner();
        let dir = std::env::temp_dir().join(format!("rimtui-steamcmd-zip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        extract_zip_exe(&bytes, &dir, "steamcmd.exe").unwrap();
        assert_eq!(fs::read(dir.join("steamcmd.exe")).unwrap(), b"fake-exe");
        let _ = fs::remove_dir_all(&dir);
    }
}
