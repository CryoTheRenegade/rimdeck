use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::model::Mod;

/// RimSort / RimPy `steamDB.json`. Keys in `database` are Steam published file IDs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SteamDb {
    pub version: i64,
    /// pfid (lowercase) -> entry
    pub database: HashMap<String, SteamDbEntry>,
    /// package_id (lowercase) -> pfid, first match wins
    package_index: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SteamDbEntry {
    pub package_id: String,
    pub name: String,
    /// Steam "required items" as published file IDs.
    pub dependencies: Vec<String>,
}

#[derive(Deserialize)]
struct SteamDbFile {
    #[serde(default)]
    version: i64,
    #[serde(default)]
    database: HashMap<String, SteamDbEntryFile>,
}

#[derive(Deserialize, Default)]
struct SteamDbEntryFile {
    // Two fields, not an alias: RimSort's DB has entries with both spellings, and an
    // alias makes serde reject that as a duplicate (which failed the whole file).
    #[serde(default, rename = "packageId")]
    package_id: String,
    #[serde(default, rename = "packageid")]
    package_id_lower: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    dependencies: HashMap<String, serde_json::Value>,
}

impl SteamDb {
    pub fn load_path(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        match fs::read_to_string(path) {
            Ok(text) => parse_steam_db_json(&text).unwrap_or_else(|e| {
                tracing::warn!("steamDB.json: {e}");
                Self::default()
            }),
            Err(e) => {
                tracing::warn!("steamDB.json: {e}");
                Self::default()
            }
        }
    }

    pub fn get_pfid(&self, pfid: &str) -> Option<&SteamDbEntry> {
        self.database.get(&pfid.to_ascii_lowercase())
    }

    pub fn get_package(&self, package_id: &str) -> Option<&SteamDbEntry> {
        let pfid = self.package_index.get(&package_id.to_ascii_lowercase())?;
        self.database.get(pfid)
    }

    pub fn pfid_for_package(&self, package_id: &str) -> Option<&str> {
        self.package_index
            .get(&package_id.to_ascii_lowercase())
            .map(String::as_str)
    }

    fn rebuild_index(&mut self) {
        self.package_index.clear();
        for (pfid, entry) in &self.database {
            if entry.package_id.is_empty() {
                continue;
            }
            self.package_index
                .entry(entry.package_id.clone())
                .or_insert_with(|| pfid.clone());
        }
    }
}

pub fn parse_steam_db_json(text: &str) -> Result<SteamDb, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let file: SteamDbFile =
        serde_json::from_str(text).map_err(|e| format!("failed to parse steam database: {e}"))?;
    let mut database = HashMap::with_capacity(file.database.len());
    for (pfid, entry) in file.database {
        let package_id = if entry.package_id.is_empty() {
            entry.package_id_lower
        } else {
            entry.package_id
        }
        .to_ascii_lowercase();
        let deps: Vec<String> = entry
            .dependencies
            .into_keys()
            .map(|k| k.to_ascii_lowercase())
            .collect();
        database.insert(
            pfid.to_ascii_lowercase(),
            SteamDbEntry {
                package_id,
                name: entry.name,
                dependencies: deps,
            },
        );
    }
    let mut db = SteamDb {
        version: file.version,
        database,
        package_index: HashMap::new(),
    };
    db.rebuild_index();
    Ok(db)
}

/// Add Steam "required items" as extra `loadAfter` after About/community/user rules.
pub fn apply_steam_db(mods: &mut [Mod], db: &SteamDb) {
    if db.database.is_empty() {
        return;
    }
    for m in mods {
        let entry = m
            .published_file_id
            .as_deref()
            .and_then(|pfid| db.get_pfid(pfid))
            .or_else(|| db.get_package(&m.package_id));
        let Some(entry) = entry else {
            continue;
        };
        for dep_pfid in &entry.dependencies {
            let Some(dep_pid) = db.get_pfid(dep_pfid).map(|e| e.package_id.clone()) else {
                continue;
            };
            if dep_pid.is_empty() || dep_pid == m.package_id {
                continue;
            }
            if !m.load_after.iter().any(|e| e == &dep_pid) {
                m.load_after.push(dep_pid);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModSource;
    use crate::sort::sort_active;
    use std::path::{Path, PathBuf};

    fn dummy(path: &str, id: &str, name: &str, pfid: Option<&str>) -> Mod {
        Mod {
            path: PathBuf::from(path),
            package_id: id.into(),
            name: name.into(),
            authors: vec![],
            description: String::new(),
            url: String::new(),
            mod_version: String::new(),
            supported_versions: vec![],
            source: ModSource::Local,
            load_after: vec![],
            load_before: vec![],
            incompatible_with: vec![],
            dependencies: vec![],
            load_first: false,
            load_last: false,
            published_file_id: pfid.map(str::to_string),
        }
    }

    const FIXTURE: &str = r#"{
        "version": 1,
        "database": {
            "111": {
                "packageId": "A.Mod",
                "name": "A",
                "dependencies": {
                    "222": ["B", "https://example.invalid/222"]
                }
            },
            "222": {
                "packageid": "b.mod",
                "name": "B"
            },
            "294100": {
                "appid": true,
                "packageid": "ludeon.rimworld",
                "name": "RimWorld",
                "packageId": "Ludeon.RimWorld"
            }
        }
    }"#;

    #[test]
    fn parse_lenient_package_id_and_array_deps() {
        let db = parse_steam_db_json(FIXTURE).unwrap();
        assert_eq!(db.version, 1);
        let a = db.get_pfid("111").unwrap();
        assert_eq!(a.package_id, "a.mod");
        assert_eq!(a.dependencies, vec!["222".to_string()]);
        assert_eq!(db.get_package("b.mod").unwrap().name, "B");
        assert_eq!(db.pfid_for_package("a.mod"), Some("111"));
        // Both spellings on one entry, as in RimSort's real steamDB.json.
        assert_eq!(db.pfid_for_package("ludeon.rimworld"), Some("294100"));
    }

    #[test]
    fn missing_file_is_empty() {
        let db = SteamDb::load_path(Path::new("definitely/missing/steamDB.json"));
        assert!(db.database.is_empty());
    }

    #[test]
    fn malformed_json_is_error_from_parse() {
        assert!(parse_steam_db_json("{not json").is_err());
    }

    #[test]
    fn apply_unions_required_items_as_load_after() {
        let db = parse_steam_db_json(FIXTURE).unwrap();
        let mut mods = vec![
            dummy("/a", "a.mod", "A", Some("111")),
            dummy("/b", "b.mod", "B", Some("222")),
        ];
        mods[0].load_after.push("already.mod".into());
        apply_steam_db(&mut mods, &db);
        assert_eq!(
            mods[0].load_after,
            vec!["already.mod".to_string(), "b.mod".to_string()]
        );
        assert!(mods[1].load_after.is_empty());
    }

    #[test]
    fn steam_required_items_affect_toposort() {
        let db = parse_steam_db_json(FIXTURE).unwrap();
        let mut mods = vec![
            dummy("/a", "a.mod", "A", Some("111")),
            dummy("/b", "b.mod", "B", Some("222")),
        ];
        apply_steam_db(&mut mods, &db);
        let active = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let sorted = sort_active(&mods, &active).unwrap();
        let pos = |p: &str| sorted.iter().position(|x| x == Path::new(p)).unwrap();
        assert!(pos("/b") < pos("/a"));
    }

    #[test]
    fn lookup_by_package_id_without_pfid() {
        let db = parse_steam_db_json(FIXTURE).unwrap();
        let mut mods = vec![dummy("/a", "a.mod", "A", None)];
        apply_steam_db(&mut mods, &db);
        assert!(mods[0].load_after.contains(&"b.mod".to_string()));
    }
}
