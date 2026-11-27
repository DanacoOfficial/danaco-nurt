// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/coverage/catalog.rs
// Responsibility:   Commands of the command catalog `catalog/*.toml` with the fields that.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Commands of the command catalog `catalog/*.toml` with the fields that
//! decide journal coverage.

use std::fs;
use std::path::PathBuf;

use nurt_contracts::kinds::{Category, EffectKind, Reversibility};
use serde::Deserialize;

use crate::table::repository_root;

const CATALOG_FORMAT: &str = "nurt-catalog/1";

#[derive(Debug, Deserialize)]
struct CatalogFile {
    format: String,
    group: String,
    #[serde(default)]
    command: Vec<CommandFile>,
}

#[derive(Debug, Deserialize)]
struct CommandFile {
    name: String,
    category: Category,
    reversibility: Reversibility,
    #[serde(default)]
    effects: Vec<Effect>,
}

/// Effect of one catalog action.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Effect {
    /// Action name.
    pub(crate) action: String,
    /// Kind of the effect.
    pub(crate) effect: EffectKind,
}

/// One catalog command.
#[derive(Debug, Clone)]
pub(crate) struct Command {
    /// Name.
    pub(crate) name: String,
    /// Group of the catalog file.
    pub(crate) group: String,
    /// Category.
    pub(crate) category: Category,
    /// Reversibility of the default form.
    pub(crate) reversibility: Reversibility,
    /// Actions with their own effect.
    pub(crate) effects: Vec<Effect>,
}

impl Command {
    /// Whether the table must describe how the command is journaled: it
    /// changes data, runs a program, or is a journal command with an effect.
    pub(crate) fn is_journaled(&self) -> bool {
        match self.category {
            Category::Change | Category::Run => true,
            Category::Journal => self.reversibility != Reversibility::Read,
            _ => false,
        }
    }

    /// Actions whose effect is not a read.
    pub(crate) fn changing_actions(&self) -> impl Iterator<Item = &str> {
        self.effects
            .iter()
            .filter(|effect| effect.effect != EffectKind::Read)
            .map(|effect| effect.action.as_str())
    }
}

/// Every command of the catalog files present in the repository, in file
/// order.
pub(crate) fn load() -> Vec<Command> {
    let dir = repository_root().join("catalog");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot list {}: {error}", dir.display()))
        .map(|entry| entry.expect("catalog entry is readable").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| !name.starts_with('_'))
        })
        .collect();
    paths.sort();
    let mut commands = Vec::new();
    for path in paths {
        let text = fs::read_to_string(&path).expect("catalog file is readable");
        let file: CatalogFile =
            toml::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        if file.format != CATALOG_FORMAT {
            continue;
        }
        commands.extend(file.command.into_iter().map(|command| Command {
            name: command.name,
            group: file.group.clone(),
            category: command.category,
            reversibility: command.reversibility,
            effects: command.effects,
        }));
    }
    commands
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
