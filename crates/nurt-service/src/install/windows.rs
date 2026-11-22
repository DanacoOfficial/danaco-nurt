// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Service of the service control manager (mode `system`) and task of the
//! Task Scheduler (mode `user`) on Windows.

use std::ffi::OsString;
use std::path::Path;

use nurt_contracts::config::ServiceMode;
use nurt_contracts::error::{Error, ErrorCode};
use windows_service::service::{
    ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use super::detect::ModeChoice;
use super::task_scheduler;
use super::{Backend, InstallReport, InstallRequest, Manifest, UninstallReport, privilege_error};
use crate::scm::SERVICE_NAME;

const DISPLAY_NAME: &str = "Danaco Nurt service";

fn scm_error(action: &str, error: &windows_service::Error) -> Error {
    Error::new(
        ErrorCode::Service,
        format!("the service control manager refused to {action} {SERVICE_NAME}: {error}"),
    )
}

fn account_id() -> Result<String, Error> {
    let user = std::env::var("USERNAME").map_err(|_| {
        Error::new(
            ErrorCode::Io,
            "USERNAME is not set; cannot name the account of the task",
        )
    })?;
    Ok(match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
        _ => user,
    })
}

/// Installs the service in the chosen mode.
pub(crate) fn install(
    request: &InstallRequest,
    executable: &Path,
    choice: ModeChoice,
) -> Result<InstallReport, Error> {
    let backend = match choice.mode {
        ServiceMode::User => {
            let task = task_scheduler::definition(&account_id()?, executable);
            task_scheduler::register(&task)?;
            Backend::TaskScheduler
        }
        ServiceMode::System => {
            if !crate::process::is_elevated() {
                return Err(privilege_error("installing"));
            }
            create_service(request, executable)?;
            Backend::WindowsService
        }
    };
    let mut manifest = Manifest::new(choice.mode, backend, executable);
    manifest.account.clone_from(&request.account);
    manifest.write(&request.paths)?;
    Ok(InstallReport {
        mode: choice.mode,
        mode_reason: choice.reason,
        backend,
        files: Vec::new(),
        started: true,
        linger: None,
        notes: Vec::new(),
    })
}

fn create_service(request: &InstallRequest, executable: &Path) -> Result<(), Error> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|error| scm_error("open", &error))?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: executable.to_owned(),
        launch_arguments: vec![OsString::from("service"), OsString::from("run")],
        dependencies: Vec::new(),
        account_name: request.account.as_ref().map(OsString::from),
        account_password: None,
    };
    let service = manager
        .create_service(&info, ServiceAccess::START | ServiceAccess::QUERY_STATUS)
        .map_err(|error| scm_error("create", &error))?;
    service
        .start::<&str>(&[])
        .map_err(|error| scm_error("start", &error))
}

/// Stops and removes the service or the task of the manifest.
pub(crate) fn uninstall(manifest: &Manifest) -> Result<UninstallReport, Error> {
    match manifest.backend {
        Backend::TaskScheduler => task_scheduler::delete(task_scheduler::TASK_NAME)?,
        _ => {
            if !crate::process::is_elevated() {
                return Err(privilege_error("removing"));
            }
            let manager =
                ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
                    .map_err(|error| scm_error("open", &error))?;
            let service = manager
                .open_service(
                    SERVICE_NAME,
                    ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
                )
                .map_err(|error| scm_error("open", &error))?;
            if service
                .query_status()
                .is_ok_and(|status| status.current_state != ServiceState::Stopped)
            {
                let _ = service.stop();
            }
            service
                .delete()
                .map_err(|error| scm_error("delete", &error))?;
        }
    }
    Ok(UninstallReport {
        mode: manifest.mode,
        backend: manifest.backend,
        removed: Vec::new(),
        notes: Vec::new(),
    })
}
