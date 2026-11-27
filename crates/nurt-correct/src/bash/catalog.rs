// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::bash::catalog
// Responsibility:   The command catalog as the translation of bash reads it: the types of the.
// Dependencies:     nurt_catalog, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The command catalog as the translation of bash reads it: the types of the
//! parameters that a long flag sets and the parameter names that mark Nurt
//! notation.

use std::sync::OnceLock;

use nurt_catalog::Registry;
use nurt_contracts::spec::{ParamType, ScalarType};

/// The embedded catalog, loaded once; `None` when it cannot be loaded.
fn registry() -> Option<&'static Registry> {
    static CATALOG: OnceLock<Option<Registry>> = OnceLock::new();
    CATALOG
        .get_or_init(|| Registry::load_embedded().ok())
        .as_ref()
}

/// Whether `param` of the command `target` is a boolean, which a long flag
/// may set without a value. A parameter that the catalog does not describe
/// is taken as a boolean, as before the catalog was consulted.
pub(super) fn is_switch(target: &str, param: &str) -> bool {
    registry()
        .and_then(|catalog| catalog.command(target))
        .and_then(|spec| spec.param(param))
        .is_none_or(|spec| matches!(spec.ty, ParamType::Scalar(ScalarType::Bool)))
}

/// Whether some command of the catalog has a parameter named `name`. Without
/// the catalog every name is taken as a parameter.
pub(super) fn is_parameter_name(name: &str) -> bool {
    registry().is_none_or(|catalog| catalog.commands().any(|spec| spec.param(name).is_some()))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
