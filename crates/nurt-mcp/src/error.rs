// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-mcp::error
// Responsibility:   Errors of the server.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Errors of the server.

use std::fmt;

/// Error that ends a server before or while it serves clients.
///
/// Errors of single requests never end the server: they are returned to the
/// client as protocol errors or as tool results with `isError: true`.
#[derive(Debug)]
#[non_exhaustive]
pub enum McpError {
    /// An input or output operation of the transport failed.
    Io(std::io::Error),
    /// The options of the server are not valid.
    InvalidOptions(String),
    /// The client did not complete the initialization of the session.
    Initialize(String),
    /// The task that serves the connection ended abnormally.
    Internal(String),
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "transport input or output failed: {error}"),
            Self::InvalidOptions(message) => write!(f, "invalid server options: {message}"),
            Self::Initialize(message) => write!(f, "session initialization failed: {message}"),
            Self::Internal(message) => write!(f, "server task failed: {message}"),
        }
    }
}

impl std::error::Error for McpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidOptions(_) | Self::Initialize(_) | Self::Internal(_) => None,
        }
    }
}

impl From<std::io::Error> for McpError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::McpError;

    #[test]
    fn displays_each_kind_with_its_cause() {
        let io = McpError::from(std::io::Error::other("pipe closed"));
        assert_eq!(
            io.to_string(),
            "transport input or output failed: pipe closed"
        );
        assert!(io.source().is_some());

        let cases = [
            (
                McpError::InvalidOptions("port 0".to_owned()),
                "invalid server options: port 0",
            ),
            (
                McpError::Initialize("closed".to_owned()),
                "session initialization failed: closed",
            ),
            (
                McpError::Internal("join".to_owned()),
                "server task failed: join",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            assert!(error.source().is_none());
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
