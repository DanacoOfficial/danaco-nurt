// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Transports and signatures for the Git operations of Danaco Nurt.
//!
//! The crate provides the smart HTTP, SSH and local transports used by fetch,
//! clone, pull and push, the client side of the send-pack protocol, credential
//! sources, and creates and verifies OpenPGP and SSH signatures. No external
//! program is run.

// Tests that write POSIX paths run on Unix only; their helpers are unused on Windows.
#![cfg_attr(all(test, windows), allow(dead_code, unused_imports))]
#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod connect;
pub mod credentials;
pub mod http;
pub mod local;
pub mod send_pack;
pub mod sign;
#[cfg(feature = "git-ssh")]
pub mod ssh;
