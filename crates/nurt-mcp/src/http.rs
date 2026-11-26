// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Streamable HTTP transport.
//!
//! The transport follows the streamable HTTP transport of the Model Context
//! Protocol, revision 2025-03-26 and later, at one endpoint,
//! [`ENDPOINT_PATH`]:
//!
//! | Method | Effect |
//! |---|---|
//! | `POST` | carries one JSON-RPC message; a request is answered with an event stream that carries the notifications of the request while it runs and then its response, a notification or a response with `202 Accepted` |
//! | `GET` | opens the event stream of the session, which carries the messages that belong to no request, such as log messages |
//! | `DELETE` | ends the session |
//!
//! An `initialize` request opens a session whose identifier the server
//! returns in the `Mcp-Session-Id` header; every later message carries the
//! header. Each session is served by its own protocol handler, and the
//! session identifier is the default engine session of its calls. A session
//! without any message for [`SESSION_IDLE_TIMEOUT`] is closed.
//!
//! Every event of a stream carries an identifier. A client that lost a
//! stream sends `GET` with the `Last-Event-ID` header and receives the
//! events of that stream again, from the named event on.
//!
//! The `Host` header must name a loopback host, the address the listener is
//! bound to or a host given in [`McpHttpOptions::with_allowed_host`]; a
//! request with an `Origin` header must come from a loopback origin or an
//! origin given in [`McpHttpOptions::with_allowed_origin`]. Both checks
//! prevent DNS rebinding.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::RoleServer;
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{
    SessionId, SessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use tokio::net::TcpListener;

use crate::backend::Backend;
use crate::error::McpError;
use crate::server::{NurtServer, Shared};

/// Path of the single endpoint.
pub const ENDPOINT_PATH: &str = "/mcp";

/// Time after which a session without any message is closed.
pub const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(60 * 60);

const SESSION_HEADER: &str = "mcp-session-id";
const BEARER_PREFIX: &str = "Bearer ";

/// Hosts accepted in the `Host` header whatever the options say.
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// Origins accepted in the `Origin` header whatever the options say, on any
/// port.
const LOOPBACK_ORIGINS: [&str; 6] = [
    "http://localhost:*",
    "https://localhost:*",
    "http://127.0.0.1:*",
    "https://127.0.0.1:*",
    "http://[::1]:*",
    "https://[::1]:*",
];

/// Options of the HTTP server.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct McpHttpOptions {
    /// Address to listen on; a loopback address unless
    /// [`allow_remote`](Self::allow_remote) is set.
    pub address: SocketAddr,
    /// Whether an address other than a loopback address is accepted.
    pub allow_remote: bool,
    /// Hosts accepted in the `Host` header in addition to the loopback hosts
    /// and the address of the listener, for example `example.com` or
    /// `example.com:8443`; an entry without a port accepts any port.
    pub allowed_hosts: Vec<String>,
    /// Origins accepted in addition to the loopback origins, for example
    /// `https://example.com:443`; `:*` in place of the port accepts any port.
    pub allowed_origins: Vec<String>,
    /// Token that every request must carry as `Authorization: Bearer
    /// <token>`; `None` disables the check.
    pub bearer_token: Option<String>,
}

impl std::fmt::Debug for McpHttpOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpHttpOptions")
            .field("address", &self.address)
            .field("allow_remote", &self.allow_remote)
            .field("allowed_hosts", &self.allowed_hosts)
            .field("allowed_origins", &self.allowed_origins)
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl McpHttpOptions {
    /// Creates options that listen on `127.0.0.1` at the given port; port 0
    /// selects a free port.
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self {
            address: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            allow_remote: false,
            allowed_hosts: Vec::new(),
            allowed_origins: Vec::new(),
            bearer_token: None,
        }
    }

    /// Returns the options with another listening address.
    #[must_use]
    pub fn with_address(mut self, address: SocketAddr) -> Self {
        self.address = address;
        self
    }

    /// Returns the options with listening on a non-loopback address allowed
    /// or forbidden.
    #[must_use]
    pub fn allow_remote(mut self, allow: bool) -> Self {
        self.allow_remote = allow;
        self
    }

    /// Returns the options with one more accepted `Host` header value.
    #[must_use]
    pub fn with_allowed_host(mut self, host: impl Into<String>) -> Self {
        self.allowed_hosts.push(host.into());
        self
    }

    /// Returns the options with one more accepted origin.
    #[must_use]
    pub fn with_allowed_origin(mut self, origin: impl Into<String>) -> Self {
        self.allowed_origins.push(origin.into());
        self
    }

    /// Returns the options with a bearer token.
    #[must_use]
    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    /// Returns the options with the bearer token read from a file; leading
    /// and trailing white space is removed.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Io`] when the file cannot be read and
    /// [`McpError::InvalidOptions`] when it holds no token.
    pub fn with_bearer_token_file(self, path: impl AsRef<Path>) -> Result<Self, McpError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)?;
        let token = non_empty_token(&text).ok_or_else(|| {
            McpError::InvalidOptions(format!(
                "token file `{}` is empty; write the token into it",
                path.display()
            ))
        })?;
        Ok(self.with_bearer_token(token))
    }

    /// Returns the options with the bearer token read from an environment
    /// variable; leading and trailing white space is removed.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::InvalidOptions`] when the variable is not set, is
    /// not valid Unicode or holds no token.
    pub fn with_bearer_token_env(self, name: &str) -> Result<Self, McpError> {
        let token = std::env::var(name)
            .ok()
            .as_deref()
            .and_then(non_empty_token)
            .ok_or_else(|| {
                McpError::InvalidOptions(format!(
                    "environment variable `{name}` holds no token; set it or omit the option"
                ))
            })?;
        Ok(self.with_bearer_token(token))
    }

    fn validate(&self, address: SocketAddr) -> Result<(), McpError> {
        if !address.ip().is_loopback() && !self.allow_remote {
            return Err(McpError::InvalidOptions(format!(
                "address {address} is not a loopback address; allow remote clients explicitly to use it"
            )));
        }
        if self
            .bearer_token
            .as_deref()
            .is_some_and(|token| token.trim().is_empty())
        {
            return Err(McpError::InvalidOptions(
                "bearer token is empty; give a token or omit the option".to_owned(),
            ));
        }
        Ok(())
    }

    /// Configuration of the transport for a listener bound to `address`.
    fn transport_config(&self, address: SocketAddr) -> StreamableHttpServerConfig {
        let mut hosts: Vec<String> = LOOPBACK_HOSTS.iter().map(|&host| host.to_owned()).collect();
        if !address.ip().is_unspecified() {
            hosts.push(address.ip().to_string());
        }
        hosts.extend(self.allowed_hosts.iter().cloned());
        let origins = LOOPBACK_ORIGINS
            .iter()
            .map(|&origin| origin.to_owned())
            .chain(self.allowed_origins.iter().cloned());
        StreamableHttpServerConfig::default()
            .with_allowed_hosts(hosts)
            .with_allowed_origins(origins)
            .enforce_origin_validation()
    }
}

fn non_empty_token(text: &str) -> Option<String> {
    let token = text.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

/// Serves clients over streamable HTTP at the address of the options until
/// the process ends.
///
/// The function starts its own asynchronous runtime and blocks the calling
/// thread.
///
/// # Errors
///
/// Returns [`McpError::InvalidOptions`] for a non-loopback address without
/// [`McpHttpOptions::allow_remote`] or for an empty token, and
/// [`McpError::Io`] when the runtime cannot start or the address cannot be
/// bound.
pub fn serve_http<B: Backend>(backend: Arc<B>, options: McpHttpOptions) -> Result<(), McpError> {
    options.validate(options.address)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = TcpListener::bind(options.address).await?;
        serve_http_listener(backend, options, listener).await
    })
}

/// Serves clients over streamable HTTP on a bound listener; the address of
/// the options is ignored.
///
/// # Errors
///
/// Returns [`McpError::InvalidOptions`] when the listener is bound to a
/// non-loopback address without [`McpHttpOptions::allow_remote`] or the
/// token is empty, and [`McpError::Io`] when accepting connections fails.
pub async fn serve_http_listener<B: Backend>(
    backend: Arc<B>,
    options: McpHttpOptions,
    listener: TcpListener,
) -> Result<(), McpError> {
    let address = listener.local_addr()?;
    options.validate(address)?;
    let shared = Shared::new(backend);
    let mut sessions = LocalSessionManager::default();
    sessions.session_config.keep_alive = Some(SESSION_IDLE_TIMEOUT);
    let sessions = Arc::new(sessions);
    let service = StreamableHttpService::new(
        move || Ok(NurtServer::new(Arc::clone(&shared), None)),
        Arc::clone(&sessions),
        options.transport_config(address),
    );
    let guard = Arc::new(Guard {
        sessions,
        bearer_token: options.bearer_token,
    });
    let router = Router::new()
        .route_service(ENDPOINT_PATH, service)
        .layer(middleware::from_fn_with_state(guard, check_request));
    axum::serve(listener, router).await?;
    Ok(())
}

/// Identifier of the HTTP session of a request; `None` for a request that
/// did not arrive over HTTP or opens the session.
pub(crate) fn session_of(context: &RequestContext<RoleServer>) -> Option<String> {
    context
        .extensions
        .get::<axum::http::request::Parts>()?
        .headers
        .get(SESSION_HEADER)?
        .to_str()
        .ok()
        .map(str::to_owned)
}

/// State of the checks that precede the transport.
struct Guard {
    sessions: Arc<LocalSessionManager>,
    bearer_token: Option<String>,
}

/// Checks the bearer token of every request, and answers `DELETE` of an
/// unknown session with `404 Not Found`, as the transport requires for any
/// request of a session that has ended.
async fn check_request(State(guard): State<Arc<Guard>>, request: Request, next: Next) -> Response {
    if let Some(expected) = &guard.bearer_token {
        let given = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.as_bytes().strip_prefix(BEARER_PREFIX.as_bytes()));
        if !given.is_some_and(|given| same_token(given, expected.as_bytes())) {
            return unauthorized();
        }
    }
    if request.method() == Method::DELETE
        && let Some(id) = request
            .headers()
            .get(SESSION_HEADER)
            .and_then(|id| id.to_str().ok())
    {
        let known = guard.sessions.has_session(&SessionId::from(id)).await;
        if !matches!(known, Ok(true)) {
            return (StatusCode::NOT_FOUND, "Not Found: Session not found").into_response();
        }
    }
    next.run(request).await
}

fn unauthorized() -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        "Unauthorized: missing or wrong bearer token; send `Authorization: Bearer <token>`",
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

/// Compares tokens in time that does not depend on the position of the
/// first difference.
fn same_token(given: &[u8], expected: &[u8]) -> bool {
    given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use super::{LOOPBACK_HOSTS, LOOPBACK_ORIGINS, McpHttpOptions, same_token};

    #[test]
    fn tokens_match_only_when_equal() {
        assert!(same_token(b"secret", b"secret"));
        assert!(!same_token(b"secret", b"secreT"));
        assert!(!same_token(b"secret", b"secret2"));
    }

    #[test]
    fn remote_address_requires_explicit_permission() {
        let remote: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        let options = McpHttpOptions::new(0);
        assert!(options.validate(remote).is_err());
        assert!(options.clone().allow_remote(true).validate(remote).is_ok());
        assert!(options.validate(options.address).is_ok());
        assert!(
            McpHttpOptions::new(0)
                .with_bearer_token(" ")
                .validate("127.0.0.1:1".parse().unwrap())
                .is_err()
        );
    }

    #[test]
    fn transport_accepts_loopback_the_listener_and_listed_names() {
        let options = McpHttpOptions::new(0)
            .with_allowed_host("example.com")
            .with_allowed_origin("https://example.com:443");
        let config = options.transport_config("192.0.2.7:8080".parse().unwrap());
        let mut hosts: Vec<&str> = LOOPBACK_HOSTS.to_vec();
        hosts.extend(["192.0.2.7", "example.com"]);
        assert_eq!(config.allowed_hosts, hosts);
        let mut origins: Vec<&str> = LOOPBACK_ORIGINS.to_vec();
        origins.push("https://example.com:443");
        assert_eq!(config.allowed_origins, origins);

        let unspecified = options.transport_config("0.0.0.0:8080".parse().unwrap());
        assert!(
            !unspecified
                .allowed_hosts
                .iter()
                .any(|host| host == "0.0.0.0")
        );
    }

    #[test]
    fn options_listen_on_the_loopback_address_and_collect_the_allowed_names() {
        let options = McpHttpOptions::new(8765);
        assert_eq!(options.address, "127.0.0.1:8765".parse::<SocketAddr>().unwrap());
        assert!(!options.allow_remote);
        assert!(options.allowed_hosts.is_empty() && options.allowed_origins.is_empty());
        assert_eq!(options.bearer_token, None);
        let address: SocketAddr = "[::1]:9000".parse().unwrap();
        let options = options
            .with_address(address)
            .allow_remote(true)
            .with_allowed_host("a.example.com")
            .with_allowed_host("b.example.com:8443")
            .with_allowed_origin("https://a.example.com:*")
            .with_bearer_token("t");
        assert_eq!(options.address, address);
        assert!(options.allow_remote);
        assert_eq!(options.allowed_hosts, ["a.example.com", "b.example.com:8443"]);
        assert_eq!(options.allowed_origins, ["https://a.example.com:*"]);
        assert_eq!(options.bearer_token.as_deref(), Some("t"));
    }

    #[test]
    fn loopback_addresses_of_both_families_need_no_permission() {
        let options = McpHttpOptions::new(0);
        for address in ["127.0.0.1:1", "127.9.9.9:1", "[::1]:1"] {
            assert!(options.validate(address.parse().unwrap()).is_ok(), "{address}");
        }
        for address in ["192.0.2.1:1", "[2001:db8::1]:1", "0.0.0.0:1", "[::]:1"] {
            assert!(options.validate(address.parse().unwrap()).is_err(), "{address}");
        }
    }

    #[test]
    fn a_token_of_white_space_only_is_not_a_token() {
        assert_eq!(non_empty_token(" \t\n"), None);
        assert_eq!(non_empty_token(""), None);
        assert_eq!(non_empty_token("  t\u{142}  \n").as_deref(), Some("t\u{142}"));
    }

    #[test]
    fn tokens_of_different_lengths_or_with_one_changed_byte_do_not_match() {
        assert!(same_token(b"", b""));
        assert!(!same_token(b"", b"a"));
        assert!(!same_token(b"a", b""));
        assert!(!same_token(b"abc", b"abd"));
        assert!(!same_token(b"abc", b"Abc"));
        assert!(same_token("t\u{142}".as_bytes(), "t\u{142}".as_bytes()));
    }

    #[test]
    fn a_listener_address_in_the_loopback_range_is_added_to_the_hosts() {
        let config = McpHttpOptions::new(0).transport_config("[::1]:8080".parse().unwrap());
        assert!(config.allowed_hosts.iter().any(|host| host == "::1"));
        let config = McpHttpOptions::new(0).transport_config("[::]:8080".parse().unwrap());
        assert!(!config.allowed_hosts.iter().any(|host| host == "::"));
    }

    #[test]
    fn debug_output_hides_the_token() {
        let options = McpHttpOptions::new(8080).with_bearer_token("secret");
        let text = format!("{options:?}");
        assert!(!text.contains("secret"));
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn token_is_read_from_a_file_without_white_space() {
        let directory = std::env::temp_dir().join(format!("nurt-mcp-token-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("token");
        std::fs::write(&path, "  abc123\n").unwrap();
        let options = McpHttpOptions::new(0)
            .with_bearer_token_file(&path)
            .unwrap();
        assert_eq!(options.bearer_token.as_deref(), Some("abc123"));
        std::fs::write(&path, "\n").unwrap();
        assert!(
            McpHttpOptions::new(0)
                .with_bearer_token_file(&path)
                .is_err()
        );
        assert!(
            McpHttpOptions::new(0)
                .with_bearer_token_file(directory.join("missing"))
                .is_err()
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn missing_environment_token_is_rejected() {
        let result =
            McpHttpOptions::new(0).with_bearer_token_env("NURT_MCP_TEST_TOKEN_THAT_IS_NEVER_SET");
        assert!(result.is_err());
    }
}
