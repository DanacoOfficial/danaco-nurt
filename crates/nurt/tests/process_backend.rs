// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt/tests/process_backend.rs
// Responsibility:   Client of `nurt serve` against an in-process server speaking `nurt-rpc/1` over pipes.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Client of `nurt serve` against an in-process server speaking
//! `nurt-rpc/1` over pipes.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{self, BufRead, BufReader, PipeWriter, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use nurt::{Describe, Error, ErrorKind, Language, Nurt, Request, Status, Transport, Variant};
use serde_json::{Value, json};

/// Writing side of the server; `None` after the server hung up.
#[derive(Clone)]
struct Peer {
    writer: Arc<Mutex<Option<PipeWriter>>>,
}

impl Peer {
    fn send(&self, message: &Value) {
        let mut writer = self.writer.lock().unwrap();
        if let Some(stream) = writer.as_mut() {
            let mut line = message.to_string();
            line.push('\n');
            let _closed = stream.write_all(line.as_bytes());
        }
    }

    fn send_raw(&self, line: &str) {
        let mut writer = self.writer.lock().unwrap();
        if let Some(stream) = writer.as_mut() {
            let _closed = stream.write_all(line.as_bytes());
        }
    }

    fn reply(&self, request: &Value, result: Value) {
        self.send(&json!({"jsonrpc": "2.0", "id": request["id"], "result": result}));
    }

    fn fail(&self, request: &Value, code: i64, message: &str) {
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "error": {"code": code, "message": message}
        }));
    }

    fn event(&self, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": "nurt/event", "params": params}));
    }

    fn hang_up(&self) {
        self.writer.lock().unwrap().take();
    }

    fn is_closed(&self) -> bool {
        self.writer.lock().unwrap().is_none()
    }
}

/// Interface version `(major << 16) | minor` the server reports.
const ABI: u64 = 1 << 16;

type Handler = Arc<dyn Fn(&Value, &Peer) + Send + Sync>;

/// Server answering `initialize` with `protocol` and every other request
/// with `handler`; it records every request it receives.
struct Server {
    handler: Handler,
    protocol: &'static str,
    abi: Option<u64>,
    received: Arc<Mutex<Vec<Value>>>,
    connections: Arc<AtomicUsize>,
}

impl Server {
    fn new(handler: impl Fn(&Value, &Peer) + Send + Sync + 'static) -> Self {
        Self {
            handler: Arc::new(handler),
            protocol: "nurt-rpc/1",
            abi: Some(ABI),
            received: Arc::default(),
            connections: Arc::default(),
        }
    }

    fn with_protocol(mut self, protocol: &'static str) -> Self {
        self.protocol = protocol;
        self
    }

    fn with_abi(mut self, abi: Option<u64>) -> Self {
        self.abi = abi;
        self
    }

    fn received(&self) -> Arc<Mutex<Vec<Value>>> {
        Arc::clone(&self.received)
    }

    fn connections(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.connections)
    }

    fn connect(self, builder: nurt::Builder) -> Result<Nurt, Error> {
        builder.connect_with(move || self.open())
    }

    fn open(&self) -> io::Result<Transport> {
        let (client_reader, server_writer) = io::pipe()?;
        let (server_reader, client_writer) = io::pipe()?;
        let peer = Peer {
            writer: Arc::new(Mutex::new(Some(server_writer))),
        };
        let handler = Arc::clone(&self.handler);
        let received = Arc::clone(&self.received);
        let protocol = self.protocol;
        let abi = self.abi;
        self.connections.fetch_add(1, Ordering::SeqCst);
        thread::spawn(move || {
            for line in BufReader::new(server_reader).lines() {
                let Ok(line) = line else { break };
                let request: Value = serde_json::from_str(&line).unwrap();
                received.lock().unwrap().push(request.clone());
                match request["method"].as_str() {
                    Some("initialize") => peer.reply(
                        &request,
                        json!({
                            "protocol": protocol,
                            "version": "0.1.0",
                            "abi": abi,
                            "features": []
                        }),
                    ),
                    Some("shutdown") => {
                        peer.reply(&request, json!({}));
                        peer.hang_up();
                    }
                    _ => handler(&request, &peer),
                }
                if peer.is_closed() {
                    break;
                }
            }
        });
        Ok(Transport::new(client_reader, client_writer))
    }
}

fn report(status: &str) -> Value {
    json!({"nurt": "1.0", "status": status, "kind": "find"})
}

fn envelope(report: Value) -> Value {
    json!({"nurt": "1.0", "report": report})
}

fn methods(received: &Mutex<Vec<Value>>) -> Vec<String> {
    received
        .lock()
        .unwrap()
        .iter()
        .filter_map(|request| request["method"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn run_sends_the_envelope_and_returns_the_response() {
    let server = Server::new(|request, peer| {
        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["method"], "run");
        let mut response = envelope(report("ok"));
        response["rendered"] = json!({"format": "text", "text": request["params"]["program"]});
        peer.reply(request, response);
    });
    let received = server.received();
    let nurt = server.connect(Nurt::builder()).unwrap();
    let request = Request::program("find \"/srv/app\"").limit(5);
    let response = nurt.run(&request).unwrap();
    assert!(response.is_ok());
    assert_eq!(response.text(), Some("find \"/srv/app\""));
    let sent = received.lock().unwrap()[1]["params"].clone();
    assert_eq!(sent, request.to_value());
}

#[test]
fn initialize_negotiates_protocol_and_settings() {
    let server = Server::new(|_, _| {});
    let received = server.received();
    let nurt = server
        .connect(
            Nurt::builder()
                .language(Language::En)
                .session("s-7")
                .config(json!({})),
        )
        .unwrap();
    let initialize = received.lock().unwrap()[0].clone();
    assert_eq!(initialize["method"], "initialize");
    assert_eq!(initialize["params"]["protocol"], "nurt-rpc/1");
    assert_eq!(initialize["params"]["abi"], ABI);
    assert_eq!(initialize["params"]["capabilities"]["events"], true);
    assert_eq!(initialize["params"]["language"], "en");
    assert_eq!(initialize["params"]["session"], "s-7");
    assert_eq!(initialize["params"]["config_overrides"], json!({}));
    drop(nurt);
}

#[test]
fn newer_minor_version_of_the_protocol_is_accepted() {
    let server = Server::new(|_, _| {}).with_protocol("nurt-rpc/1.4");
    assert!(server.connect(Nurt::builder()).is_ok());
}

#[test]
fn other_major_version_of_the_protocol_is_rejected() {
    let server = Server::new(|_, _| {}).with_protocol("nurt-rpc/2");
    let error = server.connect(Nurt::builder()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Incompatible);
    assert_eq!(error.status(), Status::Abi);
    assert!(error.message().contains("nurt-rpc/2"));
}

#[test]
fn other_major_version_of_the_interface_is_rejected() {
    let server = Server::new(|_, _| {}).with_abi(Some(2 << 16));
    let error = server.connect(Nurt::builder()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Incompatible);
    let server = Server::new(|_, _| {}).with_abi(None);
    assert!(server.connect(Nurt::builder()).is_ok());
}

#[test]
fn configuration_other_than_an_empty_object_is_rejected() {
    let server = Server::new(|_, _| {});
    let received = server.received();
    let error = server
        .connect(Nurt::builder().config(json!({"limits": {"limit": 10}})))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidArgument);
    assert!(received.lock().unwrap().is_empty());
}

#[test]
fn language_error_is_a_successful_call() {
    let server = Server::new(|request, peer| {
        let mut failed = report("failed");
        failed["error"] = json!({
            "code": "E_SYNTAX",
            "message": "unclosed list",
            "fix": ["find \"/srv/app\" glob: [\"*.go\"]"]
        });
        peer.reply(request, envelope(failed));
    });
    let nurt = server.connect(Nurt::builder()).unwrap();
    let response = nurt
        .run(&Request::program("find \"/srv/app\" glob: [\"*.go\""))
        .unwrap();
    assert!(!response.is_ok());
    let error = response.into_result().unwrap_err();
    assert_eq!(error.code(), "E_SYNTAX");
    assert_eq!(error.fix(), ["find \"/srv/app\" glob: [\"*.go\"]"]);
}

#[test]
fn application_errors_become_call_errors_with_status() {
    let server = Server::new(|request, peer| {
        let code = request["params"]["program"]
            .as_str()
            .and_then(|code| code.parse::<i64>().ok())
            .unwrap();
        peer.fail(request, code, "rejected");
    });
    let nurt = server.connect(Nurt::builder()).unwrap();
    for status in Status::ALL {
        let code = -32000 - i64::from(status.code());
        let error = nurt.run(&Request::program(code.to_string())).unwrap_err();
        assert_eq!(error.status(), *status);
    }
    let error = nurt.run(&Request::program("-32601")).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Protocol);
}

#[test]
fn result_without_report_is_a_protocol_error() {
    let server = Server::new(|request, peer| peer.reply(request, json!({"nurt": "1.0"})));
    let nurt = server.connect(Nurt::builder()).unwrap();
    let error = nurt.run(&Request::program("x")).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Protocol);
}

#[test]
fn concurrent_requests_are_matched_by_id() {
    const CALLERS: usize = 8;
    let waiting: Arc<Mutex<Vec<Value>>> = Arc::default();
    let server = Server::new(move |request, peer| {
        let mut waiting = waiting.lock().unwrap();
        waiting.push(request.clone());
        if waiting.len() == CALLERS {
            for request in waiting.drain(..).rev() {
                let mut answer = report("ok");
                answer["kind"] = request["params"]["program"].clone();
                peer.reply(&request, envelope(answer));
            }
        }
    });
    let nurt = Arc::new(server.connect(Nurt::builder()).unwrap());
    let callers: Vec<_> = (0..CALLERS)
        .map(|caller| {
            let nurt = Arc::clone(&nurt);
            thread::spawn(move || {
                let program = format!("program-{caller}");
                let response = nurt.run(&Request::program(&program)).unwrap();
                assert_eq!(response.report()["kind"], program.as_str());
            })
        })
        .collect();
    for caller in callers {
        caller.join().unwrap();
    }
}

#[test]
fn events_reach_their_execution_in_order_and_others_reach_the_listener() {
    let server = Server::new(|request, peer| {
        let id = request["id"].clone();
        peer.event(json!({"t": "job.output", "seq": 1, "request": null, "id": "j-1"}));
        for seq in 1..=3 {
            peer.event(json!({"t": "progress", "seq": seq, "request": id.to_string()}));
        }
        peer.event(json!({"t": "exec.finished", "seq": 4, "request": id.to_string()}));
        peer.reply(request, envelope(report("ok")));
    });
    let (sender, outside) = mpsc::channel();
    let sender = Mutex::new(sender);
    let nurt = server
        .connect(Nurt::builder().on_event(move |event| {
            sender.lock().unwrap().send(event).unwrap();
        }))
        .unwrap();
    let mut execution = nurt.start(&Request::program("test \"/srv/app\"")).unwrap();
    let seen: Vec<(String, Option<u64>)> = execution
        .events()
        .map(|event| (event.kind().to_owned(), event.seq()))
        .collect();
    assert_eq!(
        seen,
        [
            ("progress".to_owned(), Some(1)),
            ("progress".to_owned(), Some(2)),
            ("progress".to_owned(), Some(3)),
            ("exec.finished".to_owned(), Some(4)),
        ]
    );
    assert!(execution.wait().unwrap().is_ok());
    assert!(execution.try_event().is_none());
    let job = outside.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(job.job_id(), Some("j-1"));
}

#[test]
fn cancel_names_the_request_and_wait_returns_the_cancelled_report() {
    let running: Arc<Mutex<Option<Value>>> = Arc::default();
    let server = Server::new(move |request, peer| match request["method"].as_str() {
        Some("run") => *running.lock().unwrap() = Some(request.clone()),
        Some("cancel") => {
            let run = running.lock().unwrap().take().unwrap();
            assert_eq!(request["params"]["request_id"], run["id"]);
            peer.reply(request, json!({}));
            peer.reply(&run, envelope(report("rolled_back")));
        }
        _ => {}
    });
    let nurt = server.connect(Nurt::builder()).unwrap();
    let mut execution = nurt.start(&Request::program("sleep 1h")).unwrap();
    assert!(
        execution
            .wait_timeout(Duration::from_millis(20))
            .unwrap()
            .is_none()
    );
    execution.cancel().unwrap();
    let response = execution.wait().unwrap();
    assert_eq!(response.status(), Some("rolled_back"));
    let again = execution.wait_timeout(Duration::ZERO).unwrap().unwrap();
    assert_eq!(again, response);
}

#[test]
fn dropping_an_unfinished_execution_cancels_it() {
    let (sender, cancelled) = mpsc::channel();
    let sender = Mutex::new(sender);
    let server = Server::new(move |request, _| {
        if request["method"] == "cancel" {
            sender
                .lock()
                .unwrap()
                .send(request["params"].clone())
                .unwrap();
        }
    });
    let nurt = server.connect(Nurt::builder()).unwrap();
    let execution = nurt.start(&Request::program("sleep 1h")).unwrap();
    drop(execution);
    let params = cancelled.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(params["request_id"].is_u64());
}

#[test]
fn request_timeout_fails_the_call_and_cancels_it() {
    let (sender, cancelled) = mpsc::channel();
    let sender = Mutex::new(sender);
    let server = Server::new(move |request, _| {
        if request["method"] == "cancel" {
            sender
                .lock()
                .unwrap()
                .send(request["params"].clone())
                .unwrap();
        }
    });
    let nurt = server
        .connect(Nurt::builder().request_timeout(Duration::from_millis(200)))
        .unwrap();
    let error = nurt.run(&Request::program("sleep 1h")).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Timeout);
    assert_eq!(error.status(), Status::Timeout);
    let params = cancelled.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(params["request_id"].is_u64());
}

#[test]
fn ended_server_fails_requests_in_flight_and_is_replaced() {
    let server = Server::new(|request, peer| {
        if request["params"]["program"] == "crash" {
            peer.hang_up();
        } else {
            peer.reply(request, envelope(report("ok")));
        }
    });
    let connections = server.connections();
    let received = server.received();
    let nurt = server.connect(Nurt::builder()).unwrap();
    let error = nurt.run(&Request::program("crash")).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Disconnected);
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    assert!(
        nurt.run(&Request::program("find \"/srv\""))
            .unwrap()
            .is_ok()
    );
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    let initialized = methods(&received)
        .iter()
        .filter(|method| *method == "initialize")
        .count();
    assert_eq!(initialized, 2);
}

#[test]
fn invalid_json_from_the_server_closes_the_connection() {
    let server = Server::new(|request, peer| {
        if request["params"]["program"] == "garble" {
            peer.send_raw("{not json\n");
        } else {
            peer.reply(request, envelope(report("ok")));
        }
    });
    let nurt = server.connect(Nurt::builder()).unwrap();
    let error = nurt.run(&Request::program("garble")).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Protocol);
    assert!(
        nurt.run(&Request::program("find \"/srv\""))
            .unwrap()
            .is_ok()
    );
}

#[test]
fn blank_lines_and_unknown_notifications_are_skipped() {
    let server = Server::new(|request, peer| {
        peer.send_raw("\n  \n");
        peer.send(&json!({"jsonrpc": "2.0", "method": "nurt/unknown", "params": {}}));
        peer.send(&json!({"jsonrpc": "2.0", "id": 999_999, "result": {}}));
        peer.reply(request, envelope(report("ok")));
    });
    let nurt = server.connect(Nurt::builder()).unwrap();
    assert!(nurt.run(&Request::program("x")).unwrap().is_ok());
}

#[test]
fn close_shuts_the_server_down() {
    let server = Server::new(|_, _| {});
    let received = server.received();
    let nurt = server.connect(Nurt::builder()).unwrap();
    nurt.close().unwrap();
    assert_eq!(methods(&received), ["initialize", "shutdown"]);
}

/// Client of a server that answers `print` and `help` with `{text}`,
/// `describe` with `{description}` (`method:params`), `page` with an empty
/// report and other methods with their name and parameters.
fn echo_server() -> (Nurt, Arc<Mutex<Vec<Value>>>) {
    let server = Server::new(|request, peer| {
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "print" | "help" => json!({"text": format!("{method}:{}", request["params"])}),
            "describe" => json!({"description": format!("{method}:{}", request["params"])}),
            "page" => envelope(report("ok")),
            _ => json!({"method": method, "params": request["params"]}),
        };
        peer.reply(request, result);
    });
    let received = server.received();
    (server.connect(Nurt::builder()).unwrap(), received)
}

fn sent_params(received: &Mutex<Vec<Value>>, method: &str) -> Vec<Value> {
    received
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request["method"] == method)
        .map(|request| request["params"].clone())
        .collect()
}

#[test]
fn language_methods_send_program_tree_and_variant() -> Result<(), Error> {
    let (nurt, _) = echo_server();
    let tree = json!({"nurt": "1.0"});
    assert_eq!(
        nurt.check(&Request::program("x"))?["params"]["program"],
        "x"
    );
    assert_eq!(
        nurt.parse("find(\"/srv\")", Variant::B)?["params"],
        json!({"program": "find(\"/srv\")", "variant": "b"})
    );
    assert_eq!(
        nurt.print(&tree, Variant::C)?,
        format!("print:{}", json!({"ast": tree, "variant": "c"}))
    );
    assert_eq!(
        nurt.correct("find /srv", Variant::A)?["params"],
        json!({"program": "find /srv", "variant": "a"})
    );
    Ok(())
}

#[test]
fn text_methods_send_subject_topic_and_language() -> Result<(), Error> {
    let (nurt, _) = echo_server();
    assert_eq!(
        nurt.describe(Describe::Core, None)?,
        r#"describe:{"subject":"core"}"#
    );
    assert_eq!(
        nurt.describe(Describe::Group("git"), Some(Language::Pl))?,
        r#"describe:{"subject":"git","language":"pl"}"#
    );
    assert_eq!(nurt.help(None)?, "help:{}");
    assert_eq!(nurt.help(Some("lists"))?, r#"help:{"topic":"lists"}"#);
    Ok(())
}

#[test]
fn data_methods_send_their_selection() -> Result<(), Error> {
    let (nurt, _) = echo_server();
    assert_eq!(nurt.catalog(None)?["params"], json!({}));
    assert_eq!(
        nurt.catalog(Some(&json!({"group": "git"})))?["params"],
        json!({"group": "git"})
    );
    assert_eq!(nurt.schema("report")?["params"], json!({"name": "report"}));
    assert_eq!(nurt.history(None)?["params"], json!({}));
    assert_eq!(
        nurt.history(Some(&json!({"limit": 3})))?["params"],
        json!({"limit": 3})
    );
    assert_eq!(nurt.space(None)?["params"], json!({}));
    assert_eq!(
        nurt.space(Some("/srv"))?["params"],
        json!({"volume": "/srv"})
    );
    assert_eq!(nurt.info()?["method"], "info");
    Ok(())
}

#[test]
fn page_and_job_cancellation_send_cursor_limit_and_job() -> Result<(), Error> {
    let (nurt, received) = echo_server();
    assert!(nurt.page("c_1", Some(10))?.is_ok());
    assert!(nurt.page("c_2", None)?.is_ok());
    nurt.cancel_job("j-4")?;
    assert_eq!(
        sent_params(&received, "page"),
        [
            json!({"cursor": "c_1", "limit": 10}),
            json!({"cursor": "c_2"})
        ]
    );
    assert_eq!(sent_params(&received, "cancel"), [json!({"job_id": "j-4"})]);
    Ok(())
}

#[test]
fn describe_returns_structured_descriptions_as_json_text() {
    let server = Server::new(|request, peer| {
        peer.reply(request, json!({"description": {"rules": ["r1"]}}));
    });
    let nurt = server.connect(Nurt::builder()).unwrap();
    assert_eq!(
        nurt.describe(Describe::Core, None).unwrap(),
        r#"{"rules":["r1"]}"#
    );
}

#[test]
fn text_method_with_non_text_result_is_a_protocol_error() {
    let server = Server::new(|request, peer| peer.reply(request, json!("help")));
    let nurt = server.connect(Nurt::builder()).unwrap();
    assert_eq!(nurt.help(None).unwrap_err().kind(), ErrorKind::Protocol);
    assert_eq!(
        nurt.describe(Describe::Core, None).unwrap_err().kind(),
        ErrorKind::Protocol
    );
}

#[test]
fn failing_connect_is_a_call_error() {
    let error = Nurt::builder()
        .connect_with(|| Err(io::Error::new(io::ErrorKind::NotFound, "no socket")))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
    let error = Nurt::builder()
        .connect_with(|| Err(io::Error::other("refused")))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
}

#[test]
fn missing_executable_is_reported_before_starting() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("nurt");
    let error = Nurt::builder().executable(&missing).spawn().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message().contains(&missing.display().to_string()));
}

#[test]
fn executable_that_is_not_a_server_fails_initialization() {
    let not_a_server = std::env::current_exe().unwrap();
    let error = Nurt::builder()
        .executable(not_a_server)
        .request_timeout(Duration::from_secs(30))
        .spawn()
        .unwrap_err();
    assert!(
        matches!(error.kind(), ErrorKind::Disconnected | ErrorKind::Protocol),
        "{error}"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
