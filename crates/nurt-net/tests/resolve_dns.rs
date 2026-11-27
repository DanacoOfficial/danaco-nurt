// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-net/tests/resolve_dns.rs
// Responsibility:   Tests of `network.resolve` against a DNS server on the loopback.
// Dependencies:     exec_ctx, hickory_resolver, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of `network.resolve` against a DNS server on the loopback
//! interface.
//!
//! The server answers for the domain `example.test`: two A records, one
//! AAAA, MX, TXT and NS record; `missing.test` does not exist, `fail.test`
//! fails with SERVFAIL and `big.test` is truncated over UDP and answered in
//! full over TCP.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers fail the test on an unexpected result"
)]

#[path = "support/exec_ctx.rs"]
mod exec_ctx;

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use exec_ctx::{Events, args, ctx, ctx_with, run, text};
use hickory_resolver::proto::op::{Message, MessageType, OpCode, ResponseCode};
use hickory_resolver::proto::rr::rdata::{A, AAAA, MX, NS, TXT};
use hickory_resolver::proto::rr::{Name, RData, Record as Answer, RecordType};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::DnsRecord;
use nurt_contracts::stage::{CancelReason, CancelToken};
use nurt_contracts::value::{Duration as ValueDuration, Value};
use nurt_net::dns::{QueryType, Resolve, ResolverConfig, lookup};
#[cfg(unix)]
use nurt_net::interfaces::SysRoot;

const TTL: u32 = 60;
const POLL: Duration = Duration::from_millis(50);
/// Ports tried for a UDP socket and a TCP listener on the same port.
const BIND_ATTEMPTS: usize = 20;

struct FakeDns {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl FakeDns {
    /// Binds a UDP socket and a TCP listener to the same free port; a port
    /// free for UDP can be taken for TCP by another test, so the pair is
    /// tried again on another port.
    fn bind_pair() -> (UdpSocket, TcpListener) {
        for _ in 0..BIND_ATTEMPTS {
            let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
            if let Ok(tcp) = TcpListener::bind(udp.local_addr().unwrap()) {
                return (udp, tcp);
            }
        }
        panic!("no loopback port is free for both UDP and TCP");
    }

    fn start() -> Self {
        let (udp, tcp) = Self::bind_pair();
        let address = udp.local_addr().unwrap();
        udp.set_read_timeout(Some(POLL)).unwrap();
        tcp.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let udp_stop = Arc::clone(&stop);
        let tcp_stop = Arc::clone(&stop);
        let threads = vec![
            std::thread::spawn(move || serve_udp(&udp, &udp_stop)),
            std::thread::spawn(move || serve_tcp(&tcp, &tcp_stop)),
        ];
        Self {
            address,
            stop,
            threads,
        }
    }

    fn server(&self) -> Value {
        text(&self.address.to_string())
    }
}

impl Drop for FakeDns {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn serve_udp(socket: &UdpSocket, stop: &AtomicBool) {
    let mut buffer = [0_u8; 512];
    while !stop.load(Ordering::SeqCst) {
        let Ok((length, peer)) = socket.recv_from(&mut buffer) else {
            continue;
        };
        if let Some(answer) = answer(&buffer[..length], false) {
            let _ = socket.send_to(&answer, peer);
        }
    }
}

fn serve_tcp(listener: &TcpListener, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        let Ok((mut stream, _)) = listener.accept() else {
            std::thread::sleep(POLL);
            continue;
        };
        stream.set_nonblocking(false).unwrap();
        let mut prefix = [0_u8; 2];
        if stream.read_exact(&mut prefix).is_err() {
            continue;
        }
        let mut query = vec![0_u8; usize::from(u16::from_be_bytes(prefix))];
        if stream.read_exact(&mut query).is_err() {
            continue;
        }
        if let Some(answer) = answer(&query, true) {
            let length = u16::try_from(answer.len()).unwrap().to_be_bytes();
            let _ = stream.write_all(&[length.as_slice(), &answer].concat());
        }
    }
}

fn record(name: &Name, data: RData) -> Answer {
    Answer::from_rdata(name.clone(), TTL, data)
}

fn name(text: &str) -> Name {
    Name::from_ascii(text).unwrap()
}

fn answer(query: &[u8], over_tcp: bool) -> Option<Vec<u8>> {
    let request = Message::from_vec(query).ok()?;
    let question = request.queries.first()?.clone();
    let mut response = Message::new(request.metadata.id, MessageType::Response, OpCode::Query);
    response.add_query(question.clone());
    let asked = question.name().to_ascii().to_ascii_lowercase();
    let domain = name("example.test.");
    match (asked.as_str(), question.query_type()) {
        ("example.test.", RecordType::A) => {
            response.add_answer(record(&domain, RData::A(A(Ipv4Addr::new(192, 0, 2, 10)))));
            response.add_answer(record(&domain, RData::A(A(Ipv4Addr::new(192, 0, 2, 11)))));
        }
        ("example.test.", RecordType::AAAA) => {
            let ip = "2001:db8::10".parse::<Ipv6Addr>().unwrap();
            response.add_answer(record(&domain, RData::AAAA(AAAA(ip))));
        }
        ("example.test.", RecordType::MX) => {
            let mx = MX::new(10, name("mail.example.test."));
            response.add_answer(record(&domain, RData::MX(mx)));
        }
        ("example.test.", RecordType::TXT) => {
            let txt = TXT::new(vec!["v=spf1 -all".to_owned()]);
            response.add_answer(record(&domain, RData::TXT(txt)));
        }
        ("example.test.", RecordType::NS) => {
            response.add_answer(record(&domain, RData::NS(NS(name("ns1.example.test.")))));
        }
        ("missing.test.", _) => response.metadata.response_code = ResponseCode::NXDomain,
        ("fail.test.", _) => response.metadata.response_code = ResponseCode::ServFail,
        ("big.test.", RecordType::A) if !over_tcp => response.metadata.truncation = true,
        ("big.test.", RecordType::A) => {
            let big = name("big.test.");
            response.add_answer(record(&big, RData::A(A(Ipv4Addr::new(192, 0, 2, 20)))));
        }
        _ => {}
    }
    response.to_vec().ok()
}

fn resolve(
    server: &FakeDns,
    name: &str,
    kind: Option<&str>,
) -> nurt_contracts::error::Result<Vec<DnsRecord>> {
    let mut values = vec![("name", text(name)), ("server", server.server())];
    if let Some(kind) = kind {
        values.push(("type", Value::Enum(kind.to_owned())));
    }
    let records = run(&Resolve::new(), &args("network.resolve", &values), &ctx())?;
    Ok(records
        .into_iter()
        .map(|record| DnsRecord::try_from(record).unwrap())
        .collect())
}

#[test]
fn resolve_returns_a_records_of_the_server() {
    let server = FakeDns::start();
    let records = resolve(&server, "example.test", None).unwrap();
    let values: Vec<&str> = records.iter().map(|record| record.value.as_str()).collect();
    assert_eq!(values, ["192.0.2.10", "192.0.2.11"]);
    let first = &records[0];
    assert_eq!(first.name, "example.test");
    assert_eq!(first.kind, "A");
    assert_eq!(first.ttl, ValueDuration::from_millis(60_000));
    assert_eq!(
        first.server.as_deref(),
        Some(server.address.to_string().as_str())
    );
}

#[test]
fn resolve_any_merges_types() {
    let server = FakeDns::start();
    let records = resolve(&server, "example.test", Some("any")).unwrap();
    let kinds: Vec<&str> = records.iter().map(|record| record.kind.as_str()).collect();
    assert_eq!(kinds, ["A", "A", "AAAA", "MX", "TXT", "NS"]);
    let mx = records.iter().find(|record| record.kind == "MX").unwrap();
    assert_eq!(mx.value, "10 mail.example.test");
    let txt = records.iter().find(|record| record.kind == "TXT").unwrap();
    assert_eq!(txt.value, "v=spf1 -all");
}

#[test]
fn resolve_type_without_records_is_empty() {
    let server = FakeDns::start();
    let records = resolve(&server, "example.test", Some("cname")).unwrap();
    assert!(records.is_empty());
}

#[test]
fn resolve_unknown_type_invalid_value() {
    let server = FakeDns::start();
    let error = resolve(&server, "example.test", Some("srv")).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert_eq!(error.found.as_deref(), Some("srv"));
}

#[test]
fn resolve_missing_name_not_found() {
    let server = FakeDns::start();
    let error = resolve(&server, "missing.test", None).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
}

#[test]
fn resolve_server_failure_network_error() {
    let server = FakeDns::start();
    let error = resolve(&server, "fail.test", None).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Network);
    assert_eq!(error.problem.as_deref(), Some("dns"));
}

#[test]
fn resolve_truncated_answer_repeated_over_tcp() {
    let server = FakeDns::start();
    let records = resolve(&server, "big.test", Some("a")).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].value, "192.0.2.20");
}

#[test]
fn resolve_silent_server_network_error() {
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
    let config = ResolverConfig {
        servers: vec![silent.local_addr().unwrap()],
        timeout: Duration::from_millis(200),
        attempts: 1,
    };
    let error = lookup(&config, "example.test", QueryType::A, &ctx()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Network);
    assert_eq!(error.problem.as_deref(), Some("dns"));
}

#[test]
fn resolve_cancelled_execution_stops() {
    let server = FakeDns::start();
    let cancel = CancelToken::new();
    cancel.cancel(CancelReason::Requested);
    let ctx = ctx_with(cancel, Arc::new(Events::default()));
    let config = ResolverConfig::single(server.address);
    let error = lookup(&config, "example.test", QueryType::A, &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Cancelled);
}

#[cfg(unix)]
#[test]
fn resolve_reads_resolv_conf_below_root() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("etc")).unwrap();
    std::fs::write(
        dir.path().join("etc/resolv.conf"),
        "nameserver 192.0.2.53\noptions timeout:1 attempts:1\n",
    )
    .unwrap();
    let config = ResolverConfig::system(&SysRoot::new(dir.path())).unwrap();
    assert_eq!(
        config.servers,
        vec!["192.0.2.53:53".parse::<SocketAddr>().unwrap()]
    );
    assert_eq!(config.timeout, Duration::from_secs(1));
    assert_eq!(config.attempts, 1);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
