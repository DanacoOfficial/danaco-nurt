// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Information about the library returned by `nurt_info`.

use std::sync::OnceLock;

use nurt_catalog::Registry;
use nurt_contracts::{ast, bound, envelope, report, schema};
use serde_json::{Value, json};

use crate::abi::{NURT_ABI_MAJOR, NURT_ABI_MINOR, NURT_ABI_VERSION};

/// Cargo features compiled into the library.
const FEATURES: &[&str] = if cfg!(feature = "stub") {
    &["stub"]
} else {
    &[]
};

/// Revision of the command catalog built into the library; computed once.
/// `None` when the built-in catalog cannot be loaded.
fn catalog_rev() -> Option<&'static str> {
    static REVISION: OnceLock<Option<String>> = OnceLock::new();
    REVISION
        .get_or_init(|| {
            Registry::load_embedded()
                .and_then(|registry| registry.catalog_rev())
                .ok()
        })
        .as_deref()
}

/// Information about the library as JSON.
///
/// `catalog_rev` is the revision of the built-in command catalog, or `null`
/// when the catalog cannot be loaded.
pub(crate) fn to_json() -> Vec<u8> {
    let schemas: Vec<&str> = schema::ALL.iter().map(|(name, _)| *name).collect();
    let endian = if cfg!(target_endian = "little") {
        "little"
    } else {
        "big"
    };
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let info = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "abi": {
            "major": NURT_ABI_MAJOR,
            "minor": NURT_ABI_MINOR,
            "version": NURT_ABI_VERSION,
        },
        "formats": {
            "ast": ast::AST_VERSION,
            "bound": bound::BOUND_VERSION,
            "report": report::REPORT_VERSION,
            "request": envelope::ENVELOPE_VERSION,
            "response": envelope::ENVELOPE_VERSION,
        },
        "schemas": schemas,
        "catalog_rev": catalog_rev().map_or(Value::Null, Value::from),
        "features": FEATURES,
        "target": {
            "arch": std::env::consts::ARCH,
            "os": std::env::consts::OS,
            "family": std::env::consts::FAMILY,
            "pointer_width": usize::BITS,
            "endian": endian,
        },
        "build": { "profile": profile },
    });
    serde_json::to_vec(&info).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::to_json;

    #[test]
    fn info_names_versions_schemas_and_target() {
        let info: serde_json::Value = serde_json::from_slice(&to_json()).unwrap();
        assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(info["abi"]["version"], 0x0001_0000);
        assert_eq!(info["formats"]["report"], "1.0");
        assert!(
            info["schemas"]
                .as_array()
                .unwrap()
                .iter()
                .any(|name| name == "request")
        );
        assert_eq!(info["target"]["os"], std::env::consts::OS);
        assert_eq!(
            info["features"].as_array().unwrap().is_empty(),
            !cfg!(feature = "stub")
        );
    }

    #[test]
    fn catalog_revision_is_the_digest_of_the_built_in_catalog() {
        let info: serde_json::Value = serde_json::from_slice(&to_json()).unwrap();
        let revision = info["catalog_rev"].as_str().unwrap();
        let expected = nurt_catalog::Registry::load_embedded()
            .unwrap()
            .catalog_rev()
            .unwrap();
        assert_eq!(revision, expected);
        assert!(revision.starts_with("sha256:"));
    }
}
