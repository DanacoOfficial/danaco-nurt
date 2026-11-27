// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-net/tests/http_security.rs
// Responsibility:   Behaviour of `http.request` against hostile or careless servers.
// Dependencies:     flate2, http_server, indexmap, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Behaviour of `http.request` against hostile or careless servers:
//! credentials of the address, headers across redirects, compressed bodies
//! larger than the limit, redirect loops and deeply nested pages.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a failed step ends the test"
)]

#[path = "support/exec_ctx.rs"]
mod exec_ctx;
#[allow(
    unreachable_pub,
    reason = "the shared server module declares its helpers public"
)]
#[path = "support/http_server.rs"]
mod http_server;

use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;
use http_server::{Reply, TestServer};
use indexmap::IndexMap;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::HttpResponse;
use nurt_contracts::value::{Record, Size, Value};
use nurt_net::http::request::{COMMAND, HttpRequest};

fn response(record: &Record) -> HttpResponse {
    HttpResponse::try_from(record.clone()).unwrap()
}

fn fetch(values: &[(&str, Value)]) -> nurt_contracts::error::Result<Vec<Record>> {
    exec_ctx::run(
        &HttpRequest,
        &exec_ctx::args(COMMAND, values),
        &exec_ctx::ctx(),
    )
}

#[test]
fn credentials_in_the_address_are_sent_but_never_echoed() {
    let server = TestServer::start(|request| match request.route() {
        "/start" => Reply::redirect(302, "/final"),
        _ => Reply::body(200, "text/plain", "ok"),
    });
    let url = format!("http://user:secret@{}/start", server.address());
    let records = fetch(&[("url", exec_ctx::text(&url))]).unwrap();
    let record = response(&records[0]);
    assert_eq!(record.url, server.url("/start"));
    assert_eq!(record.final_url, server.url("/final"));
    let shown = format!("{record:?}");
    assert!(!shown.contains("secret"), "{shown}");
    for request in server.requests() {
        assert_eq!(
            request.header("authorization"),
            Some("Basic dXNlcjpzZWNyZXQ=")
        );
    }
}

#[test]
fn cross_origin_redirect_drops_credential_headers_only() {
    let target = TestServer::start(|_| Reply::body(200, "text/plain", "there"));
    let location = target.url("/b");
    let origin = TestServer::start(move |_| Reply::redirect(302, &location));
    let mut headers = IndexMap::new();
    headers.insert("authorization".to_owned(), exec_ctx::text("Bearer token"));
    headers.insert("cookie".to_owned(), exec_ctx::text("sid=1"));
    headers.insert("x-keep".to_owned(), exec_ctx::text("1"));
    fetch(&[
        ("url", exec_ctx::text(&origin.url("/a"))),
        ("headers", Value::Map(headers)),
    ])
    .unwrap();
    let sent = origin.requests();
    assert_eq!(sent[0].header("authorization"), Some("Bearer token"));
    let arrived = target.requests();
    let received = &arrived[0];
    assert_eq!(received.header("authorization"), None);
    assert_eq!(received.header("cookie"), None);
    assert_eq!(received.header("x-keep"), Some("1"));
}

#[test]
fn compressed_body_is_cut_at_the_size_limit_after_decompression() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    let chunk = vec![b'a'; 1 << 20];
    for _ in 0..64 {
        encoder.write_all(&chunk).unwrap();
    }
    let bomb = encoder.finish().unwrap();
    assert!(bomb.len() < 1 << 20);
    let server = TestServer::start(move |_| {
        Reply::body(200, "text/plain", bomb.clone()).header("content-encoding", "gzip")
    });
    let records = fetch(&[
        ("url", exec_ctx::text(&server.url("/bomb"))),
        ("max_size", Value::Size(Size::from_bytes(1 << 20))),
    ])
    .unwrap();
    let record = response(&records[0]);
    assert_eq!(record.truncated, Some(true));
    assert_eq!(record.body.len(), 1 << 20);
}

#[test]
fn redirect_loop_ends_at_the_limit() {
    let server = TestServer::start(|_| Reply::redirect(302, "/loop"));
    let error = fetch(&[
        ("url", exec_ctx::text(&server.url("/loop"))),
        ("max_redirects", Value::Int(3)),
    ])
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Network);
    assert_eq!(error.data().problem.as_deref(), Some("redirect_limit"));
    assert_eq!(server.request_count(), 4);
}

#[test]
fn deeply_nested_page_is_converted_without_exhausting_the_stack() {
    let depth = 50_000;
    let page = format!("{}deep{}", "<div>".repeat(depth), "</div>".repeat(depth));
    let server = TestServer::start(move |_| Reply::body(200, "text/html", page.clone()));
    let records = fetch(&[
        ("url", exec_ctx::text(&server.url("/deep"))),
        ("as", exec_ctx::text("text")),
    ])
    .unwrap();
    assert!(response(&records[0]).body.contains("deep"));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
