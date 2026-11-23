use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::constants::RIMWORLD_STEAM_APP_ID;
use crate::error::{Error, Result};

pub const STEAM_RUN_URL: &str = "steam://run/294100";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchKind {
    Executable(PathBuf),
    SteamProtocol,
}

/// Prefer a local RimWorld executable; fall back to `steam://run/294100`.
pub fn launch_game(game_install: &Path) -> Result<LaunchKind> {
    if let Some(exe) = find_executable(game_install) {
        spawn_executable(&exe, game_install)?;
        return Ok(LaunchKind::Executable(exe));
    }
    open_url(STEAM_RUN_URL)?;
    Ok(LaunchKind::SteamProtocol)
}

pub fn find_executable(game_install: &Path) -> Option<PathBuf> {
    if !game_install.is_dir() {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let app = game_install.join("RimWorldMac.app");
        if app.is_dir() {
            return Some(app);
        }
    }
    for name in [
        "RimWorldWin64.exe",
        "RimWorldWin.exe",
        "RimWorld.exe",
        "RimWorldLinux",
    ] {
        let candidate = game_install.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn spawn_executable(exe: &Path, game_install: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if exe.extension().and_then(|e| e.to_str()) == Some("app") || exe.is_dir() {
            let mut cmd = Command::new("open");
            cmd.arg(exe)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            cmd.spawn().map_err(|e| Error::Launch(e.to_string()))?;
            return Ok(());
        }
    }

    let mut cmd = Command::new(exe);
    cmd.current_dir(game_install)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map_err(|e| Error::Launch(e.to_string()))?;
    Ok(())
}

/// Hand a URL (`steam://...`, `https://...`) to the OS default handler.
pub fn open_url(url: &str) -> Result<()> {
    debug_assert_eq!(STEAM_RUN_URL, format!("steam://run/{RIMWORLD_STEAM_APP_ID}"));
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Error::Launch(e.to_string()))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Error::Launch(e.to_string()))?;
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Error::Launch(e.to_string()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_win64_exe() {
        let dir = std::env::temp_dir().join(format!("rimdeck-launch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("RimWorldWin64.exe");
        fs::write(&exe, b"").unwrap();
        assert_eq!(find_executable(&dir).as_deref(), Some(exe.as_path()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_dir_has_no_exe() {
        assert!(find_executable(Path::new("definitely/missing/RimWorld")).is_none());
    }
}
