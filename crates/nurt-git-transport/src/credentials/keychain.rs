// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::credentials::keychain
// Responsibility:   Internet passwords of the macOS keychain, as stored by.
// Dependencies:     security_framework
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Internet passwords of the macOS keychain, as stored by
//! `git-credential-osxkeychain`: server, account, path, port and protocol.
//!
//! The keychain is searched by account, so a lookup needs a known user name.
//! Reading an item created by another program may show a consent dialog of
//! the system; the source is therefore used only when enabled.

use security_framework::os::macos::passwords::{SecAuthenticationType, SecProtocolType};
use security_framework::passwords::{
    delete_internet_password, get_internet_password, set_internet_password,
};

use super::Query;

/// Status of the Security framework for a missing item.
const ITEM_NOT_FOUND: i32 = -25300;

fn protocol(query: &Query) -> SecProtocolType {
    if query.protocol == "http" {
        SecProtocolType::HTTP
    } else {
        SecProtocolType::HTTPS
    }
}

/// Password of `username` for `query`; `None` when the keychain has none.
pub(crate) fn get(query: &Query, username: &str) -> Result<Option<String>, String> {
    let path = query.path.as_deref().unwrap_or_default();
    match get_internet_password(
        query.host_name(),
        None,
        username,
        path,
        query.port(),
        protocol(query),
        SecAuthenticationType::Any,
    ) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(error) if error.code() == ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Stores the password of `username` for `query`.
pub(crate) fn store(query: &Query, username: &str, password: &str) -> Result<(), String> {
    let path = query.path.as_deref().unwrap_or_default();
    set_internet_password(
        query.host_name(),
        None,
        username,
        path,
        query.port(),
        protocol(query),
        SecAuthenticationType::Default,
        password.as_bytes(),
    )
    .map_err(|e| e.to_string())
}

/// Deletes the password of `username` for `query`.
pub(crate) fn erase(query: &Query, username: &str) -> Result<(), String> {
    let path = query.path.as_deref().unwrap_or_default();
    match delete_internet_password(
        query.host_name(),
        None,
        username,
        path,
        query.port(),
        protocol(query),
        SecAuthenticationType::Any,
    ) {
        Err(error) if error.code() != ITEM_NOT_FOUND => Err(error.to_string()),
        _ => Ok(()),
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
