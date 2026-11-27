// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct
// Responsibility:   Deterministic notation correction of Danaco Nurt programs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Deterministic notation correction of Danaco Nurt programs.
//!
//! Correction repairs the form of a program (spelling, habits carried over from
//! other shells, misplaced punctuation) and never its intent. Every correction
//! is reported to the caller; a program that cannot be corrected produces a
//! syntax error with a ready-made fix.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod bash;
pub mod candidates;
pub mod distance;
pub mod lexical;
pub mod normalize;
pub mod policy;
pub mod powershell;
pub mod record;
pub mod tables;
pub mod types;
pub mod values;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
