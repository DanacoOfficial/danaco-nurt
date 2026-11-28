// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::error_mode
// Responsibility:   Error dialogs of the operating system for device failures.
// Dependencies:     windows_sys
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Error dialogs of the operating system for device failures.
//!
//! Windows answers some failures of a device with a dialog of its own before
//! the call of the program returns. A read of a drive without a medium, such
//! as an empty card reader or optical drive, shows "There is no disk in the
//! drive" and waits for a person. A program of the command line, which may
//! run without a person, has to receive the error instead.

/// Tells the system to return the critical errors of devices to this process
/// instead of showing a dialog.
///
/// The setting holds for the whole process and is inherited by the programs
/// the process starts. The flags that the process inherited are kept. On a
/// system other than Windows nothing is done.
pub fn return_device_errors() {
    set_mode();
}

#[cfg(windows)]
fn set_mode() {
    use windows_sys::Win32::System::Diagnostics::Debug::{SEM_FAILCRITICALERRORS, SetErrorMode};

    // SAFETY: SetErrorMode takes the flags by value, has no pointer arguments
    // and returns the flags that were set before.
    let inherited = unsafe { SetErrorMode(SEM_FAILCRITICALERRORS) };
    // SAFETY: as above; this call adds the flag to the inherited flags, which
    // the first call replaced.
    unsafe {
        SetErrorMode(inherited | SEM_FAILCRITICALERRORS);
    }
}

#[cfg(not(windows))]
fn set_mode() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_can_be_applied_more_than_once() {
        return_device_errors();
        return_device_errors();
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
