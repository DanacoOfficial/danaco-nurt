// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/sys.rs
// Responsibility:   Calls of the Windows API: the only module with unsafe code.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Calls of the Windows API.
//!
//! Every function wraps one call and states why it is sound. Nothing else in
//! the program uses `unsafe`.

#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::fs::File;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, HWND};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DwmSetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::GetSysColor;
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, IsWow64Process2, OpenProcessToken,
    WaitForSingleObject,
};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::Shell::{
    BROWSEINFOW, SEE_MASK_NOCLOSEPROCESS, SHBrowseForFolderW, SHELLEXECUTEINFOW,
    SHGetPathFromIDListW, ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, HWND_BROADCAST, MB_ICONERROR, MB_OK, MessageBoxW, SMTO_ABORTIFHUNG,
    SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SW_SHOWNORMAL, SendMessageTimeoutW,
    SendMessageW, SystemParametersInfoW, WM_SETTINGCHANGE,
};

/// Machine architecture reported by the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Machine {
    /// x86-64.
    X64,
    /// ARM64.
    Arm64,
    /// Anything else.
    Other,
}

/// Colours of the system for the high-contrast mode, as red, green and blue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SystemColors {
    /// `COLOR_WINDOW`.
    pub(crate) window: [u8; 3],
    /// `COLOR_WINDOWTEXT`.
    pub(crate) window_text: [u8; 3],
    /// `COLOR_HIGHLIGHT`.
    pub(crate) highlight: [u8; 3],
    /// `COLOR_HIGHLIGHTTEXT`.
    pub(crate) highlight_text: [u8; 3],
    /// `COLOR_BTNFACE`.
    pub(crate) button_face: [u8; 3],
    /// `COLOR_GRAYTEXT`.
    pub(crate) gray_text: [u8; 3],
}

/// Wide string with the terminating zero.
pub(crate) fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// The process runs with an elevated token.
pub(crate) fn is_elevated() -> bool {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo handle of the current process is valid; `token` receives a handle that is closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut length = 0u32;
    // SAFETY: the buffer is a TOKEN_ELEVATION and its size is passed; `token` is a valid handle opened above.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&raw mut elevation).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &raw mut length,
        )
    };
    // SAFETY: `token` was opened above and is closed once.
    unsafe { CloseHandle(token) };
    ok != 0 && elevation.TokenIsElevated != 0
}

/// Architecture of the machine, also when the process is emulated.
pub(crate) fn native_machine() -> Machine {
    let mut process = 0u16;
    let mut native = 0u16;
    // SAFETY: both out parameters point to live variables; the handle is the pseudo handle of this process.
    if unsafe { IsWow64Process2(GetCurrentProcess(), &raw mut process, &raw mut native) } == 0 {
        return Machine::Other;
    }
    match native {
        0x8664 => Machine::X64,
        0xAA64 => Machine::Arm64,
        _ => Machine::Other,
    }
}

/// Free space for the user on the volume of a directory, in bytes.
pub(crate) fn free_space(directory: &Path) -> Option<u64> {
    let name = wide(directory);
    let mut available = 0u64;
    let mut total = 0u64;
    let mut free = 0u64;
    // SAFETY: `name` is a zero-terminated wide string and the three out parameters are live variables.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            name.as_ptr(),
            &raw mut available,
            &raw mut total,
            &raw mut free,
        )
    };
    (ok != 0).then_some(available)
}

/// Tells the running programs that the environment changed, so that new terminals read `PATH`.
pub(crate) fn broadcast_environment_change() {
    let area = wide("Environment");
    let mut result = 0usize;
    // SAFETY: HWND_BROADCAST is valid for this message; `area` outlives the call and the message is sent with a timeout.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            area.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            &raw mut result,
        );
    }
}

/// Starts a program with the verb `runas` (the elevation prompt of the system).
///
/// With `wait` the call returns the exit code of the program, otherwise `None`.
/// The error is the code of `GetLastError`; 1223 means that the user declined.
pub(crate) fn run_elevated(
    program: &Path,
    parameters: &str,
    wait: bool,
) -> Result<Option<u32>, u32> {
    let verb = wide("runas");
    let file = wide(program);
    let arguments = wide(parameters);
    // SAFETY: SHELLEXECUTEINFOW is a plain C structure for which all-zero is a valid initial state.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = if wait { SEE_MASK_NOCLOSEPROCESS } else { 0 };
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = arguments.as_ptr();
    info.nShow = SW_SHOWNORMAL;
    // SAFETY: all string pointers refer to zero-terminated buffers that live until the call returns.
    if unsafe { ShellExecuteExW(&raw mut info) } == 0 {
        // SAFETY: reads the error code of the calling thread.
        return Err(unsafe { GetLastError() });
    }
    if !wait || info.hProcess.is_null() {
        return Ok(None);
    }
    let mut code = 1u32;
    // SAFETY: `hProcess` is the handle returned by the call above (SEE_MASK_NOCLOSEPROCESS); it is closed once.
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        GetExitCodeProcess(info.hProcess, &raw mut code);
        CloseHandle(info.hProcess);
    }
    Ok(Some(code))
}

/// Connects the process to the console of its parent and opens it for writing.
///
/// The program has the Windows subsystem, so a shell shows no output of it
/// unless the console is attached explicitly.
pub(crate) fn parent_console() -> Option<File> {
    // SAFETY: AttachConsole takes a process identifier constant and has no pointer arguments.
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return None;
    }
    std::fs::OpenOptions::new().write(true).open("CONOUT$").ok()
}

/// Shows a message box with an error; used when the window cannot start.
pub(crate) fn error_box(title: &str, text: &str) {
    let title = wide(title);
    let text = wide(text);
    // SAFETY: both strings are zero-terminated and live until the call returns; the owner window is none.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// The user asked Windows to show animations ("Show animations in Windows").
pub(crate) fn animations_enabled() -> bool {
    let mut enabled: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the buffer that is passed.
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0)
    };
    ok == 0 || enabled != 0
}

/// Colours of the system when the high-contrast mode is on; `None` otherwise.
pub(crate) fn high_contrast() -> Option<SystemColors> {
    let mut info: HIGHCONTRASTW = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        dwFlags: 0,
        lpszDefaultScheme: std::ptr::null_mut(),
    };
    // SAFETY: SPI_GETHIGHCONTRAST fills the structure whose size is given in `cbSize`.
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETHIGHCONTRAST, info.cbSize, (&raw mut info).cast(), 0)
    };
    if ok == 0 || info.dwFlags & HCF_HIGHCONTRASTON == 0 {
        return None;
    }
    let color = |index: i32| {
        // SAFETY: GetSysColor takes an index of the system colours and has no pointer arguments.
        let value = unsafe { GetSysColor(index) };
        [
            (value & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            ((value >> 16) & 0xff) as u8,
        ]
    };
    Some(SystemColors {
        window: color(5),
        window_text: color(8),
        highlight: color(13),
        highlight_text: color(14),
        button_face: color(15),
        gray_text: color(17),
    })
}

/// Rounds the corners of the window of the given title (Windows 11); a quiet no-op elsewhere.
pub(crate) fn round_window_corners(title: &str) {
    let name = wide(title);
    // SAFETY: the class name is null (any class) and `name` is a zero-terminated wide string that outlives the call.
    let window: HWND = unsafe { FindWindowW(std::ptr::null(), name.as_ptr()) };
    if window.is_null() {
        return;
    }
    let preference: u32 = 2;
    // SAFETY: `window` is a valid window handle of this process and the attribute value is a u32 of the size that is passed.
    unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&raw const preference).cast(),
            std::mem::size_of::<u32>() as u32,
        );
    }
}

/// Callback of the folder picker: selects the folder that the caller named when the dialog opens.
unsafe extern "system" fn pick_callback(
    window: HWND,
    message: u32,
    _parameter: isize,
    data: isize,
) -> i32 {
    const BFFM_INITIALIZED: u32 = 1;
    const BFFM_SETSELECTIONW: u32 = 0x467;
    if message == BFFM_INITIALIZED && data != 0 {
        // SAFETY: `window` is the dialog that sent the message and `data` points to the zero-terminated path that `pick_folder` keeps alive.
        unsafe { SendMessageW(window, BFFM_SETSELECTIONW, 1, data) };
    }
    0
}

/// Opens the system dialog that picks a folder; `start` is the folder that the dialog selects first.
pub(crate) fn pick_folder(title: &str, start: &str) -> Option<std::path::PathBuf> {
    // SAFETY: initialises COM for this thread; a failure with a different threading model is harmless for the dialog.
    unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    let title = wide(title);
    let start = wide(start);
    let mut name = [0u16; 260];
    let info = BROWSEINFOW {
        hwndOwner: std::ptr::null_mut(),
        pidlRoot: std::ptr::null_mut(),
        pszDisplayName: name.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: 0x0000_0001 | 0x0000_0040,
        lpfn: Some(pick_callback),
        lParam: start.as_ptr() as isize,
        iImage: 0,
    };
    // SAFETY: `info` points to buffers that live until the call returns; the list that comes back is freed once below.
    let list = unsafe { SHBrowseForFolderW(&info) };
    if list.is_null() {
        return None;
    }
    let mut path = [0u16; 1024];
    // SAFETY: `list` is a valid item list returned above and `path` has the length that the call needs (MAX_PATH or more).
    let ok = unsafe { SHGetPathFromIDListW(list, path.as_mut_ptr()) };
    // SAFETY: the item list was allocated by the shell and is freed once.
    unsafe { CoTaskMemFree(list.cast()) };
    if ok == 0 {
        return None;
    }
    let length = path
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(path.len());
    Some(std::path::PathBuf::from(String::from_utf16_lossy(
        &path[..length],
    )))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
