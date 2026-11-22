// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion of transport and library failures into contract errors.

use nurt_contracts::error::{Error, ErrorCode};

use crate::http::{NetworkFailure, redact_text};

/// Removes control characters other than line feed and tab, so that text
/// received from a remote cannot carry terminal escape sequences.
pub(crate) fn clean_remote_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

/// Text of a library or remote failure that is safe to report: addresses
/// lose their credentials and control characters are removed.
fn scrub(text: &str) -> String {
    clean_remote_text(&redact_text(text))
}

/// Error `E_GIT_PROTOCOL` for a response that exceeds a size limit.
pub(crate) fn response_too_large(what: &str, limit: usize) -> Error {
    protocol_error(
        "response_too_large",
        format!("the remote sent more than {limit} {what}"),
    )
}

/// Error `E_GIT_PROTOCOL` with the given problem name.
pub(crate) fn protocol_error(problem: &str, message: impl Into<String>) -> Error {
    Error::new(ErrorCode::GitProtocol, message).with_problem(problem)
}

/// Error `E_INTERNAL` for an unexpected library failure.
pub(crate) fn library_error(context: &str, error: &dyn std::fmt::Display) -> Error {
    Error::new(ErrorCode::Internal, format!("{context}: {error}"))
}

/// Maps a client transport error to `E_GIT_AUTH`, `E_NETWORK` or
/// `E_GIT_PROTOCOL`.
pub(crate) fn transport_error(error: &gix_transport::client::Error) -> Error {
    use gix_transport::client::Error as Transport;
    let text = scrub(&error.to_string());
    match error {
        Transport::AuthenticationUnsupported | Transport::AuthenticationRefused(_) => Error::new(
            ErrorCode::GitAuth,
            format!("the remote refused the credentials: {text}"),
        )
        .with_problem("rejected"),
        Transport::Io(failure) => io_error(failure, &text),
        _ => protocol_error(
            "transport",
            format!("the remote broke the protocol: {text}"),
        ),
    }
}

/// Problem of the error raised when the remote asks for credentials.
const AUTHENTICATION_REQUIRED: &str = "authentication_required";

/// HTTP status of a request the credentials do not allow.
const HTTP_FORBIDDEN: u16 = 403;

/// Maps an I/O failure of the transport. The HTTP backend reports a `401`
/// response as `PermissionDenied` and other failed requests with a
/// [`NetworkFailure`] inside the I/O error.
fn io_error(failure: &std::io::Error, text: &str) -> Error {
    if failure.kind() == std::io::ErrorKind::PermissionDenied {
        return Error::new(
            ErrorCode::GitAuth,
            format!("the remote requires credentials: {text}"),
        )
        .with_problem(AUTHENTICATION_REQUIRED);
    }
    match failure
        .get_ref()
        .and_then(|inner| NetworkFailure::find(inner))
    {
        Some(network) if network.http_status() == Some(HTTP_FORBIDDEN) => Error::new(
            ErrorCode::GitAuth,
            format!("the remote refused access: {text}"),
        )
        .with_problem("forbidden"),
        Some(network) => Error::new(ErrorCode::Network, format!("the connection failed: {text}"))
            .with_problem(network.problem().as_str()),
        None => Error::new(ErrorCode::Network, format!("the connection failed: {text}")),
    }
}

/// Maps an I/O error raised while talking to the remote.
///
/// The packet line reader reports `ERR` lines of the remote as I/O errors
/// whose text starts with the remote message.
pub(crate) fn stream_error(error: &std::io::Error) -> Error {
    let text = scrub(&error.to_string());
    match error.kind() {
        std::io::ErrorKind::Other | std::io::ErrorKind::InvalidData => protocol_error(
            "remote_error",
            format!("the remote reported an error: {text}"),
        ),
        _ => Error::new(ErrorCode::Network, format!("the connection failed: {text}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_credentials_and_control_characters_from_error_text() {
        let io = gix_transport::client::Error::Io(std::io::Error::other(
            "failed https://user:secret@example.com/r.git",
        ));
        let error = transport_error(&io);
        assert!(!error.message.contains("secret"));
        assert!(error.message.contains("https://example.com/r.git"));
        let remote = std::io::Error::other("ERR \u{1b}[2Jdenied\u{7}");
        let error = stream_error(&remote);
        assert!(!error.message.contains('\u{1b}'));
        assert!(!error.message.contains('\u{7}'));
        assert!(error.message.contains("denied"));
    }

    #[test]
    fn maps_stream_errors_by_kind() {
        let broken = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed");
        assert_eq!(stream_error(&broken).code(), ErrorCode::Network);
        let remote = std::io::Error::other("ERR access denied");
        let error = stream_error(&remote);
        assert_eq!(error.code(), ErrorCode::GitProtocol);
        assert!(error.message.contains("access denied"));
    }

    #[test]
    fn maps_transport_errors() {
        let refused = gix_transport::client::Error::AuthenticationUnsupported;
        assert_eq!(transport_error(&refused).code(), ErrorCode::GitAuth);
        let io = gix_transport::client::Error::Io(std::io::Error::other("reset"));
        assert_eq!(transport_error(&io).code(), ErrorCode::Network);
        assert_eq!(library_error("pack", &"broken").code(), ErrorCode::Internal);
    }

    #[test]
    fn maps_refused_http_requests_to_authentication_errors() {
        let unauthorized = gix_transport::client::Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "401",
        ));
        let error = transport_error(&unauthorized);
        assert_eq!(error.code(), ErrorCode::GitAuth);
        assert_eq!(error.problem.as_deref(), Some(AUTHENTICATION_REQUIRED));
        let forbidden = gix_transport::client::Error::Io(
            NetworkFailure::status("https://example.com/r.git", HTTP_FORBIDDEN).into_io(),
        );
        let error = transport_error(&forbidden);
        assert_eq!(error.code(), ErrorCode::GitAuth);
        assert_eq!(error.problem.as_deref(), Some("forbidden"));
        let missing = gix_transport::client::Error::Io(
            NetworkFailure::status("https://example.com/r.git", 404).into_io(),
        );
        let error = transport_error(&missing);
        assert_eq!(error.code(), ErrorCode::Network);
        assert_eq!(error.problem.as_deref(), Some("http_status"));
    }
}
