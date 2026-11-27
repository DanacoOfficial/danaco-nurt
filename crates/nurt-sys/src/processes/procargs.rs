// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::processes::procargs
// Responsibility:   Decoder of the argument area returned by `sysctl(KERN_PROCARGS2)` on macOS.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Decoder of the argument area returned by `sysctl(KERN_PROCARGS2)` on
//! macOS.
//!
//! The area starts with the argument count as a native `int`, followed by the
//! path of the executable, zero bytes that align the next string, the
//! arguments as zero-terminated strings, and then the environment.

/// Size of the argument count at the start of the area.
const ARGC_SIZE: usize = 4;

/// Arguments in the area; `None` when the area is shorter than its argument
/// count states.
pub(crate) fn decode(area: &[u8]) -> Option<Vec<String>> {
    let count_bytes: [u8; ARGC_SIZE] = area.get(..ARGC_SIZE)?.try_into().ok()?;
    let count = usize::try_from(i32::from_ne_bytes(count_bytes)).ok()?;
    let rest = area.get(ARGC_SIZE..)?;
    let path_end = rest.iter().position(|&byte| byte == 0)?;
    let rest = rest.get(path_end..)?;
    let start = rest.iter().position(|&byte| byte != 0)?;
    let mut strings = rest.get(start..)?.split(|&byte| byte == 0);
    let mut arguments = Vec::with_capacity(count);
    for _ in 0..count {
        let argument = strings.next()?;
        arguments.push(String::from_utf8_lossy(argument).into_owned());
    }
    Some(arguments)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
