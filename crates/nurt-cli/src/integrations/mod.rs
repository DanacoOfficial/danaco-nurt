// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Connections to the components of other command groups.
//!
//! The crates of these components are optional dependencies, each linked by
//! the Cargo feature named in [`COMPONENTS`]; the default build links none of
//! them. A subcommand whose component is not connected ends with the error
//! "not available in this build" and exit code 3. [`unavailable`] is the only
//! source of that error. The language front end is chosen by [`frontend`].

use std::sync::Arc;

use nurt_core::{Frontend, LanguageFrontend, UnavailableFrontend};

use crate::app::CliError;

pub mod ext;
#[cfg(feature = "lsp")]
pub mod lsp;
pub mod mcp;
#[cfg(feature = "migrate")]
pub mod migrate;
pub mod serve;
#[cfg(feature = "service")]
pub mod service;

/// A subcommand implemented outside the command line crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    /// Name of the subcommand.
    pub subcommand: &'static str,
    /// Cargo feature that links the crate of the component; `None` when the
    /// component needs no optional crate.
    pub feature: Option<&'static str>,
    /// Whether this build links the crate of the component.
    pub linked: bool,
}

/// Subcommands implemented by components, in the order of the documentation.
pub const COMPONENTS: &[Component] = &[
    Component {
        subcommand: "serve",
        feature: None,
        linked: true,
    },
    Component {
        subcommand: "mcp",
        feature: Some("mcp"),
        linked: cfg!(feature = "mcp"),
    },
    Component {
        subcommand: "lsp",
        feature: Some("lsp"),
        linked: cfg!(feature = "lsp"),
    },
    Component {
        subcommand: "migrate",
        feature: Some("migrate"),
        linked: cfg!(feature = "migrate"),
    },
    Component {
        subcommand: "service",
        feature: Some("service"),
        linked: cfg!(feature = "service"),
    },
    Component {
        subcommand: "journal",
        feature: Some("journal"),
        linked: cfg!(feature = "journal"),
    },
    Component {
        subcommand: "ext",
        feature: None,
        linked: true,
    },
];

/// Subcommands whose component has no entry point for the command line.
///
/// A subcommand whose feature is off in this build is not connected either;
/// see [`is_connected`].
pub const NOT_CONNECTED: &[&str] = &["journal"];

/// Language front end of the executable.
///
/// Parses, corrects, binds, prints, describes and helps through the language
/// crates. When the embedded catalog or tables cannot be loaded, every
/// request is answered with `E_UNSUPPORTED`.
#[must_use]
pub fn frontend() -> Arc<dyn Frontend> {
    match LanguageFrontend::new() {
        Ok(frontend) => Arc::new(frontend),
        Err(_) => Arc::new(UnavailableFrontend),
    }
}

/// Whether the component of a subcommand is connected.
#[must_use]
pub fn is_connected(name: &str) -> bool {
    !NOT_CONNECTED.contains(&name) && component(name).is_none_or(|component| component.linked)
}

/// The component that implements a subcommand.
#[must_use]
pub fn component(name: &str) -> Option<&'static Component> {
    COMPONENTS
        .iter()
        .find(|component| component.subcommand == name)
}

/// The error of a subcommand, or of a form of it, that is not available in
/// this build.
#[must_use]
pub fn unavailable(what: &str) -> CliError {
    CliError::Unavailable(what.to_owned())
}

/// Runs a subcommand implemented by a component.
///
/// # Errors
///
/// Returns a usage error for invalid arguments of `serve` and `mcp` and the error
/// "not available in this build" for a component that is not connected.
pub fn run(name: &str, args: &[String]) -> Result<u8, CliError> {
    match name {
        "serve" => serve::run(args),
        "ext" => ext::run(args),
        #[cfg(feature = "lsp")]
        "lsp" => lsp::run(args),
        #[cfg(feature = "migrate")]
        "migrate" => migrate::run(args),
        #[cfg(feature = "service")]
        "service" => service::run(args),
        other => Err(unavailable(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_without_an_interface_are_not_connected() {
        assert!(!is_connected("journal"));
        assert!(is_connected("check"));
    }

    #[test]
    fn every_unconnected_subcommand_has_a_component() {
        for name in NOT_CONNECTED {
            assert!(component(name).is_some(), "{name}");
        }
        assert!(component("check").is_none());
    }

    #[test]
    fn default_build_links_no_optional_crate() {
        let linked: Vec<&str> = COMPONENTS
            .iter()
            .filter(|component| component.feature.is_some() && component.linked)
            .map(|component| component.subcommand)
            .collect();
        let expected: Vec<&str> = [
            cfg!(feature = "mcp").then_some("mcp"),
            cfg!(feature = "lsp").then_some("lsp"),
            cfg!(feature = "migrate").then_some("migrate"),
            cfg!(feature = "service").then_some("service"),
            cfg!(feature = "journal").then_some("journal"),
        ]
        .into_iter()
        .flatten()
        .collect();
        assert_eq!(linked, expected);
    }

    #[test]
    fn components_report_the_error_of_an_unavailable_subcommand() {
        let error = run("journal", &[]).unwrap_err();
        assert_eq!(error, CliError::Unavailable("journal".to_owned()));
    }
}
