// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::priority
// Responsibility:   Scheduling and I/O priority of service threads.
// Dependencies:     windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Scheduling and I/O priority of service threads.
//!
//! The space module releases journal data in the background below the soft
//! threshold and at full speed in the hard state. The change applies to the
//! calling thread only, so the threads of the control channel keep their
//! priority and `ping` stays within its time limit.
//!
//! | System | Background | Normal |
//! |---|---|---|
//! | Linux | `ioprio_set(IOPRIO_WHO_PROCESS, 0, IOPRIO_CLASS_IDLE)`, `sched_setscheduler(0, SCHED_IDLE)` | `IOPRIO_CLASS_BE` level 4, `SCHED_OTHER` |
//! | macOS | `setpriority(PRIO_DARWIN_THREAD, 0, PRIO_DARWIN_BG)` | `setpriority(PRIO_DARWIN_THREAD, 0, 0)` |
//! | Windows | `SetThreadPriority(THREAD_MODE_BACKGROUND_BEGIN)` | `SetThreadPriority(THREAD_MODE_BACKGROUND_END)` |
//!
//! On Linux both calls with identifier `0` act on the calling thread.

use std::io;

/// Priority requested by a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PriorityMode {
    /// Lowest CPU and I/O priority (soft threshold).
    Background,
    /// Normal priority (hard and emergency states).
    Normal,
}

/// Applies the priority to the calling thread.
///
/// # Errors
///
/// Returns the operating system error, for example when leaving
/// `SCHED_IDLE` is not permitted by `RLIMIT_NICE`.
pub fn set_current_thread(mode: PriorityMode) -> io::Result<()> {
    imp::set_current_thread(mode)
}

#[cfg(target_os = "linux")]
mod imp {
    #![allow(
        unsafe_code,
        reason = "ioprio_set has no libc wrapper and sched_setscheduler is a raw libc call"
    )]

    use super::PriorityMode;
    use std::io;

    const IOPRIO_CLASS_SHIFT: libc::c_int = 13;
    const IOPRIO_CLASS_BE: libc::c_int = 2;
    const IOPRIO_CLASS_IDLE: libc::c_int = 3;
    const IOPRIO_WHO_PROCESS: libc::c_int = 1;
    const IOPRIO_BE_DEFAULT_LEVEL: libc::c_int = 4;

    const fn ioprio_value(class: libc::c_int, level: libc::c_int) -> libc::c_int {
        (class << IOPRIO_CLASS_SHIFT) | level
    }

    pub(super) fn set_current_thread(mode: PriorityMode) -> io::Result<()> {
        let (io_class, io_level, policy) = match mode {
            PriorityMode::Background => (IOPRIO_CLASS_IDLE, 0, libc::SCHED_IDLE),
            PriorityMode::Normal => (IOPRIO_CLASS_BE, IOPRIO_BE_DEFAULT_LEVEL, libc::SCHED_OTHER),
        };
        // SAFETY: `ioprio_set` takes three integers and touches no memory of
        // the caller.
        let result = unsafe {
            libc::syscall(
                libc::SYS_ioprio_set,
                IOPRIO_WHO_PROCESS,
                0,
                ioprio_value(io_class, io_level),
            )
        };
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `sched_param` is a plain C structure for which all zero
        // bytes are valid; the priority 0 is the one the policies require, and
        // musl has more fields in the structure than glibc.
        let param: libc::sched_param = unsafe { std::mem::zeroed() };
        // SAFETY: `param` is a valid `sched_param` that outlives the call.
        if unsafe { libc::sched_setscheduler(0, policy, &param) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    #![allow(unsafe_code, reason = "setpriority is a raw libc call")]

    use super::PriorityMode;
    use std::io;

    pub(super) fn set_current_thread(mode: PriorityMode) -> io::Result<()> {
        let value = match mode {
            PriorityMode::Background => libc::PRIO_DARWIN_BG,
            PriorityMode::Normal => 0,
        };
        // SAFETY: `setpriority` takes integers only.
        if unsafe { libc::setpriority(libc::PRIO_DARWIN_THREAD, 0, value) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    #![allow(unsafe_code, reason = "SetThreadPriority is a Win32 call")]

    use super::PriorityMode;
    use std::io;
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        THREAD_MODE_BACKGROUND_END,
    };

    pub(super) fn set_current_thread(mode: PriorityMode) -> io::Result<()> {
        let value = match mode {
            PriorityMode::Background => THREAD_MODE_BACKGROUND_BEGIN,
            PriorityMode::Normal => THREAD_MODE_BACKGROUND_END,
        };
        // SAFETY: the pseudo handle of the current thread is always valid.
        if unsafe { SetThreadPriority(GetCurrentThread(), value) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    use super::PriorityMode;
    use std::io;

    pub(super) fn set_current_thread(_mode: PriorityMode) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn background_priority_applies_to_calling_thread() {
        let outcome = std::thread::spawn(|| {
            set_current_thread(PriorityMode::Background)?;
            set_current_thread(PriorityMode::Normal)
        })
        .join()
        .unwrap();
        match outcome {
            Ok(()) => {}
            Err(error) => assert_eq!(error.raw_os_error(), Some(libc::EPERM)),
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
