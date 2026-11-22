// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stages of the `code` command group.

pub mod actions;
pub mod call_hierarchy;
pub mod definition;
pub mod diagnostics;
pub mod hover;
pub mod implementations;
pub mod languages;
pub mod outline;
pub mod profile;
pub mod references;
pub mod rename;
pub mod symbols;
pub mod type_definition;

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::stage::StageRegistrar;

/// Registers the stage implementations of the `code` command group.
///
/// Adds every `code.*` command. The stages are created without a semantic
/// provider and without a repository service: they answer from syntax
/// analysis, and options that need a provider fail with `E_EXTERNAL_MISSING`.
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
/// nurt_code::register(&mut stages)?;
/// assert_eq!(stages.len(), 13);
/// assert!(stages.iter().all(|s| s.descriptor().command.starts_with("code.")));
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    registrar.register(Arc::new(symbols::CodeSymbols::new(None)))?;
    registrar.register(Arc::new(outline::CodeOutline::new(None)))?;
    registrar.register(Arc::new(definition::CodeDefinition::new(None)))?;
    registrar.register(Arc::new(references::CodeReferences::new(None)))?;
    registrar.register(Arc::new(hover::CodeHover::new(None)))?;
    registrar.register(Arc::new(diagnostics::CodeDiagnostics::new(None, None)))?;
    registrar.register(Arc::new(rename::CodeRename::new(None)))?;
    registrar.register(Arc::new(languages::CodeLanguages::new(None)))?;
    registrar.register(Arc::new(implementations::CodeImplementations::new(None)))?;
    registrar.register(Arc::new(type_definition::CodeTypeDefinition::new(None)))?;
    registrar.register(Arc::new(call_hierarchy::CodeCallHierarchy::new(None)))?;
    registrar.register(Arc::new(actions::CodeActions::new(None)))?;
    registrar.register(Arc::new(profile::CodeProfile))
}
