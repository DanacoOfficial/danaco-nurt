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

use crate::semantic::SemanticProvider;

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
    register_with(registrar, None)
}

/// Registers the stage implementations of the `code` command group with a
/// semantic provider.
///
/// With a provider the stages answer from language servers where the request
/// asks for them, and fall back to syntax analysis as described for
/// [`SemanticProvider`]. Without a provider the result is that of
/// [`register`].
///
/// # Errors
///
/// Fails with the error of the registrar when it rejects an implementation.
pub fn register_with(
    registrar: &mut dyn StageRegistrar,
    semantic: Option<Arc<dyn SemanticProvider>>,
) -> Result<()> {
    registrar.register(Arc::new(symbols::CodeSymbols::new(semantic.clone())))?;
    registrar.register(Arc::new(outline::CodeOutline::new(semantic.clone())))?;
    registrar.register(Arc::new(definition::CodeDefinition::new(semantic.clone())))?;
    registrar.register(Arc::new(references::CodeReferences::new(semantic.clone())))?;
    registrar.register(Arc::new(hover::CodeHover::new(semantic.clone())))?;
    registrar.register(Arc::new(diagnostics::CodeDiagnostics::new(
        semantic.clone(),
        None,
    )))?;
    registrar.register(Arc::new(rename::CodeRename::new(semantic.clone())))?;
    registrar.register(Arc::new(languages::CodeLanguages::new(semantic.clone())))?;
    registrar.register(Arc::new(implementations::CodeImplementations::new(
        semantic.clone(),
    )))?;
    registrar.register(Arc::new(type_definition::CodeTypeDefinition::new(
        semantic.clone(),
    )))?;
    registrar.register(Arc::new(call_hierarchy::CodeCallHierarchy::new(
        semantic.clone(),
    )))?;
    registrar.register(Arc::new(actions::CodeActions::new(semantic)))?;
    registrar.register(Arc::new(profile::CodeProfile))
}
