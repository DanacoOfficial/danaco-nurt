// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Checks the manifests of the workspace against `tables/crate-graph.toml`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use toml::{Table, Value};

const INTERNAL_PREFIX: &str = "nurt-";
/// Crates of other projects kept in the workspace with their own names.
const VENDORED: [&str; 1] = ["jaq-json"];
const FACADE: &str = "nurt";

fn workspace_root() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

fn read_toml(path: &Path) -> Table {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    toml::from_str(&text).unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()))
}

#[derive(Debug)]
struct Entry {
    layer: String,
    area: String,
    shared: bool,
    reserved: bool,
    deps: BTreeSet<String>,
    dev: BTreeSet<String>,
}

struct Graph {
    layers: Vec<String>,
    areas: BTreeSet<String>,
    integrating: BTreeSet<String>,
    dev_anywhere: BTreeSet<String>,
    crates: BTreeMap<String, Entry>,
}

impl Graph {
    fn load() -> Self {
        let table = read_toml(&workspace_root().join("tables/crate-graph.toml"));
        assert_eq!(table["format"].as_str(), Some("nurt-crate-graph/1"));
        let names = |key: &str| -> Vec<String> {
            table[key]
                .as_array()
                .unwrap_or_else(|| panic!("`{key}` must be an array of tables"))
                .iter()
                .map(|item| item["name"].as_str().unwrap().to_owned())
                .collect()
        };
        let rules = table["rules"].as_table().expect("`rules` table");
        let crates = table["crate"]
            .as_table()
            .expect("`crate` table")
            .iter()
            .map(|(name, value)| (name.clone(), entry(name, value)))
            .collect();
        Self {
            layers: names("layer"),
            areas: names("area").into_iter().collect(),
            integrating: string_set(&rules["integrating_layers"]),
            dev_anywhere: string_set(&rules["dev_anywhere"]),
            crates,
        }
    }

    fn rank(&self, layer: &str) -> usize {
        self.layers
            .iter()
            .position(|name| name == layer)
            .unwrap_or_else(|| panic!("unknown layer `{layer}`"))
    }

    fn entry(&self, name: &str) -> &Entry {
        self.crates
            .get(name)
            .unwrap_or_else(|| panic!("crate `{name}` has no entry in tables/crate-graph.toml"))
    }
}

fn string_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("array of strings")
        .iter()
        .map(|item| item.as_str().expect("string").to_owned())
        .collect()
}

fn entry(name: &str, value: &Value) -> Entry {
    let table = value
        .as_table()
        .unwrap_or_else(|| panic!("entry `{name}` must be a table"));
    let text = |key: &str| {
        table
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("entry `{name}` needs `{key}`"))
            .to_owned()
    };
    let flag = |key: &str| table.get(key).and_then(Value::as_bool).unwrap_or(false);
    let list = |key: &str| table.get(key).map(string_set).unwrap_or_default();
    assert!(
        table.get("summary").and_then(Value::as_str).is_some(),
        "entry `{name}` needs `summary`"
    );
    Entry {
        layer: text("layer"),
        area: text("area"),
        shared: flag("shared"),
        reserved: flag("reserved"),
        deps: list("deps"),
        dev: list("dev"),
    }
}

/// Dependencies of one manifest on crates of the workspace.
#[derive(Default)]
struct ManifestDeps {
    normal: BTreeSet<String>,
    dev: BTreeSet<String>,
}

fn is_internal(name: &str) -> bool {
    name == FACADE || name.starts_with(INTERNAL_PREFIX)
}

fn collect(section: Option<&Value>, into: &mut BTreeSet<String>) {
    let Some(table) = section.and_then(Value::as_table) else {
        return;
    };
    for (key, spec) in table {
        let package = spec
            .get("package")
            .and_then(Value::as_str)
            .unwrap_or(key.as_str());
        if is_internal(package) {
            into.insert(package.to_owned());
        }
    }
}

fn manifest_deps(manifest: &Table) -> ManifestDeps {
    let mut deps = ManifestDeps::default();
    let mut scan = |table: &Table| {
        collect(table.get("dependencies"), &mut deps.normal);
        collect(table.get("build-dependencies"), &mut deps.normal);
        collect(table.get("dev-dependencies"), &mut deps.dev);
    };
    scan(manifest);
    if let Some(targets) = manifest.get("target").and_then(Value::as_table) {
        for target in targets.values().filter_map(Value::as_table) {
            scan(target);
        }
    }
    deps
}

fn workspace_manifests() -> BTreeMap<String, ManifestDeps> {
    let crates_dir = workspace_root().join("crates");
    let mut manifests = BTreeMap::new();
    for item in std::fs::read_dir(&crates_dir).expect("crates directory") {
        let path = item.expect("directory entry").path().join("Cargo.toml");
        if !path.is_file() {
            continue;
        }
        let manifest = read_toml(&path);
        let name = manifest["package"]["name"]
            .as_str()
            .unwrap_or_else(|| panic!("{} has no package name", path.display()))
            .to_owned();
        manifests.insert(name, manifest_deps(&manifest));
    }
    manifests
}

#[test]
fn every_workspace_crate_has_an_entry_and_reserved_entries_do_not_exist() {
    let graph = Graph::load();
    let manifests = workspace_manifests();
    assert!(!manifests.is_empty(), "no crates found");
    for name in manifests.keys() {
        let entry = graph.entry(name);
        assert!(
            !entry.reserved,
            "crate `{name}` exists; remove `reserved = true` from its entry"
        );
    }
    for (name, entry) in &graph.crates {
        if !entry.reserved {
            assert!(
                manifests.contains_key(name),
                "entry `{name}` has no crate in crates/; mark it `reserved = true` or remove it"
            );
        }
    }
}

#[test]
fn entries_use_known_layers_and_areas() {
    let graph = Graph::load();
    for (name, entry) in &graph.crates {
        graph.rank(&entry.layer);
        assert!(
            graph.areas.contains(&entry.area),
            "entry `{name}`: unknown area `{}`",
            entry.area
        );
        assert!(
            is_internal(name) || VENDORED.contains(&name.as_str()),
            "entry `{name}` is not a workspace crate name"
        );
    }
    for layer in &graph.integrating {
        graph.rank(layer);
    }
    for name in &graph.dev_anywhere {
        assert!(
            graph.entry(name).shared,
            "`{name}` in dev_anywhere must be shared"
        );
    }
}

#[test]
fn manifest_dependencies_are_allowed() {
    let graph = Graph::load();
    let mut violations = Vec::new();
    for (name, deps) in workspace_manifests() {
        let entry = graph.entry(&name);
        for dep in &deps.normal {
            if !entry.deps.contains(dep) {
                violations.push(format!("{name} -> {dep}"));
            }
        }
        for dep in &deps.dev {
            // A development dependency of a crate on itself only enables
            // features of the crate for its own tests; it is not an edge.
            let allowed = *dep == name
                || entry.deps.contains(dep)
                || entry.dev.contains(dep)
                || (graph.dev_anywhere.contains(dep)
                    && graph.rank(&graph.entry(dep).layer) <= graph.rank(&entry.layer));
            if !allowed {
                violations.push(format!("{name} -> {dep} (dev)"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "dependencies not allowed by tables/crate-graph.toml:\n{}",
        violations.join("\n")
    );
}

#[test]
fn allowed_dependencies_follow_layers_and_areas() {
    let graph = Graph::load();
    let mut violations = Vec::new();
    for (name, entry) in &graph.crates {
        let rank = graph.rank(&entry.layer);
        let integrating = graph.integrating.contains(&entry.layer);
        for dep in entry.deps.iter().chain(&entry.dev) {
            assert_ne!(dep, name, "entry `{name}` depends on itself");
            let target = graph.entry(dep);
            if graph.rank(&target.layer) > rank {
                violations.push(format!(
                    "{name} ({}) -> {dep} ({}): higher layer",
                    entry.layer, target.layer
                ));
            }
            if target.area != entry.area && !target.shared && !integrating {
                violations.push(format!(
                    "{name} ({}) -> {dep} ({}): other area and not shared",
                    entry.area, target.area
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "graph rule violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn contracts_depend_on_no_workspace_crate() {
    let graph = Graph::load();
    let contracts = graph.entry("nurt-contracts");
    assert!(contracts.deps.is_empty() && contracts.dev.is_empty());
    assert_eq!(graph.rank(&contracts.layer), 0);
}

#[test]
fn allowed_graph_is_acyclic() {
    let graph = Graph::load();
    let mut remaining: BTreeMap<&str, BTreeSet<&str>> = graph
        .crates
        .iter()
        .map(|(name, entry)| {
            let deps = entry
                .deps
                .iter()
                .chain(&entry.dev)
                .map(String::as_str)
                .collect();
            (name.as_str(), deps)
        })
        .collect();
    while !remaining.is_empty() {
        let ready: Vec<&str> = remaining
            .iter()
            .filter(|(_, deps)| deps.iter().all(|dep| !remaining.contains_key(dep)))
            .map(|(name, _)| *name)
            .collect();
        assert!(
            !ready.is_empty(),
            "cycle among: {}",
            remaining.keys().copied().collect::<Vec<_>>().join(", ")
        );
        for name in ready {
            remaining.remove(name);
        }
    }
}
