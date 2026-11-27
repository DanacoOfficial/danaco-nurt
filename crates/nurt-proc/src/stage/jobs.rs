// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::stage::jobs
// Responsibility:   The `jobs`, `stop`, `retry`, `suspend` and `resume` commands and `remove`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `jobs`, `stop`, `retry`, `suspend` and `resume` commands and `remove`
//! on job records, and `stop`, `suspend` and `resume` on process records.

use std::sync::Arc;

use nurt_contracts::stage::StageImpl;

use crate::iface::Components;

mod args;
mod control;
mod list;
mod process;
mod remove;
mod retry;

pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    with_components(&crate::api::components())
}

/// Stages working with the given parts.
pub(crate) fn with_components(components: &Components) -> Vec<Arc<dyn StageImpl>> {
    vec![
        Arc::new(list::Jobs::new(components.clone())),
        Arc::new(control::Control::new(
            control::Kind::Stop,
            components.clone(),
        )),
        Arc::new(control::Control::new(
            control::Kind::Suspend,
            components.clone(),
        )),
        Arc::new(control::Control::new(
            control::Kind::Resume,
            components.clone(),
        )),
        Arc::new(retry::Retry::new(components.clone())),
        Arc::new(remove::Remove::new(components.clone())),
        Arc::new(process::ProcessControl::new(control::Kind::Stop)),
        Arc::new(process::ProcessControl::new(control::Kind::Suspend)),
        Arc::new(process::ProcessControl::new(control::Kind::Resume)),
    ]
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
