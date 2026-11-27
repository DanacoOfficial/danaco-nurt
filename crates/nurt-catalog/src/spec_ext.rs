// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-catalog::spec_ext
// Responsibility:   Derived queries on a command specification.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use nurt_contracts::spec::{ActionEffect, DefaultValue, IoSpec, Os, ParamSpec, StageSpec};

/// Derived queries on a command specification.
///
/// The positional parameters, the exclusive groups and the forms of a command
/// are available as methods of [`StageSpec`] itself; this trait adds the
/// queries that binding and planning need on top of them.
///
/// # Examples
///
/// ```
/// use nurt_catalog::{Registry, StageSpecExt};
/// use nurt_contracts::spec::Os;
///
/// let registry = Registry::load_embedded()?;
/// let sort = registry.command("sort").expect("core command");
/// assert!(sort.available_on(Os::Linux));
/// assert!(!sort.is_source());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub trait StageSpecExt {
    /// Returns the parameters that are required in the primary form, in
    /// declaration order.
    fn required_params(&self) -> Vec<&ParamSpec>;

    /// Returns the parameters that have a literal default, with the default.
    fn explicit_defaults(&self) -> Vec<(&ParamSpec, &DefaultValue)>;

    /// Returns `true` when the command can start a pipeline.
    fn is_source(&self) -> bool;

    /// Returns `true` when the command is available on the operating system;
    /// a command without an `os` list is available everywhere.
    fn available_on(&self, os: Os) -> bool;

    /// Returns the effect of the named action when it differs from the
    /// default effect of the command.
    fn effect_for(&self, action: &str) -> Option<&ActionEffect>;
}

impl StageSpecExt for StageSpec {
    fn required_params(&self) -> Vec<&ParamSpec> {
        self.params.iter().filter(|param| param.required).collect()
    }

    fn explicit_defaults(&self) -> Vec<(&ParamSpec, &DefaultValue)> {
        self.params
            .iter()
            .filter_map(|param| param.default.as_ref().map(|default| (param, default)))
            .collect()
    }

    fn is_source(&self) -> bool {
        self.input.contains(&IoSpec::None)
    }

    fn available_on(&self, os: Os) -> bool {
        self.os.is_empty() || self.os.contains(&os)
    }

    fn effect_for(&self, action: &str) -> Option<&ActionEffect> {
        self.effects.iter().find(|effect| effect.action == action)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
