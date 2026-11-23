use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::paths::autodetect;

pub const DEFAULT_COMMUNITY_RULES_URL: &str =
    "https://github.com/RimSort/Community-Rules-Database/archive/refs/heads/main.zip";
pub const DEFAULT_STEAM_DB_URL: &str =
    "https://github.com/RimSort/Steam-Workshop-Database/archive/refs/heads/main.zip";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortMode {
    #[default]
    Topological,
    Alphabetical,
}

impl std::fmt::Display for SortMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl SortMode {
    pub const ALL: [SortMode; 2] = [Self::Topological, Self::Alphabetical];

    pub fn label(self) -> &'static str {
        match self {
            Self::Topological => "topological",
            Self::Alphabetical => "alphabetical",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Topological => Self::Alphabetical,
            Self::Alphabetical => Self::Topological,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub game_install: PathBuf,
    #[serde(default)]
    pub local_mods: PathBuf,
    #[serde(default)]
    pub workshop: PathBuf,
    #[serde(default)]
    pub config_dir: PathBuf,
    #[serde(default = "default_community_rules_url")]
    pub community_rules_url: String,
    #[serde(default = "default_steam_db_url")]
    pub steam_db_url: String,
    #[serde(default)]
    pub sort_mode: SortMode,
    /// Override path to `steamcmd.exe` / `steamcmd.sh`. Empty means auto-detect.
    #[serde(default)]
    pub steamcmd_path: PathBuf,
    /// Steam WebAPI key for workshop search. Prefer env `STEAM_WEB_API_KEY` if empty.
    #[serde(default)]
    pub steam_api_key: String,
    /// GitHub token for release API. Prefer env `GITHUB_TOKEN` if empty.
    #[serde(default)]
    pub github_token: String,
    /// Name of the active instance; its paths are the top-level fields above.
    #[serde(default)]
    pub instance: String,
    /// Saved path sets for the other instances (RimSort "instances").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<Instance>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    pub game_install: PathBuf,
    pub local_mods: PathBuf,
    pub workshop: PathBuf,
    pub config_dir: PathBuf,
}

fn default_community_rules_url() -> String {
    DEFAULT_COMMUNITY_RULES_URL.to_string()
}

fn default_steam_db_url() -> String {
    DEFAULT_STEAM_DB_URL.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            game_install: PathBuf::new(),
            local_mods: PathBuf::new(),
            workshop: PathBuf::new(),
            config_dir: PathBuf::new(),
            community_rules_url: default_community_rules_url(),
            steam_db_url: default_steam_db_url(),
            sort_mode: SortMode::Topological,
            steamcmd_path: PathBuf::new(),
            steam_api_key: String::new(),
            github_token: String::new(),
            instance: String::new(),
            instances: Vec::new(),
        }
    }
}

impl Settings {
    pub fn config_file_path() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "rimtui", "rimtui")?;
        Some(dirs.config_dir().join("settings.toml"))
    }

    pub fn log_file_path() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "rimtui", "rimtui")?;
        Some(dirs.config_dir().join("rimtui.log"))
    }

    pub fn config_dir_path() -> Option<PathBuf> {
        Self::config_file_path()?.parent().map(Path::to_path_buf)
    }

    pub fn community_rules_path() -> Option<PathBuf> {
        Self::config_dir_path().map(|d| d.join("communityRules.json"))
    }

    pub fn user_rules_path() -> Option<PathBuf> {
        Self::config_dir_path().map(|d| d.join("userRules.json"))
    }

    pub fn steam_db_path() -> Option<PathBuf> {
        Self::config_dir_path().map(|d| d.join("steamDB.json"))
    }

    pub fn http_cache_path() -> Option<PathBuf> {
        Self::config_dir_path().map(|d| d.join("http_cache.json"))
    }

    pub fn default_list_path() -> PathBuf {
        Self::default_lists_dir().join("modlist.xml")
    }

    pub fn default_lists_dir() -> PathBuf {
        Self::config_dir_path()
            .map(|d| d.join("lists"))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Directory used to auto-install SteamCMD (`steamcmd.exe` lands here).
    pub fn steamcmd_dir() -> Option<PathBuf> {
        Self::config_dir_path().map(|d| d.join("steamcmd"))
    }

    pub fn github_mods_path() -> Option<PathBuf> {
        Self::config_dir_path().map(|d| d.join("github_mods.json"))
    }

    pub fn github_cache_path() -> Option<PathBuf> {
        Self::config_dir_path().map(|d| d.join("github_releases.json"))
    }

    pub fn resolved_steam_api_key(&self) -> String {
        let trimmed = self.steam_api_key.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
        std::env::var("STEAM_WEB_API_KEY")
            .or_else(|_| std::env::var("RIMSORT_STEAM_API_KEY"))
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    pub fn resolved_github_token(&self) -> String {
        let trimmed = self.github_token.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
        std::env::var("GITHUB_TOKEN")
            .or_else(|_| std::env::var("RIMSORT_GITHUB_TOKEN"))
            .or_else(|_| std::env::var("GH_TOKEN"))
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    pub fn load() -> Result<Self> {
        let Some(path) = Self::config_file_path() else {
            return Ok(Self::default().with_autodetect());
        };
        if !path.exists() {
            return Ok(Self::default().with_autodetect());
        }
        let text = fs::read_to_string(&path)?;
        let mut settings: Settings =
            toml::from_str(&text).map_err(|e| Error::Settings(e.to_string()))?;
        settings.fill_empty_from_autodetect();
        Ok(settings)
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::config_file_path()
            .ok_or_else(|| Error::Settings("could not determine config directory".into()))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| Error::Settings(e.to_string()))?;
        fs::write(&path, text)?;
        Ok(path)
    }

    pub fn with_autodetect(mut self) -> Self {
        self.fill_empty_from_autodetect();
        self
    }

    pub fn fill_empty_from_autodetect(&mut self) {
        let detected = autodetect();
        if self.game_install.as_os_str().is_empty() {
            if let Some(p) = detected.game_install {
                self.game_install = p;
            }
        }
        if self.local_mods.as_os_str().is_empty() {
            if let Some(p) = detected.local_mods {
                self.local_mods = p;
            } else if !self.game_install.as_os_str().is_empty() {
                self.local_mods = self.game_install.join("Mods");
            }
        }
        if self.workshop.as_os_str().is_empty() {
            if let Some(p) = detected.workshop {
                self.workshop = p;
            }
        }
        if self.config_dir.as_os_str().is_empty() {
            if let Some(p) = detected.config_dir {
                self.config_dir = p;
            }
        }
    }

    pub fn game_install_ok(&self) -> bool {
        crate::paths::is_rimworld_dir(&self.game_install)
    }

    pub fn instance_name(&self) -> &str {
        match self.instance.trim() {
            "" => "Default",
            name => name,
        }
    }

    /// Store the current paths under the active name, then make `name` active: an existing
    /// instance's paths are loaded, a new name starts as a copy of the current paths.
    pub fn switch_instance(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || name == self.instance_name() {
            return;
        }
        let current = Instance {
            name: self.instance_name().to_string(),
            game_install: self.game_install.clone(),
            local_mods: self.local_mods.clone(),
            workshop: self.workshop.clone(),
            config_dir: self.config_dir.clone(),
        };
        self.instances.retain(|i| i.name != current.name);
        self.instances.push(current);
        if let Some(pos) = self.instances.iter().position(|i| i.name == name) {
            let next = self.instances.remove(pos);
            self.game_install = next.game_install;
            self.local_mods = next.local_mods;
            self.workshop = next.workshop;
            self.config_dir = next.config_dir;
        }
        self.instance = name.to_string();
    }

    /// Forget a saved instance (not the active one; switch away first).
    pub fn remove_instance(&mut self, name: &str) {
        let name = name.trim();
        self.instances.retain(|i| i.name != name);
    }

    pub fn mods_config_path(&self) -> PathBuf {
        crate::xml::mods_config::mods_config_path(&self.config_dir)
    }

    pub fn field_display(&self, field: SettingsField) -> String {
        match field {
            SettingsField::Instance if self.instances.is_empty() => self.instance_name().into(),
            SettingsField::Instance => {
                let others: Vec<&str> = self.instances.iter().map(|i| i.name.as_str()).collect();
                format!("{} (others: {})", self.instance_name(), others.join(", "))
            }
            SettingsField::GameInstall => self.game_install.display().to_string(),
            SettingsField::LocalMods => self.local_mods.display().to_string(),
            SettingsField::Workshop => self.workshop.display().to_string(),
            SettingsField::ConfigDir => self.config_dir.display().to_string(),
            SettingsField::CommunityRulesUrl => self.community_rules_url.clone(),
            SettingsField::SteamDbUrl => self.steam_db_url.clone(),
            SettingsField::SortMode => self.sort_mode.label().to_string(),
            SettingsField::SteamCmdPath => {
                if !self.steamcmd_path.as_os_str().is_empty() {
                    self.steamcmd_path.display().to_string()
                } else if let Some(p) = crate::steamcmd::detect() {
                    format!("(auto) {}", p.display())
                } else {
                    "(auto — not found)".into()
                }
            }
            SettingsField::SteamApiKey => {
                if !self.steam_api_key.trim().is_empty() {
                    mask_api_key(&self.steam_api_key)
                } else if !self.resolved_steam_api_key().is_empty() {
                    "(from environment)".into()
                } else {
                    String::new()
                }
            }
            SettingsField::GithubToken => {
                if !self.github_token.trim().is_empty() {
                    mask_api_key(&self.github_token)
                } else if !self.resolved_github_token().is_empty() {
                    "(from environment)".into()
                } else {
                    String::new()
                }
            }
        }
    }

    pub fn set_text_field(&mut self, field: SettingsField, value: &str) {
        let value = value.trim();
        match field {
            SettingsField::Instance => match value.strip_prefix('-') {
                Some(name) => self.remove_instance(name),
                None => self.switch_instance(value),
            },
            SettingsField::GameInstall => {
                self.game_install = PathBuf::from(value);
                if self.local_mods.as_os_str().is_empty() {
                    self.local_mods = self.game_install.join("Mods");
                }
            }
            SettingsField::LocalMods => self.local_mods = PathBuf::from(value),
            SettingsField::Workshop => self.workshop = PathBuf::from(value),
            SettingsField::ConfigDir => self.config_dir = PathBuf::from(value),
            SettingsField::CommunityRulesUrl => self.community_rules_url = value.to_string(),
            SettingsField::SteamDbUrl => self.steam_db_url = value.to_string(),
            SettingsField::SortMode => {}
            SettingsField::SteamCmdPath => self.steamcmd_path = PathBuf::from(value),
            SettingsField::SteamApiKey => self.steam_api_key = value.to_string(),
            SettingsField::GithubToken => self.github_token = value.to_string(),
        }
    }

    pub fn cycle_field(&mut self, field: SettingsField) {
        if field == SettingsField::SortMode {
            self.sort_mode = self.sort_mode.toggle();
        }
    }

    /// Raw value for the settings editor (not the masked/auto display).
    pub fn field_edit_value(&self, field: SettingsField) -> String {
        match field {
            SettingsField::Instance => self.instance_name().to_string(),
            SettingsField::SteamCmdPath => self.steamcmd_path.display().to_string(),
            SettingsField::SteamApiKey => self.steam_api_key.clone(),
            SettingsField::GithubToken => self.github_token.clone(),
            _ => self.field_display(field),
        }
    }
}

fn mask_api_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.len() <= 4 {
        return "****".into();
    }
    format!("****{}", &trimmed[trimmed.len() - 4..])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsField {
    Instance,
    GameInstall,
    LocalMods,
    Workshop,
    ConfigDir,
    CommunityRulesUrl,
    SteamDbUrl,
    SortMode,
    SteamCmdPath,
    SteamApiKey,
    GithubToken,
}

impl SettingsField {
    pub const ALL: [SettingsField; 11] = [
        Self::Instance,
        Self::GameInstall,
        Self::LocalMods,
        Self::Workshop,
        Self::ConfigDir,
        Self::CommunityRulesUrl,
        Self::SteamDbUrl,
        Self::SortMode,
        Self::SteamCmdPath,
        Self::SteamApiKey,
        Self::GithubToken,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Instance => "Instance",
            Self::GameInstall => "Game install",
            Self::LocalMods => "Local mods",
            Self::Workshop => "Workshop",
            Self::ConfigDir => "Config dir",
            Self::CommunityRulesUrl => "Rules URL",
            Self::SteamDbUrl => "Steam DB URL",
            Self::SortMode => "Sort mode",
            Self::SteamCmdPath => "SteamCMD",
            Self::SteamApiKey => "API key",
            Self::GithubToken => "GitHub token",
        }
    }

    pub fn is_cycle(self) -> bool {
        matches!(self, Self::SortMode)
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_toml_fields_get_db_url_defaults() {
        let settings: Settings = toml::from_str("game_install = \"C:/RimWorld\"\n").unwrap();
        assert_eq!(settings.community_rules_url, DEFAULT_COMMUNITY_RULES_URL);
        assert_eq!(settings.steam_db_url, DEFAULT_STEAM_DB_URL);
        assert_eq!(settings.sort_mode, SortMode::Topological);
        assert!(settings.steamcmd_path.as_os_str().is_empty());
        assert!(settings.steam_api_key.is_empty());
        assert!(settings.github_token.is_empty());
    }

    #[test]
    fn masks_api_key_but_keeps_last_four() {
        assert_eq!(mask_api_key(""), "");
        assert_eq!(mask_api_key("abcd"), "****");
        assert_eq!(mask_api_key("1234567890abcdef"), "****cdef");
    }

    #[test]
    fn instances_switch_and_restore_paths() {
        let mut s = Settings {
            game_install: "C:/RimA".into(),
            config_dir: "C:/CfgA".into(),
            ..Settings::default()
        };
        s.switch_instance("Modded");
        assert_eq!(s.instance_name(), "Modded");
        assert_eq!(s.game_install, PathBuf::from("C:/RimA"), "new instance copies paths");
        s.game_install = "C:/RimB".into();
        s.switch_instance("Default");
        assert_eq!(s.game_install, PathBuf::from("C:/RimA"));
        assert_eq!(s.config_dir, PathBuf::from("C:/CfgA"));
        s.switch_instance("Modded");
        assert_eq!(s.game_install, PathBuf::from("C:/RimB"));
        assert_eq!(s.instances.len(), 1);
        let round: Settings = toml::from_str(&toml::to_string_pretty(&s).unwrap()).unwrap();
        assert_eq!(round.instances, s.instances);
        assert!(s.field_display(SettingsField::Instance).contains("others: Default"));
        s.set_text_field(SettingsField::Instance, "-Modded");
        assert_eq!(s.instance_name(), "Modded", "the active instance is not removed");
        s.set_text_field(SettingsField::Instance, "- Default");
        assert!(s.instances.is_empty());
    }

    #[test]
    fn sort_mode_toggle() {
        assert_eq!(SortMode::Topological.toggle(), SortMode::Alphabetical);
        assert_eq!(SortMode::Alphabetical.toggle(), SortMode::Topological);
    }
}
