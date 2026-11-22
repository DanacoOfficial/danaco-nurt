// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Binding of Danaco Nurt syntax trees to the command catalog.
//!
//! Binding turns a `nurt-ast/1` tree into a `nurt-bound/1` tree: positional
//! parameters are named, defaults are made explicit, literals are converted to
//! the type of their position, source forms are expanded, the single changing
//! stage rule is enforced and the effect of every stage is classified.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
