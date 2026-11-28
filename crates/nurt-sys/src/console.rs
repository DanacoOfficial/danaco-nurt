// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::console
// Responsibility:   The console of the running process.
// Dependencies:     windows_sys
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The console of the running process.
//!
//! The functions report a property of the console that the process is
//! attached to. They read the state of the process and change nothing, except
//! [`enable_virtual_terminal`], which turns on the processing of the escape
//! sequences of the output of the console.
//!
//! [`is_private`] tells whether the window of the console closes when the
//! process ends.

/// Code page that encodes text as UTF-8.
const UTF8_CODE_PAGE: u32 = 65001;

/// Whether a code page is the UTF-8 code page.
#[must_use]
pub const fn is_utf8_code_page(code_page: u32) -> bool {
    code_page == UTF8_CODE_PAGE
}

/// Whether the output code page of the console of this process is UTF-8
/// (code page 65001). On a system other than Windows, and on Windows when the
/// process has no console, the answer is `false`.
#[must_use]
pub fn output_is_utf8() -> bool {
    is_utf8_code_page(output_code_page())
}

/// Turns on the processing of the escape sequences (colours, cursor) in the
/// output of the console of this process and tells whether it is on.
///
/// A console of Windows writes the bytes of an escape sequence as text until
/// a process turns the processing on; Windows Terminal has it on from the
/// start. The standard output and the standard error output are handled one
/// by one, and the answer is `true` when the processing is on for at least one
/// of them. It is `false` when neither of them is a console and on a system
/// other than Windows.
#[must_use]
pub fn enable_virtual_terminal() -> bool {
    virtual_terminal()
}

/// Whether the console of this process processes the escape sequences of the
/// output now. Unlike [`enable_virtual_terminal`] it changes nothing. The
/// answer is `false` when the standard output is not a console and on a system
/// other than Windows.
#[must_use]
pub fn virtual_terminal_is_on() -> bool {
    virtual_terminal_state()
}

/// Whether this process is the only one attached to its console.
///
/// The system creates such a console for a program started from a shortcut,
/// by a double click or as the program of a tab of a terminal, and closes its
/// window when the process ends. A program started from the prompt of a shell
/// shares the console with the shell. The answer is `false` when the process
/// has no console and on a system other than Windows.
#[must_use]
pub fn is_private() -> bool {
    private_console()
}

#[cfg(windows)]
fn private_console() -> bool {
    use windows_sys::Win32::System::Console::GetConsoleProcessList;

    let mut attached = [0u32; 2];
    // SAFETY: `attached` holds two identifiers and the call is told that it
    // holds two; it writes no more than that. It returns the number of the
    // processes attached to the console, which may exceed two, or 0 when the
    // process has no console.
    let count = unsafe { GetConsoleProcessList(attached.as_mut_ptr(), 2) };
    count == 1
}

#[cfg(not(windows))]
fn private_console() -> bool {
    false
}

/// The output code page of the console, or 0 when there is none.
#[cfg(windows)]
fn output_code_page() -> u32 {
    use windows_sys::Win32::System::Console::GetConsoleOutputCP;

    // SAFETY: GetConsoleOutputCP takes no arguments and reads the console
    // state of the calling process; it returns 0 when there is no console.
    unsafe { GetConsoleOutputCP() }
}

#[cfg(not(windows))]
fn output_code_page() -> u32 {
    0
}

/// The handle of a standard stream of the process (`stream` is one of the
/// `STD_*_HANDLE` constants) and its console mode, or `None` when the stream is
/// not a console.
#[cfg(windows)]
fn console_of(stream: u32) -> Option<(windows_sys::Win32::Foundation::HANDLE, u32)> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{GetConsoleMode, GetStdHandle};

    // SAFETY: GetStdHandle takes a constant and returns the handle that the
    // process owns, a null handle or INVALID_HANDLE_VALUE; it is not closed here.
    let handle = unsafe { GetStdHandle(stream) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut mode = 0u32;
    // SAFETY: `handle` is not null and not invalid, and `mode` is a live
    // variable that receives the mode; the call fails for a handle that is not
    // a console and then writes nothing.
    if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
        return None;
    }
    Some((handle, mode))
}

/// Turns the processing of the escape sequences on for one standard stream and
/// tells whether it is on.
#[cfg(windows)]
fn enable_on(stream: u32) -> bool {
    use windows_sys::Win32::System::Console::{
        ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, SetConsoleMode,
    };

    let Some((handle, mode)) = console_of(stream) else {
        return false;
    };
    if mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 {
        return true;
    }
    // SAFETY: `handle` is the handle of a console, as `console_of` showed; the
    // call changes the mode of its screen buffer only.
    unsafe {
        SetConsoleMode(
            handle,
            mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        ) != 0
    }
}

#[cfg(windows)]
fn virtual_terminal() -> bool {
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};

    let output = enable_on(STD_OUTPUT_HANDLE);
    let error = enable_on(STD_ERROR_HANDLE);
    output || error
}

#[cfg(windows)]
fn virtual_terminal_state() -> bool {
    use windows_sys::Win32::System::Console::{
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE,
    };

    console_of(STD_OUTPUT_HANDLE)
        .is_some_and(|(_, mode)| mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0)
}

#[cfg(not(windows))]
fn virtual_terminal() -> bool {
    false
}

#[cfg(not(windows))]
fn virtual_terminal_state() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_code_page_65001_is_utf8() {
        assert!(is_utf8_code_page(65001));
        for other in [0, 437, 852, 1250, 1252, 65000] {
            assert!(!is_utf8_code_page(other), "{other}");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn a_system_other_than_windows_has_no_console_code_page() {
        assert!(!output_is_utf8());
    }

    #[cfg(not(windows))]
    #[test]
    fn a_system_other_than_windows_has_no_virtual_terminal_to_turn_on() {
        assert!(!enable_virtual_terminal());
        assert!(!virtual_terminal_is_on());
    }

    #[cfg(not(windows))]
    #[test]
    fn a_system_other_than_windows_has_no_private_console() {
        assert!(!is_private());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
