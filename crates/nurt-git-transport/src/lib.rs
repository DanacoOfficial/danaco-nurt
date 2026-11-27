// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport
// Responsibility:   Transports and signatures for the Git operations of Danaco Nurt.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Transports and signatures for the Git operations of Danaco Nurt.
//!
//! The crate provides the smart HTTP, SSH and local transports used by fetch,
//! clone, pull and push, the client side of the send-pack protocol, credential
//! sources, and creates and verifies OpenPGP and SSH signatures. No external
//! program is run.

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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
