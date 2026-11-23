use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::model::{mod_by_path, Mod};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub package_id: String,
    pub message: String,
}

/// `"1.7.5014 rev1195"` -> `"1.7"`, the form About.xml `supportedVersions` use.
pub fn major_minor(version: &str) -> Option<String> {
    let mut parts = version.trim().split(['.', ' ']);
    let major = parts.next().filter(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())?;
    let minor = parts.next().filter(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())?;
    Some(format!("{major}.{minor}"))
}

pub fn check_issues(
    mods: &[Mod],
    active_paths: &[PathBuf],
    missing_from_disk: &[String],
    game_version: Option<&str>,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    let game_version = game_version.and_then(major_minor);
    let mut copies: HashMap<&str, Vec<&Mod>> = HashMap::new();
    for m in mods {
        copies.entry(m.package_id.as_str()).or_default().push(m);
    }
    let active_mods: Vec<&Mod> = active_paths
        .iter()
        .filter_map(|p| mod_by_path(mods, p))
        .collect();
    let active_ids: HashSet<&str> = active_mods.iter().map(|m| m.package_id.as_str()).collect();
    let index_of: Vec<(usize, &str)> = active_mods
        .iter()
        .enumerate()
        .map(|(i, m)| (i, m.package_id.as_str()))
        .collect();

    for id in missing_from_disk {
        issues.push(Issue {
            severity: Severity::Error,
            package_id: id.clone(),
            message: format!("{id} is in ModsConfig.xml but was not found on disk"),
        });
    }

    for m in &active_mods {
        if let Some(all) = copies.get(m.package_id.as_str()).filter(|c| c.len() > 1) {
            let sources: Vec<&str> = all.iter().map(|c| c.source.label()).collect();
            issues.push(Issue {
                severity: Severity::Warning,
                package_id: m.package_id.clone(),
                message: format!(
                    "{} is installed {} times ({}); using the {} copy",
                    m.name,
                    all.len(),
                    sources.join(", "),
                    m.source.label()
                ),
            });
        }
        if let Some(v) = &game_version {
            if !m.is_ludeon()
                && !m.supported_versions.is_empty()
                && !m.supported_versions.iter().any(|s| s.trim() == v)
            {
                issues.push(Issue {
                    severity: Severity::Warning,
                    package_id: m.package_id.clone(),
                    message: format!(
                        "{} does not list RimWorld {v} (supports {})",
                        m.name,
                        m.supported_versions.join(", ")
                    ),
                });
            }
        }
        for dep in &m.dependencies {
            if !active_ids.contains(dep.package_id.as_str()) {
                let label = if dep.display_name.is_empty() {
                    dep.package_id.clone()
                } else {
                    format!("{} ({})", dep.display_name, dep.package_id)
                };
                issues.push(Issue {
                    severity: Severity::Error,
                    package_id: m.package_id.clone(),
                    message: format!("{} is missing dependency {label}", m.name),
                });
            }
        }
        for other in &m.incompatible_with {
            if active_ids.contains(other.as_str()) {
                issues.push(Issue {
                    severity: Severity::Error,
                    package_id: m.package_id.clone(),
                    message: format!("{} is incompatible with {other}", m.name),
                });
            }
        }
        let Some(self_idx) = index_of
            .iter()
            .find(|(_, id)| *id == m.package_id)
            .map(|(i, _)| *i)
        else {
            continue;
        };
        for after in &m.load_after {
            if let Some(other_idx) = index_of
                .iter()
                .find(|(_, id)| *id == after)
                .map(|(i, _)| *i)
            {
                if self_idx < other_idx {
                    issues.push(Issue {
                        severity: Severity::Warning,
                        package_id: m.package_id.clone(),
                        message: format!("{} should load after {after} (loadAfter)", m.name),
                    });
                }
            }
        }
        for before in &m.load_before {
            if let Some(other_idx) = index_of
                .iter()
                .find(|(_, id)| *id == before)
                .map(|(i, _)| *i)
            {
                if self_idx > other_idx {
                    issues.push(Issue {
                        severity: Severity::Warning,
                        package_id: m.package_id.clone(),
                        message: format!("{} should load before {before} (loadBefore)", m.name),
                    });
                }
            }
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dependency, ModSource};
    use std::path::PathBuf;

    fn mod_at(path: &str, id: &str, name: &str) -> Mod {
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
    fn reports_missing_dependency() {
        let mut a = mod_at("/a", "a.mod", "A");
        a.dependencies.push(Dependency {
            package_id: "missing.mod".into(),
            display_name: "Missing".into(),
        });
        let issues = check_issues(&[a], &[PathBuf::from("/a")], &[], None);
        assert!(issues.iter().any(|i| i.severity == Severity::Error));
    }

    #[test]
    fn reports_incompatibility() {
        let mut a = mod_at("/a", "a.mod", "A");
        a.incompatible_with.push("b.mod".into());
        let b = mod_at("/b", "b.mod", "B");
        let issues = check_issues(&[a, b], &[PathBuf::from("/a"), PathBuf::from("/b")], &[], None);
        assert!(issues.iter().any(|i| i.message.contains("incompatible")));
    }

    #[test]
    fn major_minor_parses_game_versions() {
        assert_eq!(major_minor("1.7.5014 rev1195").as_deref(), Some("1.7"));
        assert_eq!(major_minor("1.5").as_deref(), Some("1.5"));
        assert_eq!(major_minor("garbage"), None);
    }

    #[test]
    fn warns_on_unsupported_version_and_duplicates() {
        let mut a = mod_at("/a", "a.mod", "A");
        a.supported_versions = vec!["1.5".into(), "1.6".into()];
        let mut a2 = mod_at("/ws/a", "a.mod", "A");
        a2.source = ModSource::Workshop;
        let mut ok = mod_at("/ok", "ok.mod", "Ok");
        ok.supported_versions = vec!["1.7".into()];
        let issues = check_issues(
            &[a, a2, ok],
            &[PathBuf::from("/a"), PathBuf::from("/ok")],
            &[],
            Some("1.7.5014 rev1195"),
        );
        let msgs: Vec<&str> = issues.iter().map(|i| i.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("A does not list RimWorld 1.7")));
        assert!(msgs.iter().any(|m| m.contains("installed 2 times (Local, Workshop)")));
        assert!(!msgs.iter().any(|m| m.starts_with("Ok ")));
    }

    #[test]
    fn reports_load_after_violation() {
        let mut a = mod_at("/a", "a.mod", "A");
        a.load_after.push("b.mod".into());
        let b = mod_at("/b", "b.mod", "B");
        let issues = check_issues(&[a, b], &[PathBuf::from("/a"), PathBuf::from("/b")], &[], None);
        assert!(issues.iter().any(|i| i.message.contains("loadAfter")));
    }
}
