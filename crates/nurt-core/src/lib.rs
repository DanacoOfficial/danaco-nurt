// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Engine facade of Danaco Nurt.
//!
//! [`Engine`] is the single Rust interface of the program: the C interface,
//! the protocol server, the command line interface and the integrations call
//! it. The engine owns the configuration assembled from seven levels
//! ([`config`]), the sessions of callers ([`session`]), kept pages of results
//! ([`cursor`]), correction candidates ([`candidates`]), the read cache
//! ([`readcache`]) and the state directory ([`state`]). It plans programs
//! against the command registry ([`registry`]), reaches the language through
//! [`Frontend`] and delivers execution events through [`EventListener`].

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod builtin;
pub mod candidates;
pub mod config;
pub mod cursor;
pub mod engine;
pub mod error;
pub mod events;
mod extensions;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod frontend;
mod journal_setup;
pub mod language;
pub mod paths;
pub mod readcache;
pub mod registry;
pub mod session;
pub mod state;
pub mod system;

pub use engine::{CheckReport, Engine, EngineConfig, EngineInfo, ExecHandle, Formats};
pub use error::EngineError;
pub use events::{DeliveredEvent, EventBody, EventListener, Lifecycle};
pub use frontend::{Compiled, Corrected, Frontend, UnavailableFrontend};
pub use language::LanguageFrontend;
pub use nurt_catalog::CatalogQuery;
pub use nurt_plan::CheckSummary;
