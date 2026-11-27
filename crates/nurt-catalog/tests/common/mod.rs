// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-catalog/tests/common/mod.rs
// Responsibility:   Builders of small synthetic catalogs and a stage implementation for tests.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Builders of small synthetic catalogs and a stage implementation for tests.

#![allow(dead_code, reason = "each test crate uses a subset of the helpers")]

use std::sync::Arc;

use nurt_contracts::stage::{InputKinds, StageDescriptor, StageImpl};
use nurt_contracts::value::RecordKind;

/// A minimal command; `extra` is appended to the command table, so it can
/// carry `[[command.param]]` tables and further keys placed after the summary.
pub(crate) fn command(name: &str, input: &str, extra: &str) -> String {
    format!(
        "[[command]]\nname = \"{name}\"\ncategory = \"transform\"\nreversibility = \"read\"\n\
         effect = \"read\"\nimplementation = \"builtin\"\ninput = [{input}]\n\
         output = [\"int\"]\nsummary = \"Count the input records.\"\n{extra}\n"
    )
}

/// A catalog file with the group header followed by the commands.
pub(crate) fn file(group: &str, commands: &[String]) -> String {
    format!(
        "format = \"nurt-catalog/1\"\ngroup = \"{group}\"\nsummary = \"A group of commands.\"\n\n{}",
        commands.join("\n")
    )
}

/// An index; every entry is a group name with the number of its commands.
pub(crate) fn index(entries: &[(&str, usize)], total: usize) -> String {
    let mut text = format!("format = \"nurt-catalog-index/1\"\ntotal = {total}\n");
    for (name, commands) in entries {
        text.push_str(&format!(
            "\n[[file]]\nname = \"{name}\"\nfile = \"{name}.toml\"\n\
             summary = \"A group of commands.\"\ncommands = {commands}\n"
        ));
    }
    text
}

/// A catalog with the core group holding the named commands, as index text
/// and file list.
pub(crate) fn core_catalog(names: &[&str]) -> (String, Vec<(String, String)>) {
    let commands: Vec<String> = names
        .iter()
        .map(|name| command(name, "\"any\"", ""))
        .collect();
    (
        index(&[("core", names.len())], names.len()),
        vec![("core.toml".to_owned(), file("core", &commands))],
    )
}

/// A stage implementation that only has a descriptor.
pub(crate) struct Fake {
    pub(crate) command: &'static str,
    pub(crate) source: bool,
    pub(crate) input: InputKinds,
}

impl Fake {
    pub(crate) fn only(command: &'static str, kinds: &[RecordKind]) -> Arc<Self> {
        Arc::new(Self {
            command,
            source: false,
            input: InputKinds::Only(kinds.to_vec()),
        })
    }

    pub(crate) fn any(command: &'static str) -> Arc<Self> {
        Arc::new(Self {
            command,
            source: false,
            input: InputKinds::Any,
        })
    }

    pub(crate) fn source(command: &'static str) -> Arc<Self> {
        Arc::new(Self {
            command,
            source: true,
            input: InputKinds::None,
        })
    }
}

impl StageImpl for Fake {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.command.to_owned(),
            source: self.source,
            input: self.input.clone(),
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
