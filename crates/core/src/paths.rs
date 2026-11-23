use std::fs;
use std::path::{Path, PathBuf};

use crate::constants::RIMWORLD_STEAM_APP_ID;

#[derive(Debug, Clone, Default)]
pub struct DetectedPaths {
    pub steam_root: Option<PathBuf>,
    pub game_install: Option<PathBuf>,
    pub local_mods: Option<PathBuf>,
    pub workshop: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
}

pub fn autodetect() -> DetectedPaths {
    let mut detected = DetectedPaths {
        steam_root: steam_root(),
        config_dir: default_config_dir(),
        ..DetectedPaths::default()
    };

    if let Some(game) = find_rimworld(detected.steam_root.as_deref()) {
        detected.local_mods = Some(game.join("Mods"));
        if let Some(library) = library_root_for_game(&game) {
            detected.workshop = Some(
                library
                    .join("steamapps")
                    .join("workshop")
                    .join("content")
                    .join(RIMWORLD_STEAM_APP_ID),
            );
        }
        detected.game_install = Some(game);
    }

    detected
}

fn steam_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows_steam_root().or_else(fallback_steam_roots)
    }
    #[cfg(not(windows))]
    {
        fallback_steam_roots()
    }
}

#[cfg(windows)]
fn windows_steam_root() -> Option<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey(r"Software\Valve\Steam") {
        if let Ok(path) = key.get_value::<String, _>("SteamPath") {
            let p = PathBuf::from(path.replace('/', "\\"));
            if p.is_dir() {
                return Some(p);
            }
        }
    }

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    for sub in [r"SOFTWARE\WOW6432Node\Valve\Steam", r"SOFTWARE\Valve\Steam"] {
        if let Ok(key) = hklm.open_subkey(sub) {
            if let Ok(path) = key.get_value::<String, _>("InstallPath") {
                let p = PathBuf::from(path);
                if p.is_dir() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn fallback_steam_roots() -> Option<PathBuf> {
    let candidates = steam_root_candidates();
    candidates.into_iter().find(|p| p.is_dir())
}

fn steam_root_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        out.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
        out.push(PathBuf::from(r"C:\Program Files\Steam"));
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = dirs_home() {
            out.push(home.join(".steam/steam"));
            out.push(home.join(".steam/debian-installation"));
            out.push(home.join(".local/share/Steam"));
        }
    }
    out
}

#[cfg(not(windows))]
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn find_rimworld(steam_root: Option<&Path>) -> Option<PathBuf> {
    let mut libraries = Vec::new();
    if let Some(root) = steam_root {
        libraries.push(root.to_path_buf());
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = fs::read_to_string(&vdf) {
            libraries.extend(parse_library_folders(&text));
        }
    }
    for lib in libraries {
        let game = lib.join("steamapps").join("common").join("RimWorld");
        if is_rimworld_dir(&game) {
            return Some(game);
        }
    }
    None
}

fn library_root_for_game(game: &Path) -> Option<PathBuf> {
    // .../steamapps/common/RimWorld -> library root
    game.parent()?.parent()?.parent().map(Path::to_path_buf)
}

pub fn parse_library_folders(vdf: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for line in vdf.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("\"path\"") else {
            continue;
        };
        let rest = rest.trim();
        let Some(quoted) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else {
            continue;
        };
        let path = PathBuf::from(quoted.replace("\\\\", "\\"));
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

pub fn is_rimworld_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let data_core = path.join("Data").join("Core");
    if data_core.is_dir() {
        return true;
    }
    for exe in [
        "RimWorldWin64.exe",
        "RimWorld.exe",
        "RimWorldLinux",
        "RimWorldMac.app",
    ] {
        if path.join(exe).exists() {
            return true;
        }
    }
    false
}

pub fn default_config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let profile = std::env::var_os("USERPROFILE")?;
        Some(
            PathBuf::from(profile)
                .join("AppData")
                .join("LocalLow")
                .join("Ludeon Studios")
                .join("RimWorld by Ludeon Studios")
                .join("Config"),
        )
    }
    #[cfg(target_os = "linux")]
    {
        let home = dirs_home()?;
        Some(
            home.join(".config")
                .join("unity3d")
                .join("Ludeon Studios")
                .join("RimWorld by Ludeon Studios")
                .join("Config"),
        )
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs_home()?;
        Some(
            home.join("Library")
                .join("Application Support")
                .join("RimWorld")
                .join("Config"),
        )
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_libraryfolders_vdf() {
        let vdf = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}
"#;
        let paths = parse_library_folders(vdf);
        assert_eq!(paths.len(), 2);
        assert!(paths[0].to_string_lossy().contains("Steam"));
        assert!(paths[1].to_string_lossy().contains("SteamLibrary"));
    }
}
