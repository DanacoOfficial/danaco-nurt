// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Node.js addon of the Danaco Nurt library.
//!
//! The addon wraps one [`nurt_core::Engine`]. Documents cross the boundary as
//! UTF-8 JSON text: the request envelope in, the response envelope out, and
//! the results of the other operations in the form described by
//! [`dispatch::Native::call`]. The JavaScript package `nurt` selects this
//! addon as its library backend and keeps its child process backend as the
//! fallback.
//!
//! The module [`dispatch`] holds the logic and has no dependency on the
//! Node-API; the module `addon` only exposes it to JavaScript. An application
//! that assembles its own engine builds a [`dispatch::Native`] with
//! [`dispatch::Native::from_engine`].

#![deny(missing_docs)]

pub mod dispatch;
pub mod system;

#[cfg(not(test))]
#[allow(
    unsafe_code,
    unreachable_pub,
    missing_docs,
    reason = "the Node-API macros generate extern functions and registration code, and the exported items are reached by name from JavaScript"
)]
mod addon;
