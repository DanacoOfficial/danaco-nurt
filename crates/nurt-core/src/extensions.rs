// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-core::extensions
// Responsibility:   Installed extensions in the command registry of the process.
// Dependencies:     nurt_catalog, nurt_contracts, nurt_ext
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Installed extensions in the command registry of the process.

use std::path::Path;
use std::sync::Arc;

use nurt_catalog::Registry;
use nurt_contracts::config::ExtConfig;
use nurt_contracts::env::Host;
use nurt_contracts::error::Error;
use nurt_ext::locations::ExtLocations;
use nurt_ext::manifest::ExtKind;
use nurt_ext::register::{core_names, register};
use nurt_ext::store::ExtStore;
use nurt_ext::wasm::WasmHost;

use crate::paths::PlatformPaths;

/// Directory of the installed extensions of the account, or `None` when the
/// account has no configuration directory.
///
/// # Errors
///
/// Returns `E_CONFIG` when `ext.dir` is empty.
pub(crate) fn locations(
    host: &dyn Host,
    config: &ExtConfig,
) -> Result<Option<ExtLocations>, Error> {
    let paths = PlatformPaths::for_host(host);
    let Some(config_dir) = paths.user_file.as_deref().and_then(Path::parent) else {
        return Ok(None);
    };
    ExtLocations::from_config(config, config_dir).map(Some)
}

/// Registers the WebAssembly extensions installed in `locations`.
///
/// An extension that cannot be loaded or registered is left out.
///
/// # Errors
///
/// Returns the error of listing the extension directory or of starting the
/// WebAssembly host.
pub(crate) fn register_installed(
    registry: &mut Registry,
    locations: ExtLocations,
) -> Result<(), Error> {
    let store = ExtStore::new(locations, core_names(registry));
    let installed: Vec<_> = store
        .installed()?
        .into_iter()
        .filter(|extension| extension.manifest.ext.kind == ExtKind::Wasm)
        .collect();
    if installed.is_empty() {
        return Ok(());
    }
    let host = WasmHost::new()?;
    for extension in installed {
        if let Ok(runtime) = host.load(&extension) {
            let _ = register(registry, &extension, Arc::new(runtime));
        }
    }
    Ok(())
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
