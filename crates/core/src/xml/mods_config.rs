use std::fs;
use std::path::{Path, PathBuf};

use roxmltree::Document;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModsConfig {
    pub version: Option<String>,
    pub active_mods: Vec<String>,
    pub known_expansions: Vec<String>,
}

impl ModsConfig {
    pub fn to_xml(&self) -> String {
        let mut out =
            String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ModsConfigData>\n");
        if let Some(version) = &self.version {
            out.push_str(&format!("\t<version>{version}</version>\n"));
        }
        out.push_str("\t<activeMods>\n");
        for id in &self.active_mods {
            out.push_str(&format!("\t\t<li>{id}</li>\n"));
        }
        out.push_str("\t</activeMods>\n");
        out.push_str("\t<knownExpansions>\n");
        for id in &self.known_expansions {
            out.push_str(&format!("\t\t<li>{id}</li>\n"));
        }
        out.push_str("\t</knownExpansions>\n");
        out.push_str("</ModsConfigData>\n");
        out
    }
}

pub fn read_mods_config(path: &Path) -> Result<ModsConfig> {
    if !path.exists() {
        return Ok(ModsConfig::default());
    }
    let raw = fs::read_to_string(path)?;
    parse_mods_config(&raw, path)
}

pub fn parse_mods_config(raw: &str, path: &Path) -> Result<ModsConfig> {
    let sanitized = crate::xml::about::sanitize_xml(raw);
    let doc = Document::parse(&sanitized).map_err(|e| Error::Xml {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    let root = doc.root_element();
    let version = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case("version"))
        .and_then(|n| n.text())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Ok(ModsConfig {
        version,
        active_mods: list_ids(root, "activeMods"),
        known_expansions: list_ids(root, "knownExpansions"),
    })
}

fn list_ids(root: roxmltree::Node<'_, '_>, name: &str) -> Vec<String> {
    let Some(node) = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case(name))
    else {
        return Vec::new();
    };
    node.children()
        .filter(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case("li"))
        .filter_map(|li| li.text().map(|s| s.trim().to_ascii_lowercase()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Write `ModsConfig.xml`, copying the previous file to `ModsConfig.xml.bak`.
pub fn write_mods_config(path: &Path, config: &ModsConfig) -> Result<Option<PathBuf>> {
    let backup = if path.exists() {
        let mut bak = path.as_os_str().to_os_string();
        bak.push(".bak");
        let bak = PathBuf::from(bak);
        fs::copy(path, &bak)?;
        Some(bak)
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        None
    };
    fs::write(path, config.to_xml())?;
    Ok(backup)
}

pub fn mods_config_path(config_dir: &Path) -> PathBuf {
    config_dir.join("ModsConfig.xml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_mods_config() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<ModsConfigData>
	<version>1.5.4104 rev1</version>
	<activeMods>
		<li>Ludeon.RimWorld</li>
		<li>brrainz.harmony</li>
	</activeMods>
	<knownExpansions>
		<li>Ludeon.RimWorld.Royalty</li>
	</knownExpansions>
</ModsConfigData>
"#;
        let parsed = parse_mods_config(xml, Path::new("ModsConfig.xml")).unwrap();
        assert_eq!(parsed.version.as_deref(), Some("1.5.4104 rev1"));
        assert_eq!(
            parsed.active_mods,
            vec!["ludeon.rimworld", "brrainz.harmony"]
        );
        let again = parse_mods_config(&parsed.to_xml(), Path::new("out.xml")).unwrap();
        assert_eq!(parsed, again);
    }

    #[test]
    fn write_creates_backup() {
        let dir = std::env::temp_dir().join(format!("rimdeck-modsconfig-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ModsConfig.xml");
        let first = ModsConfig {
            version: Some("1".into()),
            active_mods: vec!["a".into()],
            known_expansions: vec![],
        };
        write_mods_config(&path, &first).unwrap();
        let second = ModsConfig {
            version: Some("1".into()),
            active_mods: vec!["b".into()],
            known_expansions: vec![],
        };
        let bak = write_mods_config(&path, &second).unwrap();
        assert!(bak.unwrap().exists());
        let written = read_mods_config(&path).unwrap();
        assert_eq!(written.active_mods, vec!["b"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
