// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Connection gate of an embedding application.

use std::net::IpAddr;

/// One connection the client is about to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectRequest<'a> {
    /// Address requested in this step, after normalization.
    pub url: &'a str,
    /// Host of the address.
    pub host: &'a str,
    /// Port of the address.
    pub port: u16,
    /// IP addresses the client will connect to: the resolved or pinned
    /// addresses of the host, or of the proxy when one is used.
    pub addresses: &'a [IpAddr],
    /// Proxy server, when the connection goes through one.
    pub proxy: Option<&'a str>,
    /// Redirect step, 0 for the address of the request.
    pub hop: u32,
    /// Attempt, 1 for the first one.
    pub attempt: u32,
}

/// Decision point called before every connection, after name resolution,
/// after each redirect and on each retry.
///
/// The console applies no rules of its own. An embedding application that
/// controls network access registers an implementation in the extensions of
/// the execution context (`Extensions::insert::<dyn ConnectGate>`); without
/// one every connection proceeds.
pub trait ConnectGate: Send + Sync {
    /// Accepts the connection, or refuses it with a reason shown to the
    /// caller.
    ///
    /// # Errors
    ///
    /// The reason of the refusal.
    fn check(&self, request: &ConnectRequest<'_>) -> Result<(), String>;
}
