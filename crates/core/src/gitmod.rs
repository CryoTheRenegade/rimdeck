use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

use crate::error::{Error, Result};
use crate::model::{Mod, ModSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGitUrl {
    pub clone_url: String,
    pub branch: Option<String>,
    pub repo_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitModInfo {
    pub path: PathBuf,
    pub name: String,
    pub origin: String,
    pub branch: String,
    pub short_sha: String,
    pub behind: Option<u64>,
    pub error: Option<String>,
}

impl GitModInfo {
    pub fn has_update(&self) -> bool {
        self.behind.is_some_and(|n| n > 0)
    }

    /// HEAD is a bare commit, not a branch (`git rev-parse --abbrev-ref HEAD` == "HEAD").
    pub fn is_detached(&self) -> bool {
        self.branch == "HEAD"
    }
}

#[derive(Debug, Clone)]
pub enum GitJob {
    Clone {
        clone_url: String,
        branch: Option<String>,
        dest: PathBuf,
        activate: bool,
    },
    Check {
        paths: Vec<PathBuf>,
    },
    Pull {
        paths: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone)]
pub enum GitMsg {
    Progress(String),
    Done(GitReport),
}

#[derive(Debug, Clone)]
pub enum GitReport {
    Cloned {
        dest: PathBuf,
        origin: String,
        short_sha: String,
        activate: bool,
        warning: Option<String>,
    },
    Checked {
        items: Vec<GitCheckResult>,
    },
    Pulled {
        succeeded: Vec<PathBuf>,
        failed: Vec<(PathBuf, String)>,
    },
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct GitCheckResult {
    pub path: PathBuf,
    pub origin: String,
    pub branch: String,
    pub short_sha: String,
    pub behind: Option<u64>,
    pub error: Option<String>,
}

impl GitReport {
    pub fn status_line(&self) -> String {
        match self {
            Self::Cloned {
                origin,
                short_sha,
                warning,
                ..
            } => {
                let warn = warning
                    .as_deref()
                    .map(|w| format!(" ({w})"))
                    .unwrap_or_default();
                format!("Git: cloned {origin} @{short_sha}{warn}")
            }
            Self::Checked { items } => {
                let updates = items.iter().filter(|i| i.behind.is_some_and(|n| n > 0)).count();
                let errors = items.iter().filter(|i| i.error.is_some()).count();
                if items.is_empty() {
                    "Git: no repositories to check".into()
                } else if updates == 0 && errors == 0 {
                    "Git: all clones are up to date".into()
                } else if errors == 0 {
                    format!("Git: {updates} update(s) available")
                } else {
                    format!("Git: {updates} update(s), {errors} error(s)")
                }
            }
            Self::Pulled { succeeded, failed } => {
                let ok = succeeded.len();
                let bad = failed.len();
                if bad == 0 {
                    format!("Git: pulled {ok} mod(s)")
                } else if ok == 0 {
                    let detail = failed
                        .first()
                        .map(|(_, m)| m.as_str())
                        .unwrap_or("failed");
                    format!("Git: 0 ok, {bad} failed ({detail})")
                } else {
                    format!("Git: {ok} ok, {bad} failed")
                }
            }
            Self::Failed(e) => format!("Git: {e}"),
        }
    }
}

const BROWSE_MARKERS: &[&str] = &[
    "tree", "blob", "commit", "releases", "issues", "pull", "wiki",
];

/// Parse a clone URL from GitHub/GitLab/Forgejo/Gitea/GitGud browse or SSH URLs.
pub fn parse_git_url(input: &str) -> Option<ParsedGitUrl> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("git@") {
        return parse_scp_ssh(trimmed);
    }
    if let Some(rest) = trimmed.strip_prefix("ssh://") {
        let rest = rest.strip_prefix("git@").unwrap_or(rest);
        let (host, path) = rest.split_once('/')?;
        if host.is_empty() || path.is_empty() {
            return None;
        }
        return parse_path_url(&format!("ssh://git@{host}/{path}"));
    }
    if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        return parse_https(trimmed);
    }
    None
}

fn parse_https(url: &str) -> Option<ParsedGitUrl> {
    let (scheme, rest) = url.split_once("://")?;
    let rest = rest.split_once('@').map(|(_, hostpath)| hostpath).unwrap_or(rest);
    let (host, path) = rest.split_once('/')?;
    if host.is_empty() {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let (repo_path, branch) = split_browse_path(path)?;
    let repo_name = repo_name_from_path(&repo_path)?;
    let clone_url = format!("{scheme}://{host}/{repo_path}");
    Some(ParsedGitUrl {
        clone_url,
        branch,
        repo_name,
    })
}

fn parse_path_url(url: &str) -> Option<ParsedGitUrl> {
    // ssh://git@host/owner/repo
    let without = url.strip_prefix("ssh://git@")?;
    let (host, path) = without.split_once('/')?;
    let (repo_path, branch) = split_browse_path(path)?;
    let repo_name = repo_name_from_path(&repo_path)?;
    Some(ParsedGitUrl {
        clone_url: format!("ssh://git@{host}/{repo_path}"),
        branch,
        repo_name,
    })
}

fn parse_scp_ssh(url: &str) -> Option<ParsedGitUrl> {
    let rest = url.strip_prefix("git@")?;
    let (host, path) = rest.split_once(':')?;
    if host.is_empty() || path.is_empty() {
        return None;
    }
    let (repo_path, branch) = split_browse_path(path)?;
    let repo_name = repo_name_from_path(&repo_path)?;
    Some(ParsedGitUrl {
        clone_url: format!("git@{host}:{repo_path}"),
        branch,
        repo_name,
    })
}

fn split_browse_path(path: &str) -> Option<(String, Option<String>)> {
    let path = path.trim().trim_start_matches('/');
    let mut segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return None;
    }
    let mut branch = None;
    if let Some(idx) = segs.iter().position(|s| *s == "-") {
        if idx + 1 < segs.len() && BROWSE_MARKERS.contains(&segs[idx + 1]) {
            if matches!(segs[idx + 1], "tree" | "blob" | "commit") && idx + 2 < segs.len() {
                branch = Some(segs[idx + 2..].join("/"));
            }
            segs.truncate(idx);
        }
    } else if let Some(idx) = segs.iter().position(|s| *s == "src") {
        if idx + 1 < segs.len() && matches!(segs[idx + 1], "branch" | "commit" | "tag") {
            if idx + 2 < segs.len() {
                branch = Some(segs[idx + 2..].join("/"));
            }
            segs.truncate(idx);
        }
    } else if let Some(idx) = segs.iter().position(|s| BROWSE_MARKERS.contains(s)) {
        if matches!(segs[idx], "tree" | "blob" | "commit") && idx + 1 < segs.len() {
            branch = Some(segs[idx + 1..].join("/"));
        }
        segs.truncate(idx);
    }
    if segs.len() < 2 {
        return None;
    }
    if let Some(last) = segs.last_mut() {
        *last = last.strip_suffix(".git").unwrap_or(last);
        if last.is_empty() {
            return None;
        }
    }
    Some((segs.join("/"), branch.filter(|s| !s.is_empty())))
}

fn repo_name_from_path(repo_path: &str) -> Option<String> {
    let name = repo_path
        .rsplit('/')
        .next()
        .unwrap_or(repo_path)
        .strip_suffix(".git")
        .unwrap_or(repo_path.rsplit('/').next().unwrap_or(repo_path));
    let name = sanitize_folder_name(name);
    (!name.is_empty()).then_some(name)
}

pub fn clone_dest(local_mods: &Path, parsed: &ParsedGitUrl) -> Result<PathBuf> {
    if local_mods.as_os_str().is_empty() {
        return Err(Error::Git("local mods path is not set (press p)".into()));
    }
    let name = sanitize_folder_name(&parsed.repo_name);
    if name.is_empty() {
        return Err(Error::Git("could not derive a folder name".into()));
    }
    Ok(local_mods.join(name))
}

fn sanitize_folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    cleaned.trim_matches([' ', '.']).to_string()
}

/// True when `path` itself is a git work tree (does not walk parents).
pub fn is_git_repo(path: &Path) -> bool {
    let git = path.join(".git");
    git.is_dir() || git.is_file()
}

pub fn inspect_local(path: &Path) -> Option<GitModInfo> {
    if !is_git_repo(path) {
        return None;
    }
    // Read .git directly: spawning `git` 3x per repo took minutes with hundreds of clones.
    let (origin, branch, short_sha) = read_git_dir(path).unwrap_or_default();
    Some(GitModInfo {
        path: path.to_path_buf(),
        name: path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| origin.clone()),
        origin,
        branch,
        short_sha,
        behind: None,
        error: None,
    })
}

/// `(origin url, branch or "HEAD" if detached, short sha)` from the repo's files.
/// shortcut: ignores linked worktrees' commondir; their refs fall back to empty sha.
fn read_git_dir(path: &Path) -> Option<(String, String, String)> {
    let dot_git = path.join(".git");
    let git_dir = if dot_git.is_file() {
        let text = fs::read_to_string(&dot_git).ok()?;
        let target = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
        if target.is_absolute() {
            target
        } else {
            path.join(target)
        }
    } else {
        dot_git
    };
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    let (branch, sha) = match head.strip_prefix("ref:") {
        Some(full_ref) => {
            let full_ref = full_ref.trim();
            let branch = full_ref.strip_prefix("refs/heads/").unwrap_or(full_ref);
            let sha = fs::read_to_string(git_dir.join(full_ref))
                .ok()
                .map(|s| s.trim().to_string())
                .or_else(|| {
                    let packed = fs::read_to_string(git_dir.join("packed-refs")).ok()?;
                    packed.lines().find_map(|l| {
                        let (sha, name) = l.split_once(' ')?;
                        (name.trim() == full_ref).then(|| sha.to_string())
                    })
                })
                .unwrap_or_default();
            (branch.to_string(), sha)
        }
        None => ("HEAD".to_string(), head.to_string()),
    };
    let short_sha = sha.chars().take(7).collect();
    Some((origin_url(&git_dir).unwrap_or_default(), branch, short_sha))
}

fn origin_url(git_dir: &Path) -> Option<String> {
    let config = fs::read_to_string(git_dir.join("config")).ok()?;
    let mut in_origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line.replace(' ', "") == "[remote\"origin\"]";
        } else if in_origin {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "url" {
                    return Some(unescape_config_value(value.trim()));
                }
            }
        }
    }
    None
}

/// git-config values may be quoted and use `\\`, `\"`, `\t`, `\n` escapes.
fn unescape_config_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {}
            '\\' => match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            },
            _ => out.push(c),
        }
    }
    out
}

pub fn discover_git_mods(mods: &[Mod], github_tracked: &[PathBuf]) -> Vec<GitModInfo> {
    mods.iter()
        .filter(|m| m.source == ModSource::Local)
        .filter(|m| !github_tracked.iter().any(|p| p == &m.path))
        .filter_map(|m| {
            let mut info = inspect_local(&m.path)?;
            info.name = m.name.clone();
            Some(info)
        })
        .collect()
}

pub fn git_available() -> bool {
    let mut cmd = git_cmd();
    cmd.arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

pub fn spawn(job: GitJob, tx: Sender<GitMsg>) {
    let tx_thread = tx.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("rimtui-git".into())
        .spawn(move || {
            let report = run_job(&job, &tx_thread);
            let _ = tx_thread.send(GitMsg::Done(report));
        })
    {
        let _ = tx.send(GitMsg::Done(GitReport::Failed(format!(
            "could not start git thread: {e}"
        ))));
    }
}

fn run_job(job: &GitJob, tx: &Sender<GitMsg>) -> GitReport {
    if !git_available() {
        return GitReport::Failed(
            "git is not on PATH (install Git for Windows, or clone mods yourself)".into(),
        );
    }
    match job {
        GitJob::Clone {
            clone_url,
            branch,
            dest,
            activate,
        } => clone_one(clone_url, branch.as_deref(), dest, *activate, tx),
        GitJob::Check { paths } => check_many(paths, tx),
        GitJob::Pull { paths } => pull_many(paths, tx),
    }
}

fn clone_one(
    clone_url: &str,
    branch: Option<&str>,
    dest: &Path,
    activate: bool,
    tx: &Sender<GitMsg>,
) -> GitReport {
    let _ = tx.send(GitMsg::Progress(format!(
        "Git: cloning {}...",
        redact_url(clone_url)
    )));
    if dest.exists() {
        if is_git_repo(dest) {
            let existing = git_output(dest, &["remote", "get-url", "origin"]).unwrap_or_default();
            if origins_match(&existing, clone_url) {
                match pull_one(dest) {
                    Ok(sha) => {
                        return GitReport::Cloned {
                            dest: dest.to_path_buf(),
                            origin: existing,
                            short_sha: sha,
                            activate,
                            warning: None,
                        };
                    }
                    Err(e) => return GitReport::Failed(e.to_string()),
                }
            }
            return GitReport::Failed(format!(
                "folder already exists: {}",
                dest.display()
            ));
        }
        return GitReport::Failed(format!(
            "path already exists and is not a git clone: {}",
            dest.display()
        ));
    }
    if let Some(parent) = dest.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return GitReport::Failed(e.to_string());
        }
    }
    let mut args = vec!["clone".to_string(), "--depth".into(), "1".into()];
    if let Some(branch) = branch {
        args.push("--branch".into());
        args.push(branch.to_string());
        args.push("--single-branch".into());
    }
    args.push("--".into());
    args.push(clone_url.to_string());
    args.push(dest.display().to_string());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Err(e) = git_output_cwd(None, &arg_refs) {
        let _ = fs::remove_dir_all(dest);
        return GitReport::Failed(e.to_string());
    }
    let origin = git_output(dest, &["remote", "get-url", "origin"]).unwrap_or_else(|_| clone_url.to_string());
    let short_sha = git_output(dest, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let warning = if !has_about_xml(dest) {
        Some("cloned repo has no About/About.xml".into())
    } else {
        None
    };
    GitReport::Cloned {
        dest: dest.to_path_buf(),
        origin,
        short_sha,
        activate,
        warning,
    }
}

fn check_many(paths: &[PathBuf], tx: &Sender<GitMsg>) -> GitReport {
    let mut items = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let _ = tx.send(GitMsg::Progress(format!(
            "Git: checking {} ({}/{})...",
            path.file_name()
                .map(|s| s.to_string_lossy())
                .unwrap_or_default(),
            i + 1,
            paths.len()
        )));
        items.push(check_one(path));
    }
    GitReport::Checked { items }
}

fn check_one(path: &Path) -> GitCheckResult {
    let (origin, branch, short_sha) = read_git_dir(path).unwrap_or_default();
    if let Err(e) = git_output(path, &["fetch", "--prune", "origin"]) {
        return GitCheckResult {
            path: path.to_path_buf(),
            origin,
            branch,
            short_sha,
            behind: None,
            error: Some(e.to_string()),
        };
    }
    let behind = commits_behind(path, &branch);
    GitCheckResult {
        path: path.to_path_buf(),
        origin,
        branch,
        short_sha: git_output(path, &["rev-parse", "--short", "HEAD"]).unwrap_or(short_sha),
        behind,
        error: if behind.is_none() {
            Some("no origin tracking branch".into())
        } else {
            None
        },
    }
}

fn pull_many(paths: &[PathBuf], tx: &Sender<GitMsg>) -> GitReport {
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    for path in paths {
        let _ = tx.send(GitMsg::Progress(format!(
            "Git: pulling {}...",
            path.file_name()
                .map(|s| s.to_string_lossy())
                .unwrap_or_default()
        )));
        match pull_one(path) {
            Ok(_) => succeeded.push(path.clone()),
            Err(e) => failed.push((path.clone(), e.to_string())),
        }
    }
    GitReport::Pulled { succeeded, failed }
}

fn pull_one(path: &Path) -> Result<String> {
    git_output(path, &["fetch", "--prune", "origin"])?;
    git_output(path, &["pull", "--ff-only"])?;
    git_output(path, &["rev-parse", "--short", "HEAD"])
}

fn commits_behind(path: &Path, branch: &str) -> Option<u64> {
    if branch.is_empty() || branch == "HEAD" {
        return count_ahead(path, "HEAD", "origin/HEAD")
            .or_else(|| count_ahead(path, "HEAD", "origin/main"))
            .or_else(|| count_ahead(path, "HEAD", "origin/master"));
    }
    let remote = format!("origin/{branch}");
    count_ahead(path, "HEAD", &remote).or_else(|| count_ahead(path, "HEAD", "@{upstream}"))
}

fn count_ahead(path: &Path, from: &str, to: &str) -> Option<u64> {
    let range = format!("{from}..{to}");
    let out = git_output(path, &["rev-list", "--count", &range]).ok()?;
    out.parse().ok()
}

fn origins_match(a: &str, b: &str) -> bool {
    fn norm(s: &str) -> String {
        s.trim()
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .to_ascii_lowercase()
    }
    !a.is_empty() && norm(a) == norm(b)
}

fn has_about_xml(directory: &Path) -> bool {
    directory.join("About").join("About.xml").is_file()
}

fn git_cmd() -> Command {
    let mut cmd = Command::new("git");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GCM_INTERACTIVE", "never");
    cmd.env("GIT_ASKPASS", "");
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String> {
    git_output_cwd(Some(cwd), args)
}

fn git_output_cwd(cwd: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut cmd = git_cmd();
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let output = cmd
        .output()
        .map_err(|e| Error::Git(format!("could not run git: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(Error::Git(redact_url(&detail)));
    }
    Ok(stdout)
}

fn redact_url(text: &str) -> String {
    // Strip user:token@ from URLs if git echoed them.
    let mut out = text.to_string();
    if let Some(start) = out.find("://") {
        if let Some(at) = out[start + 3..].find('@') {
            let cred_start = start + 3;
            let cred = &out[cred_start..cred_start + at];
            if cred.contains(':') {
                out.replace_range(cred_start..cred_start + at, "***");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rimtui-git-{}-{}-{}",
            label,
            std::process::id(),
            TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_github_https_and_ssh() {
        let a = parse_git_url("https://github.com/user/repo.git").unwrap();
        assert_eq!(a.clone_url, "https://github.com/user/repo");
        assert_eq!(a.repo_name, "repo");
        assert_eq!(a.branch, None);

        let b = parse_git_url("https://github.com/user/repo/tree/dev-1.6").unwrap();
        assert_eq!(b.clone_url, "https://github.com/user/repo");
        assert_eq!(b.branch.as_deref(), Some("dev-1.6"));

        let c = parse_git_url("git@github.com:user/repo.git").unwrap();
        assert_eq!(c.clone_url, "git@github.com:user/repo");
        assert_eq!(c.repo_name, "repo");
    }

    #[test]
    fn parse_gitlab_nested_and_browse() {
        let a = parse_git_url("https://gitlab.com/group/sub/mod.git").unwrap();
        assert_eq!(a.clone_url, "https://gitlab.com/group/sub/mod");
        assert_eq!(a.repo_name, "mod");

        let b = parse_git_url("https://gitgud.io/group/mod/-/tree/1.5-stable").unwrap();
        assert_eq!(b.clone_url, "https://gitgud.io/group/mod");
        assert_eq!(b.branch.as_deref(), Some("1.5-stable"));
        assert_eq!(b.repo_name, "mod");
    }

    #[test]
    fn parse_forgejo_src_branch() {
        let a = parse_git_url("https://codeberg.org/user/repo/src/branch/main").unwrap();
        assert_eq!(a.clone_url, "https://codeberg.org/user/repo");
        assert_eq!(a.branch.as_deref(), Some("main"));
    }

    #[test]
    fn parse_rejects_bare_owner_repo() {
        assert!(parse_git_url("owner/repo").is_none());
        assert!(parse_git_url("").is_none());
    }

    #[test]
    fn is_git_repo_requires_dot_git_in_folder() {
        let dir = temp_dir("dotgit");
        assert!(!is_git_repo(&dir));
        fs::create_dir_all(dir.join(".git")).unwrap();
        assert!(is_git_repo(&dir));
        let child = dir.join("nested");
        fs::create_dir_all(&child).unwrap();
        assert!(!is_git_repo(&child));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_skips_workshop_and_github_tracked() {
        let dir = temp_dir("discover");
        let local = dir.join("localgit");
        fs::create_dir_all(local.join(".git")).unwrap();
        let workshop = dir.join("ws");
        fs::create_dir_all(workshop.join(".git")).unwrap();
        let tracked = dir.join("tracked");
        fs::create_dir_all(tracked.join(".git")).unwrap();
        let mods = vec![
            dummy(&local, ModSource::Local, "Local Git"),
            dummy(&workshop, ModSource::Workshop, "WS"),
            dummy(&tracked, ModSource::Local, "Tracked"),
        ];
        let found = discover_git_mods(&mods, std::slice::from_ref(&tracked));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, local);
        let _ = fs::remove_dir_all(&dir);
    }

    fn dummy(path: &Path, source: ModSource, name: &str) -> Mod {
        Mod {
            path: path.to_path_buf(),
            package_id: "a.b".into(),
            name: name.into(),
            authors: vec![],
            description: String::new(),
            url: String::new(),
            mod_version: String::new(),
            supported_versions: vec![],
            source,
            load_after: vec![],
            load_before: vec![],
            incompatible_with: vec![],
            dependencies: vec![],
            load_first: false,
            load_last: false,
            published_file_id: None,
        }
    }

    #[test]
    fn clone_pull_check_local_repo() {
        if !git_available() {
            return;
        }
        let root = temp_dir("flow");
        let remote = root.join("remote.git");
        git_output_cwd(None, &["init", "--bare", &remote.display().to_string()]).unwrap();
        let work = root.join("work");
        fs::create_dir_all(work.join("About")).unwrap();
        fs::write(work.join("About").join("About.xml"), b"<ModMetaData/>").unwrap();
        git_output_cwd(Some(&work), &["init", "-b", "main"])
            .or_else(|_| git_output_cwd(Some(&work), &["init"]))
            .unwrap();
        git_output_cwd(Some(&work), &["config", "user.email", "t@t.test"]).unwrap();
        git_output_cwd(Some(&work), &["config", "user.name", "t"]).unwrap();
        git_output_cwd(Some(&work), &["config", "commit.gpgsign", "false"]).unwrap();
        git_output_cwd(Some(&work), &["add", "."]).unwrap();
        git_output_cwd(Some(&work), &["commit", "-m", "init"]).unwrap();
        git_output_cwd(
            Some(&work),
            &["remote", "add", "origin", &remote.display().to_string()],
        )
        .unwrap();
        git_output_cwd(Some(&work), &["push", "-u", "origin", "HEAD"]).unwrap();
        // The bare repo's HEAD follows init.defaultBranch, which may not be the pushed branch.
        let branch = git_output_cwd(Some(&work), &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap();
        let head_ref = format!("refs/heads/{}", branch.trim());
        git_output_cwd(Some(&remote), &["symbolic-ref", "HEAD", &head_ref]).unwrap();

        let dest = root.join("clone");
        let report = clone_one(
            &remote.display().to_string(),
            None,
            &dest,
            true,
            &std::sync::mpsc::channel().0,
        );
        match report {
            GitReport::Cloned { activate, .. } => assert!(activate),
            other => panic!("{other:?}"),
        }
        assert!(dest.join("About").join("About.xml").is_file());

        let info = inspect_local(&dest).unwrap();
        let cli = |args: &[&str]| git_output(&dest, args).unwrap().trim().to_string();
        assert_eq!(info.origin, cli(&["remote", "get-url", "origin"]));
        assert_eq!(info.branch, cli(&["rev-parse", "--abbrev-ref", "HEAD"]));
        assert!(cli(&["rev-parse", "HEAD"]).starts_with(&info.short_sha));
        assert_eq!(info.short_sha.len(), 7);

        fs::write(work.join("About").join("About.xml"), b"<ModMetaData>2</ModMetaData>").unwrap();
        git_output_cwd(Some(&work), &["add", "."]).unwrap();
        git_output_cwd(Some(&work), &["commit", "-m", "two"]).unwrap();
        git_output_cwd(Some(&work), &["push"]).unwrap();

        let checked = check_one(&dest);
        assert_eq!(checked.behind, Some(1), "{checked:?}");
        pull_one(&dest).unwrap();
        let after = check_one(&dest);
        assert_eq!(after.behind, Some(0));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn redacts_embedded_credentials() {
        let s = redact_url("fatal: https://user:secret@gitgud.io/x.git");
        assert!(!s.contains("secret"));
        assert!(s.contains("***"));
    }
}
