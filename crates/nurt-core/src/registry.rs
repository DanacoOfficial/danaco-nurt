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

use nurt_catalog::Registry;
use nurt_contracts::error::Error;

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
    let mut registry = Registry::load_embedded()?;
    nurt_exec::stages::register(&mut registry)?;
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
}
