//! Update `steamDB.json` from installed mods (RimSort's DB Builder, "installed mods" mode).
//!
//! The file is edited as raw JSON so fields this crate does not model (`blacklist`,
//! `unpublished`, ...) survive.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::model::Mod;
use crate::scan::scan_mods;
use crate::settings::Settings;
use crate::workshop;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct BuildReport {
    pub added: usize,
    pub updated: usize,
    pub with_dependencies: usize,
}

fn workshop_url(pfid: &str) -> String {
    format!("https://steamcommunity.com/sharedfiles/filedetails/?id={pfid}")
}

/// Upsert every installed mod that has a published file ID into `db`. `required` maps
/// pfid -> Steam "required items"; mods without an entry there keep their old dependencies.
pub fn merge_installed(
    db: &mut Value,
    mods: &[Mod],
    required: &HashMap<String, Vec<String>>,
    version: i64,
) -> BuildReport {
    if !db.is_object() {
        *db = json!({});
    }
    let root = db.as_object_mut().expect("object");
    root.insert("version".into(), json!(version));
    let database = root
        .entry("database")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .map(std::mem::take)
        .unwrap_or_default();
    let mut database: Map<String, Value> = database;

    let local_names: HashMap<&str, &str> = mods
        .iter()
        .filter_map(|m| Some((m.published_file_id.as_deref()?, m.name.as_str())))
        .collect();
    let name_of = |db: &Map<String, Value>, pfid: &str| -> String {
        local_names
            .get(pfid)
            .map(|n| n.to_string())
            .or_else(|| db.get(pfid)?.get("name")?.as_str().map(str::to_string))
            .unwrap_or_default()
    };

    let mut report = BuildReport::default();
    for m in mods {
        let Some(pfid) = m.published_file_id.as_deref() else {
            continue;
        };
        let deps = required.get(pfid).map(|children| {
            let mut obj = Map::new();
            for child in children {
                obj.insert(
                    child.clone(),
                    json!([name_of(&database, child), workshop_url(child)]),
                );
            }
            Value::Object(obj)
        });
        let entry = match database.get_mut(pfid).and_then(Value::as_object_mut) {
            Some(e) => {
                report.updated += 1;
                e
            }
            None => {
                report.added += 1;
                database.insert(pfid.to_string(), json!({}));
                database[pfid].as_object_mut().expect("object")
            }
        };
        entry.insert("packageId".into(), json!(m.package_id));
        entry.insert("name".into(), json!(m.name));
        entry.insert("url".into(), json!(workshop_url(pfid)));
        if !m.supported_versions.is_empty() {
            entry.insert("gameVersions".into(), json!(m.supported_versions));
        }
        if let Some(deps) = deps {
            entry.insert("dependencies".into(), deps);
            report.with_dependencies += 1;
        }
    }
    root.insert("database".into(), Value::Object(database));
    report
}

/// Scan the configured mods, merge them into `out` (created if missing), and write it back.
/// With a Steam WebAPI key, required items are fetched as dependencies.
pub fn build_db_file(settings: &Settings, out: &Path, fetch_deps: bool) -> Result<String> {
    let mods = scan_mods(settings)?;
    let mut db = if out.exists() {
        let text = fs::read_to_string(out)?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        // Refuse to touch a file we can't parse rather than replace it.
        serde_json::from_str(text)
            .map_err(|e| Error::SteamDb(format!("{} is not valid JSON: {e}", out.display())))?
    } else {
        json!({})
    };
    let key = settings.resolved_steam_api_key();
    let mut note = String::new();
    let required = if !fetch_deps {
        HashMap::new()
    } else if key.is_empty() {
        note = "; no Steam WebAPI key, dependencies left as they were".into();
        HashMap::new()
    } else {
        let pfids: Vec<String> = mods
            .iter()
            .filter_map(|m| m.published_file_id.clone())
            .collect();
        workshop::required_items(&key, &pfids)?
    };
    let version = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let report = merge_installed(&mut db, &mods, &required, version);
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = out.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(&db).map_err(|e| Error::SteamDb(e.to_string()))?;
    fs::write(&tmp, text)?;
    fs::rename(&tmp, out)?;
    Ok(format!(
        "{}: {} added, {} updated, {} with dependencies{note}",
        out.display(),
        report.added,
        report.updated,
        report.with_dependencies
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModSource;
    use std::path::PathBuf;

    fn installed(pfid: &str, id: &str, name: &str) -> Mod {
        Mod {
            path: PathBuf::from(format!("/ws/{pfid}")),
            package_id: id.into(),
            name: name.into(),
            authors: vec![],
            description: String::new(),
            url: String::new(),
            mod_version: String::new(),
            supported_versions: vec!["1.6".into()],
            source: ModSource::Workshop,
            load_after: vec![],
            load_before: vec![],
            incompatible_with: vec![],
            dependencies: vec![],
            load_first: false,
            load_last: false,
            published_file_id: Some(pfid.into()),
        }
    }

    #[test]
    fn merge_keeps_unknown_fields_and_adds_new_entries() {
        let mut db = json!({
            "version": 1,
            "database": {
                "100": {"packageId": "old.id", "name": "Old", "blacklist": {"value": true},
                        "dependencies": {"7": ["Seven", "u"]}},
                "999": {"packageId": "other.mod", "name": "Not installed"}
            }
        });
        let mods = [installed("100", "a.mod", "A"), installed("200", "b.mod", "B")];
        let required = HashMap::from([("200".to_string(), vec!["100".to_string()])]);
        let report = merge_installed(&mut db, &mods, &required, 42);
        assert_eq!(
            report,
            BuildReport { added: 1, updated: 1, with_dependencies: 1 }
        );
        assert_eq!(db["version"], 42);
        let d = &db["database"];
        assert_eq!(d["100"]["packageId"], "a.mod");
        assert_eq!(d["100"]["blacklist"]["value"], true, "unknown fields kept");
        assert_eq!(d["100"]["dependencies"]["7"][0], "Seven", "deps kept without new data");
        assert_eq!(d["200"]["dependencies"]["100"][0], "A");
        assert_eq!(d["200"]["gameVersions"][0], "1.6");
        assert_eq!(d["999"]["name"], "Not installed");
        // The loader reads the result back.
        let parsed = crate::steamdb::parse_steam_db_json(&db.to_string()).unwrap();
        assert_eq!(parsed.pfid_for_package("b.mod"), Some("200"));
        assert_eq!(parsed.get_pfid("200").unwrap().dependencies, vec!["100"]);
    }
}
