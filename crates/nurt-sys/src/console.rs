// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The console of the running process.
//!
//! The functions report a property of the console that the process is
//! attached to. They read the state of the process and change nothing.

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
}
