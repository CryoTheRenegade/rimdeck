use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::Mod;

/// RimPy/RimSort `communityRules.json` / `userRules.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesDb {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub rules: HashMap<String, ExternalRule>,
}

/// Per-package extra load rules. Unknown JSON keys are ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRule {
    #[serde(
        default,
        rename = "loadAfter",
        skip_serializing_if = "HashMap::is_empty"
    )]
    pub load_after: HashMap<String, SubRule>,
    #[serde(
        default,
        rename = "loadBefore",
        skip_serializing_if = "HashMap::is_empty"
    )]
    pub load_before: HashMap<String, SubRule>,
    #[serde(
        default,
        rename = "loadTop",
        skip_serializing_if = "BoolRule::is_unset"
    )]
    pub load_top: BoolRule,
    #[serde(
        default,
        rename = "loadBottom",
        skip_serializing_if = "BoolRule::is_unset"
    )]
    pub load_bottom: BoolRule,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubRule {
    #[serde(default, skip_serializing_if = "TextOrList::is_empty")]
    pub name: TextOrList,
    #[serde(default, skip_serializing_if = "TextOrList::is_empty")]
    pub comment: TextOrList,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoolRule {
    #[serde(default)]
    pub value: bool,
    #[serde(default, skip_serializing_if = "TextOrList::is_empty")]
    pub comment: TextOrList,
}

impl BoolRule {
    fn is_unset(&self) -> bool {
        !self.value && self.comment.is_empty()
    }
}

/// RimPy stores `name` / `comment` as either a string or an array of strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextOrList {
    Many(Vec<String>),
    One(String),
}

impl Default for TextOrList {
    fn default() -> Self {
        Self::Many(Vec::new())
    }
}

impl TextOrList {
    fn is_empty(&self) -> bool {
        match self {
            Self::One(s) => s.is_empty(),
            Self::Many(v) => v.is_empty() || v.iter().all(|s| s.is_empty()),
        }
    }
}

impl RulesDb {
    pub fn load_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)?;
        parse_rules_json(&text)
    }

    pub fn save_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| Error::Rules(e.to_string()))?;
        fs::write(path, text)?;
        Ok(())
    }

    pub fn get(&self, package_id: &str) -> Option<&ExternalRule> {
        self.rules.get(&package_id.to_ascii_lowercase())
    }

    pub fn load_top(&self, package_id: &str) -> bool {
        self.get(package_id).is_some_and(|r| r.load_top.value)
    }

    pub fn load_bottom(&self, package_id: &str) -> bool {
        self.get(package_id).is_some_and(|r| r.load_bottom.value)
    }

    fn rule_mut(&mut self, package_id: &str) -> &mut ExternalRule {
        self.rules
            .entry(package_id.to_ascii_lowercase())
            .or_default()
    }

    /// Set user `loadTop` and drop the package entry if it becomes empty.
    pub fn set_load_top(&mut self, package_id: &str, value: bool) {
        self.timestamp = unix_timestamp();
        let id = package_id.to_ascii_lowercase();
        self.rule_mut(&id).load_top.value = value;
        if !value {
            self.rule_mut(&id).load_top.comment = TextOrList::default();
        }
        self.prune_empty(&id);
    }

    /// Set user `loadBottom` and drop the package entry if it becomes empty.
    pub fn set_load_bottom(&mut self, package_id: &str, value: bool) {
        self.timestamp = unix_timestamp();
        let id = package_id.to_ascii_lowercase();
        self.rule_mut(&id).load_bottom.value = value;
        if !value {
            self.rule_mut(&id).load_bottom.comment = TextOrList::default();
        }
        self.prune_empty(&id);
    }

    fn prune_empty(&mut self, package_id: &str) {
        if self
            .rules
            .get(package_id)
            .is_some_and(ExternalRule::is_empty)
        {
            self.rules.remove(package_id);
        }
    }

    fn normalize(&mut self) {
        let old = std::mem::take(&mut self.rules);
        let mut next = HashMap::with_capacity(old.len());
        for (pid, mut rule) in old {
            rule.normalize();
            next.insert(pid.to_ascii_lowercase(), rule);
        }
        self.rules = next;
    }
}

impl ExternalRule {
    fn is_empty(&self) -> bool {
        self.load_after.is_empty()
            && self.load_before.is_empty()
            && self.load_top.is_unset()
            && self.load_bottom.is_unset()
    }

    fn normalize(&mut self) {
        self.load_after = lowercase_keys(std::mem::take(&mut self.load_after));
        self.load_before = lowercase_keys(std::mem::take(&mut self.load_before));
    }
}

fn lowercase_keys(map: HashMap<String, SubRule>) -> HashMap<String, SubRule> {
    map.into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect()
}

pub fn parse_rules_json(text: &str) -> Result<RulesDb> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut db: RulesDb = serde_json::from_str(text).map_err(|e| Error::Rules(e.to_string()))?;
    db.normalize();
    Ok(db)
}

/// Merge community then user rules into each mod (RimSort `overall_rules`).
///
/// About.xml fields already on the mod have lowest priority. User rules union
/// with community (and About.xml) for loadAfter/loadBefore; `loadTop` /
/// `loadBottom` are true if either database sets them.
pub fn apply_rules(mods: &mut [Mod], community: &RulesDb, user: &RulesDb) {
    for m in mods {
        let pid = m.package_id.as_str();
        if let Some(rule) = community.get(pid) {
            merge_ids(&mut m.load_after, rule.load_after.keys());
            merge_ids(&mut m.load_before, rule.load_before.keys());
        }
        if let Some(rule) = user.get(pid) {
            merge_ids(&mut m.load_after, rule.load_after.keys());
            merge_ids(&mut m.load_before, rule.load_before.keys());
        }
        m.load_first = community.load_top(pid) || user.load_top(pid);
        m.load_last = community.load_bottom(pid) || user.load_bottom(pid);
    }
}

fn merge_ids<'a>(existing: &mut Vec<String>, extra: impl Iterator<Item = &'a String>) {
    for id in extra {
        let id = id.to_ascii_lowercase();
        if !existing.iter().any(|e| e == &id) {
            existing.push(id);
        }
    }
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModSource;
    use crate::sort::sort_active;
    use std::path::PathBuf;

    fn dummy(path: &str, id: &str, name: &str) -> Mod {
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
            published_file_id: None,
        }
    }

    #[test]
    fn parse_missing_fields_is_ok() {
        let db = parse_rules_json(r#"{"rules":{}}"#).unwrap();
        assert_eq!(db.timestamp, 0);
        assert!(db.rules.is_empty());
    }

    #[test]
    fn parse_rimpy_shapes() {
        let json = r#"{
            "timestamp": 1,
            "rules": {
                "Author.TopMod": {
                    "loadAfter": {
                        "Other.Mod": { "name": ["Other"] }
                    },
                    "loadBefore": {
                        "Later.Mod": { "name": "Later", "comment": "note" }
                    },
                    "loadTop": { "value": true, "comment": "tier 1" },
                    "loadBottom": { "value": false }
                },
                "author.bottom": {
                    "loadBottom": { "value": true }
                }
            }
        }"#;
        let db = parse_rules_json(json).unwrap();
        assert!(db.load_top("author.topmod"));
        assert!(!db.load_bottom("author.topmod"));
        assert!(db.load_bottom("author.bottom"));
        let top = db.get("author.topmod").unwrap();
        assert!(top.load_after.contains_key("other.mod"));
        assert!(top.load_before.contains_key("later.mod"));
    }

    #[test]
    fn malformed_json_is_error() {
        assert!(parse_rules_json("{not json").is_err());
    }

    #[test]
    fn missing_file_is_empty() {
        let db = RulesDb::load_path(Path::new("definitely/missing/communityRules.json")).unwrap();
        assert!(db.rules.is_empty());
    }

    #[test]
    fn user_overrides_union_and_or_bools() {
        let community = parse_rules_json(
            r#"{
            "rules": {
                "a.mod": {
                    "loadAfter": { "b.mod": {} },
                    "loadTop": { "value": true }
                }
            }
        }"#,
        )
        .unwrap();
        let user = parse_rules_json(
            r#"{
            "rules": {
                "a.mod": {
                    "loadAfter": { "c.mod": {} },
                    "loadBottom": { "value": true }
                }
            }
        }"#,
        )
        .unwrap();
        let mut mods = vec![dummy("/a", "a.mod", "A")];
        mods[0].load_after.push("about.mod".into());
        apply_rules(&mut mods, &community, &user);
        assert!(mods[0].load_first);
        assert!(mods[0].load_last);
        assert_eq!(
            mods[0].load_after,
            vec![
                "about.mod".to_string(),
                "b.mod".to_string(),
                "c.mod".to_string()
            ]
        );
    }

    #[test]
    fn load_top_and_bottom_affect_sort_tiers() {
        let mut mods = vec![
            dummy("/core", "ludeon.rimworld", "Core"),
            dummy("/z", "z.regular", "Zed"),
            dummy("/top", "a.top", "Topper"),
            dummy("/bottom", "a.bottom", "Sink"),
        ];
        let community = parse_rules_json(
            r#"{
            "rules": {
                "a.top": { "loadTop": { "value": true } },
                "a.bottom": { "loadBottom": { "value": true } }
            }
        }"#,
        )
        .unwrap();
        apply_rules(&mut mods, &community, &RulesDb::default());
        let active = vec![
            PathBuf::from("/bottom"),
            PathBuf::from("/z"),
            PathBuf::from("/top"),
            PathBuf::from("/core"),
        ];
        let sorted = sort_active(&mods, &active).unwrap();
        assert_eq!(
            sorted,
            vec![
                PathBuf::from("/core"),
                PathBuf::from("/top"),
                PathBuf::from("/z"),
                PathBuf::from("/bottom"),
            ]
        );
    }

    #[test]
    fn toggle_prunes_empty_user_rule() {
        let mut user = RulesDb::default();
        user.set_load_top("a.mod", true);
        assert!(user.load_top("a.mod"));
        user.set_load_top("a.mod", false);
        assert!(!user.rules.contains_key("a.mod"));
    }

    #[test]
    fn round_trip_user_rules_file() {
        let dir = std::env::temp_dir().join(format!("rimtui-rules-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("userRules.json");
        let mut db = RulesDb::default();
        db.set_load_bottom("rocketman.mod", true);
        db.save_path(&path).unwrap();
        let loaded = RulesDb::load_path(&path).unwrap();
        assert!(loaded.load_bottom("rocketman.mod"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("loadBottom"));
        let _ = fs::remove_dir_all(&dir);
    }
}
