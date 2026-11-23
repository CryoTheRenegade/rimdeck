use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::xml::mods_config::{read_mods_config, ModsConfig};

/// Write a shareable mod list (ModsConfig.xml shape) without touching `.bak`.
pub fn export_list(path: &Path, config: &ModsConfig) -> Result<PathBuf> {
    let path = with_xml_suffix(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, config.to_xml())?;
    Ok(path)
}

pub fn import_list(path: &Path) -> Result<ModsConfig> {
    if !path.exists() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("file not found: {}", path.display()),
        )));
    }
    read_mods_config(path)
}

fn with_xml_suffix(path: &Path) -> PathBuf {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("xml") => path.to_path_buf(),
        _ => {
            let mut p = path.as_os_str().to_os_string();
            p.push(".xml");
            PathBuf::from(p)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_import_round_trip() {
        let dir = std::env::temp_dir().join(format!("rimdeck-lists-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("my-list");
        let cfg = ModsConfig {
            version: Some("1.5.4104 rev1".into()),
            active_mods: vec!["ludeon.rimworld".into(), "brrainz.harmony".into()],
            known_expansions: vec!["ludeon.rimworld.royalty".into()],
        };
        let written = export_list(&path, &cfg).unwrap();
        assert_eq!(written.extension().unwrap(), "xml");
        let loaded = import_list(&written).unwrap();
        assert_eq!(loaded.active_mods, cfg.active_mods);
        assert_eq!(loaded.version, cfg.version);
        let _ = fs::remove_dir_all(&dir);
    }
}
