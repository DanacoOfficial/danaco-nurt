// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface between the service host and the modules it runs.
//!
//! The interface is defined by the schedule crate, which provides the
//! schedule module, and is re-exported here for hosts and module authors.
//! The host runs every module on its own thread and supervises it: a module
//! that panics, returns from [`ServiceModule::start`] before it was asked to
//! stop, or reports [`ModuleHealth::Failed`] is restarted with a growing
//! delay, and a failure of one module never stops another.
//!
//! Requests whose operation starts with `schedule.` or `schedule/` are
//! forwarded to the module named [`SCHEDULE_MODULE`] with
//! [`ModuleRequest::client`] set to the identifier of the connection. Events
//! published through [`ServiceCtx::events`] reach the subscribed connections
//! ([`EventTarget::All`]) or one connection ([`EventTarget::Client`]).

pub use nurt_sched::module::{
    EventTarget, ModuleEvent, ModuleEvents, ModuleHealth, ModuleRequest, SCHEDULE_MODULE,
    ServiceCtx, ServiceModule,
};

/// Name of the module that keeps free space on journal volumes.
pub const SPACE_MODULE: &str = "space";
