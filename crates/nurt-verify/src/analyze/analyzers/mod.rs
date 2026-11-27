// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::analyze::analyzers
// Responsibility:   Parsers of the output of analyzers and formatters.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parsers of the output of analyzers and formatters.

pub(crate) mod biome;
pub(crate) mod clippy;
pub(crate) mod eslint;
pub(crate) mod gofmt;
pub(crate) mod govet;
pub(crate) mod mypy;
pub(crate) mod prettier;
pub(crate) mod ruff;
pub(crate) mod rustfmt;
pub(crate) mod tsc;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
