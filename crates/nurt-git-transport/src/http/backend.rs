// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Implementation of the HTTP interface of `gix-transport` on `ureq` with
//! `rustls`.
//!
//! Errors of a request are delivered when the transport reads the response
//! headers, as I/O errors: a `401` becomes `PermissionDenied` carrying
//! [`AuthenticationRequired`], which makes the protocol layer ask for
//! credentials; every other failure carries a [`NetworkFailure`].

use std::any::Any;
use std::io::{self, BufRead, BufReader, Cursor, Read};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use gix_transport::client::AuthenticationRequired;
use gix_transport::client::blocking_io::http::{GetResponse, Http, PostBodyDataKind, PostResponse};
use gix_utils::io::pipe;

use super::failure::{NetworkFailure, NetworkProblem};
use super::redirect::{self, MAX_REDIRECTS};
use super::settings::{FollowRedirects, HttpSettings};

/// Writes buffered between the producing and the consuming side of a pipe.
const PIPE_WRITES_IN_FLIGHT: usize = 16;

/// Size of the buffer of response bodies read directly from the connection.
const BODY_BUFFER: usize = 64 * 1024;

/// Response headers as `name: value` lines, or the failure of the request.
pub struct ResponseHeaders {
    state: HeaderState,
}

enum HeaderState {
    Pending(Receiver<io::Result<Vec<u8>>>),
    Ready(Cursor<Vec<u8>>),
    Failed(Option<io::Error>),
}

impl ResponseHeaders {
    fn ready(lines: Vec<u8>) -> Self {
        Self {
            state: HeaderState::Ready(Cursor::new(lines)),
        }
    }

    fn failed(error: io::Error) -> Self {
        Self {
            state: HeaderState::Failed(Some(error)),
        }
    }

    fn resolve(&mut self) -> io::Result<&mut Cursor<Vec<u8>>> {
        if let HeaderState::Pending(receiver) = &self.state {
            self.state = match receiver.recv() {
                Ok(Ok(lines)) => HeaderState::Ready(Cursor::new(lines)),
                Ok(Err(error)) => HeaderState::Failed(Some(error)),
                Err(_) => HeaderState::Failed(Some(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "the HTTP request ended without a response",
                ))),
            };
        }
        match &mut self.state {
            HeaderState::Ready(cursor) => Ok(cursor),
            HeaderState::Failed(error) => Err(error
                .take()
                .unwrap_or_else(|| io::Error::other("the HTTP request failed"))),
            HeaderState::Pending(_) => Err(io::Error::other("the HTTP response is not available")),
        }
    }
}

impl Read for ResponseHeaders {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.resolve()?.read(buf)
    }
}

impl BufRead for ResponseHeaders {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.resolve()?.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        if let HeaderState::Ready(cursor) = &mut self.state {
            cursor.consume(amount);
        }
    }
}

/// Body of a response.
pub type ResponseBody = Box<dyn BufRead + Send>;

/// Smart HTTP backend for [`gix_transport`], built on `ureq`.
///
/// One value serves all requests of one connection. Requests run on the
/// calling thread, except request bodies, which are produced by the caller
/// while a worker thread sends them.
pub struct UreqHttp {
    settings: Arc<HttpSettings>,
    agent: Option<ureq::Agent>,
    redirected_base: Arc<Mutex<Option<String>>>,
    requests: usize,
}

impl UreqHttp {
    /// Backend with the given options.
    #[must_use]
    pub fn new(settings: HttpSettings) -> Self {
        Self {
            settings: Arc::new(settings),
            agent: None,
            redirected_base: Arc::new(Mutex::new(None)),
            requests: 0,
        }
    }

    /// Options in use.
    #[must_use]
    pub fn settings(&self) -> &HttpSettings {
        &self.settings
    }

    fn agent(&mut self, url: &str) -> Result<ureq::Agent, NetworkFailure> {
        if let Some(agent) = &self.agent {
            return Ok(agent.clone());
        }
        let agent = build_agent(&self.settings, url)?;
        self.agent = Some(agent.clone());
        Ok(agent)
    }

    fn base(&self) -> Option<String> {
        self.redirected_base
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_base(&self, base: String) {
        *self
            .redirected_base
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(base);
    }

    fn request_headers<I, S>(&self, headers: I) -> Vec<(String, String)>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out: Vec<(String, String)> = headers
            .into_iter()
            .filter_map(|line| split_header(line.as_ref()))
            .filter(|(name, _)| !name.eq_ignore_ascii_case("user-agent"))
            .collect();
        out.push(("User-Agent".to_owned(), self.settings.user_agent.clone()));
        out.extend(
            self.settings
                .extra_headers
                .iter()
                .filter_map(|line| split_header(line)),
        );
        let has_authorization = out
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"));
        if let (Some(token), false) = (&self.settings.bearer, has_authorization) {
            out.push((
                "Authorization".to_owned(),
                format!("Bearer {}", token.expose()),
            ));
        }
        out
    }

    fn may_follow(&mut self) -> bool {
        self.requests += 1;
        match self.settings.follow_redirects {
            FollowRedirects::All => true,
            FollowRedirects::Initial => self.requests == 1,
            FollowRedirects::None => false,
        }
    }

    fn execute_get(
        &mut self,
        url: &str,
        base_url: &str,
        mut headers: Vec<(String, String)>,
    ) -> io::Result<(Vec<u8>, ResponseBody)> {
        let agent = self.agent(url).map_err(NetworkFailure::into_io)?;
        let may_follow = self.may_follow();
        let effective = redirect::swap_tails(self.base().as_deref(), base_url, url);
        if !redirect::same_authority(&effective, url) {
            strip_authorization(&mut headers);
        }
        let tail = url.strip_prefix(base_url).unwrap_or_default().to_owned();
        let mut current = effective.clone();
        for _ in 0..=MAX_REDIRECTS {
            let mut request = agent.get(&current);
            for (name, value) in &headers {
                request = request.header(name.as_str(), value.as_str());
            }
            let response = request
                .call()
                .map_err(|e| classify(&current, e).into_io())?;
            let status = response.status().as_u16();
            if is_redirect(status) {
                let next = self.redirect_target(&response, &current, &tail, may_follow)?;
                if !redirect::same_authority(&next, &current) {
                    strip_authorization(&mut headers);
                }
                current = next;
                continue;
            }
            if current != effective {
                let base =
                    redirect::base_url(&current, base_url, url).map_err(NetworkFailure::into_io)?;
                self.set_base(base);
            }
            check_status(&response, &current)?;
            let lines = header_lines(&response);
            let body = LowSpeed::new(response.into_body().into_reader(), &self.settings, &current);
            return Ok((lines, Box::new(BufReader::with_capacity(BODY_BUFFER, body))));
        }
        Err(NetworkFailure::new(NetworkProblem::Redirect, url, "too many redirects").into_io())
    }

    fn redirect_target(
        &self,
        response: &ureq::http::Response<ureq::Body>,
        current: &str,
        tail: &str,
        may_follow: bool,
    ) -> io::Result<String> {
        let refuse = |message: &str| {
            NetworkFailure::new(NetworkProblem::Redirect, current, message).into_io()
        };
        if !may_follow {
            return Err(refuse(
                "redirect not followed; set http.followRedirects to true to allow it",
            ));
        }
        if !self.settings.extra_headers.is_empty() {
            return Err(refuse(
                "refusing to follow a redirect while http.extraHeader is set",
            ));
        }
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| refuse("redirect without a Location header"))?;
        let next = url::Url::parse(current)
            .and_then(|base| base.join(location))
            .map_err(|_| refuse("redirect to an invalid address"))?
            .to_string();
        if !redirect::scheme_is_safe(&next, current) {
            return Err(refuse("refusing redirect to an insecure scheme"));
        }
        if !redirect::same_authority(&next, current) && !next.ends_with(tail) {
            return Err(refuse(
                "refusing redirect to another server with a different request path",
            ));
        }
        Ok(next)
    }
}

impl Http for UreqHttp {
    type Headers = ResponseHeaders;
    type ResponseBody = ResponseBody;
    type PostBody = pipe::Writer;

    fn get(
        &mut self,
        url: &str,
        base_url: &str,
        headers: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> gix::error::Result<GetResponse<Self::Headers, Self::ResponseBody>> {
        let headers = self.request_headers(headers);
        Ok(match self.execute_get(url, base_url, headers) {
            Ok((lines, body)) => GetResponse {
                headers: ResponseHeaders::ready(lines),
                body,
            },
            Err(error) => GetResponse {
                headers: ResponseHeaders::failed(error),
                body: Box::new(io::empty()) as ResponseBody,
            },
        })
    }

    fn post(
        &mut self,
        url: &str,
        base_url: &str,
        headers: impl IntoIterator<Item = impl AsRef<str>>,
        body: PostBodyDataKind,
    ) -> gix::error::Result<PostResponse<Self::Headers, Self::ResponseBody, Self::PostBody>> {
        let mut headers = self.request_headers(headers);
        self.requests += 1;
        let effective = redirect::swap_tails(self.base().as_deref(), base_url, url);
        if !redirect::same_authority(&effective, url) {
            strip_authorization(&mut headers);
        }
        let (post_writer, post_reader) = pipe::unidirectional(PIPE_WRITES_IN_FLIGHT);
        let agent = match self.agent(url) {
            Ok(agent) => agent,
            Err(failure) => {
                return Ok(PostResponse {
                    post_body: post_writer,
                    headers: ResponseHeaders::failed(failure.into_io()),
                    body: Box::new(io::empty()),
                });
            }
        };
        let (body_writer, body_reader) = pipe::unidirectional(PIPE_WRITES_IN_FLIGHT);
        let (header_sender, header_receiver) = mpsc::sync_channel(1);
        let job = PostJob {
            agent,
            url: effective,
            headers,
            buffer_limit: match body {
                PostBodyDataKind::BoundedAndFitsIntoMemory => usize::MAX,
                PostBodyDataKind::Unbounded => self.settings.post_buffer,
            },
            settings: Arc::clone(&self.settings),
        };
        std::thread::Builder::new()
            .name("nurt-git-http-post".to_owned())
            .spawn(move || job.run(post_reader, &header_sender, body_writer))
            .map_err(|error| gix::error::Error::from_error(error))?;
        Ok(PostResponse {
            post_body: post_writer,
            headers: ResponseHeaders {
                state: HeaderState::Pending(header_receiver),
            },
            body: Box::new(body_reader),
        })
    }

    fn configure(&mut self, config: &dyn Any) -> gix::error::Result {
        if let Some(settings) = config.downcast_ref::<HttpSettings>() {
            self.settings = Arc::new(settings.clone());
            self.agent = None;
        }
        Ok(())
    }

    fn redirected_base_url(&self) -> Option<String> {
        self.base()
    }
}

/// One `POST` request, sent by a worker thread while the caller writes the
/// body.
struct PostJob {
    agent: ureq::Agent,
    url: String,
    headers: Vec<(String, String)>,
    buffer_limit: usize,
    settings: Arc<HttpSettings>,
}

impl PostJob {
    fn run(
        self,
        mut post: pipe::Reader,
        headers: &SyncSender<io::Result<Vec<u8>>>,
        mut body: pipe::Writer,
    ) {
        let response = self.send(&mut post);
        drop(post);
        let response = match response
            .and_then(|response| check_status(&response, &self.url).map(|()| response))
        {
            Ok(response) => response,
            Err(error) => {
                headers.send(Err(error)).ok();
                return;
            }
        };
        if headers.send(Ok(header_lines(&response))).is_err() {
            return;
        }
        let mut reader = LowSpeed::new(
            response.into_body().into_reader(),
            &self.settings,
            &self.url,
        );
        if let Err(error) = io::copy(&mut reader, &mut body) {
            body.channel.send(Err(error)).ok();
        }
    }

    fn send(&self, post: &mut pipe::Reader) -> io::Result<ureq::http::Response<ureq::Body>> {
        let mut prefix = Vec::new();
        let probe = u64::try_from(self.buffer_limit)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        post.by_ref().take(probe).read_to_end(&mut prefix)?;
        let mut request = self.agent.post(&self.url);
        for (name, value) in &self.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let result = if prefix.len() <= self.buffer_limit {
            request.send(&prefix[..])
        } else {
            let rest = std::mem::replace(post, pipe::unidirectional(0).1);
            request.send(ureq::SendBody::from_owned_reader(
                Cursor::new(prefix).chain(rest),
            ))
        };
        result.map_err(|error| classify(&self.url, error).into_io())
    }
}

/// Response body reader that fails when the transfer stays slower than
/// `http.lowSpeedLimit` for `http.lowSpeedTime`.
struct LowSpeed<R> {
    inner: R,
    limit: u64,
    window: Duration,
    started: Instant,
    bytes: u64,
    url: String,
}

impl<R: Read> LowSpeed<R> {
    fn new(inner: R, settings: &HttpSettings, url: &str) -> Self {
        Self {
            inner,
            limit: u64::from(settings.low_speed_limit),
            window: settings.low_speed_time,
            started: Instant::now(),
            bytes: 0,
            url: url.to_owned(),
        }
    }
}

impl<R: Read> Read for LowSpeed<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf).map_err(|error| match error.kind() {
            io::ErrorKind::TimedOut => NetworkFailure::new(
                NetworkProblem::Timeout,
                &self.url,
                "the server stopped sending data",
            )
            .into_io(),
            _ => error,
        })?;
        if self.limit > 0 && !self.window.is_zero() {
            self.bytes += read as u64;
            let elapsed = self.started.elapsed();
            if elapsed >= self.window {
                if self.bytes < self.limit.saturating_mul(elapsed.as_secs().max(1)) {
                    return Err(NetworkFailure::new(
                        NetworkProblem::Timeout,
                        &self.url,
                        format!(
                            "transfer slower than {} bytes/s for {} s (http.lowSpeedLimit)",
                            self.limit,
                            self.window.as_secs()
                        ),
                    )
                    .into_io());
                }
                self.started = Instant::now();
                self.bytes = 0;
            }
        }
        Ok(read)
    }
}

fn split_header(line: &str) -> Option<(String, String)> {
    let (name, value) = line.split_once(':')?;
    let name = name.trim();
    (!name.is_empty()).then(|| (name.to_owned(), value.trim().to_owned()))
}

/// Removes the credentials of the original address from the headers of a
/// request that goes to another server.
fn strip_authorization(headers: &mut Vec<(String, String)>) {
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn header_lines(response: &ureq::http::Response<ureq::Body>) -> Vec<u8> {
    let mut lines = Vec::new();
    for (name, value) in response.headers() {
        lines.extend_from_slice(name.as_str().as_bytes());
        lines.extend_from_slice(b": ");
        lines.extend_from_slice(value.as_bytes());
        lines.push(b'\n');
    }
    lines
}

fn check_status(response: &ureq::http::Response<ureq::Body>, url: &str) -> io::Result<()> {
    let status = response.status().as_u16();
    if status == 401 {
        let www_authenticate = response
            .headers()
            .get_all("www-authenticate")
            .iter()
            .map(|value| value.as_bytes().into())
            .collect();
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            AuthenticationRequired { www_authenticate },
        ));
    }
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(NetworkFailure::status(url, status).into_io())
    }
}

/// Maps a `ureq` error to the kind of network failure.
fn classify(url: &str, error: ureq::Error) -> NetworkFailure {
    let problem = match &error {
        ureq::Error::HostNotFound => NetworkProblem::Dns,
        ureq::Error::Timeout(_) => NetworkProblem::Timeout,
        ureq::Error::Tls(_)
        | ureq::Error::Pem(_)
        | ureq::Error::Rustls(_)
        | ureq::Error::TlsRequired => NetworkProblem::Tls,
        ureq::Error::InvalidProxyUrl => NetworkProblem::Proxy,
        ureq::Error::Io(io_error) => classify_io(io_error),
        other if other.to_string().to_ascii_lowercase().contains("proxy") => NetworkProblem::Proxy,
        _ => NetworkProblem::Connect,
    };
    NetworkFailure::new(problem, url, error.to_string())
}

fn classify_io(error: &io::Error) -> NetworkProblem {
    if error
        .get_ref()
        .is_some_and(|inner| inner.downcast_ref::<rustls::Error>().is_some())
    {
        return NetworkProblem::Tls;
    }
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => NetworkProblem::Timeout,
        _ => NetworkProblem::Connect,
    }
}

fn build_agent(settings: &HttpSettings, url: &str) -> Result<ureq::Agent, NetworkFailure> {
    let tls_failure = |message: String| NetworkFailure::new(NetworkProblem::Tls, url, message);
    let roots = root_certificates(settings).map_err(tls_failure)?;
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(roots)
        .disable_verification(!settings.ssl_verify)
        .build();
    let proxy = settings
        .proxy
        .as_deref()
        .map(ureq::Proxy::new)
        .transpose()
        .map_err(|error| {
            NetworkFailure::new(
                NetworkProblem::Proxy,
                url,
                format!("invalid proxy address: {error}"),
            )
        })?;
    let read_timeout = (settings.low_speed_limit > 0 && !settings.low_speed_time.is_zero())
        .then_some(settings.low_speed_time);
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .tls_config(tls)
        .proxy(proxy)
        .user_agent(settings.user_agent.as_str())
        .timeout_connect(Some(settings.connect_timeout))
        .timeout_per_read(read_timeout)
        .build();
    Ok(config.new_agent())
}

fn root_certificates(settings: &HttpSettings) -> Result<ureq::tls::RootCerts, String> {
    let mut certificates = Vec::new();
    if let Some(file) = &settings.ca_info {
        read_pem(file, &mut certificates)?;
    }
    if let Some(dir) = &settings.ca_path {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("cannot read http.sslCAPath '{}': {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                read_pem(&path, &mut certificates)?;
            }
        }
    }
    if settings.ca_info.is_none() && settings.ca_path.is_none() {
        let native = rustls_native_certs::load_native_certs();
        certificates.extend(
            native
                .certs
                .iter()
                .map(|der| ureq::tls::Certificate::from_der(der.as_ref()).to_owned()),
        );
        if certificates.is_empty() {
            return Ok(ureq::tls::RootCerts::WebPki);
        }
    }
    Ok(ureq::tls::RootCerts::Specific(Arc::new(certificates)))
}

fn read_pem(path: &Path, out: &mut Vec<ureq::tls::Certificate<'static>>) -> Result<(), String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("cannot read certificate file '{}': {e}", path.display()))?;
    for item in ureq::tls::parse_pem(&bytes) {
        if let Ok(ureq::tls::PemItem::Certificate(certificate)) = item {
            out.push(certificate);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testserver::{Reply, Request, TestServer};
    use super::*;
    use std::io::Write;

    fn settings() -> HttpSettings {
        HttpSettings::default()
    }

    fn read_headers(headers: &mut ResponseHeaders) -> io::Result<String> {
        let mut text = String::new();
        headers.read_to_string(&mut text)?;
        Ok(text)
    }

    fn get(http: &mut UreqHttp, url: &str, base: &str) -> (io::Result<String>, Vec<u8>) {
        let GetResponse {
            mut headers,
            mut body,
        } = http
            .get(url, base, ["Git-Protocol: version=2"])
            .expect("request started");
        let headers = read_headers(&mut headers);
        let mut bytes = Vec::new();
        body.read_to_end(&mut bytes).ok();
        (headers, bytes)
    }

    #[test]
    fn get_returns_headers_and_body_and_sends_configured_headers() {
        let server = TestServer::start(|request: &Request| {
            let agent = request.header("user-agent").unwrap_or_default();
            let extra = request.header("x-extra").unwrap_or_default();
            Reply::ok(
                "text/plain",
                format!(
                    "{agent}|{extra}|{}",
                    request.header("git-protocol").unwrap_or_default()
                ),
            )
        });
        let mut http = UreqHttp::new(HttpSettings {
            extra_headers: vec!["X-Extra: yes".to_owned()],
            user_agent: "git/test".to_owned(),
            ..settings()
        });
        let (headers, body) = get(&mut http, &server.url("/r/info/refs"), &server.url("/r"));
        assert!(
            headers
                .expect("success")
                .to_ascii_lowercase()
                .contains("content-type: text/plain")
        );
        assert_eq!(String::from_utf8_lossy(&body), "git/test|yes|version=2");
    }

    #[test]
    fn unauthorized_response_asks_for_credentials() {
        let server = TestServer::start(|_: &Request| {
            Reply::status(401).with_header("WWW-Authenticate", "Basic realm=\"r\"")
        });
        let mut http = UreqHttp::new(settings());
        let (headers, _) = get(&mut http, &server.url("/r/info/refs"), &server.url("/r"));
        let error = headers.expect_err("authentication required");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let required = error
            .get_ref()
            .and_then(|e| e.downcast_ref::<AuthenticationRequired>());
        assert_eq!(required.map(|r| r.www_authenticate.len()), Some(1));
    }

    #[test]
    fn error_status_is_a_classified_failure() {
        let server = TestServer::start(|_: &Request| Reply::status(404));
        let mut http = UreqHttp::new(settings());
        let (headers, _) = get(&mut http, &server.url("/r/info/refs"), &server.url("/r"));
        let error = headers.expect_err("not found");
        let failure = NetworkFailure::find(&error).expect("classified");
        assert_eq!(
            (failure.problem(), failure.http_status()),
            (NetworkProblem::HttpStatus, Some(404))
        );
    }

    #[test]
    fn follows_initial_redirect_and_records_new_base() {
        let server = TestServer::start(|request: &Request| match request.path.as_str() {
            "/old/info/refs" => Reply::status(301).with_header("Location", "/new/info/refs"),
            "/new/info/refs" => Reply::ok("text/plain", "moved".to_owned()),
            _ => Reply::status(404),
        });
        let mut http = UreqHttp::new(settings());
        let (headers, body) = get(
            &mut http,
            &server.url("/old/info/refs"),
            &server.url("/old"),
        );
        assert!(headers.is_ok());
        assert_eq!(body, b"moved");
        assert_eq!(http.redirected_base_url(), Some(server.url("/new")));
    }

    #[test]
    fn refuses_redirect_after_the_first_request_and_with_extra_headers() {
        let server = TestServer::start(|request: &Request| match request.path.as_str() {
            "/a/x" => Reply::status(302).with_header("Location", "/b/x"),
            _ => Reply::ok("text/plain", "ok".to_owned()),
        });
        let mut http = UreqHttp::new(settings());
        assert!(
            get(&mut http, &server.url("/c/x"), &server.url("/c"))
                .0
                .is_ok()
        );
        let error = get(&mut http, &server.url("/a/x"), &server.url("/a"))
            .0
            .expect_err("second request");
        assert_eq!(
            NetworkFailure::find(&error).map(NetworkFailure::problem),
            Some(NetworkProblem::Redirect)
        );

        let mut http = UreqHttp::new(HttpSettings {
            extra_headers: vec!["Authorization: Bearer t".to_owned()],
            ..settings()
        });
        let error = get(&mut http, &server.url("/a/x"), &server.url("/a"))
            .0
            .expect_err("extra header");
        assert!(error.to_string().contains("http.extraHeader"));
    }

    #[test]
    fn post_sends_small_bodies_with_length_and_large_ones_chunked() {
        let server = TestServer::start(|request: &Request| {
            let mode = if request.header("transfer-encoding").is_some() {
                "chunked"
            } else {
                "length"
            };
            Reply::ok(
                "application/x-git-upload-pack-result",
                format!("{mode}:{}", request.body.len()),
            )
        });
        for (size, expected) in [(10, "length:10"), (100, "chunked:100")] {
            let mut http = UreqHttp::new(HttpSettings {
                post_buffer: 50,
                ..settings()
            });
            let PostResponse {
                mut post_body,
                mut headers,
                mut body,
            } = http
                .post(
                    &server.url("/r/git-upload-pack"),
                    &server.url("/r"),
                    ["Content-Type: x"],
                    PostBodyDataKind::Unbounded,
                )
                .expect("request started");
            post_body
                .write_all(&vec![b'a'; size])
                .expect("body written");
            drop(post_body);
            assert!(read_headers(&mut headers).is_ok());
            let mut text = String::new();
            body.read_to_string(&mut text).expect("body read");
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn bearer_token_is_sent_without_basic_credentials() {
        let server = TestServer::start(|request: &Request| {
            Reply::ok(
                "text/plain",
                request.header("authorization").unwrap_or_default(),
            )
        });
        let mut http = UreqHttp::new(HttpSettings {
            bearer: Some(super::super::settings::BearerToken::new("tok")),
            ..settings()
        });
        assert_eq!(
            get(&mut http, &server.url("/r/x"), &server.url("/r")).1,
            b"Bearer tok"
        );
        let GetResponse { mut body, .. } = http
            .get(
                &server.url("/r/x"),
                &server.url("/r"),
                ["Authorization: Basic abc"],
            )
            .expect("request started");
        let mut text = String::new();
        body.read_to_string(&mut text).expect("body read");
        assert_eq!(text, "Basic abc");
    }

    /// Origin server that redirects the reference discovery to another server,
    /// which answers every request with the `Authorization` header it received.
    fn redirecting_origin() -> (TestServer, TestServer) {
        let target = TestServer::start(|request: &Request| match request.path.as_str() {
            "/new/info/refs" | "/new/git-upload-pack" => Reply::ok(
                "text/plain",
                request.header("authorization").unwrap_or_default(),
            ),
            _ => Reply::status(404),
        });
        let location = target.url("/new/info/refs");
        let origin = TestServer::start(move |request: &Request| match request.path.as_str() {
            "/old/info/refs" => Reply::status(301).with_header("Location", &location),
            _ => Reply::status(404),
        });
        (origin, target)
    }

    #[test]
    fn get_after_a_redirect_to_another_server_does_not_send_credentials() {
        let (origin, _target) = redirecting_origin();
        let mut http = UreqHttp::new(settings());
        for _ in 0..2 {
            let GetResponse { mut body, .. } = http
                .get(
                    &origin.url("/old/info/refs"),
                    &origin.url("/old"),
                    ["Authorization: Basic abc"],
                )
                .expect("request started");
            let mut text = String::new();
            body.read_to_string(&mut text).expect("body read");
            assert_eq!(text, "");
        }
        assert!(http.redirected_base_url().is_some());
    }

    #[test]
    fn post_after_a_redirect_to_another_server_does_not_send_credentials() {
        let (origin, _target) = redirecting_origin();
        let mut http = UreqHttp::new(settings());
        let (headers, _) = get(
            &mut http,
            &origin.url("/old/info/refs"),
            &origin.url("/old"),
        );
        assert!(headers.is_ok());
        let PostResponse {
            mut post_body,
            mut headers,
            mut body,
        } = http
            .post(
                &origin.url("/old/git-upload-pack"),
                &origin.url("/old"),
                ["Authorization: Basic abc"],
                PostBodyDataKind::Unbounded,
            )
            .expect("request started");
        post_body.write_all(b"0000").expect("body written");
        drop(post_body);
        assert!(read_headers(&mut headers).is_ok());
        let mut text = String::new();
        body.read_to_string(&mut text).expect("body read");
        assert_eq!(text, "");
    }

    #[test]
    fn refused_connection_is_a_connect_failure() {
        let url = TestServer::closed_url();
        let mut http = UreqHttp::new(settings());
        let error = get(&mut http, &format!("{url}/r/x"), &format!("{url}/r"))
            .0
            .expect_err("refused");
        assert_eq!(
            NetworkFailure::find(&error).map(NetworkFailure::problem),
            Some(NetworkProblem::Connect)
        );
    }

    #[test]
    fn configure_replaces_settings() {
        let mut http = UreqHttp::new(settings());
        let changed = HttpSettings {
            post_buffer: 7,
            ..settings()
        };
        http.configure(&changed).expect("accepted");
        assert_eq!(http.settings().post_buffer, 7);
        http.configure(&5_u8).expect("ignored");
        assert_eq!(http.settings().post_buffer, 7);
    }

    #[test]
    fn low_speed_reader_fails_below_the_limit() {
        let settings = HttpSettings {
            low_speed_limit: 1_000_000,
            low_speed_time: Duration::from_millis(1),
            ..settings()
        };
        let mut reader = LowSpeed::new(io::repeat(0).take(10), &settings, "https://h");
        reader.started = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        let error = reader.read(&mut [0; 4]).expect_err("too slow");
        assert_eq!(
            NetworkFailure::find(&error).map(NetworkFailure::problem),
            Some(NetworkProblem::Timeout)
        );
    }

    #[test]
    fn splits_header_lines_and_detects_redirect_statuses() {
        assert_eq!(
            split_header("A: b c"),
            Some(("A".to_owned(), "b c".to_owned()))
        );
        assert_eq!(split_header("no colon"), None);
        assert!(is_redirect(308) && !is_redirect(304));
    }
}
