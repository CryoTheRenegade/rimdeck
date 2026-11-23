use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::constants::{known_tier_one_mods, known_tier_zero_mods};
use crate::error::{Error, Result};
use crate::model::{mod_by_path, Mod};

/// Compiled load-order graph. Ported from RimSort `CompiledDependencyData`.
#[derive(Debug, Clone, Default)]
pub struct CompiledDependencyData {
    /// package_id -> package_ids that must load *before* it
    pub deps_graph: HashMap<String, HashSet<String>>,
    pub rev_deps_graph: HashMap<String, HashSet<String>>,
    pub tier_zero_mods: HashSet<String>,
    pub tier_one_mods: HashSet<String>,
    pub tier_three_mods: HashSet<String>,
    pub incompatibilities: HashMap<String, HashSet<String>>,
}

impl CompiledDependencyData {
    pub fn build(mods: &[Mod], use_dependencies_as_load_after: bool) -> Self {
        let mut compiled = Self {
            tier_zero_mods: known_tier_zero_mods(),
            tier_one_mods: known_tier_one_mods(),
            ..Self::default()
        };

        let all_ids: HashSet<String> = mods.iter().map(|m| m.package_id.clone()).collect();

        for m in mods {
            let pid = &m.package_id;
            for dep in &m.load_after {
                if !all_ids.contains(dep) {
                    continue;
                }
                compiled
                    .deps_graph
                    .entry(pid.clone())
                    .or_default()
                    .insert(dep.clone());
                compiled
                    .rev_deps_graph
                    .entry(dep.clone())
                    .or_default()
                    .insert(pid.clone());
            }
            for target in &m.load_before {
                if !all_ids.contains(target) {
                    continue;
                }
                compiled
                    .deps_graph
                    .entry(target.clone())
                    .or_default()
                    .insert(pid.clone());
                compiled
                    .rev_deps_graph
                    .entry(pid.clone())
                    .or_default()
                    .insert(target.clone());
            }
            for incompat in &m.incompatible_with {
                if !all_ids.contains(incompat) {
                    continue;
                }
                compiled
                    .incompatibilities
                    .entry(pid.clone())
                    .or_default()
                    .insert(incompat.clone());
                compiled
                    .incompatibilities
                    .entry(incompat.clone())
                    .or_default()
                    .insert(pid.clone());
            }
            if m.load_first && !compiled.tier_zero_mods.contains(pid) {
                compiled.tier_one_mods.insert(pid.clone());
            }
            if m.load_last {
                compiled.tier_three_mods.insert(pid.clone());
            }
        }

        if use_dependencies_as_load_after {
            let excluded: HashSet<String> = compiled
                .tier_one_mods
                .union(&compiled.tier_three_mods)
                .cloned()
                .collect();
            for m in mods {
                if excluded.contains(&m.package_id) {
                    continue;
                }
                for dep in &m.dependencies {
                    let dep_id = &dep.package_id;
                    if !all_ids.contains(dep_id) || excluded.contains(dep_id) {
                        continue;
                    }
                    if compiled
                        .deps_graph
                        .get(dep_id)
                        .is_some_and(|s| s.contains(&m.package_id))
                    {
                        continue;
                    }
                    compiled
                        .deps_graph
                        .entry(m.package_id.clone())
                        .or_default()
                        .insert(dep_id.clone());
                    compiled
                        .rev_deps_graph
                        .entry(dep_id.clone())
                        .or_default()
                        .insert(m.package_id.clone());
                }
            }
        }

        compiled
    }
}

/// Topological sort of active mods, matching RimSort `Sorter` + `do_topo_sort`.
pub fn sort_active(mods: &[Mod], active_paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let compiled = CompiledDependencyData::build(mods, true);
    let active_ids: HashSet<String> = active_paths
        .iter()
        .filter_map(|p| mod_by_path(mods, p).map(|m| m.package_id.clone()))
        .collect();

    let active_deps = filter_graph_to_active(&compiled.deps_graph, &active_ids);
    let active_rev = filter_graph_to_active(&compiled.rev_deps_graph, &active_ids);

    let tier_zero = collect_tier_mods(&compiled.tier_zero_mods, &active_deps, &active_ids);
    let tier_one = collect_tier_mods(&compiled.tier_one_mods, &active_deps, &active_ids);
    let tier_three = collect_tier_three(&compiled.tier_three_mods, &active_rev, &active_ids);
    let all_tiered: HashSet<String> = tier_zero
        .iter()
        .chain(tier_one.iter())
        .chain(tier_three.iter())
        .cloned()
        .collect();
    let tier_two: HashSet<String> = active_ids.difference(&all_tiered).cloned().collect();

    let graphs = [
        extract_tier_subgraph(&active_deps, &tier_zero),
        extract_tier_subgraph(&active_deps, &tier_one),
        extract_tier_subgraph(&active_deps, &tier_two),
        extract_tier_subgraph(&active_deps, &tier_three),
    ];

    let mut sorted_paths = Vec::new();
    for graph in &graphs {
        let level_paths = do_topo_sort(graph, active_paths, mods)?;
        sorted_paths.extend(level_paths);
    }

    let mut seen = HashSet::new();
    sorted_paths.retain(|p| seen.insert(p.clone()));
    Ok(sorted_paths)
}

/// RimPy-style alphabetical sort: order by name, then force-insert dependencies.
///
/// Unlike topological sort this has no cycle detection; a cycle still produces
/// some order. Ported from RimSort `do_alphabetical_sort`.
pub fn sort_active_alphabetical(mods: &[Mod], active_paths: &[PathBuf]) -> Vec<PathBuf> {
    let compiled = CompiledDependencyData::build(mods, true);
    let mut packageid_to_name: HashMap<String, String> = HashMap::new();
    let mut packageid_to_path: HashMap<String, PathBuf> = HashMap::new();
    let mut active_ids: HashSet<String> = HashSet::new();
    for path in active_paths {
        if let Some(m) = mod_by_path(mods, path) {
            active_ids.insert(m.package_id.clone());
            packageid_to_name.insert(m.package_id.clone(), m.name.clone());
            packageid_to_path.insert(m.package_id.clone(), path.clone());
        }
    }

    let mut graph = filter_graph_to_active(&compiled.deps_graph, &active_ids);
    for id in &active_ids {
        graph.entry(id.clone()).or_default();
    }

    let mut alpha: Vec<(String, String)> = packageid_to_name
        .iter()
        .map(|(pid, name)| (pid.clone(), name.clone()))
        .collect();
    alpha.sort_by_key(|a| a.1.to_ascii_lowercase());

    let mut load_order: Vec<String> = Vec::new();
    for (pid, _) in &alpha {
        if !load_order.iter().any(|e| e == pid) {
            load_order.push(pid.clone());
            let idx = load_order.len() - 1;
            recursively_force_insert(&mut load_order, &graph, pid, &packageid_to_name, idx);
        }
    }

    load_order
        .into_iter()
        .filter_map(|pid| packageid_to_path.get(&pid).cloned())
        .collect()
}

fn recursively_force_insert(
    load_order: &mut Vec<String>,
    graph: &HashMap<String, HashSet<String>>,
    package_id: &str,
    names: &HashMap<String, String>,
    index_just_appended: usize,
) {
    let mut deps: Vec<(String, String)> = graph
        .get(package_id)
        .into_iter()
        .flatten()
        .filter_map(|dep| names.get(dep).map(|n| (dep.clone(), n.clone())))
        .collect();
    deps.sort_by_key(|a| a.1.clone());

    for (dep_id, _) in deps {
        if load_order.iter().any(|e| e == &dep_id) {
            continue;
        }
        let parent_idx = load_order
            .iter()
            .position(|e| e == package_id)
            .unwrap_or(load_order.len());
        let mut insert_at = index_just_appended;
        let slice_end = parent_idx;
        if insert_at < slice_end {
            for e in load_order[insert_at..slice_end].iter().rev() {
                if graph.get(&dep_id).is_some_and(|s| s.contains(e)) {
                    insert_at = load_order.iter().position(|x| x == e).unwrap_or(insert_at) + 1;
                    break;
                }
            }
        }
        let insert_at = insert_at.min(load_order.len());
        load_order.insert(insert_at, dep_id.clone());
        let new_idx = load_order
            .iter()
            .position(|e| e == &dep_id)
            .unwrap_or(insert_at);
        recursively_force_insert(load_order, graph, &dep_id, names, new_idx);
    }
}

fn filter_graph_to_active(
    graph: &HashMap<String, HashSet<String>>,
    active: &HashSet<String>,
) -> HashMap<String, HashSet<String>> {
    graph
        .iter()
        .filter(|(pid, _)| active.contains(*pid))
        .map(|(pid, deps)| {
            (
                pid.clone(),
                deps.iter()
                    .filter(|d| active.contains(*d))
                    .cloned()
                    .collect(),
            )
        })
        .collect()
}

fn extract_tier_subgraph(
    full: &HashMap<String, HashSet<String>>,
    tier: &HashSet<String>,
) -> HashMap<String, HashSet<String>> {
    tier.iter()
        .map(|id| {
            let deps = full
                .get(id)
                .map(|s| s.intersection(tier).cloned().collect())
                .unwrap_or_default();
            (id.clone(), deps)
        })
        .collect()
}

fn collect_tier_mods(
    known: &HashSet<String>,
    deps: &HashMap<String, HashSet<String>>,
    active: &HashSet<String>,
) -> HashSet<String> {
    let mut tier = HashSet::new();
    for id in known {
        if active.contains(id) {
            tier.insert(id.clone());
            if deps.contains_key(id) {
                tier.extend(get_dependencies_recursive(id, deps, &mut HashSet::new()));
            }
        }
    }
    tier.intersection(active).cloned().collect()
}

fn collect_tier_three(
    known: &HashSet<String>,
    rev: &HashMap<String, HashSet<String>>,
    active: &HashSet<String>,
) -> HashSet<String> {
    let mut tier = HashSet::new();
    for id in known {
        if active.contains(id) {
            tier.insert(id.clone());
            tier.extend(get_reverse_dependencies_recursive(
                id,
                rev,
                &mut HashSet::new(),
            ));
        }
    }
    tier.intersection(active).cloned().collect()
}

fn get_dependencies_recursive(
    package_id: &str,
    graph: &HashMap<String, HashSet<String>>,
    processed: &mut HashSet<String>,
) -> HashSet<String> {
    let mut out = HashSet::new();
    if let Some(deps) = graph.get(package_id) {
        for dep in deps {
            if processed.insert(dep.clone()) {
                out.insert(dep.clone());
                out.extend(get_dependencies_recursive(dep, graph, processed));
            }
        }
    }
    out
}

fn get_reverse_dependencies_recursive(
    package_id: &str,
    graph: &HashMap<String, HashSet<String>>,
    processed: &mut HashSet<String>,
) -> HashSet<String> {
    get_dependencies_recursive(package_id, graph, processed)
}

/// Port of Python `toposort` used by RimSort: `{item: set(deps)}` yields levels.
pub fn toposort_levels(data: &HashMap<String, HashSet<String>>) -> Result<Vec<HashSet<String>>> {
    let mut data: HashMap<String, HashSet<String>> = data
        .iter()
        .map(|(k, v)| {
            let mut deps = v.clone();
            deps.remove(k);
            (k.clone(), deps)
        })
        .collect();

    let mut extra = HashSet::new();
    for deps in data.values() {
        extra.extend(deps.iter().cloned());
    }
    for item in extra {
        data.entry(item).or_default();
    }

    let mut levels = Vec::new();
    loop {
        let ordered: HashSet<String> = data
            .iter()
            .filter(|(_, deps)| deps.is_empty())
            .map(|(k, _)| k.clone())
            .collect();
        if ordered.is_empty() {
            break;
        }
        for item in &ordered {
            data.remove(item);
        }
        for deps in data.values_mut() {
            for item in &ordered {
                deps.remove(item);
            }
        }
        levels.push(ordered);
    }
    if !data.is_empty() {
        let mut involved: Vec<_> = data.keys().cloned().collect();
        involved.sort();
        return Err(Error::Cycle(involved.join(", ")));
    }
    Ok(levels)
}

fn do_topo_sort(
    graph: &HashMap<String, HashSet<String>>,
    active_paths: &[PathBuf],
    mods: &[Mod],
) -> Result<Vec<PathBuf>> {
    let levels = toposort_levels(graph)?;
    let mut packageid_to_path: HashMap<String, PathBuf> = HashMap::new();
    let mut path_to_name: HashMap<PathBuf, String> = HashMap::new();
    for path in active_paths {
        if let Some(m) = mod_by_path(mods, path) {
            packageid_to_path.insert(m.package_id.clone(), path.clone());
            path_to_name.insert(path.clone(), m.name.clone());
        }
    }

    let mut reordered = Vec::new();
    for level in levels {
        let mut temp: Vec<PathBuf> = level
            .into_iter()
            .filter_map(|pid| packageid_to_path.get(&pid).cloned())
            .collect();
        temp.sort_by_key(|p| {
            path_to_name
                .get(p)
                .map(|n| n.to_ascii_lowercase())
                .unwrap_or_else(|| "name error in mod about.xml".into())
        });
        reordered.extend(temp);
    }
    Ok(reordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dependency, ModSource};
    use std::path::{Path, PathBuf};

    fn dummy(path: &str, id: &str, name: &str, load_after: &[&str], load_before: &[&str]) -> Mod {
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
            load_after: load_after.iter().map(|s| s.to_string()).collect(),
            load_before: load_before.iter().map(|s| s.to_string()).collect(),
            incompatible_with: vec![],
            dependencies: vec![],
            load_first: false,
            load_last: false,
            published_file_id: None,
        }
    }

    #[test]
    fn toposort_respects_edges_and_alpha_within_level() {
        let mods = vec![
            dummy("/c", "c.mod", "Charlie", &["a.mod"], &[]),
            dummy("/a", "a.mod", "Alpha", &[], &[]),
            dummy("/b", "b.mod", "Bravo", &[], &[]),
        ];
        let active = vec![
            PathBuf::from("/c"),
            PathBuf::from("/b"),
            PathBuf::from("/a"),
        ];
        let sorted = sort_active(&mods, &active).unwrap();
        let pos = |p: &str| sorted.iter().position(|x| x == Path::new(p)).unwrap();
        assert!(pos("/a") < pos("/c"));
    }

    #[test]
    fn toposort_detects_cycle() {
        let mut graph = HashMap::new();
        graph.insert("a".into(), HashSet::from(["b".into()]));
        graph.insert("b".into(), HashSet::from(["a".into()]));
        let err = toposort_levels(&graph).unwrap_err();
        match err {
            Error::Cycle(s) => {
                assert!(s.contains('a'));
                assert!(s.contains('b'));
            }
            other => panic!("expected cycle, got {other}"),
        }
    }

    #[test]
    fn load_before_creates_reverse_edge() {
        let mods = vec![
            dummy("/framework", "fw.mod", "Framework", &[], &["patch.mod"]),
            dummy("/patch", "patch.mod", "Patch", &[], &[]),
        ];
        let active = vec![PathBuf::from("/patch"), PathBuf::from("/framework")];
        let sorted = sort_active(&mods, &active).unwrap();
        let pos = |p: &str| sorted.iter().position(|x| x == Path::new(p)).unwrap();
        assert!(pos("/framework") < pos("/patch"));
    }

    #[test]
    fn core_is_tier_zero() {
        let mods = vec![
            dummy("/core", "ludeon.rimworld", "Core", &[], &[]),
            dummy("/mod", "z.mod", "Zed", &[], &[]),
        ];
        let active = vec![PathBuf::from("/mod"), PathBuf::from("/core")];
        let sorted = sort_active(&mods, &active).unwrap();
        assert_eq!(sorted[0], PathBuf::from("/core"));
    }

    #[test]
    fn fixture_cycle_fails_sort() {
        use crate::scan::scan_mods;
        use crate::settings::Settings;
        let game = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/game");
        let settings = Settings {
            game_install: game.clone(),
            local_mods: game.join("Mods"),
            workshop: PathBuf::new(),
            config_dir: game.join("Config"),
            ..Settings::default()
        };
        let mods = scan_mods(&settings).unwrap();
        let cycle: Vec<_> = mods
            .iter()
            .filter(|m| m.package_id.starts_with("author.cycle"))
            .map(|m| m.path.clone())
            .collect();
        assert_eq!(cycle.len(), 2);
        assert!(sort_active(&mods, &cycle).is_err());
    }

    #[test]
    fn dependency_treated_as_load_after() {
        let mut patch = dummy("/patch", "p.mod", "Patch", &[], &[]);
        patch.dependencies = vec![Dependency {
            package_id: "fw.mod".into(),
            display_name: "FW".into(),
        }];
        let mods = vec![dummy("/fw", "fw.mod", "FW", &[], &[]), patch];
        let active = vec![PathBuf::from("/patch"), PathBuf::from("/fw")];
        let sorted = sort_active(&mods, &active).unwrap();
        let pos = |p: &str| sorted.iter().position(|x| x == Path::new(p)).unwrap();
        assert!(pos("/fw") < pos("/patch"));
    }

    #[test]
    fn alphabetical_orders_by_name_and_inserts_deps() {
        let mods = vec![
            dummy("/z", "z.mod", "Zebra", &["a.mod"], &[]),
            dummy("/a", "a.mod", "Apple", &[], &[]),
            dummy("/b", "b.mod", "Banana", &[], &[]),
        ];
        let active = vec![
            PathBuf::from("/z"),
            PathBuf::from("/b"),
            PathBuf::from("/a"),
        ];
        let sorted = sort_active_alphabetical(&mods, &active);
        let pos = |p: &str| sorted.iter().position(|x| x == Path::new(p)).unwrap();
        assert!(pos("/a") < pos("/z"));
        assert_eq!(sorted.len(), 3);
    }

    #[test]
    fn alphabetical_includes_mods_with_no_edges() {
        let mods = vec![
            dummy("/c", "c.mod", "Charlie", &[], &[]),
            dummy("/a", "a.mod", "Alpha", &[], &[]),
        ];
        let active = vec![PathBuf::from("/c"), PathBuf::from("/a")];
        let sorted = sort_active_alphabetical(&mods, &active);
        assert_eq!(
            sorted,
            vec![PathBuf::from("/a"), PathBuf::from("/c")]
        );
    }
}
