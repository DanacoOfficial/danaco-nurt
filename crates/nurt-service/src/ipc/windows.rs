// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::ipc::windows
// Responsibility:   Named pipe transport of the control channel on Windows.
// Dependencies:     windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Named pipe transport of the control channel on Windows.
//!
//! The pipe `\\.\pipe\nurt-service-<SID>` is created with
//! `FILE_FLAG_FIRST_PIPE_INSTANCE`, so the service fails instead of joining a
//! pipe created by another process, rejects remote clients and carries a
//! security descriptor that grants access only to the account SID and
//! `SYSTEM`. Reads use overlapped I/O to enforce the idle limit.

#![allow(
    unsafe_code,
    reason = "named pipes, overlapped I/O and security descriptors are Win32 calls"
)]

use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::ptr;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows_sys::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE};

use super::{Accepted, Listener, TimedRead};
use crate::process::current_user_sid;

const BUFFER_SIZE: u32 = 64 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const BUSY_RETRIES: u32 = 20;
const BUSY_RETRY_DELAY: Duration = Duration::from_millis(5);

/// Owned Win32 handle.
struct Handle(HANDLE);

// SAFETY: a pipe handle may be used from any thread; overlapped operations
// on it are independent, each with its own `OVERLAPPED` structure.
unsafe impl Send for Handle {}
// SAFETY: see `Send`; the handle value is never changed after creation.
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful Win32 call and is closed
        // exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

// SAFETY: the descriptor is immutable after creation and only read by
// `CreateNamedPipeW`.
unsafe impl Send for SecurityDescriptor {}
// SAFETY: see `Send`.
unsafe impl Sync for SecurityDescriptor {}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: the descriptor was allocated by
        // `ConvertStringSecurityDescriptorToSecurityDescriptorW`.
        unsafe { LocalFree(self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Name of the pipe of the current account.
pub(super) fn pipe_name() -> io::Result<String> {
    Ok(format!(r"\\.\pipe\nurt-service-{}", current_user_sid()?))
}

fn owner_only_descriptor(sid: &str) -> io::Result<SecurityDescriptor> {
    let sddl = wide(&format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)"));
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated; `descriptor` receives a buffer owned
    // by `SecurityDescriptor`.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(SecurityDescriptor(descriptor))
}

/// Listening named pipe.
pub(super) struct PipeListener {
    name: String,
    wide_name: Vec<u16>,
    descriptor: SecurityDescriptor,
    next: Mutex<Option<Arc<Handle>>>,
}

impl PipeListener {
    /// Creates the first instance of the pipe.
    pub(super) fn bind() -> io::Result<Self> {
        let sid = current_user_sid()?;
        let name = format!(r"\\.\pipe\nurt-service-{sid}");
        let listener = Self {
            wide_name: wide(&name),
            name,
            descriptor: owner_only_descriptor(&sid)?,
            next: Mutex::new(None),
        };
        let first = listener.instance(true)?;
        *listener.next.lock().unwrap_or_else(PoisonError::into_inner) = Some(first);
        Ok(listener)
    }

    fn instance(&self, first: bool) -> io::Result<Arc<Handle>> {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
            lpSecurityDescriptor: self.descriptor.0,
            bInheritHandle: 0,
        };
        let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        // SAFETY: the name is NUL-terminated and the attributes, with the
        // descriptor they point to, outlive the call.
        let handle = unsafe {
            CreateNamedPipeW(
                self.wide_name.as_ptr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER_SIZE,
                BUFFER_SIZE,
                0,
                &attributes,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Arc::new(Handle(handle)))
    }
}

impl Listener for PipeListener {
    fn accept(&self) -> io::Result<Accepted> {
        let taken = self
            .next
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let handle = match taken {
            Some(handle) => handle,
            None => self.instance(false)?,
        };
        let connected = overlapped(&handle, None, |overlapped| {
            // SAFETY: `overlapped` lives until the operation completes in
            // `overlapped`.
            let ok = unsafe { ConnectNamedPipe(handle.0, overlapped) };
            // SAFETY: reads the error of the call above.
            if ok == 0 && unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
                return Completion::Done(0);
            }
            Completion::from_call(ok)
        });
        let next = self.instance(false);
        *self.next.lock().unwrap_or_else(PoisonError::into_inner) = next.ok();
        connected?;
        let closer = Arc::clone(&handle);
        Ok(Accepted {
            reader: Box::new(PipeStream {
                handle: Arc::clone(&handle),
                idle: None,
            }),
            writer: Box::new(PipeStream {
                handle,
                idle: Some(WRITE_TIMEOUT),
            }),
            closer: Box::new(move || {
                // SAFETY: cancels pending operations of a valid handle.
                unsafe { CancelIoEx(closer.0, ptr::null()) };
            }),
        })
    }

    fn wake(&self) {
        let _ = OpenOptions::new().read(true).write(true).open(&self.name);
    }

    fn remove(&self) {
        self.next
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

enum Completion {
    Done(u32),
    Pending,
    Failed(io::Error),
}

impl Completion {
    fn from_call(ok: i32) -> Self {
        if ok != 0 {
            return Self::Pending;
        }
        // SAFETY: reads the error of the call that just failed.
        match unsafe { GetLastError() } {
            ERROR_IO_PENDING => Self::Pending,
            ERROR_BROKEN_PIPE => Self::Done(0),
            code => Self::Failed(io::Error::from_raw_os_error(
                i32::try_from(code).unwrap_or(i32::MAX),
            )),
        }
    }
}

/// Starts an overlapped operation and waits for it, at most `timeout`.
fn overlapped<F>(handle: &Handle, timeout: Option<Duration>, start: F) -> io::Result<usize>
where
    F: FnOnce(*mut OVERLAPPED) -> Completion,
{
    // SAFETY: creates an unnamed manual-reset event owned by `Handle`.
    let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if event.is_null() {
        return Err(io::Error::last_os_error());
    }
    let event = Handle(event);
    let mut state = OVERLAPPED {
        hEvent: event.0,
        ..OVERLAPPED::default()
    };
    match start(&mut state) {
        Completion::Done(bytes) => return Ok(bytes as usize),
        Completion::Failed(error) => return Err(error),
        Completion::Pending => {}
    }
    let wait = timeout.map_or(INFINITE, |limit| {
        u32::try_from(limit.as_millis()).unwrap_or(INFINITE - 1)
    });
    let mut bytes = 0_u32;
    // SAFETY: `state` and `bytes` outlive the wait; the operation was
    // started on `handle` with `state`.
    let ok = unsafe { GetOverlappedResultEx(handle.0, &state, &mut bytes, wait, 0) };
    if ok != 0 {
        return Ok(bytes as usize);
    }
    // SAFETY: reads the error of the wait above.
    match unsafe { GetLastError() } {
        WAIT_TIMEOUT => {
            // SAFETY: cancels the operation started with `state` and waits
            // for its completion, after which `state` may be released.
            unsafe {
                CancelIoEx(handle.0, &state);
                GetOverlappedResult(handle.0, &state, &mut bytes, 1);
            }
            Err(io::Error::from(io::ErrorKind::TimedOut))
        }
        ERROR_BROKEN_PIPE => Ok(0),
        code => Err(io::Error::from_raw_os_error(
            i32::try_from(code).unwrap_or(i32::MAX),
        )),
    }
}

/// One direction of a connected pipe.
struct PipeStream {
    handle: Arc<Handle>,
    idle: Option<Duration>,
}

impl Read for PipeStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let handle = Arc::clone(&self.handle);
        overlapped(&handle, self.idle, |state| {
            // SAFETY: `buffer` holds `length` bytes and outlives the
            // operation, which completes before `overlapped` returns.
            let ok = unsafe {
                ReadFile(
                    handle.0,
                    buffer.as_mut_ptr(),
                    length,
                    ptr::null_mut(),
                    state,
                )
            };
            Completion::from_call(ok)
        })
    }
}

impl Write for PipeStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let handle = Arc::clone(&self.handle);
        overlapped(&handle, self.idle, |state| {
            // SAFETY: `buffer` holds `length` bytes and outlives the
            // operation, which completes before `overlapped` returns.
            let ok =
                unsafe { WriteFile(handle.0, buffer.as_ptr(), length, ptr::null_mut(), state) };
            Completion::from_call(ok)
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl TimedRead for PipeStream {
    fn set_idle(&mut self, limit: Option<Duration>) -> io::Result<()> {
        self.idle = limit;
        Ok(())
    }
}

/// Connects to the pipe of the current account.
pub(super) fn connect() -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    let name = pipe_name()?;
    let mut attempt = 0;
    loop {
        match OpenOptions::new().read(true).write(true).open(&name) {
            Ok(file) => {
                let writer = file.try_clone()?;
                return Ok((Box::new(file), Box::new(writer)));
            }
            Err(error)
                if error.raw_os_error() == i32::try_from(ERROR_PIPE_BUSY).ok()
                    && attempt < BUSY_RETRIES =>
            {
                attempt += 1;
                thread::sleep(BUSY_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
