// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::fetch::error
// Responsibility:   Errors of a transfer: network failures, authentication, interruption and protocol failures, with every address stripped of its credentials.
// Dependencies:     nurt_contracts, nurt_git_transport
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Errors of a transfer: network failures, authentication, interruption and
//! protocol failures, with every address stripped of its credentials.

use std::error::Error as StdError;
use std::fmt::Display;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::stage::ExecCtx;
use nurt_git_transport::http::{NetworkFailure, NetworkProblem};

use super::head;
use super::transfer::Network;

/// HTTP status of a request without valid credentials.
const UNAUTHORIZED: u16 = 401;

/// HTTP status of a request for a resource the credentials do not grant.
const FORBIDDEN: u16 = 403;

/// Error of a failed read of the repository.
pub(crate) fn read_error(what: &str, failure: &dyn Display) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot {what}: {}", head::scrub(&failure.to_string())),
    )
}

/// Error of a network failure found by the HTTP transport.
pub(crate) fn network(failure: &NetworkFailure) -> Error {
    let advice = match failure.problem() {
        NetworkProblem::Dns => "check the host name of the address",
        NetworkProblem::Connect => "check that the server is reachable from this machine",
        NetworkProblem::Tls => "check the certificate of the server or http.sslCAInfo",
        NetworkProblem::Timeout => "retry, or raise http.lowSpeedTime and git.network.timeout",
        NetworkProblem::Proxy => "check http.proxy and the proxy variables",
        _ => "check the address and the server",
    };
    Error::new(
        ErrorCode::Network,
        format!("{}; {advice}", head::scrub(&failure.to_string())),
    )
    .with_problem(failure.problem().as_str())
    .with_found(head::anonymize(failure.url()))
}

/// First error of type `T` in the chain of `failure`.
fn find<'a, T: StdError + 'static>(failure: &'a (dyn StdError + 'static)) -> Option<&'a T> {
    let mut current = Some(failure);
    while let Some(error) = current {
        if let Some(found) = error.downcast_ref::<T>() {
            return Some(found);
        }
        current = error.source();
    }
    None
}

/// Error of a failed step `what` of the transfer.
pub(crate) fn transfer(network_settings: &Network, failure: &gix::Error, what: &str) -> Error {
    let chain: &(dyn StdError + 'static) = failure;
    if let Some(found) = NetworkFailure::find(chain) {
        return match found.http_status() {
            Some(status @ (UNAUTHORIZED | FORBIDDEN)) => {
                network_settings.credentials.error(Some(status))
            }
            _ => network(found),
        };
    }
    let report = network_settings.credentials.report();
    if report.missing || report.rejected || report.store_locked {
        return network_settings.credentials.error(None);
    }
    if let Some(found) = find::<Error>(chain) {
        return found.clone();
    }
    if let Some(found) = find::<std::io::Error>(chain) {
        return io_failure(network_settings, found, what);
    }
    protocol(&network_settings.url, &failure.to_string(), what)
}

/// Error of an input or output failure of the connection.
fn io_failure(network_settings: &Network, failure: &std::io::Error, what: &str) -> Error {
    let problem = match failure.kind() {
        std::io::ErrorKind::TimedOut => NetworkProblem::Timeout,
        std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::NotConnected => NetworkProblem::Connect,
        _ => return protocol(&network_settings.url, &failure.to_string(), what),
    };
    network(&NetworkFailure::new(
        problem,
        &network_settings.url,
        format!("{what} failed: {failure}"),
    ))
}

/// Error of data that the remote sent and that cannot be used.
pub(crate) fn protocol(url: &str, message: &str, what: &str) -> Error {
    Error::new(
        ErrorCode::GitProtocol,
        format!(
            "{what} with {} failed: {}; check that the address names a git repository",
            head::anonymize(url),
            head::scrub(message)
        ),
    )
    .with_problem("protocol")
    .with_found(head::anonymize(url))
}

/// Error of a transfer stopped by cancellation or by the time limit.
pub(crate) fn interrupted(ctx: &ExecCtx) -> Error {
    if ctx.cancel().is_cancelled() {
        Error::new(ErrorCode::Cancelled, "the transfer was cancelled")
    } else {
        Error::new(
            ErrorCode::Timeout,
            "the transfer did not finish within the time limit; raise the timeout of the call and retry",
        )
    }
}

/// Error for a remote with another object format.
pub(crate) fn object_format(url: &str, remote: gix::hash::Kind, local: gix::hash::Kind) -> Error {
    Error::new(
        ErrorCode::GitUnsupported,
        format!(
            "the remote {} uses {remote} object names and the repository uses {local}; clone it into a new repository instead",
            head::anonymize(url)
        ),
    )
    .with_problem("object_format")
    .with_found(remote.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_errors_keep_the_problem_and_hide_credentials() {
        let failure = NetworkFailure::new(
            NetworkProblem::Dns,
            "https://user:secret@nowhere.invalid/r.git",
            "cannot resolve https://user:secret@nowhere.invalid",
        );
        let error = network(&failure);
        assert_eq!(error.code, ErrorCode::Network);
        assert_eq!(error.problem.as_deref(), Some("dns"));
        assert!(!error.message.contains("secret"));
    }

    #[test]
    fn protocol_errors_name_the_address_without_credentials() {
        let error = protocol("https://u:pw@h.example/r", "bad packet", "the handshake");
        assert_eq!(error.code, ErrorCode::GitProtocol);
        assert!(error.message.contains("https://h.example/r"));
        assert!(!error.message.contains("pw"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
