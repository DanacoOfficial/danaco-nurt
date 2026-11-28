// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::commands
// Responsibility:   Stages of the `documents` command group.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Stages of the `documents` command group.

pub mod query;
pub mod set;
pub mod sql;

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::stage::StageRegistrar;

use crate::registry::Registry;

/// Registers the stage implementations of the `documents` command group.
///
/// Adds `query`, `set`, `sql` and the `remove` implementation for `item`
/// input. Documents are opened with the formats of [`Registry::builtin`], and
/// the format of a document is chosen with the context of its path.
///
/// # Errors
///
/// Fails with the error of the registrar when it rejects an implementation.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::stage::StageImpl;
///
/// let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
/// nurt_doc::register(&mut stages)?;
/// let mut commands: Vec<String> = stages.iter().map(|s| s.descriptor().command).collect();
/// commands.sort();
/// assert_eq!(commands, ["query", "remove", "set", "sql"]);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    let registry = Arc::new(Registry::builtin());
    registrar.register(Arc::new(query::QueryStage::new(Arc::clone(&registry))))?;
    registrar.register(Arc::new(set::SetStage::new(Arc::clone(&registry))))?;
    registrar.register(Arc::new(set::RemoveItemsStage::new(registry)))?;
    registrar.register(Arc::new(sql::SqlStage))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
