// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Build script of the C library.
//!
//! On Linux it gives the shared library the SONAME `libnurt.so.<ABI major>`,
//! so that programs linked against it record the name that the dynamic linker
//! resolves, whichever file name the package installs.

use std::io::Write;

/// Major version of the C interface; part of the SONAME on Linux.
const ABI_MAJOR: u32 = 1;

fn main() -> Result<(), String> {
    let mut out = std::io::stdout();
    writeln!(out, "cargo:rerun-if-changed=build.rs").map_err(|error| error.to_string())?;
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "linux") {
        writeln!(
            out,
            "cargo:rustc-cdylib-link-arg=-Wl,-soname,libnurt.so.{ABI_MAJOR}"
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}
