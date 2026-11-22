// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Command catalog of Danaco Nurt.
//!
//! The catalog is the registry of every command known to the engine. It loads
//! the command signatures from the TOML catalog data files, validates them
//! against the catalog contract, resolves command names and group membership,
//! and exposes the stage specifications used by binding, planning and help.
//!
//! A [`Registry`] holds two tables. The catalog table maps command names to
//! [`StageSpec`](nurt_contracts::spec::StageSpec) values; command names and
//! group names live in separate namespaces ([`NameKind`]), so `verify` is both
//! a command and a group. The implementation table maps the pair of a command
//! and an input record kind to a [`StageImpl`](nurt_contracts::stage::StageImpl).
//!
//! # Examples
//!
//! ```
//! use nurt_catalog::Registry;
//! use nurt_contracts::kinds::EffectKind;
//!
//! let registry = Registry::load_embedded()?;
//! let remove = registry.command("remove").expect("remove is a core command");
//! assert_eq!(remove.effect, EffectKind::Local);
//! assert_eq!(registry.commands().count(), 148);
//! # Ok::<(), nurt_contracts::error::Error>(())
//! ```

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod error;
mod load;
mod output;
mod registry;
mod spec_ext;
mod stages;

pub use output::{CATALOG_RESULT_FORMAT, CatalogQuery};
pub use registry::{Group, GroupOrigin, NameEntry, NameKind, Registry};
pub use spec_ext::StageSpecExt;
pub use stages::AcceptedKinds;
