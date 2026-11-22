// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Network operations for Danaco Nurt.
//!
//! The crate performs HTTP requests and downloads with TLS from the system
//! certificate store, DNS resolution and network diagnostics. Unsafe code is
//! limited to operating system interfaces for ICMP and network interfaces.

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod cache;
pub mod check;
pub mod client;
pub mod dns;
pub mod html;
pub mod http;
pub mod interfaces;
pub mod ping;
