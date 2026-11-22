// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Source code analysis for Danaco Nurt.
//!
//! Syntax analysis uses built-in tree-sitter grammars: symbols, outlines,
//! scope-based definitions and references, syntax diagnostics and structural
//! patterns with metavariables. Semantic analysis is delegated to a
//! [`semantic::SemanticProvider`] backed by language servers.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod commands;
pub mod enclosing;
pub mod grammars;
pub mod index;
pub mod local;
pub mod outline;
pub mod pattern;
pub mod profile;
pub mod registry;
pub mod semantic;
pub mod service;
pub mod spans;
pub mod structural;
pub mod symbols;
pub mod syntax_check;
