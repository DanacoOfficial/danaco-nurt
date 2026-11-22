// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registration of the stages of the `code` command group.

use std::collections::BTreeSet;
use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::stage::StageImpl;

const CATALOG: &str = include_str!("../../../catalog/code.toml");

fn catalog_commands() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut after_header = false;
    for line in CATALOG.lines() {
        let line = line.trim();
        if line == "[[command]]" {
            after_header = true;
        } else if after_header {
            if let Some(name) = line
                .strip_prefix("name = \"")
                .and_then(|rest| rest.strip_suffix('"'))
            {
                names.insert(name.to_owned());
            }
            after_header = false;
        }
    }
    names
}

fn registered() -> Result<Vec<Arc<dyn StageImpl>>> {
    let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
    nurt_code::register(&mut stages)?;
    Ok(stages)
}

#[test]
fn registers_every_catalog_command() -> Result<()> {
    let registered: BTreeSet<String> = registered()?
        .iter()
        .map(|stage| stage.descriptor().command)
        .collect();
    let missing: Vec<_> = catalog_commands()
        .difference(&registered)
        .cloned()
        .collect();
    assert!(missing.is_empty(), "commands without a stage: {missing:?}");
    Ok(())
}

#[test]
fn catalog_lists_commands() {
    assert!(!catalog_commands().is_empty());
}

#[test]
fn registers_each_command_once() -> Result<()> {
    let stages = registered()?;
    let names: BTreeSet<String> = stages.iter().map(|s| s.descriptor().command).collect();
    assert_eq!(names.len(), stages.len());
    Ok(())
}
