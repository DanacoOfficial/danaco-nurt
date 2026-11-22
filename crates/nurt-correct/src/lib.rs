// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Deterministic notation correction of Danaco Nurt programs.
//!
//! Correction repairs the form of a program (spelling, habits carried over from
//! other shells, misplaced punctuation) and never its intent. Every correction
//! is reported to the caller; a program that cannot be corrected produces a
//! syntax error with a ready-made fix.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
