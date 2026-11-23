use std::collections::HashSet;
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

pub fn check_issues(
    mods: &[Mod],
    active_paths: &[PathBuf],
    missing_from_disk: &[String],
) -> Vec<Issue> {
    let mut issues = Vec::new();
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
        let issues = check_issues(&[a], &[PathBuf::from("/a")], &[]);
        assert!(issues.iter().any(|i| i.severity == Severity::Error));
    }

    #[test]
    fn reports_incompatibility() {
        let mut a = mod_at("/a", "a.mod", "A");
        a.incompatible_with.push("b.mod".into());
        let b = mod_at("/b", "b.mod", "B");
        let issues = check_issues(&[a, b], &[PathBuf::from("/a"), PathBuf::from("/b")], &[]);
        assert!(issues.iter().any(|i| i.message.contains("incompatible")));
    }

    #[test]
    fn reports_load_after_violation() {
        let mut a = mod_at("/a", "a.mod", "A");
        a.load_after.push("b.mod".into());
        let b = mod_at("/b", "b.mod", "B");
        let issues = check_issues(&[a, b], &[PathBuf::from("/a"), PathBuf::from("/b")], &[]);
        assert!(issues.iter().any(|i| i.message.contains("loadAfter")));
    }
}
