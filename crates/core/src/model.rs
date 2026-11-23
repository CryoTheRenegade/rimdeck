use std::path::{Path, PathBuf};

use crate::constants::is_ludeon_package;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModSource {
    Dlc,
    Local,
    Workshop,
}

impl ModSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dlc => "DLC",
            Self::Local => "Local",
            Self::Workshop => "Workshop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub package_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub struct Mod {
    pub path: PathBuf,
    pub package_id: String,
    pub name: String,
    pub authors: Vec<String>,
    pub description: String,
    pub url: String,
    pub mod_version: String,
    pub supported_versions: Vec<String>,
    pub source: ModSource,
    pub load_after: Vec<String>,
    pub load_before: Vec<String>,
    pub incompatible_with: Vec<String>,
    pub dependencies: Vec<Dependency>,
    pub load_first: bool,
    pub load_last: bool,
    pub published_file_id: Option<String>,
}

impl Mod {
    pub fn author_line(&self) -> String {
        if self.authors.is_empty() {
            String::from("(unknown)")
        } else {
            self.authors.join(", ")
        }
    }

    pub fn is_ludeon(&self) -> bool {
        is_ludeon_package(&self.package_id)
    }

    pub fn matches_query(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_ascii_lowercase();
        self.name.to_ascii_lowercase().contains(&q)
            || self.package_id.contains(&q)
            || self
                .authors
                .iter()
                .any(|a| a.to_ascii_lowercase().contains(&q))
    }

    /// RimWorld stores the workshop/local preview at `About/Preview.png`.
    pub fn preview_path(&self) -> Option<PathBuf> {
        let about = self.path.join("About");
        const NAMES: &[&str] = &[
            "Preview.png",
            "Preview.jpg",
            "preview.png",
            "About.png",
            "About.jpg",
        ];
        for name in NAMES {
            let candidate = about.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }
}

/// Source priority when several copies share a package ID (RimSort default).
pub fn source_priority_default() -> [ModSource; 3] {
    [ModSource::Dlc, ModSource::Local, ModSource::Workshop]
}

pub fn source_priority_steam() -> [ModSource; 3] {
    [ModSource::Workshop, ModSource::Local, ModSource::Dlc]
}

pub fn resolve_mod<'a>(mods: &'a [Mod], package_id: &str) -> Option<&'a Mod> {
    let pid = package_id.to_ascii_lowercase();
    let (pid, prefer_steam) = pid
        .strip_suffix("_steam")
        .map(|p| (p.to_string(), true))
        .unwrap_or((pid, false));
    let order = if prefer_steam {
        source_priority_steam()
    } else {
        source_priority_default()
    };
    for source in order {
        if let Some(m) = mods
            .iter()
            .find(|m| m.package_id == pid && m.source == source)
        {
            return Some(m);
        }
    }
    mods.iter().find(|m| m.package_id == pid)
}

pub fn mod_by_path<'a>(mods: &'a [Mod], path: &Path) -> Option<&'a Mod> {
    mods.iter().find(|m| m.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn preview_path_finds_about_preview_png() {
        let dir = std::env::temp_dir().join(format!("rimdeck-preview-{}", std::process::id()));
        let about = dir.join("About");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&about).unwrap();
        let preview = about.join("Preview.png");
        fs::write(&preview, b"not-a-real-png").unwrap();
        let m = Mod {
            path: dir.clone(),
            package_id: "a.b".into(),
            name: "A".into(),
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
        };
        assert_eq!(m.preview_path().as_deref(), Some(preview.as_path()));
        let _ = fs::remove_dir_all(&dir);
    }
}
