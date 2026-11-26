// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! HTTP client shared by the commands of the `network` group.
//!
//! The client resolves names itself through a [`NameResolver`] and connects
//! only to the resolved or pinned ([`Pins`]) addresses. Every attempt opens
//! its own connection; connections are never pooled, so the address of each
//! connection is known and reported. Before each connection an optional
//! [`ConnectGate`] registered by the embedding application is consulted.
//!
//! Redirects are followed by the client, not by the HTTP library, so that
//! every step is resolved, pinned and gated like the first one and recorded
//! in [`ResponseHead::redirects`]. The time limit of a request covers name
//! resolution, connections, redirects, retries and reading the body.
//! Cancelling the execution closes the connection at once.

mod address;
mod body;
mod gate;
mod proxy;
mod resolve;
mod retry;
mod tls;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use nurt_contracts::config::Config;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{Event, ExecCtx, Progress};
use ureq::http;
use ureq::tls::RootCerts;
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use url::Url;

pub use address::{HostKind, MAX_URL_CHARS, Target};
pub use body::{BodyEncoding, LimitedBody, ResponseBody};
pub use gate::{ConnectGate, ConnectRequest};
pub use proxy::{ProxyChoice, effective_proxy};
pub use resolve::{NameResolver, Pins, SystemResolver};
pub use retry::RetryPolicy;

use body::{BodySource, CancelWatch};

/// `User-Agent` sent when the request gives none.
pub const DEFAULT_USER_AGENT: &str = concat!("nurt/", env!("CARGO_PKG_VERSION"));
/// Bytes of body between two progress events.
pub const DEFAULT_PROGRESS_STEP: u64 = 256 * 1024;
/// Longest accepted value of a request header, in characters.
pub const MAX_HEADER_VALUE_CHARS: usize = 1024;
/// Time limit of a request when the caller gives none.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Redirects followed when the caller gives no limit.
pub const DEFAULT_MAX_REDIRECTS: u32 = 10;

/// Content codings the client decodes.
const ACCEPT_ENCODING: &str = "gzip, deflate";

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// `GET`.
    Get,
    /// `HEAD`.
    Head,
    /// `POST`.
    Post,
    /// `PUT`.
    Put,
    /// `PATCH`.
    Patch,
    /// `DELETE`.
    Delete,
    /// `OPTIONS`.
    Options,
}

impl Method {
    /// Parses the lower-case name used by the language.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "get" => Self::Get,
            "head" => Self::Head,
            "post" => Self::Post,
            "put" => Self::Put,
            "patch" => Self::Patch,
            "delete" => Self::Delete,
            "options" => Self::Options,
            _ => return None,
        })
    }

    /// Lower-case name, as written in the language.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Head => "head",
            Self::Post => "post",
            Self::Put => "put",
            Self::Patch => "patch",
            Self::Delete => "delete",
            Self::Options => "options",
        }
    }

    /// Whether the method only reads the remote side (`get`, `head`,
    /// `options`).
    #[must_use]
    pub const fn is_read_only(self) -> bool {
        matches!(self, Self::Get | Self::Head | Self::Options)
    }

    /// Whether failed attempts may be repeated; `post` and `patch` are not
    /// repeated.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        !matches!(self, Self::Post | Self::Patch)
    }

    fn to_http(self) -> http::Method {
        match self {
            Self::Get => http::Method::GET,
            Self::Head => http::Method::HEAD,
            Self::Post => http::Method::POST,
            Self::Put => http::Method::PUT,
            Self::Patch => http::Method::PATCH,
            Self::Delete => http::Method::DELETE,
            Self::Options => http::Method::OPTIONS,
        }
    }
}

/// Body of a request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RequestBody {
    /// No body.
    #[default]
    None,
    /// Bytes sent with an optional `Content-Type`; a `Content-Type` header
    /// of the request takes precedence.
    Bytes {
        /// Content.
        data: Vec<u8>,
        /// Media type.
        content_type: Option<String>,
    },
}

/// Settings shared by all requests of a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSettings {
    /// `User-Agent` sent when the request gives none.
    pub user_agent: String,
    /// Additional trusted certificates in PEM form.
    pub ca_file: Option<PathBuf>,
    /// Proxy of the `http.proxy` setting.
    pub proxy: Option<String>,
    /// Pauses between attempts.
    pub retry: RetryPolicy,
    /// Bytes of body between two progress events.
    pub progress_step: u64,
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            ca_file: None,
            proxy: None,
            retry: RetryPolicy::default(),
            progress_step: DEFAULT_PROGRESS_STEP,
        }
    }
}

impl ClientSettings {
    /// Settings from the configuration; values without a configuration key
    /// keep their defaults.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        Self {
            proxy: config.http.proxy.clone(),
            ..Self::default()
        }
    }
}

/// One request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSpec {
    /// Method.
    pub method: Method,
    /// Address.
    pub target: Target,
    /// Headers in order; names are case-insensitive.
    pub headers: Vec<(String, String)>,
    /// Body.
    pub body: RequestBody,
    /// Time limit of the whole request.
    pub timeout: Duration,
    /// Follow redirects.
    pub follow_redirects: bool,
    /// Largest number of redirects followed.
    pub max_redirects: u32,
    /// Retries after a failed attempt (ignored for `post` and `patch`).
    pub retries: u32,
    /// Verify the certificate of the server.
    pub verify_tls: bool,
    /// Pinned addresses.
    pub pins: Pins,
    /// Proxy server.
    pub proxy: ProxyChoice,
    /// Ask for compressed bodies and decompress them.
    pub compression: bool,
}

impl RequestSpec {
    /// A request with default settings: no headers and no body, 30 s, up
    /// to 10 redirects, no retries, verified TLS, the system proxy and
    /// compression.
    #[must_use]
    pub fn new(method: Method, target: Target) -> Self {
        Self {
            method,
            target,
            headers: Vec::new(),
            body: RequestBody::None,
            timeout: DEFAULT_TIMEOUT,
            follow_redirects: true,
            max_redirects: DEFAULT_MAX_REDIRECTS,
            retries: 0,
            verify_tls: true,
            pins: Pins::new(),
            proxy: ProxyChoice::System,
            compression: true,
        }
    }

    /// Whether a header with this name is set.
    #[must_use]
    pub fn has_header(&self, name: &str) -> bool {
        self.headers
            .iter()
            .any(|(header, _)| header.eq_ignore_ascii_case(name))
    }
}

/// One redirect followed by the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectStep {
    /// Status of the redirect response.
    pub status: u16,
    /// Address that answered with the redirect.
    pub url: String,
    /// Address the redirect points to.
    pub location: String,
    /// IP addresses connected to for `url`.
    pub addresses: Vec<IpAddr>,
}

/// Status line and headers of a response with facts about the exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseHead {
    /// Status code.
    pub status: u16,
    /// Headers in order of arrival, names in lower case.
    pub headers: Vec<(String, String)>,
    /// Address of the request.
    pub url: String,
    /// Address that gave this response, after redirects.
    pub final_url: String,
    /// Redirects followed, in order.
    pub redirects: Vec<RedirectStep>,
    /// Address of the connection that gave the response: the server, or the
    /// proxy when one is used.
    pub remote_address: Option<SocketAddr>,
    /// Proxy server used.
    pub proxy: Option<String>,
    /// Attempts made, 1 without retries.
    pub attempts: u32,
}

impl ResponseHead {
    /// First value of a header.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Media type of the `Content-Type` header in lower case, without
    /// parameters.
    #[must_use]
    pub fn media_type(&self) -> Option<String> {
        let value = self.header("content-type")?;
        let media = value.split(';').next().unwrap_or_default().trim();
        (!media.is_empty()).then(|| media.to_ascii_lowercase())
    }

    /// `charset` parameter of the `Content-Type` header in lower case.
    #[must_use]
    pub fn charset(&self) -> Option<String> {
        self.header("content-type")?
            .split(';')
            .skip(1)
            .filter_map(|parameter| parameter.split_once('='))
            .find(|(name, _)| name.trim().eq_ignore_ascii_case("charset"))
            .map(|(_, value)| value.trim().trim_matches('"').to_ascii_lowercase())
            .filter(|value| !value.is_empty())
    }
}

/// A response: its head and the body still to be read.
#[derive(Debug)]
pub struct Exchange {
    /// Status, headers and facts about the exchange.
    pub head: ResponseHead,
    /// Body.
    pub body: ResponseBody,
}

/// HTTP client.
///
/// ```no_run
/// use nurt_net::client::{ClientSettings, HttpClient, Method, RequestSpec, Target};
/// # fn run(ctx: &nurt_contracts::stage::ExecCtx) -> nurt_contracts::error::Result<()> {
/// let client = HttpClient::new(ClientSettings::from_config(ctx.config()));
/// let spec = RequestSpec::new(Method::Get, Target::parse("https://example.test/")?);
/// let mut exchange = client.send(&spec, ctx)?;
/// let body = exchange.body.read_limited(2 * 1024 * 1024)?;
/// assert_eq!(exchange.head.status, 200);
/// # let _ = body;
/// # Ok(())
/// # }
/// ```
pub struct HttpClient {
    settings: ClientSettings,
    resolver: Arc<dyn NameResolver>,
    roots: Mutex<Option<RootCerts>>,
}

impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

struct Connected {
    response: http::Response<ureq::Body>,
    remote: SocketAddr,
    addresses: Vec<IpAddr>,
    proxy: Option<Url>,
    watch: CancelWatch,
}

struct Hop<'a> {
    target: &'a Target,
    method: Method,
    headers: &'a [(String, String)],
    body: &'a RequestBody,
    number: u32,
    attempt: u32,
}

impl HttpClient {
    /// A client that resolves names with the resolver of the operating
    /// system.
    #[must_use]
    pub fn new(settings: ClientSettings) -> Self {
        Self {
            settings,
            resolver: Arc::new(SystemResolver),
            roots: Mutex::new(None),
        }
    }

    /// Replaces the name resolver.
    #[must_use]
    pub fn with_resolver(mut self, resolver: Arc<dyn NameResolver>) -> Self {
        self.resolver = resolver;
        self
    }

    /// Settings of the client.
    #[must_use]
    pub fn settings(&self) -> &ClientSettings {
        &self.settings
    }

    /// Sends a request and returns the response once its head arrived.
    ///
    /// Statuses of 400 and above are responses, not errors. Attempts that
    /// end with a connection failure, status 429 or a 5xx status are
    /// repeated up to `spec.retries` times (not for `post` and `patch`),
    /// with a `progress` event of phase `retry` before each repetition.
    ///
    /// # Errors
    ///
    /// - `E_ARG` for an invalid header or proxy;
    /// - `E_NETWORK` with `problem` `dns`, `connect`, `reset`, `tls`,
    ///   `proxy`, `redirect_limit`, `bad_location`, `protocol` or `denied`
    ///   (refused by the [`ConnectGate`]);
    /// - `E_TIMEOUT` when the time limit of the request or of the program
    ///   elapses, `E_CANCELLED` when the execution is cancelled;
    /// - `E_CONFIG` when `ca_file` cannot be read.
    pub fn send(&self, spec: &RequestSpec, ctx: &ExecCtx) -> Result<Exchange> {
        validate_headers(&spec.headers)?;
        let budget = ctx
            .remaining()
            .map_or(spec.timeout, |remaining| remaining.min(spec.timeout));
        let deadline = Instant::now() + budget;
        let retries = if spec.method.is_retryable() {
            spec.retries
        } else {
            0
        };
        let mut attempt = 1;
        loop {
            let outcome = self.follow(spec, ctx, deadline, attempt);
            let retry = attempt <= retries;
            let (reason, retry_after) = match &outcome {
                Ok(exchange) if retry && retry::retryable_status(exchange.head.status) => (
                    format!("status {}", exchange.head.status),
                    exchange.head.header("retry-after").map(str::to_owned),
                ),
                Err(error) if retry && retry::retryable_error(error) => {
                    (error.data().message.clone(), None)
                }
                _ => {
                    return outcome.map(|mut exchange| {
                        exchange.head.attempts = attempt;
                        exchange
                    });
                }
            };
            drop(outcome);
            emit_retry(ctx, spec, attempt, retries, &reason);
            let pause = self
                .settings
                .retry
                .pause(attempt - 1, retry_after.as_deref());
            wait(ctx, pause, deadline, &spec.target)?;
            attempt += 1;
        }
    }

    fn follow(
        &self,
        spec: &RequestSpec,
        ctx: &ExecCtx,
        deadline: Instant,
        attempt: u32,
    ) -> Result<Exchange> {
        let mut target = spec.target.clone();
        let mut method = spec.method;
        let mut headers = spec.headers.clone();
        let mut body = spec.body.clone();
        let mut redirects = Vec::new();
        loop {
            let hop = Hop {
                target: &target,
                method,
                headers: &headers,
                body: &body,
                number: u32::try_from(redirects.len()).unwrap_or(u32::MAX),
                attempt,
            };
            let connected = self.connect(spec, &hop, ctx, deadline)?;
            let status = connected.response.status().as_u16();
            let location = redirect_location(&connected.response, spec.follow_redirects);
            let Some(location) = location else {
                return Ok(self.exchange(ctx, &target, spec, connected, redirects));
            };
            if redirects.len() >= usize::try_from(spec.max_redirects).unwrap_or(usize::MAX) {
                return Err(Error::new(
                    ErrorCode::Network,
                    format!(
                        "`{}` redirected more than {} times",
                        spec.target.display_url(),
                        spec.max_redirects
                    ),
                )
                .with_problem("redirect_limit"));
            }
            let next = target.join(&location)?;
            redirects.push(RedirectStep {
                status,
                url: target.display_url(),
                location: next.display_url(),
                addresses: connected.addresses.clone(),
            });
            drop(connected);
            if status == SEE_OTHER || (matches!(status, 301 | 302) && method == Method::Post) {
                method = Method::Get;
                body = RequestBody::None;
                headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-type"));
            }
            if !next.same_origin(&target) {
                headers.retain(|(name, _)| !is_credential_header(name));
            }
            target = next;
        }
    }

    fn exchange(
        &self,
        ctx: &ExecCtx,
        target: &Target,
        spec: &RequestSpec,
        connected: Connected,
        redirects: Vec<RedirectStep>,
    ) -> Exchange {
        let (parts, response_body) = connected.response.into_parts();
        let headers: Vec<(String, String)> = parts
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let head = ResponseHead {
            status: parts.status.as_u16(),
            url: spec.target.display_url(),
            final_url: target.display_url(),
            redirects,
            remote_address: Some(connected.remote),
            proxy: connected.proxy.map(|proxy| proxy.as_str().to_owned()),
            attempts: 1,
            headers,
        };
        let encoding = BodyEncoding::from_header(head.header("content-encoding"));
        let source = BodySource {
            content_length: response_body.content_length(),
            reader: response_body.into_with_config().limit(u64::MAX).reader(),
            encoding,
            watch: connected.watch,
        };
        let body = ResponseBody::new(
            source,
            ctx,
            target.as_str().to_owned(),
            self.settings.progress_step,
        );
        Exchange { head, body }
    }

    fn connect(
        &self,
        spec: &RequestSpec,
        hop: &Hop<'_>,
        ctx: &ExecCtx,
        deadline: Instant,
    ) -> Result<Connected> {
        let proxy = effective_proxy(
            &spec.proxy,
            self.settings.proxy.as_deref(),
            ctx.host(),
            hop.target,
        )?;
        let addresses = self.addresses(spec, hop.target, proxy.as_ref(), ctx, deadline)?;
        check_gate(ctx, hop, &addresses, proxy.as_ref())?;
        let port = proxy
            .as_ref()
            .and_then(Url::port_or_known_default)
            .unwrap_or_else(|| hop.target.port());
        let roots = self.roots()?;
        let mut last = None;
        for address in &addresses {
            let remote = SocketAddr::new(*address, port);
            let agent = self.agent(spec, remote, proxy.as_ref(), &roots, deadline)?;
            let watch = CancelWatch::start(agent.clone(), ctx.cancel().clone());
            match run(&agent, hop, &self.settings) {
                Ok(response) => {
                    return Ok(Connected {
                        response,
                        remote,
                        addresses,
                        proxy,
                        watch,
                    });
                }
                Err(error) => {
                    drop(watch);
                    let error = map_error(error, ctx, hop.target, spec.timeout);
                    if error.data().problem.as_deref() != Some("connect") {
                        return Err(error);
                    }
                    last = Some(error);
                }
            }
        }
        Err(last.unwrap_or_else(|| {
            Error::new(
                ErrorCode::Network,
                format!(
                    "no address to connect to for `{}`",
                    hop.target.display_url()
                ),
            )
            .with_problem("connect")
        }))
    }

    fn addresses(
        &self,
        spec: &RequestSpec,
        target: &Target,
        proxy: Option<&Url>,
        ctx: &ExecCtx,
        deadline: Instant,
    ) -> Result<Vec<IpAddr>> {
        let (host, port, ip) = match proxy {
            Some(proxy) => {
                let host = proxy.host_str().unwrap_or_default();
                let host = host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_owned();
                let ip = host.parse::<IpAddr>().ok();
                (host, proxy.port_or_known_default().unwrap_or_default(), ip)
            }
            None => {
                if let Some(pinned) = spec.pins.get(&target.host(), target.port()) {
                    return Ok(pinned.to_vec());
                }
                (target.host(), target.port(), target.ip())
            }
        };
        if let Some(ip) = ip {
            return Ok(vec![ip]);
        }
        let wait = deadline.saturating_duration_since(Instant::now());
        resolve::lookup(&self.resolver, &host, port, wait, ctx.cancel())
    }

    fn roots(&self) -> Result<RootCerts> {
        let mut roots = self.roots.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(roots) = roots.as_ref() {
            return Ok(roots.clone());
        }
        let loaded = tls::load_roots(self.settings.ca_file.as_deref())?;
        *roots = Some(loaded.clone());
        Ok(loaded)
    }

    fn agent(
        &self,
        spec: &RequestSpec,
        remote: SocketAddr,
        proxy: Option<&Url>,
        roots: &RootCerts,
        deadline: Instant,
    ) -> Result<ureq::Agent> {
        let proxy = proxy
            .map(|url| {
                ureq::Proxy::new(url.as_str()).map_err(|error| {
                    Error::new(
                        ErrorCode::Arg,
                        format!("the proxy `{url}` is not valid: {error}"),
                    )
                })
            })
            .transpose()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .max_idle_connections(0)
            .accept_encoding(if spec.compression {
                ACCEPT_ENCODING
            } else {
                "identity"
            })
            .timeout_global(Some(remaining))
            .tls_config(tls::tls_config(roots, spec.verify_tls))
            .proxy(proxy)
            .build();
        Ok(ureq::Agent::with_parts(
            config,
            DefaultConnector::new(),
            FixedAddress(remote),
        ))
    }
}

/// Status `303 See Other`.
const SEE_OTHER: u16 = 303;

/// Resolver that answers every lookup with the one address chosen by the
/// client for the current connection.
#[derive(Debug)]
struct FixedAddress(SocketAddr);

impl Resolver for FixedAddress {
    fn resolve(
        &self,
        _uri: &http::Uri,
        _config: &ureq::config::Config,
        _timeout: NextTimeout,
    ) -> std::result::Result<ResolvedSocketAddrs, ureq::Error> {
        let mut addresses = self.empty();
        addresses.push(self.0);
        Ok(addresses)
    }
}

fn run(
    agent: &ureq::Agent,
    hop: &Hop<'_>,
    settings: &ClientSettings,
) -> std::result::Result<http::Response<ureq::Body>, ureq::Error> {
    let mut builder = http::Request::builder()
        .method(hop.method.to_http())
        .uri(hop.target.without_credentials().as_str());
    for (name, value) in hop.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let has = |name: &str| {
        hop.headers
            .iter()
            .any(|(header, _)| header.eq_ignore_ascii_case(name))
    };
    if let Some((user, password)) = hop.target.credentials()
        && !has("authorization")
    {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
        builder = builder.header("authorization", format!("Basic {token}"));
    }
    if !has("user-agent") {
        builder = builder.header("user-agent", settings.user_agent.as_str());
    }
    match hop.body {
        RequestBody::None => agent.run(builder.body(())?),
        RequestBody::Bytes { data, content_type } => {
            if let Some(content_type) = content_type
                && !has("content-type")
            {
                builder = builder.header("content-type", content_type.as_str());
            }
            agent.run(builder.body(data.clone())?)
        }
    }
}

fn redirect_location(response: &http::Response<ureq::Body>, follow: bool) -> Option<String> {
    let status = response.status().as_u16();
    if !follow || !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    response
        .headers()
        .get("location")
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
}

fn is_credential_header(name: &str) -> bool {
    ["authorization", "cookie", "proxy-authorization"]
        .iter()
        .any(|header| name.eq_ignore_ascii_case(header))
}

fn validate_headers(headers: &[(String, String)]) -> Result<()> {
    for (name, value) in headers {
        let valid_name = !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && !b"()<>@,;:\\\"/[]?={}".contains(&byte));
        if !valid_name {
            return Err(Error::new(
                ErrorCode::Arg,
                format!("`{name}` is not a valid header name"),
            ));
        }
        if value.chars().any(char::is_control) {
            return Err(Error::new(
                ErrorCode::Arg,
                format!("the value of header `{name}` contains control characters"),
            ));
        }
        if value.chars().count() > MAX_HEADER_VALUE_CHARS {
            return Err(Error::new(
                ErrorCode::Arg,
                format!(
                    "the value of header `{name}` is longer than {MAX_HEADER_VALUE_CHARS} characters"
                ),
            ));
        }
    }
    Ok(())
}

fn check_gate(
    ctx: &ExecCtx,
    hop: &Hop<'_>,
    addresses: &[IpAddr],
    proxy: Option<&Url>,
) -> Result<()> {
    let Some(gate) = ctx.extensions().get::<dyn ConnectGate>() else {
        return Ok(());
    };
    let host = hop.target.host();
    let url = hop.target.display_url();
    let request = ConnectRequest {
        url: &url,
        host: &host,
        port: hop.target.port(),
        addresses,
        proxy: proxy.map(Url::as_str),
        hop: hop.number,
        attempt: hop.attempt,
    };
    gate.check(&request).map_err(|reason| {
        Error::new(
            ErrorCode::Network,
            format!("the connection to `{host}` was refused: {reason}"),
        )
        .with_problem("denied")
    })
}

fn emit_retry(ctx: &ExecCtx, spec: &RequestSpec, attempt: u32, retries: u32, reason: &str) {
    ctx.emit(Event::Progress(Progress {
        command: ctx.frame().command.clone(),
        phase: "retry".to_owned(),
        done: u64::from(attempt),
        total: Some(u64::from(retries) + 1),
        bytes: None,
        message: Some(format!(
            "retry {attempt}/{retries} of {} after {reason}",
            spec.target.display_url()
        )),
    }));
}

fn wait(ctx: &ExecCtx, pause: Duration, deadline: Instant, target: &Target) -> Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if pause >= remaining {
        return Err(timeout_error(target));
    }
    ctx.sleep(pause)
}

fn timeout_error(target: &Target) -> Error {
    Error::new(
        ErrorCode::Timeout,
        format!(
            "the request to `{}` exceeded its time limit",
            target.display_url()
        ),
    )
}

fn map_error(error: ureq::Error, ctx: &ExecCtx, target: &Target, limit: Duration) -> Error {
    if let Some(reason) = ctx.cancel().reason() {
        return reason.to_error();
    }
    let shown = target.display_url();
    let url = shown.as_str();
    let network = |problem: &str, message: String| {
        Error::new(ErrorCode::Network, message).with_problem(problem)
    };
    match error {
        ureq::Error::Timeout(_) => Error::new(
            ErrorCode::Timeout,
            format!(
                "the request to `{url}` exceeded its time limit of {} ms",
                limit.as_millis()
            ),
        ),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::Pem(_) => {
            tls_error(url, &error.to_string())
        }
        ureq::Error::ConnectProxyFailed(_) | ureq::Error::InvalidProxyUrl => network(
            "proxy",
            format!("the proxy refused or broke the connection to `{url}`: {error}"),
        ),
        ureq::Error::HostNotFound => network("dns", format!("the host of `{url}` was not found")),
        ureq::Error::ConnectionFailed => {
            network("connect", format!("connecting to `{url}` failed"))
        }
        ureq::Error::Io(io) => map_io(&io, url),
        ureq::Error::Decompress(..) => network(
            "decode",
            format!("the compressed response of `{url}` is damaged: {error}"),
        ),
        other => network(
            "protocol",
            format!("the exchange with `{url}` failed: {other}"),
        ),
    }
}

fn map_io(io: &std::io::Error, url: &str) -> Error {
    use std::io::ErrorKind;
    if io
        .get_ref()
        .is_some_and(|inner| inner.downcast_ref::<rustls::Error>().is_some())
    {
        return tls_error(url, &io.to_string());
    }
    let problem = match io.kind() {
        ErrorKind::TimedOut => {
            return Error::new(
                ErrorCode::Timeout,
                format!("the request to `{url}` exceeded its time limit"),
            );
        }
        ErrorKind::ConnectionRefused
        | ErrorKind::AddrNotAvailable
        | ErrorKind::HostUnreachable
        | ErrorKind::NetworkUnreachable
        | ErrorKind::NetworkDown => "connect",
        _ => "reset",
    };
    Error::new(
        ErrorCode::Network,
        format!("the connection to `{url}` failed: {io}"),
    )
    .with_problem(problem)
}

fn tls_error(url: &str, reason: &str) -> Error {
    Error::new(
        ErrorCode::Network,
        format!("the TLS connection to `{url}` failed: {reason}"),
    )
    .with_problem("tls")
    .with_fix(format!("http.request \"{url}\" verify_tls: false"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_round_trip_and_classify() {
        for name in ["get", "head", "post", "put", "patch", "delete", "options"] {
            assert_eq!(Method::parse(name).unwrap().as_str(), name);
        }
        assert!(Method::parse("trace").is_none());
        assert!(Method::Head.is_read_only());
        assert!(!Method::Post.is_retryable());
        assert!(Method::Put.is_retryable());
    }

    #[test]
    fn validates_header_names_and_values() {
        let ok = vec![("Accept-Language".to_owned(), "pl".to_owned())];
        assert!(validate_headers(&ok).is_ok());
        let bad_name = vec![("A b".to_owned(), "x".to_owned())];
        assert_eq!(
            validate_headers(&bad_name).unwrap_err().code(),
            ErrorCode::Arg
        );
        let split = vec![("X".to_owned(), "a\r\nInjected: 1".to_owned())];
        assert_eq!(validate_headers(&split).unwrap_err().code(), ErrorCode::Arg);
        let long = vec![("X".to_owned(), "v".repeat(MAX_HEADER_VALUE_CHARS + 1))];
        assert_eq!(validate_headers(&long).unwrap_err().code(), ErrorCode::Arg);
    }

    #[test]
    fn reads_media_type_and_charset() {
        let head = ResponseHead {
            status: 200,
            headers: vec![(
                "content-type".to_owned(),
                "Text/HTML; Charset=\"Windows-1250\"".to_owned(),
            )],
            url: String::new(),
            final_url: String::new(),
            redirects: Vec::new(),
            remote_address: None,
            proxy: None,
            attempts: 1,
        };
        assert_eq!(head.media_type().as_deref(), Some("text/html"));
        assert_eq!(head.charset().as_deref(), Some("windows-1250"));
        assert_eq!(head.header("Content-Type").map(str::len), Some(33));
    }

    #[test]
    fn maps_library_errors_to_problems() {
        let ctx = nurt_testing_ctx();
        let target = Target::parse("https://a.test/").unwrap();
        let limit = Duration::from_secs(1);
        let tls = map_error(ureq::Error::Tls("bad"), &ctx, &target, limit);
        assert_eq!(tls.data().problem.as_deref(), Some("tls"));
        assert!(!tls.data().fix.is_empty());
        let refused = map_error(
            ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)),
            &ctx,
            &target,
            limit,
        );
        assert_eq!(refused.data().problem.as_deref(), Some("connect"));
        let proxy = map_error(ureq::Error::InvalidProxyUrl, &ctx, &target, limit);
        assert_eq!(proxy.data().problem.as_deref(), Some("proxy"));
        let other = map_error(ureq::Error::RedirectFailed, &ctx, &target, limit);
        assert_eq!(other.data().problem.as_deref(), Some("protocol"));
    }

    fn nurt_testing_ctx() -> ExecCtx {
        use nurt_contracts::env::{
            BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime,
        };
        use nurt_contracts::stage::{CancelReason, CancelToken};
        struct Fixed;
        impl Clock for Fixed {
            fn now(&self) -> WallTime {
                WallTime::from_unix_nanos(0)
            }
            fn monotonic(&self) -> Duration {
                Duration::ZERO
            }
            fn stamp(&self) -> ClockStamp {
                ClockStamp {
                    wall: self.now(),
                    boot: BootId(String::new()),
                    since_boot_with_suspend: Duration::ZERO,
                    since_boot_active: Duration::ZERO,
                }
            }
            fn sleep(
                &self,
                _duration: Duration,
                cancel: &CancelToken,
            ) -> std::result::Result<(), CancelReason> {
                cancel.reason().map_or(Ok(()), Err)
            }
        }
        struct NoEnv(HostInfo);
        impl Host for NoEnv {
            fn info(&self) -> &HostInfo {
                &self.0
            }
            fn env_var(&self, _name: &str) -> Option<std::ffi::OsString> {
                None
            }
            fn env_vars(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
                Vec::new()
            }
        }
        let info = HostInfo {
            os: Os::Linux,
            os_version: String::new(),
            architecture: String::new(),
            hostname: String::new(),
            user: String::new(),
            home: PathBuf::from("/"),
            temp: PathBuf::from("/tmp"),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        };
        ExecCtx::builder(Arc::new(Fixed), Arc::new(NoEnv(info))).build()
    }
}
