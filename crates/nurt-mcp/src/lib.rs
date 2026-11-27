// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-mcp
// Responsibility:   Model Context Protocol server of Danaco Nurt.
// Dependencies:     rmcp, tokio
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Model Context Protocol server of Danaco Nurt.
//!
//! The server lets any MCP client, such as the host of an AI agent, run
//! programs of the Nurt language. It exposes three tools and a set of
//! read-only resources:
//!
//! | Name | Kind | Content |
//! |---|---|---|
//! | `nurt` | tool | runs a program; the input schema is the request `nurt-request/1` with the options at the top level |
//! | `nurt_help` | tool | returns built-in help as text |
//! | `nurt_check` | tool | checks a program without running it and returns the check document `nurt-check/1` |
//! | `nurt://help/{topic}` | resource | built-in help of a topic |
//! | `nurt://schema/{name}` | resource | published JSON schemas |
//! | `nurt://describe/core`, `nurt://describe/group/{name}` | resource | description layers for models |
//!
//! A `nurt` call returns the rendered text of the report as the first
//! content block and the report `nurt-report/1` as structured content. The
//! result has `isError: true` only when the report carries an error or the
//! call itself failed. Progress of the program is sent as
//! `notifications/progress` when the client gives a progress token, log
//! messages of the engine as `notifications/message`, and
//! `notifications/cancelled` stops the program.
//!
//! The server reaches the engine through the [`Backend`] trait. Standard
//! input and output carry only protocol messages; diagnostics are emitted
//! through `tracing`. With the `http` feature the server also serves the
//! streamable HTTP transport.
//!
//! # Examples
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use nurt_mcp::{Backend, McpOptions, serve_stdio};
//!
//! fn serve(engine: Arc<impl Backend>) -> Result<(), nurt_mcp::McpError> {
//!     serve_stdio(engine, McpOptions::new())
//! }
//! ```

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod backend;
mod engine;
mod error;
mod events;
#[cfg(feature = "http")]
mod http;
mod resources;
mod server;
mod tool_schema;

use std::sync::Arc;
use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::service::ServerInitializeError;
use tokio::io::{AsyncRead, AsyncWrite};

pub use crate::backend::{
    Backend, BackendInfo, CallError, CheckOutcome, DescribeSubject, HelpKind, HelpQuery, SchemaName,
};
pub use crate::error::McpError;
#[cfg(feature = "http")]
pub use crate::http::{
    ENDPOINT_PATH, McpHttpOptions, SESSION_IDLE_TIMEOUT, serve_http, serve_http_listener,
};
use crate::server::{NurtServer, Shared};

/// Time given to engine calls still in progress when the server stops.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Options of a server on a byte stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct McpOptions {
    /// Engine session used by calls that do not name one; `None` leaves the
    /// choice to the engine.
    pub session: Option<String>,
}

impl McpOptions {
    /// Creates the default options.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the options with a default engine session.
    #[must_use]
    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }
}

/// Serves one client over standard input and output until the client
/// closes the input.
///
/// The function starts its own asynchronous runtime and blocks the calling
/// thread.
///
/// # Errors
///
/// Returns [`McpError::Io`] when the runtime cannot start, and the errors of
/// [`serve_io`].
pub fn serve_stdio<B: Backend>(backend: Arc<B>, options: McpOptions) -> Result<(), McpError> {
    let runtime = runtime()?;
    let result = runtime.block_on(serve_io(
        backend,
        options,
        tokio::io::stdin(),
        tokio::io::stdout(),
    ));
    runtime.shutdown_timeout(SHUTDOWN_GRACE);
    result
}

/// Serves one client over a pair of byte streams with one JSON-RPC message
/// per line, until the client closes `reader`.
///
/// A client that closes the stream before the initialization is not an
/// error.
///
/// # Errors
///
/// Returns [`McpError::Initialize`] when the first message is not a valid
/// `initialize` request or the initialization fails, and
/// [`McpError::Internal`] when the serving task ends abnormally.
pub async fn serve_io<B, R, W>(
    backend: Arc<B>,
    options: McpOptions,
    reader: R,
    writer: W,
) -> Result<(), McpError>
where
    B: Backend,
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let server = NurtServer::new(Shared::new(backend), options.session);
    match server.serve((reader, writer)).await {
        Ok(running) => running
            .waiting()
            .await
            .map(|_| ())
            .map_err(|error| McpError::Internal(error.to_string())),
        Err(error) => initialization_failure(error),
    }
}

/// Outcome of a connection that ended before the initialization finished.
fn initialization_failure(error: ServerInitializeError) -> Result<(), McpError> {
    match error {
        ServerInitializeError::ConnectionClosed(reason) => {
            tracing::debug!(%reason, "client closed the connection before initialization");
            Ok(())
        }
        error => Err(McpError::Initialize(error.to_string())),
    }
}

/// Builds the multi-threaded runtime of a blocking entry point.
fn runtime() -> Result<tokio::runtime::Runtime, McpError> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_start_without_a_session() {
        assert_eq!(McpOptions::new().session, None);
        assert_eq!(McpOptions::new(), McpOptions::default());
    }

    #[test]
    fn a_session_is_set_from_any_text_and_the_last_one_wins() {
        let options = McpOptions::new().with_session("s1");
        assert_eq!(options.session.as_deref(), Some("s1"));
        let options = options.with_session(String::from("\u{142}\u{f3}d\u{17a}"));
        assert_eq!(options.session.as_deref(), Some("\u{142}\u{f3}d\u{17a}"));
        assert_eq!(
            McpOptions::new().with_session("").session.as_deref(),
            Some("")
        );
    }

    #[test]
    fn the_runtime_of_a_blocking_entry_point_starts() {
        let runtime = runtime().unwrap();
        assert_eq!(runtime.block_on(async { 1 + 1 }), 2);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
