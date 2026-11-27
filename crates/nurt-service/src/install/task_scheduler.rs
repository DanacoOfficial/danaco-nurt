// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::install::task_scheduler
// Responsibility:   Task of the Windows Task Scheduler for mode `user`.
// Dependencies:     com, nurt_contracts, windows
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Task of the Windows Task Scheduler for mode `user`.
//!
//! The task starts `nurt service run` at logon of the account with its
//! interactive token, has no execution time limit, ignores a second start
//! while it runs and also runs on battery power. It is registered through the
//! COM interface `ITaskService`, without `schtasks.exe`.

use std::path::{Path, PathBuf};

/// Name of the task in the root folder of the Task Scheduler.
pub(crate) const TASK_NAME: &str = "Nurt Service";

/// Settings of the task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskDefinition {
    pub(crate) name: &'static str,
    /// Account in the form `DOMAIN\user`; also the user of the logon trigger.
    pub(crate) user_id: String,
    pub(crate) executable: PathBuf,
    pub(crate) arguments: &'static str,
    /// ISO 8601 duration; `PT0S` means no limit.
    pub(crate) execution_time_limit: &'static str,
    /// `TASK_INSTANCES_IGNORE_NEW` instead of parallel instances.
    pub(crate) ignore_new_instances: bool,
    pub(crate) disallow_start_if_on_batteries: bool,
    pub(crate) stop_if_going_on_batteries: bool,
    /// `TASK_LOGON_INTERACTIVE_TOKEN`: no stored password.
    pub(crate) interactive_token: bool,
}

/// Definition of the task for the account and program.
pub(crate) fn definition(user_id: &str, executable: &Path) -> TaskDefinition {
    TaskDefinition {
        name: TASK_NAME,
        user_id: user_id.to_owned(),
        executable: executable.to_owned(),
        arguments: "service run",
        execution_time_limit: "PT0S",
        ignore_new_instances: true,
        disallow_start_if_on_batteries: false,
        stop_if_going_on_batteries: false,
        interactive_token: true,
    }
}

#[cfg(windows)]
pub(crate) use com::{delete, register};

#[cfg(windows)]
mod com {
    #![allow(
        unsafe_code,
        reason = "the Task Scheduler is reachable only through COM calls"
    )]

    use nurt_contracts::error::{Error, ErrorCode};
    use windows::Win32::Foundation::{
        RPC_E_CHANGED_MODE, VARIANT_BOOL, VARIANT_FALSE, VARIANT_TRUE,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::System::TaskScheduler::{
        IExecAction, ILogonTrigger, ITaskFolder, ITaskService, TASK_ACTION_EXEC,
        TASK_CREATE_OR_UPDATE, TASK_INSTANCES_IGNORE_NEW, TASK_INSTANCES_PARALLEL,
        TASK_LOGON_INTERACTIVE_TOKEN, TASK_LOGON_PASSWORD, TASK_TRIGGER_LOGON, TaskScheduler,
    };
    use windows::Win32::System::Variant::VARIANT;
    use windows::core::{BSTR, Interface};

    use super::TaskDefinition;

    fn com_error(action: &str, error: &windows::core::Error) -> Error {
        Error::new(
            ErrorCode::Service,
            format!("the Task Scheduler refused to {action}: {error}"),
        )
    }

    fn flag(value: bool) -> VARIANT_BOOL {
        if value { VARIANT_TRUE } else { VARIANT_FALSE }
    }

    /// COM initialised on the calling thread for the lifetime of the value.
    struct Apartment {
        initialised: bool,
    }

    impl Apartment {
        fn enter() -> Result<Self, Error> {
            // SAFETY: initialises COM for the calling thread; balanced by
            // `CoUninitialize` in `drop` when it succeeded.
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if result == RPC_E_CHANGED_MODE {
                return Ok(Self { initialised: false });
            }
            result
                .ok()
                .map_err(|error| com_error("initialise COM", &error))?;
            Ok(Self { initialised: true })
        }
    }

    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.initialised {
                // SAFETY: balances the successful `CoInitializeEx` above.
                unsafe { CoUninitialize() };
            }
        }
    }

    fn connect() -> Result<(ITaskService, ITaskFolder), Error> {
        // SAFETY: COM is initialised by the caller; the arguments are valid
        // for the duration of every call.
        unsafe {
            let service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| com_error("start", &error))?;
            let empty = VARIANT::default();
            service
                .Connect(&empty, &empty, &empty, &empty)
                .map_err(|error| com_error("connect", &error))?;
            let folder = service
                .GetFolder(&BSTR::from("\\"))
                .map_err(|error| com_error("open the root folder", &error))?;
            Ok((service, folder))
        }
    }

    /// Registers the task and starts it.
    pub(crate) fn register(task: &TaskDefinition) -> Result<(), Error> {
        let _apartment = Apartment::enter()?;
        let (service, folder) = connect()?;
        let user = BSTR::from(task.user_id.as_str());
        let empty = VARIANT::default();
        // SAFETY: COM is initialised; every interface pointer comes from the
        // Task Scheduler and lives across the calls.
        unsafe {
            let definition = service
                .NewTask(0)
                .map_err(|error| com_error("create a task", &error))?;
            let settings = definition
                .Settings()
                .map_err(|error| com_error("read task settings", &error))?;
            settings
                .SetExecutionTimeLimit(&BSTR::from(task.execution_time_limit))
                .and_then(|()| {
                    settings.SetMultipleInstances(if task.ignore_new_instances {
                        TASK_INSTANCES_IGNORE_NEW
                    } else {
                        TASK_INSTANCES_PARALLEL
                    })
                })
                .and_then(|()| {
                    settings
                        .SetDisallowStartIfOnBatteries(flag(task.disallow_start_if_on_batteries))
                })
                .and_then(|()| {
                    settings.SetStopIfGoingOnBatteries(flag(task.stop_if_going_on_batteries))
                })
                .map_err(|error| com_error("set task settings", &error))?;
            let logon_type = if task.interactive_token {
                TASK_LOGON_INTERACTIVE_TOKEN
            } else {
                TASK_LOGON_PASSWORD
            };
            let principal = definition
                .Principal()
                .map_err(|error| com_error("read the task principal", &error))?;
            principal
                .SetUserId(&user)
                .and_then(|()| principal.SetLogonType(logon_type))
                .map_err(|error| com_error("set the task principal", &error))?;
            let trigger = definition
                .Triggers()
                .and_then(|triggers| triggers.Create(TASK_TRIGGER_LOGON))
                .and_then(|trigger| trigger.cast::<ILogonTrigger>())
                .map_err(|error| com_error("create the logon trigger", &error))?;
            trigger
                .SetUserId(&user)
                .map_err(|error| com_error("set the logon trigger", &error))?;
            let action = definition
                .Actions()
                .and_then(|actions| actions.Create(TASK_ACTION_EXEC))
                .and_then(|action| action.cast::<IExecAction>())
                .map_err(|error| com_error("create the task action", &error))?;
            action
                .SetPath(&BSTR::from(task.executable.display().to_string()))
                .and_then(|()| action.SetArguments(&BSTR::from(task.arguments)))
                .map_err(|error| com_error("set the task action", &error))?;
            let registered = folder
                .RegisterTaskDefinition(
                    &BSTR::from(task.name),
                    &definition,
                    TASK_CREATE_OR_UPDATE.0,
                    &empty,
                    &empty,
                    logon_type,
                    &empty,
                )
                .map_err(|error| com_error("register the task", &error))?;
            registered
                .Run(&empty)
                .map_err(|error| com_error("start the task", &error))?;
        }
        Ok(())
    }

    /// Stops and deletes the task.
    pub(crate) fn delete(name: &str) -> Result<(), Error> {
        let _apartment = Apartment::enter()?;
        let (_service, folder) = connect()?;
        // SAFETY: COM is initialised and `folder` is a valid interface.
        unsafe {
            if let Ok(task) = folder.GetTask(&BSTR::from(name)) {
                let _ = task.Stop(0);
            }
            folder
                .DeleteTask(&BSTR::from(name), 0)
                .map_err(|error| com_error("delete the task", &error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_windows_task_definition_fields() {
        let task = definition(
            r"WORKSTATION\dev",
            Path::new(r"C:\Program Files\Nurt\nurt.exe"),
        );
        assert_eq!(task.name, "Nurt Service");
        assert_eq!(task.user_id, r"WORKSTATION\dev");
        assert_eq!(
            task.executable,
            Path::new(r"C:\Program Files\Nurt\nurt.exe")
        );
        assert_eq!(task.arguments, "service run");
        assert_eq!(task.execution_time_limit, "PT0S");
        assert!(task.ignore_new_instances);
        assert!(!task.disallow_start_if_on_batteries);
        assert!(!task.stop_if_going_on_batteries);
        assert!(task.interactive_token);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
