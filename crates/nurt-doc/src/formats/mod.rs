// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats
// Responsibility:   Supported document formats.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Supported document formats.

pub mod csv;
pub mod env;
pub mod ini;
pub mod json;
pub mod plist;
pub mod properties;
pub mod toml;
pub mod xml;
pub mod yaml;

/// Registration functions of the built-in formats, in detection priority
/// order; [`crate::registry::Registry::builtin`] registers their formats.
pub(crate) const BUILTIN: &[crate::registry::FormatsFn] = &[
    json::formats,
    toml::formats,
    ini::formats,
    properties::formats,
    env::formats,
    csv::formats,
    plist::formats,
    yaml::formats,
    xml::formats,
];

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
