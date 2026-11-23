use std::fs;
use std::path::Path;

use tracing::warn;

use crate::error::Result;
use crate::model::{Mod, ModSource};
use crate::settings::Settings;
use crate::xml::about::parse_about_file;

pub fn scan_mods(settings: &Settings) -> Result<Vec<Mod>> {
    let mut mods = Vec::new();
    let data = settings.game_install.join("Data");
    scan_dir(&data, ModSource::Dlc, &mut mods);
    scan_dir(&settings.local_mods, ModSource::Local, &mut mods);
    scan_dir(&settings.workshop, ModSource::Workshop, &mut mods);
    Ok(mods)
}

fn scan_dir(root: &Path, source: ModSource, out: &mut Vec<Mod>) {
    if !root.is_dir() {
        return;
    }
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(e) => {
            warn!("cannot read {}: {e}", root.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match load_mod(&path, source) {
            Ok(Some(m)) => out.push(m),
            Ok(None) => {}
            Err(e) => warn!("skipping {}: {e}", path.display()),
        }
    }
}

pub fn load_mod(path: &Path, source: ModSource) -> Result<Option<Mod>> {
    let about_path = path.join("About").join("About.xml");
    if !about_path.is_file() {
        return Ok(None);
    }
    let about = parse_about_file(&about_path)?;
    let published_file_id = read_published_file_id(path);
    Ok(Some(Mod {
        path: path.to_path_buf(),
        package_id: about.package_id,
        name: about.name,
        authors: about.authors,
        description: about.description,
        url: about.url,
        mod_version: about.mod_version,
        supported_versions: about.supported_versions,
        source,
        load_after: about.load_after,
        load_before: about.load_before,
        incompatible_with: about.incompatible_with,
        dependencies: about.dependencies,
        load_first: false,
        load_last: false,
        published_file_id,
    }))
}

fn read_published_file_id(mod_path: &Path) -> Option<String> {
    let file = mod_path.join("About").join("PublishedFileId.txt");
    if file.is_file() {
        if let Ok(text) = fs::read_to_string(&file) {
            let id = text.trim();
            if id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty() && id != "0" {
                return Some(id.to_string());
            }
        }
    }
    let folder = mod_path.file_name()?.to_string_lossy();
    if folder.chars().all(|c| c.is_ascii_digit()) && folder != "0" {
        return Some(folder.into_owned());
    }
    None
}

#[cfg(test)]
fn fixture_game_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/game")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn scans_fixture_game() {
        let game = fixture_game_dir();
        let settings = Settings {
            game_install: game.clone(),
            local_mods: game.join("Mods"),
            workshop: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workshop"),
            config_dir: game.join("Config"),
            ..Settings::default()
        };
        let mods = scan_mods(&settings).unwrap();
        let ids: Vec<_> = mods.iter().map(|m| m.package_id.as_str()).collect();
        assert!(ids.contains(&"ludeon.rimworld"));
        assert!(ids.contains(&"brrainz.harmony"));
        assert!(ids.contains(&"author.patchmod"));
        assert!(ids.contains(&"author.workshopmod"));
        let ws = mods
            .iter()
            .find(|m| m.package_id == "author.workshopmod")
            .unwrap();
        assert_eq!(ws.source, ModSource::Workshop);
        let dlc = mods
            .iter()
            .find(|m| m.package_id == "ludeon.rimworld")
            .unwrap();
        assert_eq!(dlc.source, ModSource::Dlc);
    }
}
