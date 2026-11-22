// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Classified failures of HTTP requests.

use std::error::Error;
use std::fmt;
use std::io;

/// Kind of a network failure, reported as the `problem` of `E_NETWORK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetworkProblem {
    /// The host name could not be resolved.
    Dns,
    /// The connection could not be established or was interrupted.
    Connect,
    /// The TLS handshake or certificate verification failed.
    Tls,
    /// A time limit or the minimum transfer speed was not met.
    Timeout,
    /// The proxy could not be used.
    Proxy,
    /// The server answered with an unexpected HTTP status.
    HttpStatus,
    /// A redirect was refused by the redirect policy.
    Redirect,
}

impl NetworkProblem {
    /// Stable name of the problem.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_transport::http::NetworkProblem;
    ///
    /// assert_eq!(NetworkProblem::HttpStatus.as_str(), "http_status");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dns => "dns",
            Self::Connect => "connect",
            Self::Tls => "tls",
            Self::Timeout => "timeout",
            Self::Proxy => "proxy",
            Self::HttpStatus => "http_status",
            Self::Redirect => "redirect",
        }
    }
}

/// Failure of an HTTP request, with the address stripped of credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkFailure {
    problem: NetworkProblem,
    status: Option<u16>,
    url: String,
    message: String,
}

impl NetworkFailure {
    /// Failure of `problem` for the request to `url`.
    ///
    /// User name and password in `url` are removed.
    #[must_use]
    pub fn new(problem: NetworkProblem, url: &str, message: impl Into<String>) -> Self {
        Self {
            problem,
            status: None,
            url: redact_url(url),
            message: message.into(),
        }
    }

    /// Failure caused by the HTTP `status` of the response to `url`.
    #[must_use]
    pub fn status(url: &str, status: u16) -> Self {
        Self {
            problem: NetworkProblem::HttpStatus,
            status: Some(status),
            url: redact_url(url),
            message: format!("the server answered with HTTP status {status}"),
        }
    }

    /// Kind of the failure.
    #[must_use]
    pub fn problem(&self) -> NetworkProblem {
        self.problem
    }

    /// HTTP status, for [`NetworkProblem::HttpStatus`].
    #[must_use]
    pub fn http_status(&self) -> Option<u16> {
        self.status
    }

    /// Address of the request, without credentials.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Description of the failure.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Finds a failure in the chain of `error`, looking through the I/O and
    /// transport errors that wrap it.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_transport::http::{NetworkFailure, NetworkProblem};
    ///
    /// let failure = NetworkFailure::new(NetworkProblem::Dns, "https://example.invalid/r", "unknown host");
    /// let error = std::io::Error::other(failure.clone());
    /// assert_eq!(NetworkFailure::find(&error), Some(&failure));
    /// ```
    #[must_use]
    pub fn find<'a>(error: &'a (dyn Error + 'static)) -> Option<&'a Self> {
        let mut current = Some(error);
        while let Some(error) = current {
            if let Some(found) = error.downcast_ref::<Self>() {
                return Some(found);
            }
            if let Some(inner) = error.downcast_ref::<io::Error>().and_then(io::Error::get_ref) {
                if let Some(found) = Self::find(inner) {
                    return Some(found);
                }
            }
            current = error.source();
        }
        None
    }

    pub(crate) fn into_io(self) -> io::Error {
        let kind = match self.problem {
            NetworkProblem::Timeout => io::ErrorKind::TimedOut,
            NetworkProblem::Connect | NetworkProblem::Dns => io::ErrorKind::ConnectionAborted,
            _ => io::ErrorKind::Other,
        };
        io::Error::new(kind, self)
    }
}

impl fmt::Display for NetworkFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.url, self.message)
    }
}

impl Error for NetworkFailure {}

/// Removes the user name and password from an address.
///
/// # Examples
///
/// ```
/// use nurt_git_transport::http::redact_url;
///
/// assert_eq!(redact_url("https://user:token@example.com/r.git"), "https://example.com/r.git");
/// assert_eq!(redact_url("/srv/repo"), "/srv/repo");
/// ```
#[must_use]
pub fn redact_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_owned();
    };
    let authority_start = scheme_end + "://".len();
    let rest = &url[authority_start..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    match rest[..authority_end].rfind('@') {
        Some(at) => format!("{}{}", &url[..authority_start], &rest[at + 1..]),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_user_and_password_only_in_the_authority() {
        assert_eq!(redact_url("https://u@h/p@x"), "https://h/p@x");
        assert_eq!(redact_url("http://h:8080/a"), "http://h:8080/a");
    }

    #[test]
    fn finds_failure_nested_in_io_errors_and_sources() {
        let failure = NetworkFailure::status("https://h/r", 503);
        let nested = io::Error::other(io::Error::other(failure.clone()));
        assert_eq!(NetworkFailure::find(&nested).and_then(NetworkFailure::http_status), Some(503));
        assert_eq!(failure.problem().as_str(), "http_status");
    }

    #[test]
    fn maps_timeouts_to_timed_out_io_errors() {
        let error = NetworkFailure::new(NetworkProblem::Timeout, "https://h", "slow").into_io();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "https://h: slow");
    }
}
