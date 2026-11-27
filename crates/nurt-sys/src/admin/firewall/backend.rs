// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Firewall backends: the interface and the choice of the backend.

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::model::Rule;
use crate::admin::{Call, Tool};

/// A rule found in the firewall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The rule when it has the supported shape.
    pub rule: Option<Rule>,
    /// Rule as the firewall prints it.
    pub raw: String,
    /// Place of the rule in the firewall (table and chain, zone), for
    /// records.
    pub place: String,
    /// The rule was created by Nurt (carries the Nurt label).
    pub managed: bool,
}

/// A firewall that Nurt can read and change.
pub trait Backend {
    /// Name of the backend as given in the parameter `backend`.
    fn name(&self) -> &'static str;

    /// Fills the fields of `rule` that the backend resolves itself, for
    /// example the default zone of firewalld.
    ///
    /// # Errors
    ///
    /// `E_UNSUPPORTED` for a field the backend cannot express and the error
    /// of the program that failed.
    fn normalize(&self, _tool: &Tool<'_>, _rule: &mut Rule) -> Result<()> {
        Ok(())
    }

    /// Whether `listed` is a rule that this backend manages; rules of other
    /// places (tables, for example) are listed but never changed.
    fn owns(&self, _listed: &Listed) -> bool {
        true
    }

    /// Rules of the firewall.
    ///
    /// # Errors
    ///
    /// The error of the program that failed.
    fn list(&self, tool: &Tool<'_>) -> Result<Vec<Listed>>;

    /// Adds `rule`.
    ///
    /// # Errors
    ///
    /// The error of the program that failed.
    fn add(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()>;

    /// Removes `rule`.
    ///
    /// # Errors
    ///
    /// The error of the program that failed.
    fn remove(&self, tool: &Tool<'_>, rule: &Rule) -> Result<()>;
}

/// Names accepted by the parameter `backend`, `auto` first.
pub const NAMES: [&str; 6] = ["auto", "nftables", "firewalld", "ufw", "pf", "windows"];

/// Backend named `name` for the system of `tool`; `auto` chooses the first
/// firewall that is installed and running.
///
/// # Errors
///
/// `E_OS_ONLY` when the backend does not exist on the system and
/// `E_EXTERNAL_MISSING` when no firewall program is found.
pub fn select(tool: &Tool<'_>, name: &str, table: &str) -> Result<Box<dyn Backend>> {
    let os = tool.os();
    let known = |wanted: &str, systems: &[Os]| -> Result<()> {
        if systems.contains(&os) {
            Ok(())
        } else {
            Err(Error::new(
                ErrorCode::OsOnly,
                format!("the firewall backend `{wanted}` is not available on this system"),
            )
            .with_detail("backend", wanted.into()))
        }
    };
    match name {
        "nftables" => {
            known("nftables", &[Os::Linux])?;
            Ok(Box::new(super::nftables::Nftables::new(table)))
        }
        "firewalld" => {
            known("firewalld", &[Os::Linux])?;
            Ok(Box::new(super::firewalld::Firewalld))
        }
        "ufw" => {
            known("ufw", &[Os::Linux])?;
            Ok(Box::new(super::ufw::Ufw))
        }
        "pf" => {
            known("pf", &[Os::Macos])?;
            Ok(Box::new(super::pf::Pf))
        }
        "windows" => {
            known("windows", &[Os::Windows])?;
            Ok(Box::new(super::windows::WindowsFirewall))
        }
        _ => auto(tool, table),
    }
}

fn auto(tool: &Tool<'_>, table: &str) -> Result<Box<dyn Backend>> {
    match tool.os() {
        Os::Windows => Ok(Box::new(super::windows::WindowsFirewall)),
        Os::Macos => {
            if tool.available("pfctl") {
                Ok(Box::new(super::pf::Pf))
            } else {
                Err(missing("pfctl"))
            }
        }
        Os::Linux => {
            if tool.available("firewall-cmd")
                && tool
                    .run(&Call::new("firewall-cmd", &["--state"]))
                    .is_ok_and(|output| output.success())
            {
                return Ok(Box::new(super::firewalld::Firewalld));
            }
            if tool.available("ufw") {
                return Ok(Box::new(super::ufw::Ufw));
            }
            if tool.available("nft") {
                return Ok(Box::new(super::nftables::Nftables::new(table)));
            }
            Err(missing("nft, firewall-cmd or ufw"))
        }
    }
}

fn missing(program: &str) -> Error {
    Error::new(
        ErrorCode::ExternalMissing,
        format!("no firewall program found ({program})"),
    )
}
