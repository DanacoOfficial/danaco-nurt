// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Connections to the components of other command groups.
//!
//! The crates of these components are optional dependencies, each linked by
//! the Cargo feature named in [`COMPONENTS`]; the default build links all of
//! them. A subcommand whose component is not connected ends with the error
//! "not available in this build" and exit code 3. [`unavailable`] is the only
//! source of that error. The language front end is chosen by [`frontend`].

use std::sync::Arc;

use nurt_core::{Frontend, LanguageFrontend, UnavailableFrontend};

use crate::app::CliError;

pub mod ext;
#[cfg(feature = "journal")]
pub mod journal;
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
pub const NOT_CONNECTED: &[&str] = &[];

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
        #[cfg(feature = "journal")]
        "journal" => journal::run(args),
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
    fn connected_subcommands_are_reported_as_connected() {
        assert!(NOT_CONNECTED.is_empty());
        assert!(is_connected("journal"));
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
    fn build_links_the_optional_crates_of_its_features() {
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
    fn components_are_listed_in_the_order_of_the_documentation() {
        let names: Vec<&str> = COMPONENTS
            .iter()
            .map(|component| component.subcommand)
            .collect();
        assert_eq!(
            names,
            [
                "serve", "mcp", "lsp", "migrate", "service", "journal", "ext"
            ]
        );
        assert_eq!(component("mcp").and_then(|c| c.feature), Some("mcp"));
        assert_eq!(component("serve").and_then(|c| c.feature), None);
        assert!(component("doctor").is_none());
    }

    #[test]
    fn components_without_an_optional_crate_are_always_connected() {
        for name in ["serve", "ext", "exec", "doctor"] {
            assert!(is_connected(name), "{name}");
        }
        for component in COMPONENTS {
            assert_eq!(
                is_connected(component.subcommand),
                component.linked,
                "{}",
                component.subcommand
            );
        }
    }

    #[test]
    fn components_with_a_wrong_form_report_a_usage_error() {
        assert!(matches!(run("serve", &[]), Err(CliError::Usage(_))));
        assert!(matches!(run("ext", &[]), Err(CliError::Usage(_))));
        assert!(matches!(
            run("serve", &["--tcp".to_owned()]),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn unavailable_names_what_is_not_available() {
        assert_eq!(
            unavailable("mcp --http"),
            CliError::Unavailable("mcp --http".to_owned())
        );
    }

    #[test]
    fn names_without_a_component_are_unavailable() {
        let error = run("unknown", &[]).unwrap_err();
        assert_eq!(error, CliError::Unavailable("unknown".to_owned()));
    }
}
