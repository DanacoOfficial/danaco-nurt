// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion of transport and library failures into contract errors.

use nurt_contracts::error::{Error, ErrorCode};

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
    let text = error.to_string();
    match error {
        Transport::AuthenticationUnsupported | Transport::AuthenticationRefused(_) => Error::new(
            ErrorCode::GitAuth,
            format!("the remote refused the credentials: {text}"),
        )
        .with_problem("rejected"),
        Transport::Io(_) => {
            Error::new(ErrorCode::Network, format!("the connection failed: {text}"))
        }
        _ => protocol_error(
            "transport",
            format!("the remote broke the protocol: {text}"),
        ),
    }
}

/// Maps an I/O error raised while talking to the remote.
///
/// The packet line reader reports `ERR` lines of the remote as I/O errors
/// whose text starts with the remote message.
pub(crate) fn stream_error(error: &std::io::Error) -> Error {
    let text = error.to_string();
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
}
