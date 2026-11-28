// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::analyze
// Responsibility:   Analyzer and formatter detection, commands, output parsers and verdict.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Analyzer and formatter detection, commands, output parsers and verdict.

pub(crate) mod analyzers;
pub(crate) mod check;
pub(crate) mod execute;
pub(crate) mod format;
pub(crate) mod output;
pub(crate) mod scope;
pub(crate) mod tools;
pub(crate) mod verdict;

#[cfg(test)]
mod tests;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
