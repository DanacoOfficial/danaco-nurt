// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/env_host.rs
// Responsibility:   Serialization of the facts about the host.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Serialization of the facts about the host.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::path::PathBuf;

use nurt_contracts::env::{HostInfo, Os, PathStyle};

fn info(home: PathBuf, temp: PathBuf) -> HostInfo {
    HostInfo {
        os: Os::Linux,
        os_version: "6.8.0".to_owned(),
        architecture: "x86_64".to_owned(),
        hostname: "host".to_owned(),
        user: "user".to_owned(),
        home,
        temp,
        path_style: PathStyle::Posix,
        timezone: "UTC".to_owned(),
        locale: None,
        cpus: 4,
    }
}

#[test]
fn paths_of_the_host_round_trip_as_text() {
    let host = info(PathBuf::from("/home/user"), PathBuf::from("/tmp"));
    let json = serde_json::to_value(&host).unwrap();
    assert_eq!(json["home"], "/home/user");
    assert_eq!(json["temp"], "/tmp");
    assert_eq!(serde_json::from_value::<HostInfo>(json).unwrap(), host);
}

#[cfg(unix)]
#[test]
fn a_home_that_is_not_unicode_is_serialized_lossily() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let home = PathBuf::from(OsString::from_vec(b"/home/\xff\xfe".to_vec()));
    let host = info(home, PathBuf::from("/tmp"));
    let json = serde_json::to_value(&host).unwrap();
    assert_eq!(json["home"], "/home/\u{fffd}\u{fffd}");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
