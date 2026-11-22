// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Facts about processes and the account: liveness of a process
//! identifier, administrator rights and, on Windows, the account SID.

/// Whether a process with the identifier exists.
///
/// A process owned by another account counts as existing.
#[must_use]
pub fn pid_alive(pid: u32) -> bool {
    imp::pid_alive(pid)
}

/// Whether the current process runs with administrator rights (`root` on
/// Unix, an elevated token on Windows).
#[must_use]
pub fn is_elevated() -> bool {
    imp::is_elevated()
}

#[cfg(unix)]
mod imp {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::{Pid, geteuid};

    pub(super) fn pid_alive(pid: u32) -> bool {
        let Ok(raw) = i32::try_from(pid) else {
            return false;
        };
        if raw <= 0 {
            return false;
        }
        match kill(Pid::from_raw(raw), None) {
            Ok(()) | Err(Errno::EPERM) => true,
            Err(_) => false,
        }
    }

    pub(super) fn is_elevated() -> bool {
        geteuid().is_root()
    }
}

#[cfg(windows)]
pub(crate) use imp::current_user_sid;

#[cfg(windows)]
mod imp {
    #![allow(
        unsafe_code,
        reason = "process and token queries are Win32 calls without safe wrappers"
    )]

    use std::ffi::c_void;
    use std::io;
    use std::mem;
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, GetLastError, HANDLE, LocalFree, STILL_ACTIVE,
    };
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    const TOKEN_USER_BUFFER: usize = 256;

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: the handle was returned by a successful Win32 call and
            // is closed exactly once here.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub(super) fn pid_alive(pid: u32) -> bool {
        // SAFETY: plain call with constant access rights; the returned
        // handle is checked before use.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            // SAFETY: reads the error code of the failed call above.
            return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
        }
        let handle = OwnedHandle(handle);
        let mut code = 0_u32;
        // SAFETY: `handle` is a valid process handle and `code` outlives
        // the call.
        let ok = unsafe { GetExitCodeProcess(handle.0, &mut code) };
        ok != 0 && i32::try_from(code).is_ok_and(|code| code == STILL_ACTIVE)
    }

    fn process_token() -> io::Result<OwnedHandle> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: the pseudo handle of the current process needs no closing;
        // `token` receives a new handle owned by `OwnedHandle`.
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(OwnedHandle(token))
    }

    pub(super) fn is_elevated() -> bool {
        let Ok(token) = process_token() else {
            return false;
        };
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0_u32;
        let size = u32::try_from(mem::size_of::<TOKEN_ELEVATION>()).unwrap_or(0);
        // SAFETY: the buffer is a `TOKEN_ELEVATION` of the declared size and
        // outlives the call.
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenElevation,
                (&raw mut elevation).cast::<c_void>(),
                size,
                &mut returned,
            )
        };
        ok != 0 && elevation.TokenIsElevated != 0
    }

    /// SID of the account running the process, in string form
    /// (`S-1-5-21-…`).
    pub(crate) fn current_user_sid() -> io::Result<String> {
        let token = process_token()?;
        let mut buffer = [0_u64; TOKEN_USER_BUFFER];
        let size = u32::try_from(mem::size_of_val(&buffer)).unwrap_or(0);
        let mut returned = 0_u32;
        // SAFETY: the buffer is 8-byte aligned, as `TOKEN_USER` requires, and
        // its size is passed; it outlives the call.
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast::<c_void>(),
                size,
                &mut returned,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: on success the buffer starts with a `TOKEN_USER` whose SID
        // points into the same buffer.
        let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        let mut text: *mut u16 = ptr::null_mut();
        // SAFETY: `sid` is valid while `buffer` lives; `text` receives a
        // buffer allocated by the system and released with `LocalFree`.
        let ok = unsafe { ConvertSidToStringSidW(sid, &mut text) };
        if ok == 0 || text.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut length = 0_usize;
        // SAFETY: `text` is a NUL-terminated wide string from the system.
        while unsafe { *text.add(length) } != 0 {
            length += 1;
        }
        // SAFETY: `length` wide characters precede the terminator.
        let wide = unsafe { std::slice::from_raw_parts(text, length) };
        let sid_text = String::from_utf16_lossy(wide);
        // SAFETY: releases the buffer allocated by `ConvertSidToStringSidW`.
        unsafe { LocalFree(text.cast::<c_void>()) };
        Ok(sid_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_is_alive_and_unused_pid_is_not() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(0));
        assert!(!pid_alive(u32::MAX));
    }

    #[test]
    fn elevation_matches_effective_user() {
        #[cfg(unix)]
        assert_eq!(is_elevated(), nix::unistd::geteuid().is_root());
        #[cfg(not(unix))]
        let _ = is_elevated();
    }
}
