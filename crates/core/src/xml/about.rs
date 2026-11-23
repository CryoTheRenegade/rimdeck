use std::fs;
use std::path::Path;

use roxmltree::Document;

use crate::constants::DEFAULT_MISSING_PACKAGEID;
use crate::error::{Error, Result};
use crate::model::Dependency;

#[derive(Debug, Clone, Default)]
pub struct AboutXml {
    pub name: String,
    pub authors: Vec<String>,
    pub package_id: String,
    pub description: String,
    pub url: String,
    pub mod_version: String,
    pub supported_versions: Vec<String>,
    pub load_after: Vec<String>,
    pub load_before: Vec<String>,
    pub incompatible_with: Vec<String>,
    pub dependencies: Vec<Dependency>,
}

pub fn parse_about_file(path: &Path) -> Result<AboutXml> {
    let raw = fs::read_to_string(path)?;
    parse_about_str(&raw, path)
}

pub fn parse_about_str(raw: &str, path: &Path) -> Result<AboutXml> {
    let sanitized = sanitize_xml(raw);
    let doc = Document::parse(&sanitized).map_err(|e| Error::Xml {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    let root = doc.root_element();
    Ok(about_from_root(root, path))
}

fn about_from_root(root: roxmltree::Node<'_, '_>, path: &Path) -> AboutXml {
    let mut authors = child_li_texts(root, "authors");
    if authors.is_empty() {
        if let Some(author) = child_text(root, "author") {
            authors.push(author);
        }
    }

    let package_id = child_text(root, "packageId")
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_MISSING_PACKAGEID.to_string());

    let folder_name = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unknown Mod".to_string());

    let mut load_after = child_li_ids(root, "loadAfter");
    load_after.extend(child_li_ids(root, "forceLoadAfter"));
    let mut load_before = child_li_ids(root, "loadBefore");
    load_before.extend(child_li_ids(root, "forceLoadBefore"));

    AboutXml {
        name: child_text(root, "name").unwrap_or(folder_name),
        authors,
        package_id,
        description: child_text(root, "description").unwrap_or_default(),
        url: child_text(root, "url").unwrap_or_default(),
        mod_version: child_text(root, "modVersion").unwrap_or_default(),
        supported_versions: child_li_texts(root, "supportedVersions"),
        load_after: unique_lower(load_after),
        load_before: unique_lower(load_before),
        incompatible_with: unique_lower(child_li_ids(root, "incompatibleWith")),
        dependencies: parse_dependencies(root),
    }
}

fn parse_dependencies(root: roxmltree::Node<'_, '_>) -> Vec<Dependency> {
    let Some(node) = child_elem(root, "modDependencies") else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    for li in children_named(node, "li") {
        let package_id = child_text(li, "packageId")
            .map(|s| s.to_ascii_lowercase())
            .filter(|s| !s.is_empty());
        let Some(package_id) = package_id else {
            continue;
        };
        let display_name = child_text(li, "displayName").unwrap_or_default();
        deps.push(Dependency {
            package_id,
            display_name,
        });
    }
    deps
}

fn child_elem<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case(name))
}

fn children_named<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case(name))
        .collect()
}

fn child_text(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    let elem = child_elem(node, name)?;
    let text = elem.text()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn child_li_texts(node: roxmltree::Node<'_, '_>, name: &str) -> Vec<String> {
    let Some(elem) = child_elem(node, name) else {
        return Vec::new();
    };
    children_named(elem, "li")
        .into_iter()
        .filter_map(|li| li.text().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn child_li_ids(node: roxmltree::Node<'_, '_>, name: &str) -> Vec<String> {
    child_li_texts(node, name)
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

fn unique_lower(ids: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Replace bare `&` that would break XML parsers. RimWorld About.xml often
/// contains unescaped ampersands in descriptions.
pub fn sanitize_xml(input: &str) -> String {
    let s = input.trim_start_matches('\u{feff}');
    let mut out = String::with_capacity(s.len() + 16);
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&' {
            let rest: String = chars[i + 1..].iter().take(12).collect();
            if is_entity_start(&rest) {
                out.push('&');
            } else {
                out.push_str("&amp;");
            }
        } else {
            out.push(chars[i]);
        }
        i += 1;
    }
    out
}

fn is_entity_start(rest: &str) -> bool {
    rest.starts_with("amp;")
        || rest.starts_with("lt;")
        || rest.starts_with("gt;")
        || rest.starts_with("quot;")
        || rest.starts_with("apos;")
        || rest.starts_with('#')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parses_basic_about() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<ModMetaData>
  <name>Amazing Mod</name>
  <author>Ludeon</author>
  <packageId>Author.Mod</packageId>
  <description>hello</description>
  <supportedVersions><li>1.5</li></supportedVersions>
  <loadAfter><li>Ludeon.RimWorld</li></loadAfter>
  <loadBefore><li>other.mod</li></loadBefore>
  <incompatibleWith><li>CETeam.CombatExtended</li></incompatibleWith>
  <modDependencies>
    <li>
      <packageId>Ludeon.RimWorld.Biotech</packageId>
      <displayName>Biotech</displayName>
    </li>
  </modDependencies>
</ModMetaData>
"#;
        let about = parse_about_str(xml, Path::new("About/About.xml")).unwrap();
        assert_eq!(about.package_id, "author.mod");
        assert_eq!(about.name, "Amazing Mod");
        assert_eq!(about.load_after, vec!["ludeon.rimworld"]);
        assert_eq!(about.dependencies[0].package_id, "ludeon.rimworld.biotech");
        assert_eq!(about.incompatible_with, vec!["ceteam.combatextended"]);
    }

    #[test]
    fn sanitizes_bare_ampersand() {
        let xml = r#"<ModMetaData>
  <name>A & B</name>
  <packageId>a.b</packageId>
</ModMetaData>"#;
        let about = parse_about_str(xml, Path::new("About.xml")).unwrap();
        assert_eq!(about.name, "A & B");
    }

    #[test]
    fn missing_package_id_uses_sentinel() {
        let xml = r#"<ModMetaData><name>X</name></ModMetaData>"#;
        let about = parse_about_str(xml, Path::new("About.xml")).unwrap();
        assert_eq!(about.package_id, DEFAULT_MISSING_PACKAGEID);
    }
}
