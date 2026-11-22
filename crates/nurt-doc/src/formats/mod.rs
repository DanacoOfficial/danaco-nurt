// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Supported document formats.

pub mod json;

/// Registration functions of the built-in formats, in detection priority
/// order; [`crate::registry::Registry::builtin`] registers their formats.
pub(crate) const BUILTIN: &[crate::registry::FormatsFn] = &[json::formats];
