// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Extension interface of Danaco Nurt.
//!
//! Extensions add commands, document formats, grammars, path providers,
//! schedule actions and classification rules without changing the program.
//! The default distribution format is a WebAssembly component hosted by
//! `wasmtime` (feature `wasm`); native libraries are available behind the
//! `native` feature and are disabled by default.
//!
//! Every extension directory holds a manifest ([`manifest::MANIFEST_FILE`])
//! that names the extension, the API version it requires
//! (`nurt:ext@MAJOR.MINOR`), its commands, formats and grammars and the
//! paths and hosts it may reach ([`manifest::Manifest`]).

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod manifest;
