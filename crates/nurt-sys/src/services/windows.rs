// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::services::windows
// Responsibility:   Windows service control manager backend.
// Dependencies:     nurt_contracts, windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Windows service control manager backend.
//!
//! Services are read and controlled through the Win32 service functions with
//! the least access each request needs. The scope `system` covers ordinary
//! services; the scope `user` covers the instances of per-user services
//! (`SERVICE_USERSERVICE_INSTANCE`) that the manager created for signed-in
//! accounts. Templates of per-user services belong to neither scope.

mod scm;

use std::ptr;
use std::slice;
use std::thread;
use std::time::Duration;

use nurt_contracts::error::{Error, Result};
use nurt_contracts::record::{Service, ServiceManager, ServiceScope};
use nurt_contracts::stage::ExecCtx;
use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, GetLastError};
use windows_sys::Win32::System::Diagnostics::Debug::{
    FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS, FormatMessageW,
};
use windows_sys::Win32::System::Services::{
    ChangeServiceConfigW, CloseServiceHandle, ControlService, ENUM_SERVICE_STATUS_PROCESSW,
    ENUM_SERVICE_STATUSW, EnumDependentServicesW, EnumServicesStatusExW, OpenSCManagerW,
    OpenServiceW, QUERY_SERVICE_CONFIGW, QueryServiceConfig2W, QueryServiceConfigW,
    QueryServiceStatusEx, SC_ENUM_PROCESS_INFO, SC_HANDLE, SC_MANAGER_CONNECT,
    SC_MANAGER_ENUMERATE_SERVICE, SC_STATUS_PROCESS_INFO, SERVICE_ACTIVE, SERVICE_CHANGE_CONFIG,
    SERVICE_CONFIG_DELAYED_AUTO_START_INFO, SERVICE_CONFIG_DESCRIPTION, SERVICE_CONTROL_STOP,
    SERVICE_DELAYED_AUTO_START_INFO, SERVICE_DESCRIPTIONW, SERVICE_ENUMERATE_DEPENDENTS,
    SERVICE_NO_CHANGE, SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS, SERVICE_START,
    SERVICE_STATE_ALL, SERVICE_STATUS, SERVICE_STATUS_PROCESS, SERVICE_STOP, SERVICE_WIN32,
    StartServiceW,
};

use self::scm::{
    ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_NOT_ACTIVE, Failure, OsError, RestartError,
    SERVICE_AUTO_START, SERVICE_DISABLED, ScmOps, ServiceFacts, classify, exit_result, map_state,
    scope_of, state_name, tolerate,
};
use super::{
    ServiceBackend, ServiceQuery, ServiceStatus, service_error, service_not_found,
    service_privilege,
};

const MANAGER: ServiceManager = ServiceManager::Windows;

/// Privilege named in `E_PRIVILEGE` when the manager refuses access.
const REQUIRED_PRIVILEGE: &str = "administrator";

/// Service types enumerated: Win32 services and per-user services.
const ENUMERATED_TYPES: u32 =
    SERVICE_WIN32 | scm::SERVICE_USER_SERVICE | scm::SERVICE_USERSERVICE_INSTANCE;

/// Size of the buffer for a system message, in UTF-16 units.
const MESSAGE_UNITS: u32 = 512;

/// Size of `SERVICE_STATUS_PROCESS` in bytes.
const STATUS_PROCESS_BYTES: u32 = size_of::<SERVICE_STATUS_PROCESS>() as u32;

/// Calls of a variable-size query before the growing result is given up.
const MAX_ATTEMPTS: usize = 8;

/// Service handle, closed on drop.
struct Handle(SC_HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by `OpenSCManagerW` or
        // `OpenServiceW`, is owned by this value and is closed once.
        unsafe {
            CloseServiceHandle(self.0);
        }
    }
}

struct ScmBackend;

pub(crate) fn backend(_ctx: &ExecCtx) -> Box<dyn ServiceBackend + '_> {
    Box::new(ScmBackend)
}

impl ServiceBackend for ScmBackend {
    fn manager(&self) -> ServiceManager {
        MANAGER
    }

    fn list(&self, query: &ServiceQuery) -> Result<Vec<Service>> {
        let manager = open_manager(SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE)
            .map_err(|error| to_error(error, ""))?;
        let entries = enumerate(&manager).map_err(|error| to_error(error, ""))?;
        Ok(entries
            .into_iter()
            .filter(|facts| scope_of(facts.service_type) == Some(query.scope))
            .map(|mut facts| {
                if let Ok(service) = open_in(&manager, &facts.name, SERVICE_QUERY_CONFIG) {
                    read_config(&service, &mut facts);
                }
                facts.to_record()
            })
            .collect())
    }

    fn get(&self, name: &str, scope: ServiceScope) -> Result<Service> {
        let service =
            open_service(name, SERVICE_QUERY_STATUS).map_err(|error| to_error(error, name))?;
        let status = query_status(&service).map_err(|error| to_error(error, name))?;
        if scope_of(status.dwServiceType) != Some(scope) {
            return Err(service_not_found(MANAGER, name));
        }
        let mut facts = ServiceFacts {
            name: name.to_owned(),
            state: status.dwCurrentState,
            pid: status.dwProcessId,
            service_type: status.dwServiceType,
            exit_code: status.dwWin32ExitCode,
            specific_exit_code: status.dwServiceSpecificExitCode,
            ..ServiceFacts::default()
        };
        // Services that deny the configuration to the account still report
        // their state, as in the listing.
        if let Ok(configured) = open_service(name, SERVICE_QUERY_CONFIG) {
            read_config(&configured, &mut facts);
        }
        Ok(facts.to_record())
    }

    fn status(&self, name: &str, _scope: ServiceScope) -> Result<ServiceStatus> {
        let service =
            open_service(name, SERVICE_QUERY_STATUS).map_err(|error| to_error(error, name))?;
        let status = query_status(&service).map_err(|error| to_error(error, name))?;
        Ok(ServiceStatus {
            state: map_state(status.dwCurrentState),
            state_raw: Some(state_name(status.dwCurrentState)),
            result: exit_result(status.dwWin32ExitCode, status.dwServiceSpecificExitCode),
        })
    }

    fn start(&self, name: &str, _scope: ServiceScope) -> Result<()> {
        tolerate(Win32Ops.start(name), ERROR_SERVICE_ALREADY_RUNNING)
            .map_err(|error| to_error(error, name))
    }

    fn stop(&self, name: &str, _scope: ServiceScope) -> Result<()> {
        tolerate(Win32Ops.stop(name), ERROR_SERVICE_NOT_ACTIVE)
            .map_err(|error| to_error(error, name))
    }

    fn restart(&self, name: &str, _scope: ServiceScope) -> Result<()> {
        scm::restart(&Win32Ops, name).map_err(|error| match error {
            RestartError::Os(error) => to_error(error, name),
            other => service_error(MANAGER, name, &other.os_code(), &other.message()),
        })
    }

    fn enable(&self, name: &str, _scope: ServiceScope) -> Result<()> {
        change_start_type(name, SERVICE_AUTO_START).map_err(|error| to_error(error, name))
    }

    fn disable(&self, name: &str, _scope: ServiceScope) -> Result<()> {
        change_start_type(name, SERVICE_DISABLED).map_err(|error| to_error(error, name))
    }
}

/// Requests of the restart sequence through the Win32 interface.
struct Win32Ops;

impl ScmOps for Win32Ops {
    fn state(&self, name: &str) -> std::result::Result<u32, OsError> {
        let service = open_service(name, SERVICE_QUERY_STATUS)?;
        Ok(query_status(&service)?.dwCurrentState)
    }

    fn running_dependents(&self, name: &str) -> std::result::Result<Vec<String>, OsError> {
        let service = open_service(name, SERVICE_ENUMERATE_DEPENDENTS)?;
        let mut count = 0u32;
        let buffer = query_buffer(|data, size, needed| {
            // SAFETY: `data` is writable for `size` bytes and aligned for
            // `ENUM_SERVICE_STATUSW`; the outputs are valid locations.
            unsafe {
                EnumDependentServicesW(
                    service.0,
                    SERVICE_ACTIVE,
                    data.cast(),
                    size,
                    needed,
                    &mut count,
                )
            }
        })?;
        let count = usize::try_from(count).unwrap_or_default();
        // SAFETY: the manager wrote `count` entries at the start of the
        // buffer, which is aligned for the entry type and outlives the slice.
        let entries =
            unsafe { slice::from_raw_parts(buffer.as_ptr().cast::<ENUM_SERVICE_STATUSW>(), count) };
        Ok(entries
            .iter()
            // SAFETY: the name points to a NUL-terminated string inside the
            // buffer written by the manager.
            .map(|entry| unsafe { read_wide(entry.lpServiceName) })
            .collect())
    }

    fn stop(&self, name: &str) -> std::result::Result<(), OsError> {
        let service = open_service(name, SERVICE_STOP)?;
        let mut status = SERVICE_STATUS::default();
        // SAFETY: the handle is open with `SERVICE_STOP` and `status` is a
        // valid output location.
        let done = unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) };
        succeeded(done)
    }

    fn start(&self, name: &str) -> std::result::Result<(), OsError> {
        let service = open_service(name, SERVICE_START)?;
        // SAFETY: the handle is open with `SERVICE_START`; no arguments are
        // passed to the service.
        let done = unsafe { StartServiceW(service.0, 0, ptr::null()) };
        succeeded(done)
    }

    fn pause(&self, interval: Duration) {
        thread::sleep(interval);
    }
}

fn enumerate(manager: &Handle) -> std::result::Result<Vec<ServiceFacts>, OsError> {
    let mut count = 0u32;
    let buffer = query_buffer(|data, size, needed| {
        let mut resume = 0u32;
        // SAFETY: `data` is writable for `size` bytes and aligned for
        // `ENUM_SERVICE_STATUS_PROCESSW`; the outputs are valid locations.
        // Every call starts from the first service, so partial results of a
        // too small buffer are discarded.
        unsafe {
            EnumServicesStatusExW(
                manager.0,
                SC_ENUM_PROCESS_INFO,
                ENUMERATED_TYPES,
                SERVICE_STATE_ALL,
                data,
                size,
                needed,
                &mut count,
                &mut resume,
                ptr::null(),
            )
        }
    })?;
    let count = usize::try_from(count).unwrap_or_default();
    // SAFETY: the manager wrote `count` entries at the start of the buffer,
    // which is aligned for the entry type and outlives the slice.
    let entries = unsafe {
        slice::from_raw_parts(
            buffer.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>(),
            count,
        )
    };
    Ok(entries.iter().map(facts_of_entry).collect())
}

fn facts_of_entry(entry: &ENUM_SERVICE_STATUS_PROCESSW) -> ServiceFacts {
    let status = &entry.ServiceStatusProcess;
    ServiceFacts {
        // SAFETY: both names point to NUL-terminated strings inside the
        // buffer written by the manager.
        name: unsafe { read_wide(entry.lpServiceName) },
        // SAFETY: as above.
        display_name: Some(unsafe { read_wide(entry.lpDisplayName) }),
        state: status.dwCurrentState,
        pid: status.dwProcessId,
        service_type: status.dwServiceType,
        exit_code: status.dwWin32ExitCode,
        specific_exit_code: status.dwServiceSpecificExitCode,
        ..ServiceFacts::default()
    }
}

/// Adds the configuration, description and delayed start to `facts`; parts
/// that cannot be read stay empty.
fn read_config(service: &Handle, facts: &mut ServiceFacts) {
    let config = query_buffer(|data, size, needed| {
        // SAFETY: `data` is writable for `size` bytes and aligned for
        // `QUERY_SERVICE_CONFIGW`.
        unsafe { QueryServiceConfigW(service.0, data.cast(), size, needed) }
    });
    if let Ok(buffer) = config {
        // SAFETY: the manager wrote a `QUERY_SERVICE_CONFIGW` at the start of
        // the buffer; its strings point into the same buffer.
        unsafe {
            let config = &*buffer.as_ptr().cast::<QUERY_SERVICE_CONFIGW>();
            facts.start_type = Some(config.dwStartType);
            facts.path = Some(read_wide(config.lpBinaryPathName));
            facts.account = Some(read_wide(config.lpServiceStartName));
            facts.display_name = Some(read_wide(config.lpDisplayName));
        }
    }
    if let Ok(buffer) = query_config2(service, SERVICE_CONFIG_DESCRIPTION) {
        // SAFETY: the manager wrote a `SERVICE_DESCRIPTIONW` at the start of
        // the buffer; the description points into the same buffer.
        facts.description = Some(unsafe {
            read_wide((*buffer.as_ptr().cast::<SERVICE_DESCRIPTIONW>()).lpDescription)
        });
    }
    if let Ok(buffer) = query_config2(service, SERVICE_CONFIG_DELAYED_AUTO_START_INFO) {
        // SAFETY: the manager wrote a `SERVICE_DELAYED_AUTO_START_INFO` at
        // the start of the buffer.
        facts.delayed = unsafe {
            (*buffer.as_ptr().cast::<SERVICE_DELAYED_AUTO_START_INFO>()).fDelayedAutostart != 0
        };
    }
}

fn query_config2(service: &Handle, level: u32) -> std::result::Result<Vec<u64>, OsError> {
    query_buffer(|data, size, needed| {
        // SAFETY: `data` is writable for `size` bytes and aligned for every
        // structure of the information levels.
        unsafe { QueryServiceConfig2W(service.0, level, data, size, needed) }
    })
}

fn query_status(service: &Handle) -> std::result::Result<SERVICE_STATUS_PROCESS, OsError> {
    let mut status = SERVICE_STATUS_PROCESS::default();
    let mut needed = 0u32;
    // SAFETY: `status` is a writable `SERVICE_STATUS_PROCESS` of the size
    // passed to the call.
    let done = unsafe {
        QueryServiceStatusEx(
            service.0,
            SC_STATUS_PROCESS_INFO,
            ptr::from_mut(&mut status).cast(),
            STATUS_PROCESS_BYTES,
            &mut needed,
        )
    };
    succeeded(done)?;
    Ok(status)
}

fn change_start_type(name: &str, start_type: u32) -> std::result::Result<(), OsError> {
    let service = open_service(name, SERVICE_CHANGE_CONFIG)?;
    // SAFETY: the handle is open with `SERVICE_CHANGE_CONFIG`; every other
    // setting is left unchanged through `SERVICE_NO_CHANGE` and null
    // pointers.
    let done = unsafe {
        ChangeServiceConfigW(
            service.0,
            SERVICE_NO_CHANGE,
            start_type,
            SERVICE_NO_CHANGE,
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
        )
    };
    succeeded(done)
}

/// Runs a query that reports the required size, growing the buffer until
/// the result fits. The buffer is made of `u64` so that the structures the
/// manager writes into it are aligned.
fn query_buffer(
    mut call: impl FnMut(*mut u8, u32, &mut u32) -> i32,
) -> std::result::Result<Vec<u64>, OsError> {
    let mut buffer: Vec<u64> = Vec::new();
    for _ in 0..MAX_ATTEMPTS {
        let size = u32::try_from(buffer.len() * size_of::<u64>()).unwrap_or(u32::MAX);
        let mut needed = 0u32;
        if call(buffer.as_mut_ptr().cast(), size, &mut needed) != 0 {
            return Ok(buffer);
        }
        let error = last_error();
        if error.code != ERROR_INSUFFICIENT_BUFFER && error.code != ERROR_MORE_DATA {
            return Err(error);
        }
        let bytes = usize::try_from(size.saturating_add(needed)).unwrap_or(usize::MAX);
        buffer.resize(bytes.div_ceil(size_of::<u64>()), 0);
    }
    Err(last_error())
}

fn open_manager(access: u32) -> std::result::Result<Handle, OsError> {
    // SAFETY: null machine and database names select the local active
    // database.
    let handle = unsafe { OpenSCManagerW(ptr::null(), ptr::null(), access) };
    if handle.is_null() {
        Err(last_error())
    } else {
        Ok(Handle(handle))
    }
}

fn open_service(name: &str, access: u32) -> std::result::Result<Handle, OsError> {
    let manager = open_manager(SC_MANAGER_CONNECT)?;
    open_in(&manager, name, access)
}

fn open_in(manager: &Handle, name: &str, access: u32) -> std::result::Result<Handle, OsError> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the
    // call and the manager handle is open.
    let handle = unsafe { OpenServiceW(manager.0, wide.as_ptr(), access) };
    if handle.is_null() {
        Err(last_error())
    } else {
        Ok(Handle(handle))
    }
}

fn succeeded(done: i32) -> std::result::Result<(), OsError> {
    if done == 0 { Err(last_error()) } else { Ok(()) }
}

fn last_error() -> OsError {
    // SAFETY: `GetLastError` reads the error code of the calling thread.
    let code = unsafe { GetLastError() };
    OsError {
        code,
        message: format_message(code),
    }
}

fn format_message(code: u32) -> String {
    let mut buffer = [0u16; MESSAGE_UNITS as usize];
    // SAFETY: `buffer` holds `MESSAGE_UNITS` writable UTF-16 units; no
    // inserts are expanded, so no argument list is needed.
    let length = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            ptr::null(),
            code,
            0,
            buffer.as_mut_ptr(),
            MESSAGE_UNITS,
            ptr::null(),
        )
    };
    let length = usize::try_from(length).unwrap_or_default();
    String::from_utf16_lossy(buffer.get(..length).unwrap_or_default())
        .trim_end()
        .to_owned()
}

/// Reads a NUL-terminated UTF-16 string; a null pointer reads as empty.
///
/// # Safety
///
/// `text` is null or points to a readable NUL-terminated UTF-16 string.
unsafe fn read_wide(text: *const u16) -> String {
    if text.is_null() {
        return String::new();
    }
    let mut length = 0;
    // SAFETY: the caller guarantees a NUL terminator, so every unit up to and
    // including it is readable.
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: the first `length` units were read above.
    String::from_utf16_lossy(unsafe { slice::from_raw_parts(text, length) })
}

fn to_error(error: OsError, name: &str) -> Error {
    match classify(error.code) {
        Failure::NotFound => service_not_found(MANAGER, name),
        Failure::Privilege => service_privilege(MANAGER, name, REQUIRED_PRIVILEGE),
        Failure::Manager => service_error(MANAGER, name, &error.code.to_string(), &error.message),
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use windows_sys::Win32::Foundation::SetLastError;

    use super::*;

    fn fail_with(code: u32) -> i32 {
        // SAFETY: `SetLastError` stores the error code of the calling thread.
        unsafe { SetLastError(code) };
        0
    }

    #[test]
    fn query_buffer_grows_until_the_result_fits() {
        let mut calls = 0;
        let buffer = query_buffer(|_, size, needed| {
            calls += 1;
            if size < 100 {
                *needed = 100;
                fail_with(ERROR_INSUFFICIENT_BUFFER)
            } else {
                1
            }
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert!(buffer.len() * size_of::<u64>() >= 100);
        let mut more_data = 0;
        let buffer = query_buffer(|_, size, needed| {
            more_data += 1;
            if size < 20 {
                *needed = 20;
                fail_with(ERROR_MORE_DATA)
            } else {
                1
            }
        })
        .unwrap();
        assert_eq!(more_data, 2);
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn query_buffer_returns_other_errors_and_gives_up_after_the_attempt_limit() {
        let mut calls = 0;
        let error = query_buffer(|_, _, _| {
            calls += 1;
            fail_with(scm::ERROR_ACCESS_DENIED)
        })
        .unwrap_err();
        assert_eq!(error.code, scm::ERROR_ACCESS_DENIED);
        assert_eq!(calls, 1);
        let mut attempts = 0;
        let outcome = query_buffer(|_, _, needed| {
            attempts += 1;
            *needed = 8;
            fail_with(ERROR_MORE_DATA)
        });
        assert!(outcome.is_err());
        assert_eq!(attempts, MAX_ATTEMPTS);
        assert!(query_buffer(|_, _, _| 1).unwrap().is_empty());
    }

    #[test]
    fn read_wide_stops_at_the_terminator_and_reads_null_as_empty() {
        let text: Vec<u16> = "Café ß 日本 🙂"
            .encode_utf16()
            .chain([0, u16::from(b'x')])
            .collect();
        // SAFETY: `text` holds a NUL-terminated UTF-16 string.
        assert_eq!(unsafe { read_wide(text.as_ptr()) }, "Café ß 日本 🙂");
        // SAFETY: a null pointer is permitted.
        assert_eq!(unsafe { read_wide(ptr::null()) }, "");
        let empty = [0u16];
        // SAFETY: `empty` holds a NUL-terminated UTF-16 string.
        assert_eq!(unsafe { read_wide(empty.as_ptr()) }, "");
        let lone_surrogate = [0xD800u16, u16::from(b'a'), 0];
        // SAFETY: `lone_surrogate` holds a NUL-terminated UTF-16 string.
        assert_eq!(unsafe { read_wide(lone_surrogate.as_ptr()) }, "\u{FFFD}a");
    }

    #[test]
    fn errors_of_the_manager_map_to_the_error_table() {
        let os_error = |code| OsError {
            code,
            message: "message of the system".to_owned(),
        };
        let missing = to_error(os_error(scm::ERROR_SERVICE_DOES_NOT_EXIST), "Absent");
        assert_eq!(missing.code(), ErrorCode::NotFound);
        let denied = to_error(os_error(scm::ERROR_ACCESS_DENIED), "Spooler");
        assert_eq!(denied.code(), ErrorCode::Privilege);
        assert_eq!(denied.details["required"], REQUIRED_PRIVILEGE);
        let other = to_error(os_error(1051), "Spooler");
        assert_eq!(other.code(), ErrorCode::Service);
        assert_eq!(other.details["os_code"], "1051");
        assert_eq!(other.details["os_message"], "message of the system");
    }

    #[test]
    fn format_message_describes_a_known_code_and_leaves_an_unknown_one_empty() {
        assert!(!format_message(scm::ERROR_ACCESS_DENIED).is_empty());
        assert!(!format_message(scm::ERROR_ACCESS_DENIED).ends_with(char::is_whitespace));
        assert!(format_message(0x3FFF_FFFF).is_empty());
    }

    #[test]
    #[ignore = "requires the Windows service control manager"]
    fn it_scm_get_reads_the_state_of_every_listed_service() {
        let backend = ScmBackend;
        let query = ServiceQuery {
            scope: ServiceScope::System,
        };
        for listed in backend.list(&query).unwrap() {
            let service = backend
                .get(&listed.name, ServiceScope::System)
                .unwrap_or_else(|error| panic!("{}: {error:?}", listed.name));
            assert_eq!(service.name.to_lowercase(), listed.name.to_lowercase());
        }
    }

    #[test]
    #[ignore = "requires the Windows service control manager"]
    fn it_scm_lists_spooler() {
        let backend = ScmBackend;
        let query = ServiceQuery {
            scope: ServiceScope::System,
        };
        let services = backend.list(&query).unwrap();
        let spooler = services
            .iter()
            .find(|service| service.name.eq_ignore_ascii_case("Spooler"))
            .expect("the print spooler service is installed on every edition");
        assert_eq!(spooler.manager, ServiceManager::Windows);
        let single = backend.get("spooler", ServiceScope::System).unwrap();
        assert_eq!(single.start_type, spooler.start_type);
        let status = backend.status("Spooler", ServiceScope::System).unwrap();
        assert_eq!(status.state, spooler.state);
        let missing = backend
            .get("NurtMissingService", ServiceScope::System)
            .unwrap_err();
        assert_eq!(missing.code(), ErrorCode::NotFound);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
