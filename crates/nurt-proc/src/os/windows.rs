// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::os::windows
// Responsibility:   Process trees of Windows: one job object per run.
// Dependencies:     nurt_contracts, windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Process trees of Windows: one job object per run.
//!
//! The program is created suspended, assigned to a job object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and only then resumed, so every
//! process it starts belongs to the job. The job carries the processor time
//! and memory limits and reports reaching them through an I/O completion
//! port. Only the standard handles of the program are inherited
//! (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`).
//!
//! Suspension uses `NtSuspendProcess` on every process of the job: it stops
//! all threads of a process at once, including threads created while the
//! process is being suspended, which a thread-by-thread snapshot cannot do.

use std::ffi::{OsStr, c_void};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{LimitHit, Signal};
use windows_sys::Win32::Foundation::{
    FILETIME, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, NTSTATUS, STATUS_ACCESS_VIOLATION,
    STATUS_CONTROL_C_EXIT, STATUS_DATATYPE_MISALIGNMENT, STATUS_FLOAT_DIVIDE_BY_ZERO,
    STATUS_ILLEGAL_INSTRUCTION, STATUS_INTEGER_DIVIDE_BY_ZERO, STATUS_STACK_BUFFER_OVERRUN,
    SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent, GetConsoleProcessList,
};
use windows_sys::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_JOB_MEMORY,
    JOB_OBJECT_LIMIT_JOB_TIME, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOBOBJECTINFOCLASS,
    JobObjectAssociateCompletionPortInformation, JobObjectBasicAccountingInformation,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, CREATE_NEW_PROCESS_GROUP,
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
    GetProcessTimes, IDLE_PRIORITY_CLASS, INFINITE, InitializeProcThreadAttributeList,
    NORMAL_PRIORITY_CLASS, OpenProcess, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_CREATION_FLAGS,
    PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SUSPEND_RESUME,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::{
    Exit, Finished, Streams, ended_by_supervisor, io_error, process_denied, process_gone,
    process_replaced, same_start, spawn_error, unsupported_signal,
};
use crate::iface::{Priority, ResourceUsage, SpawnRequest, StdinSource};

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtSuspendProcess(process: HANDLE) -> NTSTATUS;
    fn NtResumeProcess(process: HANDLE) -> NTSTATUS;
}

/// Exit code given to the processes of a job ended by the console.
const TERMINATED_EXIT_CODE: u32 = 1;
/// Job notification: the processor time limit of the job was reached.
const JOB_OBJECT_MSG_END_OF_JOB_TIME: u32 = 1;
/// Job notification: the memory limit of the job was reached.
const JOB_OBJECT_MSG_JOB_MEMORY_LIMIT: u32 = 10;
/// Processor times of job objects are counted in units of 100 ns.
const TIME_UNIT_NANOS: u64 = 100;
/// Time from 1601-01-01, the epoch of file times, to the Unix epoch, in units
/// of 100 ns.
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;
/// `ERROR_INVALID_PARAMETER`: no process has the given identifier.
const ERROR_INVALID_PARAMETER_CODE: i32 = 87;
/// `ERROR_ACCESS_DENIED`: the account may not open or end the process.
const ERROR_ACCESS_DENIED_CODE: i32 = 5;
/// Initial capacity of the process list of a job; doubled while too small.
const PROCESS_LIST_CAPACITY: usize = 64;
/// Niceness from which [`Priority::Nice`] maps to the idle class.
const IDLE_NICENESS: u8 = 15;
/// Niceness from which [`Priority::Nice`] maps to the below normal class.
const BELOW_NORMAL_NICENESS: u8 = 1;

/// Program in a job object.
pub(crate) struct Tree {
    pid: u32,
    process: OwnedHandle,
    job: OwnedHandle,
    port: OwnedHandle,
    terminated: AtomicBool,
    limit: Mutex<Option<LimitHit>>,
}

impl Tree {
    /// Starts the program of `request` in a new job object.
    pub(crate) fn spawn(request: &SpawnRequest) -> Result<(Self, Streams)> {
        let fail = |error: io::Error| spawn_error(&request.program, &error);
        if !request.cwd.is_dir() {
            return Err(fail(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "the working directory {} does not exist",
                    request.cwd.display()
                ),
            )));
        }
        let job = create_job(request).map_err(fail)?;
        let port = associate_port(&job).map_err(fail)?;
        let input = ChildInput::open(&request.stdin).map_err(fail)?;
        let (stdout_read, stdout_write) = pipe().map_err(fail)?;
        let (stderr_read, stderr_write) = if request.merge_stderr {
            (None, None)
        } else {
            let (read, write) = pipe().map_err(fail)?;
            (Some(read), Some(write))
        };
        let handles = StdHandles {
            input: input.child_handle(),
            output: raw(&stdout_write),
            error: stderr_write.as_ref().map_or(raw(&stdout_write), raw),
        };
        let command_line = command_line(request)?;
        let environment = environment_block(&request.env)?;
        let application = wide(request.program.as_os_str())?;
        let cwd = wide(request.cwd.as_os_str())?;
        let info = create_suspended(
            &application,
            command_line,
            &environment,
            &cwd,
            handles,
            priority_class(request.limits.priority),
        )
        .map_err(fail)?;
        // SAFETY: `CreateProcessW` returned owned handles of the new process
        // and its main thread.
        let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
        // SAFETY: as above.
        let thread = unsafe { OwnedHandle::from_raw_handle(info.hThread) };
        // SAFETY: both handles are open; the process has not run yet.
        if unsafe { AssignProcessToJobObject(raw(&job), raw(&process)) } == 0 {
            let error = io::Error::last_os_error();
            // SAFETY: the process handle is open and the process is ours.
            unsafe { TerminateProcess(raw(&process), TERMINATED_EXIT_CODE) };
            return Err(fail(error));
        }
        // SAFETY: the thread handle is open and the thread was created
        // suspended.
        if unsafe { ResumeThread(raw(&thread)) } == u32::MAX {
            let error = io::Error::last_os_error();
            // SAFETY: the job handle is open.
            unsafe { TerminateJobObject(raw(&job), TERMINATED_EXIT_CODE) };
            return Err(fail(error));
        }
        // The program must be the only writer of its output pipes for the
        // end of output to be seen.
        drop((thread, stdout_write, stderr_write));
        let streams = Streams {
            stdin: input.writer(),
            stdout: Box::new(File::from(stdout_read)),
            stderr: stderr_read.map(|read| Box::new(File::from(read)) as Box<dyn Read + Send>),
        };
        let tree = Self {
            pid: info.dwProcessId,
            process,
            job,
            port,
            terminated: AtomicBool::new(false),
            limit: Mutex::new(None),
        };
        Ok((tree, streams))
    }

    /// Process identifier of the main process.
    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    /// Blocks until the main process ends.
    pub(crate) fn wait_exit(&self) -> Result<()> {
        // SAFETY: the process handle is open for the lifetime of `self`.
        if unsafe { WaitForSingleObject(raw(&self.process), INFINITE) } == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(io_error(
                "wait for the program",
                &io::Error::last_os_error(),
            ))
        }
    }

    /// Sends `signal` to the tree.
    ///
    /// Console control events reach only processes attached to the console
    /// of this process. A program with a console of its own is ended
    /// through the job object for `terminate`, `interrupt` and `hangup`.
    pub(crate) fn signal(&self, signal: Signal) -> Result<()> {
        match signal {
            Signal::Kill => self.kill(),
            Signal::Terminate | Signal::Interrupt | Signal::Hangup => {
                if self.shares_console() {
                    // SAFETY: plain call; the group identifier is the
                    // identifier of the main process, created with
                    // `CREATE_NEW_PROCESS_GROUP`.
                    if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, self.pid) } != 0 {
                        return Ok(());
                    }
                }
                self.kill()
            }
            other => Err(unsupported_signal(other)),
        }
    }

    /// Ends every process of the job.
    ///
    /// The tree records that the supervisor ended it only when the main
    /// process was still running at that moment, so an exit that happened by
    /// itself at the same time keeps its own code.
    pub(crate) fn kill(&self) -> Result<()> {
        // SAFETY: the process handle is open for the lifetime of `self`.
        let running = unsafe { WaitForSingleObject(raw(&self.process), 0) } != WAIT_OBJECT_0;
        if running {
            self.terminated.store(true, Ordering::Release);
        }
        let ended = self.terminate_job();
        if ended.is_err() {
            self.terminated.store(false, Ordering::Release);
        }
        ended
    }

    /// Suspends every process of the job.
    pub(crate) fn suspend(&self) -> Result<()> {
        // Processes started while the list is being suspended appear in the
        // next listing; the loop ends when a listing adds nothing.
        let mut suspended: Vec<u32> = Vec::new();
        loop {
            let pending: Vec<u32> = self
                .process_ids()?
                .into_iter()
                .filter(|pid| !suspended.contains(pid))
                .collect();
            if pending.is_empty() {
                return Ok(());
            }
            for pid in pending {
                // SAFETY: `NtSuspendProcess` receives an open handle with
                // `PROCESS_SUSPEND_RESUME` access.
                with_process(&self.job, pid, |handle| unsafe { NtSuspendProcess(handle) })?;
                suspended.push(pid);
            }
        }
    }

    /// Resumes every process of the job.
    pub(crate) fn resume(&self) -> Result<()> {
        for pid in self.process_ids()? {
            // SAFETY: `NtResumeProcess` receives an open handle with
            // `PROCESS_SUSPEND_RESUME` access.
            with_process(&self.job, pid, |handle| unsafe { NtResumeProcess(handle) })?;
        }
        Ok(())
    }

    /// Processor time and peak committed memory of the job.
    pub(crate) fn sample(&self) -> Result<ResourceUsage> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        query_job(
            &self.job,
            JobObjectBasicAccountingInformation,
            &mut accounting,
        )?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        query_job(&self.job, JobObjectExtendedLimitInformation, &mut limits)?;
        let units = accounting
            .TotalUserTime
            .saturating_add(accounting.TotalKernelTime);
        Ok(ResourceUsage {
            cpu_time: Duration::from_nanos(
                u64::try_from(units)
                    .unwrap_or(0)
                    .saturating_mul(TIME_UNIT_NANOS),
            ),
            max_rss: u64::try_from(limits.PeakJobMemoryUsed).unwrap_or(u64::MAX),
        })
    }

    /// Limit the job reported since the start.
    pub(crate) fn limit_hit(&self) -> Option<LimitHit> {
        let mut limit = self
            .limit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            let mut message: u32 = 0;
            let mut key: usize = 0;
            let mut overlapped: *mut OVERLAPPED = ptr::null_mut();
            // SAFETY: the port handle is open and the output locations are
            // valid; a zero timeout returns at once when no message waits.
            let received = unsafe {
                GetQueuedCompletionStatus(
                    raw(&self.port),
                    &raw mut message,
                    &raw mut key,
                    &raw mut overlapped,
                    0,
                )
            };
            if received == 0 {
                break;
            }
            match message {
                JOB_OBJECT_MSG_JOB_MEMORY_LIMIT => *limit = Some(LimitHit::Memory),
                JOB_OBJECT_MSG_END_OF_JOB_TIME => *limit = Some(LimitHit::Cpu),
                _ => {}
            }
        }
        *limit
    }

    /// Ends the remaining processes of the job and reports how the main
    /// process ended. Call once, after [`Tree::wait_exit`].
    pub(crate) fn finish(&self) -> Result<Finished> {
        let usage = self.sample()?;
        self.terminate_job()?;
        let mut code: u32 = 0;
        // SAFETY: the process handle is open and `code` is writable.
        if unsafe { GetExitCodeProcess(raw(&self.process), &raw mut code) } == 0 {
            return Err(io_error(
                "read the exit code of the program",
                &io::Error::last_os_error(),
            ));
        }
        let ended = self.terminated.load(Ordering::Acquire);
        let exit = if ended_by_supervisor(code, TERMINATED_EXIT_CODE, ended) {
            Exit::Signal {
                signal: Signal::Kill,
                number: None,
            }
        } else {
            decode_exit_code(code)
        };
        Ok(Finished {
            exit,
            usage,
            limit_hit: self.limit_hit(),
        })
    }

    fn terminate_job(&self) -> Result<()> {
        // SAFETY: the job handle is open for the lifetime of `self`.
        if unsafe { TerminateJobObject(raw(&self.job), TERMINATED_EXIT_CODE) } == 0 {
            return Err(io_error("end the program", &io::Error::last_os_error()));
        }
        Ok(())
    }

    fn shares_console(&self) -> bool {
        console_has(self.pid)
    }

    fn process_ids(&self) -> Result<Vec<u32>> {
        let header = std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList);
        let entry = std::mem::size_of::<usize>();
        let mut capacity = PROCESS_LIST_CAPACITY;
        loop {
            let words = header.div_ceil(entry) + capacity;
            let mut buffer = vec![0usize; words];
            let size = u32::try_from(words * entry).unwrap_or(u32::MAX);
            // SAFETY: the buffer is aligned for the structure and holds
            // `size` writable bytes.
            let status = unsafe {
                QueryInformationJobObject(
                    raw(&self.job),
                    JobObjectBasicProcessIdList,
                    buffer.as_mut_ptr().cast(),
                    size,
                    ptr::null_mut(),
                )
            };
            // SAFETY: the buffer starts with the structure header, written
            // by the call or zero.
            let list = unsafe { &*buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
            let assigned = usize::try_from(list.NumberOfAssignedProcesses).unwrap_or(0);
            if status != 0 && assigned <= capacity {
                let listed = usize::try_from(list.NumberOfProcessIdsInList).unwrap_or(0);
                let first = header / entry;
                return Ok(buffer[first..first + listed.min(capacity)]
                    .iter()
                    .filter_map(|id| u32::try_from(*id).ok())
                    .collect());
            }
            if assigned <= capacity {
                return Err(io_error(
                    "list the processes of the program",
                    &io::Error::last_os_error(),
                ));
            }
            capacity = assigned.saturating_mul(2);
        }
    }
}

/// Whether process `pid` is attached to the console of this process.
fn console_has(pid: u32) -> bool {
    let mut ids = vec![0u32; PROCESS_LIST_CAPACITY];
    let capacity = u32::try_from(ids.len()).unwrap_or(u32::MAX);
    // SAFETY: the buffer holds `capacity` writable identifiers.
    let count = unsafe { GetConsoleProcessList(ids.as_mut_ptr(), capacity) };
    let count = usize::try_from(count).unwrap_or(0).min(ids.len());
    ids[..count].contains(&pid)
}

/// Process named by identifier and start time. The handle keeps the
/// identifier from being reused while the process is signalled, so a signal
/// cannot reach another process that received the identifier after the first
/// one ended.
pub(crate) struct Target {
    process: OwnedHandle,
    pid: u32,
}

impl Target {
    /// Opens process `pid` after checking that it started at `started`.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` when no such process runs, with `E_STALE`
    /// when the identifier names a process that started at another time and
    /// with `E_PERMISSION` when the account may not open the process.
    pub(crate) fn open(pid: u64, started: WallTime) -> Result<Self> {
        let Ok(id) = u32::try_from(pid) else {
            return Err(process_gone(pid));
        };
        let access = PROCESS_TERMINATE
            | PROCESS_SUSPEND_RESUME
            | PROCESS_QUERY_LIMITED_INFORMATION
            | PROCESS_SYNCHRONIZE;
        // SAFETY: plain call; a null result is checked below.
        let handle = unsafe { OpenProcess(access, 0, id) };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            return Err(match error.raw_os_error() {
                Some(ERROR_INVALID_PARAMETER_CODE) => process_gone(pid),
                Some(ERROR_ACCESS_DENIED_CODE) => process_denied(pid),
                _ => io_error("open the process", &error),
            });
        }
        // SAFETY: `OpenProcess` returned an open handle owned by the caller.
        let process = unsafe { OwnedHandle::from_raw_handle(handle) };
        let actual = creation_time(&process)
            .ok_or_else(|| io_error("read the start time", &io::Error::last_os_error()))?;
        let target = Self { process, pid: id };
        if target.has_ended() {
            return Err(process_gone(pid));
        }
        if !same_start(started, actual) {
            return Err(process_replaced(pid));
        }
        Ok(target)
    }

    /// Sends `signal` to the process. `kill` ends it. The other signals send
    /// a console break event to a process that shares the console of this
    /// process and has its own process group; any other process is ended.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNSUPPORTED` for a signal that has no equivalent and
    /// with `E_PERMISSION` or `E_IO` when the process cannot be ended.
    pub(crate) fn signal(&self, signal: Signal) -> Result<()> {
        match signal {
            Signal::Kill => self.terminate(),
            Signal::Terminate | Signal::Interrupt | Signal::Hangup => {
                let sent = console_has(self.pid) && {
                    // SAFETY: plain call; the group identifier is the process
                    // identifier, which names a group only for a process
                    // created with `CREATE_NEW_PROCESS_GROUP`; otherwise the
                    // call fails.
                    unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, self.pid) != 0 }
                };
                if sent {
                    return Ok(());
                }
                self.terminate()
            }
            other => Err(unsupported_signal(other)),
        }
    }

    /// Stops every thread of the process until it is resumed.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the system refuses the request.
    pub(crate) fn suspend(&self) -> Result<()> {
        // SAFETY: `NtSuspendProcess` receives an open handle with
        // `PROCESS_SUSPEND_RESUME` access.
        self.status(unsafe { NtSuspendProcess(raw(&self.process)) })
    }

    /// Continues a suspended process.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the system refuses the request.
    pub(crate) fn resume(&self) -> Result<()> {
        // SAFETY: `NtResumeProcess` receives an open handle with
        // `PROCESS_SUSPEND_RESUME` access.
        self.status(unsafe { NtResumeProcess(raw(&self.process)) })
    }

    /// Whether the process ended.
    pub(crate) fn has_ended(&self) -> bool {
        // SAFETY: the process handle is open for the lifetime of `self`; a
        // zero timeout only polls.
        unsafe { WaitForSingleObject(raw(&self.process), 0) == WAIT_OBJECT_0 }
    }

    fn terminate(&self) -> Result<()> {
        // SAFETY: the process handle is open and has `PROCESS_TERMINATE`
        // access.
        if unsafe { TerminateProcess(raw(&self.process), TERMINATED_EXIT_CODE) } != 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        let pid = u64::from(self.pid);
        Err(match error.raw_os_error() {
            Some(ERROR_ACCESS_DENIED_CODE) => process_denied(pid),
            _ if self.has_ended() => process_gone(pid),
            _ => io_error("end the process", &error),
        })
    }

    fn status(&self, status: NTSTATUS) -> Result<()> {
        if status >= 0 {
            return Ok(());
        }
        Err(Error::new(
            ErrorCode::Io,
            format!(
                "cannot suspend or resume process {}: status {status:#x}",
                self.pid
            ),
        ))
    }
}

/// Creation time of a process as wall time.
fn creation_time(process: &OwnedHandle) -> Option<WallTime> {
    let zero = || FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero(), zero(), zero(), zero());
    // SAFETY: the four pointers refer to writable `FILETIME` values of this
    // frame.
    let read = unsafe {
        GetProcessTimes(
            raw(process),
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    };
    if read == 0 {
        return None;
    }
    let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    let since_epoch = ticks.checked_sub(FILETIME_UNIX_EPOCH)?;
    Some(WallTime::from_unix_nanos(
        i128::from(since_epoch) * i128::from(TIME_UNIT_NANOS),
    ))
}

/// Job object of a helper process started outside process control; every
/// process of the helper ends when the job is closed.
pub(crate) struct ServiceJob {
    job: OwnedHandle,
}

impl ServiceJob {
    /// Places `process`, created with `CREATE_SUSPENDED`, in a new job
    /// object and resumes it, so every process it starts belongs to the job.
    ///
    /// # Errors
    ///
    /// `E_SPAWN` when the job object cannot be created or the process cannot
    /// be assigned to it or resumed; the process is then terminated.
    pub(crate) fn adopt(process: &std::process::Child) -> Result<Self> {
        let handle: HANDLE = process.as_raw_handle();
        let fail = |error: io::Error| {
            // SAFETY: the process handle stays open while `process` lives.
            unsafe { TerminateProcess(handle, TERMINATED_EXIT_CODE) };
            Error::new(
                ErrorCode::Spawn,
                format!("cannot place the helper process in a job object: {error}"),
            )
        };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let job = new_job(&info).map_err(fail)?;
        // SAFETY: both handles are open; the process has not run yet.
        if unsafe { AssignProcessToJobObject(raw(&job), handle) } == 0 {
            return Err(fail(io::Error::last_os_error()));
        }
        // SAFETY: the process handle has full access; resuming the process
        // releases the main thread created suspended.
        let status = unsafe { NtResumeProcess(handle) };
        if status < 0 {
            return Err(fail(io::Error::other(format!("status {status:#x}"))));
        }
        Ok(Self { job })
    }

    /// Ends every process of the job.
    ///
    /// # Errors
    ///
    /// `E_IO` when the job cannot be terminated.
    pub(crate) fn kill(&self) -> Result<()> {
        // SAFETY: the job handle is open for the lifetime of `self`.
        if unsafe { TerminateJobObject(raw(&self.job), TERMINATED_EXIT_CODE) } == 0 {
            return Err(io_error(
                "end the helper process",
                &io::Error::last_os_error(),
            ));
        }
        Ok(())
    }
}

/// Standard handles passed to the program.
#[derive(Clone, Copy)]
struct StdHandles {
    input: HANDLE,
    output: HANDLE,
    error: HANDLE,
}

/// Standard input of the program and, for bytes, the writer of this
/// process.
struct ChildInput {
    child: OwnedHandle,
    writer: Option<OwnedHandle>,
}

impl ChildInput {
    fn open(source: &StdinSource) -> io::Result<Self> {
        match source {
            StdinSource::Null => Ok(Self {
                child: inheritable(File::open("NUL")?.into())?,
                writer: None,
            }),
            StdinSource::File(path) => Ok(Self {
                child: inheritable(File::open(path)?.into())?,
                writer: None,
            }),
            StdinSource::Bytes(_) => {
                let (read, write) = pipe_raw()?;
                Ok(Self {
                    child: inheritable(read)?,
                    writer: Some(write),
                })
            }
        }
    }

    fn child_handle(&self) -> HANDLE {
        raw(&self.child)
    }

    fn writer(self) -> Option<Box<dyn Write + Send>> {
        self.writer
            .map(|handle| Box::new(File::from(handle)) as Box<dyn Write + Send>)
    }
}

/// Pipe whose write end is inherited by the program.
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (read, write) = pipe_raw()?;
    Ok((read, inheritable(write)?))
}

fn pipe_raw() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read: HANDLE = ptr::null_mut();
    let mut write: HANDLE = ptr::null_mut();
    // SAFETY: the output locations are valid; without security attributes
    // neither handle is inheritable.
    if unsafe { CreatePipe(&raw mut read, &raw mut write, ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `CreatePipe` returned two open handles owned by the caller.
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        )
    })
}

/// Marks a handle inheritable. Only handles in the handle list of the
/// program are inherited, so other processes started at the same time do
/// not receive it through this flag alone.
fn inheritable(handle: OwnedHandle) -> io::Result<OwnedHandle> {
    // SAFETY: the handle is open.
    if unsafe { SetHandleInformation(raw(&handle), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(handle)
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle()
}

/// New job object with the limits of `info`.
fn new_job(info: &JOBOBJECT_EXTENDED_LIMIT_INFORMATION) -> io::Result<OwnedHandle> {
    // SAFETY: an unnamed job with default security.
    let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `CreateJobObjectW` returned an open handle owned by the caller.
    let job = unsafe { OwnedHandle::from_raw_handle(handle) };
    set_job(&job, JobObjectExtendedLimitInformation, info)?;
    Ok(job)
}

fn create_job(request: &SpawnRequest) -> io::Result<OwnedHandle> {
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
    if let Some(bytes) = request.limits.max_memory {
        info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
        info.JobMemoryLimit = usize::try_from(bytes).unwrap_or(usize::MAX);
    }
    if let Some(cpu) = request.limits.max_cpu {
        info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_TIME;
        let units = cpu.as_nanos() / u128::from(TIME_UNIT_NANOS);
        info.BasicLimitInformation.PerJobUserTimeLimit = i64::try_from(units).unwrap_or(i64::MAX);
    }
    new_job(&info)
}

fn associate_port(job: &OwnedHandle) -> io::Result<OwnedHandle> {
    // SAFETY: a new port not bound to a file.
    let handle = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, ptr::null_mut(), 0, 1) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `CreateIoCompletionPort` returned an open handle owned by the
    // caller.
    let port = unsafe { OwnedHandle::from_raw_handle(handle) };
    let association = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
        CompletionKey: ptr::null_mut(),
        CompletionPort: raw(&port),
    };
    set_job(
        job,
        JobObjectAssociateCompletionPortInformation,
        &association,
    )?;
    Ok(port)
}

fn set_job<T>(job: &OwnedHandle, class: JOBOBJECTINFOCLASS, info: &T) -> io::Result<()> {
    let size = u32::try_from(std::mem::size_of::<T>()).unwrap_or(u32::MAX);
    // SAFETY: `info` is the structure of `class` and lives for the call.
    let status =
        unsafe { SetInformationJobObject(raw(job), class, ptr::from_ref(info).cast(), size) };
    if status == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn query_job<T>(job: &OwnedHandle, class: JOBOBJECTINFOCLASS, info: &mut T) -> Result<()> {
    let size = u32::try_from(std::mem::size_of::<T>()).unwrap_or(u32::MAX);
    // SAFETY: `info` is the writable structure of `class`.
    let status = unsafe {
        QueryInformationJobObject(
            raw(job),
            class,
            ptr::from_mut(info).cast(),
            size,
            ptr::null_mut(),
        )
    };
    if status == 0 {
        return Err(io_error("measure the program", &io::Error::last_os_error()));
    }
    Ok(())
}

/// Runs `action` on process `pid` when it belongs to `job`. A process that
/// ended after the listing is skipped, and so is an unrelated process that
/// received its reused identifier.
fn with_process(
    job: &OwnedHandle,
    pid: u32,
    action: impl FnOnce(HANDLE) -> NTSTATUS,
) -> Result<()> {
    // SAFETY: plain call; a null result is checked below.
    let handle = unsafe {
        OpenProcess(
            PROCESS_SUSPEND_RESUME | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if handle.is_null() {
        // The process ended after the listing.
        return Ok(());
    }
    // SAFETY: `OpenProcess` returned an open handle owned by the caller.
    let process = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut member = 0;
    // SAFETY: both handles are open and `member` is writable.
    if unsafe { IsProcessInJob(raw(&process), raw(job), &raw mut member) } == 0 {
        return Err(io_error(
            "check the job of a process",
            &io::Error::last_os_error(),
        ));
    }
    if member == 0 {
        return Ok(());
    }
    let status = action(raw(&process));
    if status < 0 {
        return Err(Error::new(
            ErrorCode::Io,
            format!("cannot suspend or resume process {pid}: status {status:#x}"),
        ));
    }
    Ok(())
}

fn create_suspended(
    application: &[u16],
    mut command_line: Vec<u16>,
    environment: &[u16],
    cwd: &[u16],
    handles: StdHandles,
    priority: PROCESS_CREATION_FLAGS,
) -> io::Result<PROCESS_INFORMATION> {
    let mut inherited = vec![handles.input, handles.output];
    if handles.error != handles.output {
        inherited.push(handles.error);
    }
    let mut attributes = AttributeList::with_handles(&inherited)?;
    // SAFETY: `STARTUPINFOEXW` is a plain structure; zero is its empty value.
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = u32::try_from(std::mem::size_of::<STARTUPINFOEXW>()).unwrap_or(0);
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = handles.input;
    startup.StartupInfo.hStdOutput = handles.output;
    startup.StartupInfo.hStdError = handles.error;
    startup.lpAttributeList = attributes.as_ptr();
    let flags = CREATE_SUSPENDED
        | CREATE_NEW_PROCESS_GROUP
        | CREATE_NO_WINDOW
        | CREATE_UNICODE_ENVIRONMENT
        | EXTENDED_STARTUPINFO_PRESENT
        | priority;
    // SAFETY: `PROCESS_INFORMATION` is a plain structure written by the call.
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every string is NUL-terminated and outlives the call, the
    // command line is writable, the attribute list and the handles in it
    // stay valid until the call returns.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            flags,
            environment.as_ptr().cast::<c_void>(),
            cwd.as_ptr(),
            &raw const startup.StartupInfo,
            &raw mut info,
        )
    };
    drop(attributes);
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

/// Thread attribute list with the handles the program inherits.
struct AttributeList<'a> {
    buffer: Vec<usize>,
    handles: &'a [HANDLE],
}

impl<'a> AttributeList<'a> {
    fn with_handles(handles: &'a [HANDLE]) -> io::Result<Self> {
        let mut size: usize = 0;
        // SAFETY: a null list queries the required size; the call reports
        // an insufficient buffer by design.
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &raw mut size) };
        let words = size.div_ceil(std::mem::size_of::<usize>());
        let mut list = Self {
            buffer: vec![0usize; words],
            handles,
        };
        // SAFETY: the buffer holds `size` writable bytes, aligned for the
        // list.
        if unsafe { InitializeProcThreadAttributeList(list.as_ptr(), 1, 0, &raw mut size) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the list is initialized; the handle array outlives the
        // list, which is deleted before the array is released.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                list.as_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                list.handles.as_ptr().cast(),
                std::mem::size_of_val(list.handles),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if updated == 0 {
            let error = io::Error::last_os_error();
            list.buffer.clear();
            return Err(error);
        }
        Ok(list)
    }

    fn as_ptr(&mut self) -> *mut c_void {
        self.buffer.as_mut_ptr().cast()
    }
}

impl Drop for AttributeList<'_> {
    fn drop(&mut self) {
        if !self.buffer.is_empty() {
            // SAFETY: the list was initialized by `with_handles`.
            unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
        }
    }
}

fn priority_class(priority: Priority) -> PROCESS_CREATION_FLAGS {
    match priority {
        Priority::Low => BELOW_NORMAL_PRIORITY_CLASS,
        Priority::Normal => NORMAL_PRIORITY_CLASS,
        Priority::High => ABOVE_NORMAL_PRIORITY_CLASS,
        Priority::Nice(value) if value >= IDLE_NICENESS => IDLE_PRIORITY_CLASS,
        Priority::Nice(value) if value >= BELOW_NORMAL_NICENESS => BELOW_NORMAL_PRIORITY_CLASS,
        Priority::Nice(_) => NORMAL_PRIORITY_CLASS,
    }
}

/// Signals named after the exception codes that end a process.
const FAULTS: [(NTSTATUS, Signal); 7] = [
    (STATUS_ACCESS_VIOLATION, Signal::Segv),
    (STATUS_DATATYPE_MISALIGNMENT, Signal::Bus),
    (STATUS_ILLEGAL_INSTRUCTION, Signal::Ill),
    (STATUS_INTEGER_DIVIDE_BY_ZERO, Signal::Fpe),
    (STATUS_FLOAT_DIVIDE_BY_ZERO, Signal::Fpe),
    (STATUS_STACK_BUFFER_OVERRUN, Signal::Abort),
    (STATUS_CONTROL_C_EXIT, Signal::Interrupt),
];

fn decode_exit_code(code: u32) -> Exit {
    FAULTS
        .iter()
        .find(|(status, _)| status.cast_unsigned() == code)
        .map_or(Exit::Code(code.cast_signed()), |(_, signal)| Exit::Signal {
            signal: *signal,
            number: None,
        })
}

fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut encoded: Vec<u16> = value.encode_wide().collect();
    if encoded.contains(&0) {
        return Err(nul_error());
    }
    encoded.push(0);
    Ok(encoded)
}

fn nul_error() -> Error {
    Error::new(
        ErrorCode::Arg,
        "the command, the working directory or the environment contains a NUL character",
    )
    .with_problem("remove the NUL character")
}

/// Command line of the program: the prepared line of the command
/// interpreter, or the program and the arguments quoted for
/// `CommandLineToArgvW` and the C runtime.
fn command_line(request: &SpawnRequest) -> Result<Vec<u16>> {
    if let Some(line) = &request.raw_command_line {
        return wide(line);
    }
    let mut line: Vec<u16> = Vec::new();
    append_quoted(&mut line, request.program.as_os_str(), true);
    for argument in &request.args {
        line.push(u16::from(b' '));
        append_quoted(&mut line, argument, false);
    }
    if line.contains(&0) {
        return Err(nul_error());
    }
    line.push(0);
    Ok(line)
}

fn append_quoted(line: &mut Vec<u16>, argument: &OsStr, always: bool) {
    let units: Vec<u16> = argument.encode_wide().collect();
    let needs_quotes = always
        || units.is_empty()
        || units
            .iter()
            .any(|unit| matches!(*unit, 0x20 | 0x09 | 0x0A | 0x0B | 0x22));
    if !needs_quotes {
        line.extend_from_slice(&units);
        return;
    }
    let backslash = u16::from(b'\\');
    let quote = u16::from(b'"');
    line.push(quote);
    let mut backslashes = 0usize;
    for unit in units {
        if unit == backslash {
            backslashes += 1;
            continue;
        }
        if unit == quote {
            line.extend(std::iter::repeat_n(backslash, backslashes * 2 + 1));
        } else {
            line.extend(std::iter::repeat_n(backslash, backslashes));
        }
        backslashes = 0;
        line.push(unit);
    }
    line.extend(std::iter::repeat_n(backslash, backslashes * 2));
    line.push(quote);
}

/// Environment block: `NAME=value` entries sorted by name without regard
/// to case, each NUL-terminated, followed by one more NUL.
fn environment_block(env: &[(std::ffi::OsString, std::ffi::OsString)]) -> Result<Vec<u16>> {
    let mut entries: Vec<(String, Vec<u16>)> = Vec::with_capacity(env.len());
    for (name, value) in env {
        let mut entry: Vec<u16> = name.encode_wide().collect();
        entry.push(u16::from(b'='));
        entry.extend(value.encode_wide());
        if entry.contains(&0) {
            return Err(nul_error());
        }
        entries.push((name.to_string_lossy().to_uppercase(), entry));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut block: Vec<u16> = Vec::new();
    for (_, entry) in entries {
        block.extend(entry);
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quoted(argument: &str) -> String {
        let mut line = Vec::new();
        append_quoted(&mut line, OsStr::new(argument), false);
        String::from_utf16_lossy(&line)
    }

    #[test]
    fn plain_arguments_are_not_quoted() {
        assert_eq!(quoted("plain"), "plain");
        assert_eq!(quoted(r"C:\dir\file"), r"C:\dir\file");
    }

    #[test]
    fn arguments_with_spaces_and_quotes_are_quoted() {
        assert_eq!(quoted("two words"), "\"two words\"");
        assert_eq!(quoted(""), "\"\"");
        assert_eq!(quoted("say \"x\""), r#""say \"x\"""#);
        assert_eq!(quoted(r"dir\ end\"), r#""dir\ end\\""#);
    }

    #[test]
    fn environment_block_is_sorted_and_terminated() {
        let env = vec![("b".into(), "2".into()), ("A".into(), "1".into())];
        let block = environment_block(&env).unwrap_or_default();
        assert_eq!(String::from_utf16_lossy(&block), "A=1\0b=2\0\0");
    }

    #[test]
    fn faults_map_to_signal_names() {
        assert_eq!(
            decode_exit_code(STATUS_ACCESS_VIOLATION.cast_unsigned()),
            Exit::Signal {
                signal: Signal::Segv,
                number: None
            }
        );
        assert_eq!(decode_exit_code(3), Exit::Code(3));
    }

    #[test]
    fn a_process_outside_the_job_is_left_alone() {
        let job = new_job(&JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default()).unwrap();
        let mut touched = false;
        with_process(&job, std::process::id(), |_| {
            touched = true;
            0
        })
        .unwrap();
        assert!(!touched);
    }

    #[test]
    fn a_process_that_ended_is_skipped() {
        let job = new_job(&JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default()).unwrap();
        let mut touched = false;
        with_process(&job, u32::MAX, |_| {
            touched = true;
            0
        })
        .unwrap();
        assert!(!touched);
    }

    #[test]
    fn exit_codes_that_are_not_faults_stay_codes() {
        assert_eq!(decode_exit_code(0), Exit::Code(0));
        assert_eq!(
            decode_exit_code(STATUS_CONTROL_C_EXIT.cast_unsigned()),
            Exit::Signal {
                signal: Signal::Interrupt,
                number: None
            }
        );
        assert_eq!(decode_exit_code(u32::MAX), Exit::Code(-1));
    }

    #[test]
    fn arguments_with_trailing_backslashes_are_doubled_before_the_closing_quote() {
        assert_eq!(quoted(r"a b\\"), r#""a b\\\\""#);
        assert_eq!(quoted("tab\there"), "\"tab\there\"");
    }

    #[test]
    fn a_nul_character_is_refused() {
        assert!(wide(OsStr::new("a\0b")).is_err());
        assert_eq!(wide(OsStr::new("ab")).unwrap_or_default(), vec![97, 98, 0]);
        let env = vec![("A".into(), "x\0y".into())];
        assert!(environment_block(&env).is_err());
    }

    #[test]
    fn an_empty_environment_is_two_terminators() {
        assert_eq!(environment_block(&[]).unwrap_or_default(), vec![0, 0]);
    }

    #[test]
    fn a_process_is_opened_only_with_its_own_start_time() {
        let pid = std::process::id();
        // SAFETY: plain call; the result is checked below.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        assert!(!handle.is_null());
        // SAFETY: `OpenProcess` returned an open handle owned by the test.
        let own = unsafe { OwnedHandle::from_raw_handle(handle) };
        let started = creation_time(&own).unwrap();
        let target = Target::open(u64::from(pid), started).unwrap();
        assert!(!target.has_ended());
        let earlier = WallTime::from_unix_nanos(started.unix_nanos() - 3_600_000_000_000);
        let error = Target::open(u64::from(pid), earlier).err().unwrap();
        assert_eq!(error.code(), ErrorCode::Stale);
    }

    #[test]
    fn an_identifier_that_names_no_process_is_not_found() {
        for pid in [0, u64::from(u32::MAX), u64::MAX] {
            let error = Target::open(pid, WallTime::UNIX_EPOCH).err().unwrap();
            assert_eq!(error.code(), ErrorCode::NotFound);
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
