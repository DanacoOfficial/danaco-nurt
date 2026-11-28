// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-core::extensions
// Responsibility:   Installed extensions in the command registry of the process.
// Dependencies:     nurt_catalog, nurt_contracts, nurt_ext
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
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
/// An extension that cannot be loaded or registered is left out. The same holds
/// when the extension directory cannot be listed or the WebAssembly host cannot
/// start: the commands of the program do not depend on extensions, so the
/// engine opens without them.
///
/// # Errors
///
/// Never fails; the result type is kept for the caller.
#[allow(
    clippy::unnecessary_wraps,
    reason = "the caller of the registry assembly propagates errors with `?`"
)]
pub(crate) fn register_installed(
    registry: &mut Registry,
    locations: ExtLocations,
) -> Result<(), Error> {
    let store = ExtStore::new(locations, core_names(registry));
    let Ok(listed) = store.installed() else {
        return Ok(());
    };
    let installed: Vec<_> = listed
        .into_iter()
        .filter(|extension| extension.manifest.ext.kind == ExtKind::Wasm)
        .collect();
    if installed.is_empty() {
        return Ok(());
    }
    let Ok(host) = WasmHost::new() else {
        return Ok(());
    };
    for extension in installed {
        if let Ok(runtime) = host.load(&extension) {
            let _ = register(registry, &extension, Arc::new(runtime));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nurt_catalog::Registry;
    use nurt_ext::locations::ExtLocations;
    use nurt_ext::register::core_names;
    use nurt_ext::store::ExtStore;

    use super::register_installed;

    #[test]
    fn an_extension_directory_that_cannot_be_listed_leaves_the_registry_as_it_was() {
        let temp = tempfile::tempdir().unwrap();
        let not_a_directory = temp.path().join("ext");
        std::fs::write(&not_a_directory, "x").unwrap();
        let mut registry = Registry::load_embedded().unwrap();
        let before = core_names(&registry);
        let store = ExtStore::new(ExtLocations::new(not_a_directory.clone()), before.clone());
        assert!(store.installed().is_err());
        register_installed(&mut registry, ExtLocations::new(not_a_directory)).unwrap();
        assert_eq!(core_names(&registry), before);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
