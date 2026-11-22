// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Background service of Danaco Nurt.
//!
//! The service process ([`run`]) hosts modules such as the schedule module
//! ([`ScheduleService`]) and the free space module of the journal. It holds
//! the single-instance lock `service.lock`, writes the heartbeat
//! `service.hb` and answers on a control channel: the Unix socket
//! `service.sock` in the state directory or, on Windows, a named pipe of the
//! account. The protocol is described in [`ipc::protocol`].
//!
//! [`install`] registers the service with systemd, launchd, the Windows
//! service control manager or the Windows Task Scheduler, [`uninstall`]
//! removes it and [`status`] reports whether it is installed and running.
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use nurt_contracts::config::Config;
//! use nurt_service::module::ServiceModule;
//! use nurt_service::{ScheduleService, ServiceSetup, run};
//!
//! let setup = ServiceSetup::new(Config::default())?;
//! let modules: Vec<Arc<dyn ServiceModule>> = vec![Arc::new(ScheduleService::new())];
//! run(setup, modules)?;
//! # Ok::<(), nurt_contracts::error::Error>(())
//! ```

#![deny(missing_docs)]
#![deny(unsafe_code)]

mod clock;
pub mod heartbeat;
mod host;
mod hub;
pub mod install;
pub mod ipc;
mod lock;
pub mod module;
mod paths;
pub mod priority;
pub mod process;
mod schedule;
#[cfg(windows)]
mod scm;
#[cfg(unix)]
mod signals;
pub mod status;
mod supervisor;

pub use clock::SystemClock;
pub use host::{ConfigSource, ServiceSetup, run};
pub use install::{
    Backend, InstallReport, InstallRequest, Manifest, UninstallReport, UninstallRequest, install,
    uninstall,
};
pub use paths::{ServicePaths, default_state_dir};
pub use schedule::ScheduleService;
pub use status::{InactiveReason, Liveness, ServiceStatus, StatusRequest, status};
