// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Command registry of the engine.
//!
//! The registry holds the command catalog embedded in the build and the
//! stage implementations of the commands. [`standard_registry`] is the single
//! place where implementations are registered with the catalog; the planner,
//! the check without effect and the catalog queries of the engine all read
//! the registry built here.

use std::sync::Arc;

use nurt_catalog::{Registry, descriptors_overlap};
use nurt_code::semantic::SemanticProvider;
use nurt_contracts::error::{Error, Result};
use nurt_contracts::stage::{StageDescriptor, StageImpl, StageRegistrar};
use nurt_ext::locations::ExtLocations;

/// Builds the registry of this build: the embedded command catalog and the
/// stage implementations compiled into the build.
///
/// Every command group that provides stage implementations registers them
/// in this function through the registration trait `StageRegistrar` of the
/// stage contract. The record stages (`filter`, `select`, `sort`, `first`,
/// `last`, `skip`, `count`, `assert`, `verify`, `group`, `summarize`,
/// `distinct`, `parse`, `split`) are registered here.
///
/// # Errors
///
/// Returns `E_CATALOG` when the embedded catalog is inconsistent or when two
/// implementations of one command accept the same record kind.
///
/// # Examples
///
/// ```
/// let registry = nurt_core::registry::standard_registry()?;
/// assert!(registry.command("filter").is_some());
/// assert!(registry.implementations("filter").next().is_some());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn standard_registry() -> Result<Registry, Error> {
    standard_registry_with(&[])
}

/// Builds the registry of this build with further stage implementations. A
/// further implementation replaces the compiled-in one that accepts the same
/// input.
///
/// # Errors
///
/// Returns the errors of [`standard_registry`], and `E_CATALOG` when an
/// implementation names a command that is not in the catalog or overlaps
/// another implementation of its command.
pub fn standard_registry_with(extra: &[Arc<dyn StageImpl>]) -> Result<Registry, Error> {
    registry_with(extra, ProcessComponents::default())
}

/// Components that only a running process provides.
#[derive(Default)]
pub(crate) struct ProcessComponents {
    /// Directory of the installed extensions.
    pub(crate) extensions: Option<ExtLocations>,
    /// Provider of semantic code analysis backed by language servers.
    pub(crate) semantic: Option<Arc<dyn SemanticProvider>>,
}

/// Registrar that adds the compiled-in stages to a registry and leaves out
/// the ones that an injected implementation replaces.
struct Precedence<'a> {
    registry: &'a mut Registry,
    injected: Vec<StageDescriptor>,
}

impl StageRegistrar for Precedence<'_> {
    fn register(&mut self, stage: Arc<dyn StageImpl>) -> Result<()> {
        let descriptor = stage.descriptor();
        if self
            .injected
            .iter()
            .any(|injected| descriptors_overlap(injected, &descriptor))
        {
            return Ok(());
        }
        self.registry.register_impl(stage)
    }
}

/// Builds the registry of this build with the components of the process: the
/// code stages use the semantic provider, and the commands of the installed
/// extensions are added.
///
/// An injected implementation takes the place of a compiled-in implementation
/// of the same command that accepts the same input. Two injected
/// implementations that accept the same input are rejected.
///
/// An extension that cannot be loaded or registered is left out, so that the
/// other commands stay available.
pub(crate) fn registry_with(
    extra: &[Arc<dyn StageImpl>],
    components: ProcessComponents,
) -> Result<Registry, Error> {
    let mut registry = Registry::load_embedded()?;
    let mut compiled = Precedence {
        registry: &mut registry,
        injected: extra.iter().map(|stage| stage.descriptor()).collect(),
    };
    nurt_exec::stages::register(&mut compiled)?;
    nurt_journal::register(&mut compiled)?;
    nurt_files::register(&mut compiled)?;
    nurt_doc::register(&mut compiled)?;
    nurt_code::register_with(&mut compiled, components.semantic)?;
    nurt_proc::register(&mut compiled)?;
    nurt_verify::register(&mut compiled)?;
    nurt_syslog::register(&mut compiled)?;
    nurt_git::register(&mut compiled)?;
    nurt_git_ops::register(&mut compiled)?;
    nurt_sys::register(&mut compiled)?;
    nurt_net::register(&mut compiled)?;
    nurt_sched::register(&mut compiled)?;
    nurt_programs::register(&mut compiled)?;
    crate::builtin::register(&mut compiled)?;
    for stage in extra {
        registry.register_impl(Arc::clone(stage))?;
    }
    if let Some(locations) = components.extensions {
        crate::extensions::register_installed(&mut registry, locations)?;
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_stages_are_registered_with_the_catalog() {
        let registry = standard_registry().unwrap();
        for command in nurt_exec::stages::COMMANDS {
            assert!(registry.command(command).is_some(), "{command}");
            assert!(
                registry.implementations(command).next().is_some(),
                "{command}"
            );
        }
        assert!(registry.validate_implementations().is_empty());
    }

    #[test]
    fn every_command_of_the_catalog_has_an_implementation() {
        let registry = standard_registry().unwrap();
        let missing: Vec<String> = registry
            .commands()
            .filter(|command| registry.implementations(&command.name).next().is_none())
            .map(|command| command.name.clone())
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
    }
}
