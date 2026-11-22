// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Smart HTTP client backend.
//!
//! [`UreqHttp`] implements the HTTP interface of `gix-transport` on `ureq`
//! and `rustls`, without `curl`, OpenSSL or an asynchronous runtime.
//! [`HttpSettings::resolve`] reads the options of one remote address from the
//! repository configuration and the process environment, and [`connect`]
//! returns the transport used by fetch, clone and push.

mod backend;
mod failure;
mod redirect;
mod settings;
#[cfg(test)]
mod testserver;
pub(crate) mod urlmatch;

pub use backend::{ResponseBody, ResponseHeaders, UreqHttp};
pub use failure::{NetworkFailure, NetworkProblem, redact_url};
pub use settings::{
    BearerToken, DEFAULT_CONNECT_TIMEOUT, DEFAULT_POST_BUFFER, FollowRedirects, HttpNote,
    HttpSettings, default_user_agent,
};

/// Transport of the `git` protocol over smart HTTP.
pub type HttpTransport = gix_transport::client::blocking_io::http::Transport<UreqHttp>;

/// Tells whether the address of a configuration subsection applies to `url`.
///
/// The subsections are those of `http.<url>.*` and `credential.<url>.*`, and
/// the matching rules are those of `git`.
#[must_use]
pub fn url_matches(pattern: &str, url: &str) -> bool {
    urlmatch::matches(pattern, url)
}

/// Opens a smart HTTP transport to `url` with `settings`.
///
/// The connection is made by the first request of the protocol. User name
/// and password in `url` become the identity sent with Basic authentication;
/// over `http://` the transport refuses to send them.
#[must_use]
pub fn connect(
    url: gix_url::Url,
    settings: HttpSettings,
    version: gix_transport::Protocol,
) -> HttpTransport {
    gix_transport::client::blocking_io::http::connect_http(
        UreqHttp::new(settings),
        url,
        version,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::testserver::{Reply, Request, TestServer, pkt_lines};
    use super::*;
    use gix_transport::Service;
    use gix_transport::client::blocking_io::Transport as _;

    #[test]
    fn handshake_reads_protocol_v2_advertisement() {
        let server = TestServer::start(|request: &Request| {
            if request.method != "GET" || request.path != "/r.git/info/refs?service=git-upload-pack"
            {
                return Reply::status(404);
            }
            let mut body = pkt_lines(&["# service=git-upload-pack\n"]);
            body.extend(pkt_lines(&[
                "version 2\n",
                "agent=test\n",
                "ls-refs\n",
                "fetch=shallow\n",
            ]));
            Reply::ok("application/x-git-upload-pack-advertisement", body)
        });
        let url =
            gix_url::parse(bstr::BStr::new(server.url("/r.git").as_bytes())).expect("valid URL");
        let mut transport = connect(url, HttpSettings::default(), gix_transport::Protocol::V2);
        let response = transport
            .handshake(Service::UploadPack, &[])
            .expect("handshake");
        assert_eq!(response.actual_protocol, gix_transport::Protocol::V2);
        assert!(response.capabilities.contains("ls-refs"));
    }

    #[test]
    fn handshake_rejects_dumb_servers() {
        let server = TestServer::start(|_: &Request| Reply::ok("text/plain", "0000"));
        let url =
            gix_url::parse(bstr::BStr::new(server.url("/r.git").as_bytes())).expect("valid URL");
        let mut transport = connect(url, HttpSettings::default(), gix_transport::Protocol::V2);
        assert!(transport.handshake(Service::UploadPack, &[]).is_err());
    }

    #[test]
    fn handshake_failure_keeps_the_network_failure() {
        let server = TestServer::start(|_: &Request| Reply::status(503));
        let url =
            gix_url::parse(bstr::BStr::new(server.url("/r.git").as_bytes())).expect("valid URL");
        let mut transport = connect(url, HttpSettings::default(), gix_transport::Protocol::V2);
        let error = transport
            .handshake(Service::UploadPack, &[])
            .err()
            .expect("unavailable");
        assert_eq!(
            NetworkFailure::find(&error).and_then(NetworkFailure::http_status),
            Some(503)
        );
    }
}
