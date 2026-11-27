// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin
// Responsibility:   Native administration commands: accounts, groups, mounts, kernel.
// Dependencies:     inverse, nurt_contracts, stage, tool
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Native administration commands: accounts, groups, mounts, kernel
//! parameters and modules, power and disks.
//!
//! | Group | Commands | Inverse |
//! |---|---|---|
//! | accounts | `accounts.create`, `accounts.modify`, `accounts.remove`, `accounts.password` | create and remove undo each other; modify restores the recorded values; a password is irreversible |
//! | groups | `groups.create`, `groups.remove`, `groups.add_member`, `groups.remove_member` | pairwise |
//! | mounts | `mount`, `unmount` | pairwise |
//! | kernel | `sysctl.set`, `modules.load`, `modules.unload` | the previous value; load and unload undo each other |
//! | power | `power.restart`, `power.shutdown`, `power.cancel` | a scheduled restart or shutdown is undone by `power.cancel`; an immediate one is irreversible |
//! | disks | `disks.partition`, `disks.format` | irreversible; the preview lists the data that is lost |
//! | firewall | `firewall.rules`, `firewall.allow`, `firewall.deny`, `firewall.remove` | the opposite rule change in the same backend |
//! | network | `network.route_add`, `network.route_remove`, `network.dns_set` | the opposite route command, the previous DNS state |
//! | clock | `clock.timezone_set`, `clock.set`, `clock.ntp` | the previous value; `clock.set` is irreversible |
//!
//! The first six groups keep their states as JSON documents ([`inverse`]);
//! the firewall, network and clock commands record `state` changes of the
//! targets `firewall:<rule>`, `route:<destination>`, `dns:<interface>`,
//! `clock:timezone` and `clock:ntp`, and [`undo`] derives their inverses.
//!
//! Every command reads the state it changes, plans the programs that change
//! it, and reports the state before and after as the `before` and `after` of
//! a change (`account:alice`, `group:ops`, `mount:/srv`, `sysctl:key`,
//! `module:name`, `power:schedule`, `disk:/dev/sdb`). The states are JSON
//! documents, so that [`inverse`] derives the program that undoes the change
//! from the report alone. Files that a command edits (`/etc/fstab`,
//! `/etc/sysctl.d/90-nurt.conf`, the home directory of a removed account) go
//! through the journal like every other file.
//!
//! Rights are not handled here: the executor runs a call with
//! `privilege: elevated` in the elevated process, and a program that fails for
//! lack of rights ends the command with `E_PERMISSION`, which `privilege: auto`
//! repeats once elevated.

pub mod clock;
pub(crate) mod effects;
pub mod firewall;
pub mod network;
#[cfg(test)]
pub(crate) mod testing;
mod tool;
pub mod undo;

mod accounts;
mod args;
mod disks;
mod groups;
mod inverse;
mod kernel;
mod mounts;
mod power;
mod stage;
mod sys;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod testsys;

pub use inverse::{Inverse, inverse};
pub use stage::AdminCommand;
pub use tool::{Call, Output, System, Tool};

/// Every stage of the administration commands, in catalog order: accounts,
/// groups, mounts, kernel, power and disks, then firewall, network and clock.
#[must_use]
pub fn stages() -> Vec<std::sync::Arc<dyn nurt_contracts::stage::StageImpl>> {
    let mut stages = stage::stages();
    stages.extend(firewall::stages());
    stages.extend(network::stages());
    stages.extend(clock::stages());
    stages
}

use nurt_contracts::journal::FileOp;

/// Change of the state of one target that is not a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StateChange {
    /// Target such as `account:alice`.
    pub target: String,
    /// State before, `None` when the target did not exist.
    pub before: Option<String>,
    /// State after, `None` when the target does not exist afterwards.
    pub after: Option<String>,
}

impl StateChange {
    /// A change of `target` from `before` to `after`.
    pub(crate) fn new(
        target: impl Into<String>,
        before: Option<String>,
        after: Option<String>,
    ) -> Self {
        Self {
            target: target.into(),
            before,
            after,
        }
    }
}

/// What a command will do: the programs to run, the files to edit and the
/// states that result.
#[derive(Debug, Clone, Default)]
pub(crate) struct Plan {
    /// Programs, run in order after the files are preserved.
    pub calls: Vec<sys::Call>,
    /// Edits of files, applied after the programs.
    pub files: Vec<FileOp>,
    /// States that change.
    pub changes: Vec<StateChange>,
    /// The reason when the change cannot be undone.
    pub irreversible: Option<String>,
    /// The system already has the requested state; nothing runs.
    pub unchanged: Option<String>,
    /// Texts that describe the effect of the change for a preview, in
    /// addition to the changes.
    pub notes: Vec<String>,
}

/// Plans the command of `args` on `system`.
///
/// # Errors
///
/// Fails with the error of the area of the command.
pub(crate) fn plan(
    system: &dyn sys::Sys,
    args: &nurt_contracts::stage::BoundArgs,
) -> nurt_contracts::error::Result<Plan> {
    let command = args.command();
    match command {
        c if c.starts_with("accounts.") => accounts::plan(system, args),
        c if c.starts_with("groups.") => groups::plan(system, args),
        "mount" | "unmount" => mounts::plan(system, args),
        "sysctl.set" | "modules.load" | "modules.unload" => kernel::plan(system, args),
        c if c.starts_with("power.") => power::plan(system, args),
        c if c.starts_with("disks.") => disks::plan(system, args),
        other => Err(nurt_contracts::error::Error::new(
            nurt_contracts::error::ErrorCode::UnknownCommand,
            format!("`{other}` is not an administration command"),
        )),
    }
}

/// The absolute path of a system file of Linux. The tests run on every
/// system, so they take the path with the drive that the system needs.
pub(crate) fn system_file(
    path: &str,
) -> nurt_contracts::error::Result<nurt_contracts::fs::AbsPath> {
    #[cfg(test)]
    {
        Ok(nurt_contracts::fs::AbsPath::for_test(path))
    }
    #[cfg(not(test))]
    {
        nurt_contracts::fs::AbsPath::new(path).map_err(|error| {
            nurt_contracts::error::Error::new(
                nurt_contracts::error::ErrorCode::Io,
                error.to_string(),
            )
        })
    }
}

/// Names of the administration commands, in catalog order.
pub(crate) const COMMANDS: [&str; 18] = [
    "accounts.create",
    "accounts.modify",
    "accounts.remove",
    "accounts.password",
    "groups.create",
    "groups.remove",
    "groups.add_member",
    "groups.remove_member",
    "mount",
    "unmount",
    "sysctl.set",
    "modules.load",
    "modules.unload",
    "power.restart",
    "power.shutdown",
    "power.cancel",
    "disks.partition",
    "disks.format",
];

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
