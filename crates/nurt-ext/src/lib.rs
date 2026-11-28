// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-ext
// Responsibility:   Extension interface of Danaco Nurt.
// Dependencies:     nurt_catalog, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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
//!
//! | Module | Content |
//! |---|---|
//! | [`manifest`] | manifest `nurt-ext.toml`, API version `nurt:ext@MAJOR.MINOR` |
//! | [`locations`] | extension directory from `ext.dir` |
//! | [`store`] | installation, removal and listing of extensions |
//! | [`policy`] | paths granted to an extension for one execution |
//! | [`wire`] | JSON form of records and arguments |
//! | [`stage`] | commands of extensions as stage implementations |
//! | [`register`] | registration of extension commands in the registry |
//! | `wasm` | WebAssembly host (feature `wasm`), interface `wit/nurt-ext.wit` |
//! | `native` | native libraries (feature `native`) |
//!
//! Loading the installed extensions into a registry:
//!
//! ```no_run
//! use std::sync::Arc;
//! use nurt_catalog::Registry;
//! use nurt_ext::locations::ExtLocations;
//! use nurt_ext::register::{core_names, register};
//! use nurt_ext::store::ExtStore;
//! use nurt_ext::wasm::WasmHost;
//!
//! let mut registry = Registry::load_embedded()?;
//! let store = ExtStore::new(ExtLocations::new("/etc/nurt/ext"), core_names(&registry));
//! let host = WasmHost::new()?;
//! for extension in store.installed()? {
//!     let runtime = Arc::new(host.load(&extension)?);
//!     register(&mut registry, &extension, runtime)?;
//! }
//! # Ok::<(), nurt_contracts::error::Error>(())
//! ```

#![deny(missing_docs)]
#![cfg_attr(not(feature = "native"), forbid(unsafe_code))]

mod error;
pub mod locations;
pub mod manifest;
#[cfg(feature = "native")]
pub mod native;
pub mod policy;
pub mod register;
pub mod stage;
pub mod store;
#[cfg(feature = "wasm")]
pub mod wasm;
pub mod wire;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
