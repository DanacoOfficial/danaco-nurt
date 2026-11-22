// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Start of the service by the Windows service control manager.
//!
//! [`dispatch`] connects to the service control manager. When the process
//! was started by it, the service runs on the thread the manager creates and
//! reports `StartPending`, `Running`, `StopPending` and `Stopped`; `Stop` and
//! `Shutdown` cancel the stop token of the setup. When the process was
//! started from a console, the setup is handed back to run in the calling
//! thread.

#![allow(
    unsafe_code,
    reason = "the entry point macro of windows-service expands to an extern function with an unsafe block"
)]

use std::ffi::OsString;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::stage::CancelReason;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
    ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use crate::host::{ServiceSetup, serve};
use crate::module::ServiceModule;

/// Name of the service registered with the service control manager.
pub(crate) const SERVICE_NAME: &str = "NurtService";

const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: i32 = 1063;
const START_WAIT_HINT: Duration = Duration::from_secs(10);
const FAILURE_EXIT_CODE: u32 = 1;

type Pending = (ServiceSetup, Vec<Arc<dyn ServiceModule>>);

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
static OUTCOME: Mutex<Option<Result<(), Error>>> = Mutex::new(None);

/// Result of [`dispatch`].
pub(crate) enum Dispatch {
    /// The service ran under the service control manager and ended.
    Done(Result<(), Error>),
    /// The process was not started by the service control manager.
    Console(ServiceSetup, Vec<Arc<dyn ServiceModule>>),
}

define_windows_service!(ffi_service_main, service_main);

/// Runs the service under the service control manager when it started the
/// process.
pub(crate) fn dispatch(setup: ServiceSetup, modules: Vec<Arc<dyn ServiceModule>>) -> Dispatch {
    *PENDING.lock().unwrap_or_else(PoisonError::into_inner) = Some((setup, modules));
    let started = service_dispatcher::start(SERVICE_NAME, ffi_service_main);
    let pending = PENDING.lock().unwrap_or_else(PoisonError::into_inner).take();
    match started {
        Ok(()) => Dispatch::Done(
            OUTCOME
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .unwrap_or(Ok(())),
        ),
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT) =>
        {
            match pending {
                Some((setup, modules)) => Dispatch::Console(setup, modules),
                None => Dispatch::Done(Err(Error::new(
                    ErrorCode::Internal,
                    "the service setup was consumed by the service control manager",
                ))),
            }
        }
        Err(error) => Dispatch::Done(Err(Error::new(
            ErrorCode::Service,
            format!("cannot connect to the service control manager: {error}"),
        ))),
    }
}

fn status(state: ServiceState, exit_code: ServiceExitCode) -> ServiceStatus {
    let controls_accepted = if state == ServiceState::Running {
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
    } else {
        ServiceControlAccept::empty()
    };
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted,
        exit_code,
        checkpoint: 0,
        wait_hint: START_WAIT_HINT,
        process_id: None,
    }
}

fn service_main(_arguments: Vec<OsString>) {
    let Some((setup, modules)) = PENDING.lock().unwrap_or_else(PoisonError::into_inner).take()
    else {
        return;
    };
    let stop = setup.stop();
    let handler = service_control_handler::register(SERVICE_NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            stop.cancel(CancelReason::Shutdown);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    });
    let outcome = match handler {
        Ok(handle) => {
            let _ = handle.set_service_status(status(
                ServiceState::StartPending,
                ServiceExitCode::Win32(0),
            ));
            let _ = handle.set_service_status(status(ServiceState::Running, ServiceExitCode::Win32(0)));
            let outcome = serve(setup, modules);
            let _ = handle.set_service_status(status(
                ServiceState::StopPending,
                ServiceExitCode::Win32(0),
            ));
            let exit_code = match outcome {
                Ok(()) => ServiceExitCode::Win32(0),
                Err(_) => ServiceExitCode::ServiceSpecific(FAILURE_EXIT_CODE),
            };
            let _ = handle.set_service_status(status(ServiceState::Stopped, exit_code));
            outcome
        }
        Err(error) => Err(Error::new(
            ErrorCode::Service,
            format!("cannot register the service control handler: {error}"),
        )),
    };
    *OUTCOME.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
}
