use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::error::{Error, Result};

const USER_AGENT: &str = concat!("rimdeck/", env!("CARGO_PKG_VERSION"));
const TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchKind {
    CommunityRules,
    SteamDb,
}

impl FetchKind {
    pub fn filename(self) -> &'static str {
        match self {
            Self::CommunityRules => "communityRules.json",
            Self::SteamDb => "steamDB.json",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::CommunityRules => "community rules",
            Self::SteamDb => "Steam DB",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    Updated,
    UpToDate,
    Skipped,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct FetchReport {
    pub community: FetchOutcome,
    pub steam: FetchOutcome,
}

#[derive(Debug, Clone)]
pub enum FetchMsg {
    Progress(String),
    Done(FetchReport),
}

#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub community_url: String,
    pub steam_url: String,
    pub community_dest: PathBuf,
    pub steam_dest: PathBuf,
    pub cache_path: PathBuf,
    /// When true, skip a target that already exists on disk.
    pub only_missing: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HttpCache {
    #[serde(flatten)]
    entries: HashMap<String, CacheEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CacheEntry {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    etag: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    last_modified: String,
}

pub fn spawn(req: FetchRequest, tx: Sender<FetchMsg>) {
    let tx_thread = tx.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("rimdeck-fetch".into())
        .spawn(move || {
            let report = run_fetch(&req, &tx_thread);
            let _ = tx_thread.send(FetchMsg::Done(report));
        })
    {
        let _ = tx.send(FetchMsg::Done(FetchReport {
            community: FetchOutcome::Failed(format!("could not start download thread: {e}")),
            steam: FetchOutcome::Skipped,
        }));
    }
}

fn run_fetch(req: &FetchRequest, tx: &Sender<FetchMsg>) -> FetchReport {
    let mut cache = load_cache(&req.cache_path);
    let client = match blocking_client() {
        Ok(c) => c,
        Err(e) => {
            let msg = e.to_string();
            return FetchReport {
                community: FetchOutcome::Failed(msg.clone()),
                steam: FetchOutcome::Failed(msg),
            };
        }
    };

    let community = fetch_one(
        &client,
        &req.community_url,
        &req.community_dest,
        FetchKind::CommunityRules,
        req.only_missing,
        &mut cache,
        tx,
    );
    let steam = fetch_one(
        &client,
        &req.steam_url,
        &req.steam_dest,
        FetchKind::SteamDb,
        req.only_missing,
        &mut cache,
        tx,
    );
    if let Err(e) = save_cache(&req.cache_path, &cache) {
        tracing::warn!("http cache: {e}");
    }
    FetchReport { community, steam }
}

fn fetch_one(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    kind: FetchKind,
    only_missing: bool,
    cache: &mut HttpCache,
    tx: &Sender<FetchMsg>,
) -> FetchOutcome {
    if url.trim().is_empty() {
        return FetchOutcome::Skipped;
    }
    if only_missing && dest.is_file() {
        return FetchOutcome::Skipped;
    }
    let _ = tx.send(FetchMsg::Progress(format!("Downloading {}...", kind.label())));
    match download_named_json(client, url, kind.filename(), cache) {
        Ok(DownloadResult::NotModified) => FetchOutcome::UpToDate,
        Ok(DownloadResult::Bytes(bytes)) => match write_atomic(dest, &bytes) {
            Ok(()) => FetchOutcome::Updated,
            Err(e) => FetchOutcome::Failed(e.to_string()),
        },
        Err(e) => FetchOutcome::Failed(e.to_string()),
    }
}

enum DownloadResult {
    Bytes(Vec<u8>),
    NotModified,
}

fn blocking_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|e| Error::Fetch(e.to_string()))
}

fn download_named_json(
    client: &reqwest::blocking::Client,
    url: &str,
    filename: &str,
    cache: &mut HttpCache,
) -> Result<DownloadResult> {
    let mut req = client.get(url);
    if let Some(entry) = cache.entries.get(filename) {
        if !entry.etag.is_empty() {
            req = req.header(reqwest::header::IF_NONE_MATCH, entry.etag.clone());
        }
        if !entry.last_modified.is_empty() {
            req = req.header(
                reqwest::header::IF_MODIFIED_SINCE,
                entry.last_modified.clone(),
            );
        }
    }
    let resp = req.send().map_err(|e| Error::Fetch(e.to_string()))?;
    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(DownloadResult::NotModified);
    }
    if !resp.status().is_success() {
        return Err(Error::Fetch(format!(
            "{} returned HTTP {}",
            filename,
            resp.status()
        )));
    }
    let etag = header_string(resp.headers(), reqwest::header::ETAG);
    let last_modified = header_string(resp.headers(), reqwest::header::LAST_MODIFIED);
    let bytes = resp.bytes().map_err(|e| Error::Fetch(e.to_string()))?;
    let json = extract_named_json(&bytes, filename)?;
    cache.entries.insert(
        filename.to_string(),
        CacheEntry {
            etag,
            last_modified,
        },
    );
    Ok(DownloadResult::Bytes(json))
}

fn header_string(headers: &reqwest::header::HeaderMap, name: reqwest::header::HeaderName) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// If `bytes` is a zip archive, pull out `filename`; otherwise treat the body as JSON.
pub fn extract_named_json(bytes: &[u8], filename: &str) -> Result<Vec<u8>> {
    if looks_like_zip(bytes) {
        return extract_from_zip(bytes, filename);
    }
    let trimmed = strip_bom(bytes);
    if trimmed.first() == Some(&b'{') || trimmed.first() == Some(&b'[') {
        return Ok(trimmed.to_vec());
    }
    Err(Error::Fetch(format!(
        "response is not JSON or a zip containing {filename}"
    )))
}

fn looks_like_zip(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0] == b'P' && bytes[1] == b'K'
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

fn extract_from_zip(bytes: &[u8], filename: &str) -> Result<Vec<u8>> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Fetch(format!("zip: {e}")))?;
    let mut best: Option<(usize, usize)> = None;
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| Error::Fetch(format!("zip: {e}")))?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().replace('\\', "/");
        if name.split('/').any(|p| p == "__MACOSX") {
            continue;
        }
        let base = name.rsplit('/').next().unwrap_or(&name);
        if base.eq_ignore_ascii_case(filename) {
            let score = name.len();
            if best.is_none_or(|(_, s)| score < s) {
                best = Some((i, score));
            }
        }
    }
    let Some((idx, _)) = best else {
        return Err(Error::Fetch(format!("{filename} not found in zip")));
    };
    let mut file = archive
        .by_index(idx)
        .map_err(|e| Error::Fetch(format!("zip: {e}")))?;
    let mut out = Vec::new();
    file.read_to_end(&mut out)
        .map_err(|e| Error::Fetch(e.to_string()))?;
    Ok(out)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn load_cache(path: &Path) -> HttpCache {
    let Ok(text) = fs::read_to_string(path) else {
        return HttpCache::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_cache(path: &Path, cache: &HttpCache) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(cache).map_err(|e| Error::Fetch(e.to_string()))?;
    fs::write(path, text)?;
    Ok(())
}

impl FetchReport {
    pub fn status_line(&self) -> String {
        format!(
            "DBs: community {} · steam {}",
            outcome_text(&self.community),
            outcome_text(&self.steam)
        )
    }

    pub fn any_updated(&self) -> bool {
        matches!(self.community, FetchOutcome::Updated)
            || matches!(self.steam, FetchOutcome::Updated)
    }
}

fn outcome_text(outcome: &FetchOutcome) -> String {
    match outcome {
        FetchOutcome::Updated => "updated".into(),
        FetchOutcome::UpToDate => "up to date".into(),
        FetchOutcome::Skipped => "skipped".into(),
        FetchOutcome::Failed(e) => format!("failed ({e})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn zip_with_nested(filename: &str, payload: &[u8]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            zip.start_file(
                format!("Repo-main/{filename}"),
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(payload).unwrap();
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn extract_json_body_passthrough() {
        let raw = br#"{"rules":{}}"#;
        let got = extract_named_json(raw, "communityRules.json").unwrap();
        assert_eq!(got, raw);
    }

    #[test]
    fn extract_finds_nested_github_zip_entry() {
        let payload = br#"{"version":1,"database":{}}"#;
        let zip = zip_with_nested("steamDB.json", payload);
        let got = extract_named_json(&zip, "steamDB.json").unwrap();
        assert_eq!(got, payload);
    }

    #[test]
    fn extract_errors_when_zip_missing_file() {
        let zip = zip_with_nested("other.json", b"{}");
        let err = extract_named_json(&zip, "steamDB.json").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn write_atomic_replaces_file() {
        let dir = std::env::temp_dir().join(format!("rimdeck-fetch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("steamDB.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        let _ = fs::remove_dir_all(&dir);
    }
}
