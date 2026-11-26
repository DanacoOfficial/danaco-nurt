// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The revision recorded in a bound tree and the revision reported by the
//! command registry follow one definition.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by panicking"
)]

use std::path::Path;

use nurt_bind::Catalog;
use nurt_catalog::Registry;
use nurt_contracts::spec::StageSpec;

const CATALOG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../catalog");

#[test]
fn bound_tree_and_registry_report_the_same_revision() {
    let catalog = Catalog::load_dir(Path::new(CATALOG_DIR)).unwrap();
    let registry = Registry::load_dir(Path::new(CATALOG_DIR)).unwrap();
    assert_eq!(catalog.revision(), registry.catalog_rev().unwrap());
    assert_eq!(
        Registry::load_embedded().unwrap().catalog_rev().unwrap(),
        catalog.revision()
    );
}

#[test]
fn an_extension_changes_the_registry_revision_only() {
    let catalog = Catalog::load_dir(Path::new(CATALOG_DIR)).unwrap();
    let mut registry = Registry::load_dir(Path::new(CATALOG_DIR)).unwrap();
    let mut spec: StageSpec = registry.command("count").cloned().unwrap();
    spec.name = "metrics.count".to_owned();
    spec.group.clear();
    registry
        .register_extension("metrics", "Metrics commands.", vec![spec])
        .unwrap();
    assert_ne!(catalog.revision(), registry.catalog_rev().unwrap());
}
