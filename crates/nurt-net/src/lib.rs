// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Network operations for Danaco Nurt.
//!
//! The crate performs HTTP requests and downloads with TLS from the system
//! certificate store, DNS resolution and network diagnostics. Unsafe code is
//! limited to operating system interfaces for ICMP and network interfaces.

// Tests that write POSIX paths run on Unix only; their helpers are unused on Windows.
#![cfg_attr(all(test, windows), allow(dead_code, unused_imports))]
#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod cache;
pub mod check;
pub mod client;
pub mod dns;
pub mod html;
pub mod http;
pub mod interfaces;
pub mod ping;

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::stage::{StageImpl, StageRegistrar};

/// Adds the `http.*` and `network.*` stages of the host to `registrar`.
///
/// # Errors
///
/// Fails with the first error of the registrar, such as `E_UNKNOWN_COMMAND`
/// when a command is not in the catalog of the registry.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::stage::StageImpl;
///
/// let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
/// nurt_net::register(&mut stages)?;
/// assert_eq!(stages.len(), 5);
/// assert_eq!(stages[0].descriptor().command, "http.request");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    let stages: [Arc<dyn StageImpl>; 5] = [
        Arc::new(http::request::HttpRequest),
        Arc::new(http::download::Download::new()),
        Arc::new(ping::Ping::new()),
        Arc::new(dns::Resolve::new()),
        Arc::new(interfaces::Interfaces::new()),
    ];
    stages
        .into_iter()
        .try_for_each(|stage| registrar.register(stage))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_contracts::stage::StageImpl;

    #[test]
    fn registers_one_stage_per_catalog_command() {
        let path = format!(
            "{}/../../catalog/network.toml",
            std::env::var("CARGO_MANIFEST_DIR")
                .as_deref()
                .unwrap_or(env!("CARGO_MANIFEST_DIR"))
        );
        let catalog: toml::Table = std::fs::read_to_string(path).unwrap().parse().unwrap();
        let mut expected: Vec<String> = catalog["command"]
            .as_array()
            .unwrap()
            .iter()
            .map(|command| command["name"].as_str().unwrap().to_owned())
            .collect();
        expected.sort_unstable();

        let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
        super::register(&mut stages).unwrap();
        let mut names: Vec<String> = stages
            .iter()
            .map(|stage| stage.descriptor().command)
            .collect();
        names.sort_unstable();
        assert_eq!(names, expected);
    }
}
