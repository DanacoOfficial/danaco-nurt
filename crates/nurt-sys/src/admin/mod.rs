// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Administration commands: firewall, network and clock.
//!
//! Every command of this module changes the system through the programs of
//! the operating system (`nft`, `firewall-cmd`, `ufw`, `pfctl`, `ip`,
//! `resolvectl`, `timedatectl`, `networksetup`, `netsh`, PowerShell). The
//! rights of the operation come from the call parameter `privilege`: the
//! executor runs the stage with the elevated process, the stage never starts
//! `sudo` itself.
//!
//! A change is recorded in its report as a `state` change of a target of the
//! form `firewall:<rule>`, `route:<destination>`, `dns:<interface>`,
//! `clock:timezone` or `clock:ntp` with the value before and after. The
//! operation is marked with `external_effect` so that the journal opens an
//! entry; the engine derives from the recorded values the program that
//! undoes the change (see [`inverse`]) and drops the marker when it does.
//! `clock.set` stays irreversible.

pub mod clock;
pub(crate) mod effects;
pub mod firewall;
pub mod inverse;
pub mod network;
#[cfg(test)]
pub(crate) mod testing;
mod tool;

use std::sync::Arc;

use nurt_contracts::stage::StageImpl;

pub use self::tool::{Call, Output, System, Tool};

/// Every stage of the firewall, network and clock commands, in catalog order.
#[must_use]
pub fn stages() -> Vec<Arc<dyn StageImpl>> {
    let mut stages = firewall::stages();
    stages.extend(network::stages());
    stages.extend(clock::stages());
    stages
}
