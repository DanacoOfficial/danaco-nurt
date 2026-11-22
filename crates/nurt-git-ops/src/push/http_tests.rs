// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Pushes over smart HTTP to the in-process `receive-pack` service behind a
//! loopback server, and the stage interface of the command.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

use nurt_contracts::stage::{ArgValue, StageData};
use nurt_git_transport::local::receive::TxSink;
use nurt_git_transport::send_pack::testing::{Fixture, RecordingTx};

use super::*;

/// First packet lines of the smart HTTP advertisement of `receive-pack`.
const SERVICE_HEADER: &[u8] = b"001f# service=git-receive-pack\n0000";

/// Path of the repository on the server.
const REPOSITORY: &str = "/remote.git";

/// Loopback HTTP server exposing one repository as [`REPOSITORY`].
struct Server {
    port: u16,
}

impl Server {
    fn start(git_dir: &Path, require_credentials: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let git_dir = git_dir.to_path_buf();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let git_dir = git_dir.clone();
                std::thread::spawn(move || serve(stream, &git_dir, require_credentials));
            }
        });
        Self { port }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}{REPOSITORY}", self.port)
    }
}

struct Request {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut length = 0;
    let mut chunked = false;
    let mut authorization = None;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':')?;
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().ok()?,
            "transfer-encoding" => chunked = value.eq_ignore_ascii_case("chunked"),
            "authorization" => authorization = Some(value.to_owned()),
            _ => {}
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size).ok()?;
            let size = usize::from_str_radix(size.trim().split(';').next()?, 16).ok()?;
            let mut chunk = vec![0; size + 2];
            reader.read_exact(&mut chunk).ok()?;
            if size == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..size]);
        }
    } else {
        body.resize(length, 0);
        reader.read_exact(&mut body).ok()?;
    }
    Some(Request {
        method,
        path,
        authorization,
        body,
    })
}

fn server_options() -> ReceiveOptions {
    ReceiveOptions {
        agent: "test/1".to_owned(),
        report_status_v2: true,
        run_hooks: false,
        identity: identity(),
    }
}

fn respond(
    request: &Request,
    git_dir: &Path,
    require_credentials: bool,
) -> (u16, &'static str, Vec<u8>) {
    let authorized = request
        .authorization
        .as_deref()
        .is_some_and(|value| value.starts_with("Basic "));
    if require_credentials && !authorized {
        return (401, "text/plain", Vec::new());
    }
    let advertisement = format!("{REPOSITORY}/info/refs?service=git-receive-pack");
    let receive = format!("{REPOSITORY}/git-receive-pack");
    match request.method.as_str() {
        "GET" if request.path == advertisement => {
            let service = ReceivePack::open(git_dir, server_options()).unwrap();
            let mut body = SERVICE_HEADER.to_vec();
            service.advertise(&mut body).unwrap();
            (200, "application/x-git-receive-pack-advertisement", body)
        }
        "POST" if request.path == receive => {
            let mut service = ReceivePack::open(git_dir, server_options()).unwrap();
            let mut tx = RecordingTx::new();
            let mut sink = TxSink::new(&mut tx);
            let mut body = Vec::new();
            service
                .serve(&mut request.body.as_slice(), &mut body, None, &mut sink)
                .unwrap();
            (200, "application/x-git-receive-pack-result", body)
        }
        _ => (404, "text/plain", Vec::new()),
    }
}

fn serve(stream: TcpStream, git_dir: &Path, require_credentials: bool) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let Some(request) = read_request(&mut reader) else {
        return;
    };
    let (status, content_type, body) = respond(&request, git_dir, require_credentials);
    let mut head = format!(
        "HTTP/1.1 {status} Status\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if status == HTTP_UNAUTHORIZED {
        head.push_str("WWW-Authenticate: Basic realm=\"push\"\r\n");
    }
    head.push_str("\r\n");
    let mut stream = stream;
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(&body).unwrap();
}

fn identity() -> Identity {
    Identity {
        name: "Pusher".into(),
        email: "pusher@example.com".into(),
        seconds: 1_700_000_500,
        offset: 0,
    }
}

fn linked(url: &str) -> Fixture {
    let local = Fixture::new();
    local.config(&format!(
        "[remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"
    ));
    local
}

fn push(
    local: &Fixture,
    request: &PushRequest,
    network: &NetworkOptions,
) -> (Result<PushOutcome>, RecordingTx) {
    let cancel = CancelToken::new();
    let context = PushContext {
        identity: identity(),
        host: None,
        cancel: &cancel,
    };
    let mut tx = RecordingTx::new();
    let outcome = plan_push(local.root(), request, network)
        .and_then(|planned| planned.apply(&mut tx, &context));
    (outcome, tx)
}

#[test]
fn pushes_over_http_and_marks_the_remote_update_irreversible() {
    let remote = Fixture::bare();
    let server = Server::start(remote.git_dir(), false);
    let local = linked(&server.url());
    let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
    let network = NetworkOptions::default();
    let planned = plan_push(local.root(), &PushRequest::new("main"), &network).unwrap();
    assert_eq!(
        planned.irreversible().map(|entry| entry.reason),
        Some(Irreversible::RemoteRefUpdated)
    );
    let (outcome, tx) = push(&local, &PushRequest::new("main"), &network);
    let outcome = outcome.unwrap();
    assert_eq!(outcome.updates[0].status, GitRefStatus::New);
    assert_eq!(outcome.objects, 3);
    assert!(!outcome.status_unknown);
    assert!(outcome.notable().is_empty());
    assert_eq!(remote.ref_value("refs/heads/main"), Some(tip));
    assert_eq!(local.ref_value("refs/remotes/origin/main"), Some(tip));
    assert_eq!(tx.irreversible.len(), 1);
    assert_eq!(tx.irreversible[0].reason, Irreversible::RemoteRefUpdated);
    assert_eq!(tx.irreversible[0].count, Some(1));
}

#[test]
fn undo_of_an_http_push_restores_only_the_tracking_ref() {
    let remote = Fixture::bare();
    let server = Server::start(remote.git_dir(), false);
    let local = linked(&server.url());
    let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
    let network = NetworkOptions::default();
    let (outcome, mut tx) = push(&local, &PushRequest::new("main"), &network);
    assert!(outcome.is_ok());
    tx.undo();
    assert_eq!(local.ref_value("refs/remotes/origin/main"), None);
    assert_eq!(remote.ref_value("refs/heads/main"), Some(tip));
    let (outcome, tx) = push(&local, &PushRequest::new("main"), &network);
    assert_eq!(outcome.unwrap().updates[0].status, GitRefStatus::UpToDate);
    assert!(tx.irreversible.is_empty());
    assert_eq!(local.ref_value("refs/remotes/origin/main"), Some(tip));
}

#[test]
fn reports_rejections_of_the_remote_over_http() {
    let remote = Fixture::bare();
    remote.config("[receive]\n\tdenyDeletes = true\n");
    let server = Server::start(remote.git_dir(), false);
    let local = linked(&server.url());
    let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
    let network = NetworkOptions::default();
    push(&local, &PushRequest::new("main"), &network).0.unwrap();
    let delete = PushRequest {
        delete: true,
        ..PushRequest::new("main")
    };
    let (outcome, tx) = push(&local, &delete, &network);
    let outcome = outcome.unwrap();
    assert_eq!(outcome.updates[0].status, GitRefStatus::Rejected);
    assert_eq!(
        outcome.updates[0].reason.as_deref(),
        Some("deletion_prohibited")
    );
    assert_eq!(
        outcome.check().unwrap_err().code(),
        ErrorCode::GitPushRejected
    );
    assert!(tx.irreversible.is_empty());
    assert_eq!(remote.ref_value("refs/heads/main"), Some(tip));
    assert_eq!(local.ref_value("refs/remotes/origin/main"), Some(tip));
}

#[test]
fn asks_for_credentials_and_never_sends_them_over_plain_http() {
    let remote = Fixture::bare();
    let server = Server::start(remote.git_dir(), true);
    let local = linked(&server.url());
    local.commit("main", &[("a.txt", "one\n")], &[]);
    let (outcome, _) = push(
        &local,
        &PushRequest::new("main"),
        &NetworkOptions::default(),
    );
    let error = outcome.unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitAuth);
    assert_eq!(error.problem.as_deref(), Some("missing_credentials"));
    let network = NetworkOptions {
        env: BTreeMap::from([("NURT_GIT_TOKEN".to_owned(), "secret-token".to_owned())]),
        ssh: None,
    };
    let (outcome, _) = push(&local, &PushRequest::new("main"), &network);
    let error = outcome.unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitAuth);
    assert!(error.message.contains("clear text"), "{}", error.message);
    assert!(!error.message.contains("secret-token"));
    assert_eq!(remote.ref_value("refs/heads/main"), None);
    assert!(!format!("{network:?}").contains("secret-token"));
}

#[test]
fn reports_unreachable_and_invalid_addresses() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let local = linked(&format!("http://127.0.0.1:{port}{REPOSITORY}"));
    local.commit("main", &[("a.txt", "one\n")], &[]);
    let network = NetworkOptions::default();
    let error = plan_push(local.root(), &PushRequest::new("main"), &network).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Network);
    let request = PushRequest {
        remote: "http://[invalid".to_owned(),
        ..PushRequest::new("main")
    };
    let error = plan_push(local.root(), &request, &network).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn reports_the_pack_and_unknown_status_as_notable() {
    let outcome = PushOutcome {
        pack: Some(PackSummary {
            objects: 10,
            deltas: 0,
            bytes: send_pack::DELTA_NOTICE_BYTES,
        }),
        status_unknown: true,
        transport_notable: vec![http_notable(&HttpNote::TlsVerificationDisabled)],
        ..PushOutcome::default()
    };
    let codes: Vec<Notable> = outcome.notable().iter().map(|entry| entry.code).collect();
    assert_eq!(
        codes,
        [
            Notable::PackWithoutDeltas,
            Notable::PushStatusUnknown,
            Notable::TlsVerificationDisabled
        ]
    );
    let ignored = http_notable(&HttpNote::OptionIgnored {
        key: "http.sslCert".to_owned(),
    });
    assert_eq!(ignored.code, Notable::HttpOptionIgnored);
    assert_eq!(ignored.detail.as_deref(), Some("http.sslCert"));
}

#[test]
fn maps_refused_requests_to_http_statuses() {
    let required = Error::new(ErrorCode::GitAuth, "required").with_problem(AUTHENTICATION_REQUIRED);
    assert_eq!(refused_status(&required), Some(HTTP_UNAUTHORIZED));
    let forbidden = Error::new(ErrorCode::GitAuth, "forbidden").with_problem(FORBIDDEN);
    assert_eq!(refused_status(&forbidden), Some(HTTP_FORBIDDEN));
    let network = Error::new(ErrorCode::Network, "down").with_problem(FORBIDDEN);
    assert_eq!(refused_status(&network), None);
    assert!(is_http("https://example.com/r.git"));
    assert!(!is_http("ssh://example.com/r.git"));
}

#[test]
fn writes_changed_credential_stores_only() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("existing");
    std::fs::write(&existing, b"https://a:b@example.com\n").unwrap();
    let unchanged = dir.path().join("unchanged");
    std::fs::write(&unchanged, b"same\n").unwrap();
    let missing = dir.path().join("missing");
    let updates = [
        StoreUpdate {
            path: existing.clone(),
            existed: true,
            contents: b"https://a:c@example.com\n".to_vec(),
        },
        StoreUpdate {
            path: unchanged,
            existed: true,
            contents: b"same\n".to_vec(),
        },
        StoreUpdate {
            path: missing.clone(),
            existed: false,
            contents: b"new\n".to_vec(),
        },
    ];
    let mut tx = RecordingTx::new();
    write_store_updates(&mut tx, &updates).unwrap();
    assert_eq!(tx.targets().len(), 1);
    assert_eq!(
        std::fs::read(&existing).unwrap(),
        b"https://a:c@example.com\n"
    );
    assert!(!missing.exists());
    tx.undo();
    assert_eq!(
        std::fs::read(&existing).unwrap(),
        b"https://a:b@example.com\n"
    );
}

fn stage_args(local: &Fixture, extra: &[(&str, Value)]) -> BoundArgs {
    let repo = local.root().to_string_lossy().into_owned();
    let mut args =
        BoundArgs::new("git.push").with("repo", ArgValue::Value(Value::Path(repo)), false);
    for (name, value) in extra {
        args = args.with(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

fn run_stage(args: &BoundArgs, tx: &mut RecordingTx) -> Result<Vec<Record>> {
    let ctx = nurt_git::testutil::test_ctx();
    let effect = PushCommand.as_effect().unwrap();
    let plan = effect.plan(args, StageInput::none(), &ctx)?;
    let outcome = effect.apply(args, plan, tx, &ctx)?;
    match outcome.output.data {
        StageData::Records(stream) => stream.collect(),
        _ => panic!("the stage produced no records"),
    }
}

#[test]
fn stage_pushes_to_a_local_remote_and_reports_records() {
    let remote = Fixture::bare();
    let local = linked(&remote.root().display().to_string());
    let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
    let args = stage_args(
        &local,
        &[
            ("branch", Value::Text("main".to_owned())),
            ("remote", Value::Text("origin".to_owned())),
            ("force", Value::Enum("no".to_owned())),
            ("set_upstream", Value::Bool(true)),
            ("tags", Value::Bool(false)),
            ("hooks", Value::Bool(false)),
        ],
    );
    let mut tx = RecordingTx::new();
    let records = run_stage(&args, &mut tx).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(remote.ref_value("refs/heads/main"), Some(tip));
    assert!(tx.irreversible.is_empty());
    let config = std::fs::read_to_string(local.git_dir().join("config")).unwrap();
    assert!(config.contains("[branch \"main\"]"));
    tx.undo();
    assert_eq!(remote.ref_value("refs/heads/main"), None);
}

#[test]
fn stage_fails_when_every_update_is_rejected() {
    let remote = Fixture::bare();
    let local = linked(&remote.root().display().to_string());
    remote.commit("main", &[("r.txt", "remote\n")], &[]);
    local.commit("main", &[("a.txt", "one\n")], &[]);
    let args = stage_args(&local, &[("branch", Value::Text("main".to_owned()))]);
    let mut tx = RecordingTx::new();
    let error = run_stage(&args, &mut tx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitPushRejected);
    assert!(!error.effect_applied);
    let missing = stage_args(&local, &[]);
    assert!(run_stage(&missing, &mut tx).is_err());
}

#[test]
fn stage_plan_of_an_http_push_is_irreversible() {
    let remote = Fixture::bare();
    let server = Server::start(remote.git_dir(), false);
    let local = linked(&server.url());
    local.commit("main", &[("a.txt", "one\n")], &[]);
    let args = stage_args(&local, &[("branch", Value::Text("main".to_owned()))]);
    let ctx = nurt_git::testutil::test_ctx();
    let plan = PushCommand
        .as_effect()
        .unwrap()
        .plan(&args, StageInput::none(), &ctx)
        .unwrap();
    assert_eq!(plan.irreversible.len(), 1);
    assert_eq!(plan.preview.len(), 2);
    assert_eq!(remote.ref_value("refs/heads/main"), None);
}
