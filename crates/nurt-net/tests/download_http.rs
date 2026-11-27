// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-net/tests/download_http.rs
// Responsibility:   `http.download` against a server on the loopback interface: ranges.
// Dependencies:     http_server, nurt_contracts, nurt_testkit, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `http.download` against a server on the loopback interface: ranges,
//! entity tags, digests, status errors, cancellation and the journal.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers fail the test on an unexpected result"
)]

#[allow(
    unreachable_pub,
    reason = "the shared server module declares its helpers public"
)]
#[path = "support/http_server.rs"]
mod http_server;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use http_server::{Reply, Request, TestServer, requested_range};
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, RollbackReason};
use nurt_contracts::record::{ChangeAction, Effect};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, ExecCtx, StageData, StageImpl, StageInput,
};
use nurt_contracts::value::{Duration as ValueDuration, Size, Value};
use nurt_net::http::download::{COMMAND, Download};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use sha2::{Digest, Sha256, Sha512};

/// Length of the served file; larger than one read step of the command.
const FILE_SIZE: usize = 200_000;
/// Bytes sent before a stalled response stops.
const HALF: usize = 80_000;
/// Stall of an interrupted response; longer than the time limit of the test.
const STALL: Duration = Duration::from_secs(5);
/// Time limit of a download that is interrupted.
const SHORT_LIMIT_MS: i64 = 1000;
/// Stall of a response that the client ends by closing the connection; the
/// server stops waiting as soon as the connection is closed.
const HOLD: Duration = Duration::from_secs(60);
/// Longest wait for the partial file to reach `HALF` bytes.
const PART_WAIT: Duration = Duration::from_secs(30);
/// Interval at which the size of the partial file is checked.
const PART_POLL: Duration = Duration::from_millis(5);

/// Content of version `seed` of the served file.
fn content(seed: u8) -> Vec<u8> {
    (0..FILE_SIZE)
        .map(|index| u8::try_from(index % 251).unwrap_or(0) ^ seed)
        .collect()
}

/// A file served at `/file` with `Range`, `If-Range` and `ETag` support.
struct Resource {
    body: Mutex<Vec<u8>>,
    etag: Mutex<String>,
    stall_next: Mutex<Option<Duration>>,
}

impl Resource {
    fn new(body: Vec<u8>, etag: &str) -> Arc<Self> {
        Arc::new(Self {
            body: Mutex::new(body),
            etag: Mutex::new(etag.to_owned()),
            stall_next: Mutex::new(None),
        })
    }

    /// Makes the next response stop after `HALF` bytes for `stall`.
    fn stall_next(&self, stall: Duration) {
        *self.stall_next.lock().unwrap() = Some(stall);
    }

    fn replace(&self, body: Vec<u8>, etag: &str) {
        *self.body.lock().unwrap() = body;
        *self.etag.lock().unwrap() = etag.to_owned();
    }

    fn reply(&self, request: &Request) -> Reply {
        let body = self.body.lock().unwrap().clone();
        let etag = self.etag.lock().unwrap().clone();
        match request.route() {
            "/file" => {}
            "/moved" => return Reply::redirect(302, "/file"),
            "/broken" => return Reply::status(500),
            _ => return Reply::status(404),
        }
        let current = request.header("if-range").is_none_or(|tag| tag == etag);
        let reply = match requested_range(request, body.len()) {
            Some((start, end)) if current => Reply::status(206)
                .header(
                    "content-range",
                    &format!("bytes {start}-{}/{}", end - 1, body.len()),
                )
                .with_body(body[start..end].to_vec()),
            None if current && request.header("range").is_some() => {
                Reply::status(416).header("content-range", &format!("bytes */{}", body.len()))
            }
            _ => Reply::status(200).with_body(body),
        };
        let reply = reply.header("etag", &etag);
        match self.stall_next.lock().unwrap().take() {
            Some(stall) => reply.stalled(HALF, stall),
            None => reply,
        }
    }
}

fn serve(resource: &Arc<Resource>) -> TestServer {
    let shared = Arc::clone(resource);
    TestServer::start(move |request| shared.reply(request))
}

/// Temporary directory present both on disk, for the partial file, and in
/// the virtual file system of the journal, for the target.
struct Place {
    dir: tempfile::TempDir,
    context: TestContext,
}

impl Place {
    fn new() -> Self {
        let place = Self {
            dir: tempfile::tempdir().unwrap(),
            context: TestContext::new(),
        };
        let dir = AbsPath::new(place.dir.path()).unwrap();
        place.context.fs().create_dir_all(&dir).unwrap();
        place
    }

    fn target(&self) -> PathBuf {
        self.dir.path().join("file.bin")
    }

    fn target_abs(&self) -> AbsPath {
        AbsPath::new(self.target()).unwrap()
    }

    fn part(&self) -> PathBuf {
        self.dir.path().join("file.bin.part")
    }

    fn written(&self) -> Option<Vec<u8>> {
        self.context.fs().read_file(&self.target_abs()).ok()
    }

    fn args(&self, url: &str, extra: &[(&str, Value)]) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND)
            .with("url", ArgValue::Value(Value::Text(url.to_owned())), false)
            .with(
                "to",
                ArgValue::Value(Value::Path(self.target().display().to_string())),
                false,
            );
        for (name, value) in extra {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn run(&self, args: &BoundArgs) -> Result<Effect> {
        self.run_with(args, &self.context.ctx())
    }

    /// Plans and applies the command in one transaction, committing on
    /// success and rolling back on failure.
    fn run_with(&self, args: &BoundArgs, ctx: &ExecCtx) -> Result<Effect> {
        let stage = Download::new();
        let effect = stage.as_effect().unwrap();
        let plan = effect.plan(args, StageInput::none(), ctx)?;
        let mut tx = self.context.journal().begin(begin_request(COMMAND));
        match effect.apply(args, plan, tx.as_mut(), ctx) {
            Ok(outcome) => {
                let StageData::Records(stream) = outcome.output.data else {
                    panic!("the command returns records");
                };
                let records = stream.collect::<Result<Vec<_>>>()?;
                tx.commit()?;
                Ok(Effect::try_from(records[0].clone()).unwrap())
            }
            Err(error) => {
                tx.rollback(RollbackReason::TargetFailed)?;
                Err(error)
            }
        }
    }

    /// Runs a download that the server interrupts after `HALF` bytes.
    fn interrupted(&self, resource: &Resource, url: &str) {
        resource.stall_next(STALL);
        let limit = Value::Duration(ValueDuration::from_millis(SHORT_LIMIT_MS));
        let error = self
            .run(&self.args(url, &[("timeout", limit)]))
            .unwrap_err();
        assert!(
            matches!(error.code(), ErrorCode::Timeout | ErrorCode::Network),
            "{error:?}"
        );
        assert_eq!(part_size(&self.part()), Some(HALF as u64));
        assert_eq!(self.written(), None);
    }
}

fn part_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|meta| meta.len())
}

fn download_field<'a>(effect: &'a Effect, name: &str) -> &'a Value {
    let Some(Value::Map(fields)) = &effect.after else {
        panic!("the effect carries the download object");
    };
    &fields[name]
}

fn size(bytes: usize) -> Value {
    Value::Size(Size::from_bytes(bytes as u64))
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(bytes)
}

#[test]
fn download_writes_target_and_reports_effect() {
    let resource = Resource::new(content(0), "\"v1\"");
    let server = serve(&resource);
    let place = Place::new();
    let effect = place.run(&place.args(&server.url("/file"), &[])).unwrap();

    assert_eq!(place.written(), Some(content(0)));
    assert_eq!(effect.kind, COMMAND);
    assert_eq!(effect.targets, Some(1));
    let changes = effect.changes.clone().unwrap();
    assert_eq!(changes[0].action, ChangeAction::Create);
    assert_eq!(
        changes[0].size_after,
        Some(Size::from_bytes(FILE_SIZE as u64))
    );
    assert_eq!(
        changes[0].hash_after.as_deref(),
        Some(format!("sha256:{}", hex_digest(Sha256::digest(content(0)))).as_str())
    );
    assert_eq!(download_field(&effect, "status"), &Value::Int(200));
    assert_eq!(download_field(&effect, "bytes"), &size(FILE_SIZE));
    assert_eq!(download_field(&effect, "resumed_from"), &size(0));
    assert_eq!(
        download_field(&effect, "hash_verified"),
        &Value::Bool(false)
    );
    assert_eq!(download_field(&effect, "hash_algorithm"), &Value::Null);
    assert!(!place.part().exists());
    let request = &server.requests()[0];
    assert_eq!(request.header("range"), None);
    assert_eq!(request.header("accept-encoding"), Some("identity"));
}

#[test]
fn download_resume_range() {
    let resource = Resource::new(content(0), "\"v1\"");
    let server = serve(&resource);
    let place = Place::new();
    let url = server.url("/file");
    place.interrupted(&resource, &url);

    let effect = place.run(&place.args(&url, &[])).unwrap();
    assert_eq!(place.written(), Some(content(0)));
    assert_eq!(download_field(&effect, "status"), &Value::Int(206));
    assert_eq!(download_field(&effect, "resumed_from"), &size(HALF));
    assert_eq!(download_field(&effect, "bytes"), &size(FILE_SIZE));
    let last = server.requests().pop().unwrap();
    assert_eq!(
        last.header("range"),
        Some(format!("bytes={HALF}-").as_str())
    );
    assert_eq!(last.header("if-range"), Some("\"v1\""));
    assert!(!place.part().exists());
}

#[test]
fn download_resume_etag_changed_restarts() {
    let resource = Resource::new(content(0), "\"v1\"");
    let server = serve(&resource);
    let place = Place::new();
    let url = server.url("/file");
    place.interrupted(&resource, &url);

    resource.replace(content(7), "\"v2\"");
    let effect = place.run(&place.args(&url, &[])).unwrap();
    assert_eq!(place.written(), Some(content(7)));
    assert_eq!(download_field(&effect, "status"), &Value::Int(200));
    assert_eq!(download_field(&effect, "resumed_from"), &size(0));
}

#[test]
fn download_resume_false_restarts() {
    let resource = Resource::new(content(0), "\"v1\"");
    let server = serve(&resource);
    let place = Place::new();
    let url = server.url("/file");
    place.interrupted(&resource, &url);

    let effect = place
        .run(&place.args(&url, &[("resume", Value::Bool(false))]))
        .unwrap();
    assert_eq!(place.written(), Some(content(0)));
    assert_eq!(download_field(&effect, "resumed_from"), &size(0));
    assert_eq!(server.requests().pop().unwrap().header("range"), None);
}

#[test]
fn download_complete_part_answers_416() {
    let resource = Resource::new(content(0), "\"v1\"");
    let server = serve(&resource);
    let place = Place::new();
    let url = server.url("/file");
    place.interrupted(&resource, &url);
    let mut whole = fs::read(place.part()).unwrap();
    whole.extend_from_slice(&content(0)[HALF..]);
    fs::write(place.part(), &whole).unwrap();

    let effect = place.run(&place.args(&url, &[])).unwrap();
    assert_eq!(place.written(), Some(content(0)));
    assert_eq!(download_field(&effect, "status"), &Value::Int(416));
    assert_eq!(download_field(&effect, "resumed_from"), &size(FILE_SIZE));
}

#[test]
fn download_hash_mismatch_no_target() {
    let resource = Resource::new(content(0), "\"v1\"");
    let server = serve(&resource);
    let place = Place::new();
    let wrong = format!("sha256:{}", "00".repeat(32));
    let error = place
        .run(&place.args(
            &server.url("/file"),
            &[("hash", Value::Text(wrong.clone()))],
        ))
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::HashMismatch);
    assert_eq!(error.expected, Some(serde_json::Value::from(wrong)));
    assert_eq!(place.written(), None);
    assert!(!place.part().exists());
}

#[test]
fn td_02_hash_mismatch_keeps_previous_file() {
    let resource = Resource::new(content(0), "\"v1\"");
    let server = serve(&resource);
    let place = Place::new();
    let previous = b"previous release".to_vec();
    place
        .context
        .fs()
        .write_file(&place.target_abs(), &previous)
        .unwrap();
    let wrong = format!("blake3:{}", "11".repeat(32));
    let error = place
        .run(&place.args(&server.url("/file"), &[("hash", Value::Text(wrong))]))
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::HashMismatch);
    let actual = format!(
        "blake3:{}",
        hex_digest(blake3::hash(&content(0)).as_bytes())
    );
    assert_eq!(error.actual, Some(serde_json::Value::from(actual)));
    assert_eq!(place.written(), Some(previous));
    assert!(!place.part().exists());
}

#[test]
fn download_hash_verified_for_each_algorithm() {
    let resource = Resource::new(content(3), "\"v1\"");
    let server = serve(&resource);
    let digests = [
        ("sha256", hex_digest(Sha256::digest(content(3)))),
        ("sha512", hex_digest(Sha512::digest(content(3)))),
        ("blake3", hex_digest(blake3::hash(&content(3)).as_bytes())),
    ];
    for (algorithm, digest) in digests {
        let place = Place::new();
        let hash = Value::Text(format!("{}:{}", algorithm.to_uppercase(), digest));
        let effect = place
            .run(&place.args(&server.url("/file"), &[("hash", hash)]))
            .unwrap();
        assert_eq!(place.written(), Some(content(3)), "{algorithm}");
        assert_eq!(
            download_field(&effect, "hash_algorithm"),
            &Value::Text(algorithm.to_owned())
        );
        assert_eq!(download_field(&effect, "hash_verified"), &Value::Bool(true));
        let change = &effect.changes.clone().unwrap()[0];
        assert_eq!(
            change.hash_after.as_deref(),
            Some(format!("{algorithm}:{digest}").as_str())
        );
    }
}

#[test]
fn download_404_e_http_status() {
    let resource = Resource::new(content(0), "\"v1\"");
    let server = serve(&resource);
    let place = Place::new();
    for (path, status) in [("/missing", 404_u16), ("/broken", 500)] {
        let error = place.run(&place.args(&server.url(path), &[])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Network, "{path}");
        assert_eq!(error.problem.as_deref(), Some("http_status"));
        assert_eq!(
            error.data().details.get("status"),
            Some(&serde_json::Value::from(status))
        );
        assert_eq!(place.written(), None);
        assert!(!place.part().exists());
    }
}

#[test]
fn download_follows_redirect() {
    let resource = Resource::new(content(1), "\"v1\"");
    let server = serve(&resource);
    let place = Place::new();
    let effect = place.run(&place.args(&server.url("/moved"), &[])).unwrap();
    assert_eq!(place.written(), Some(content(1)));
    assert_eq!(
        download_field(&effect, "final_url"),
        &Value::Text(server.url("/file"))
    );
    assert_eq!(
        download_field(&effect, "url"),
        &Value::Text(server.url("/moved"))
    );
}

#[test]
fn download_atomic_replace_journaled() {
    let resource = Resource::new(content(2), "\"v1\"");
    let server = serve(&resource);
    let place = Place::new();
    let previous = b"previous content".to_vec();
    place
        .context
        .fs()
        .write_file(&place.target_abs(), &previous)
        .unwrap();
    place.context.journal().clear();

    let effect = place.run(&place.args(&server.url("/file"), &[])).unwrap();
    assert_eq!(place.written(), Some(content(2)));
    let change = &effect.changes.clone().unwrap()[0];
    assert_eq!(change.action, ChangeAction::Modify);
    assert_eq!(
        change.size_before,
        Some(Size::from_bytes(previous.len() as u64))
    );
    let calls = place.context.journal().calls();
    let position = |wanted: fn(&JournalCall) -> bool| calls.iter().position(wanted);
    let preserve = position(|call| matches!(call, JournalCall::Preserve { .. })).unwrap();
    let stage = position(|call| matches!(call, JournalCall::Stage { .. })).unwrap();
    let apply = position(|call| matches!(call, JournalCall::Apply { ok: true, .. })).unwrap();
    let commit = position(|call| matches!(call, JournalCall::Commit { .. })).unwrap();
    assert!(
        preserve < stage && stage < apply && apply < commit,
        "{calls:?}"
    );
}

#[test]
fn download_cancel_leaves_part() {
    let resource = Resource::new(content(0), "\"v1\"");
    let server = serve(&resource);
    let place = Place::new();
    resource.stall_next(HOLD);
    let cancel = CancelToken::new();
    let ctx = place.context.builder().cancel(cancel.clone()).build();
    let part = place.part();
    let trigger = thread::spawn(move || {
        let deadline = Instant::now() + PART_WAIT;
        while part_size(&part) < Some(HALF as u64) && Instant::now() < deadline {
            thread::sleep(PART_POLL);
        }
        cancel.cancel(CancelReason::Requested);
    });

    let error = place
        .run_with(&place.args(&server.url("/file"), &[]), &ctx)
        .unwrap_err();
    trigger.join().unwrap();
    assert_eq!(error.code(), ErrorCode::Cancelled, "{error:?}");
    assert_eq!(part_size(&place.part()), Some(HALF as u64));
    assert_eq!(place.written(), None);
}

#[test]
fn download_missing_directory_not_found() {
    let place = Place::new();
    let missing = place.dir.path().join("absent").join("file.bin");
    let args = place.args("http://127.0.0.1:9/file", &[]).with(
        "to",
        ArgValue::Value(Value::Path(missing.display().to_string())),
        false,
    );
    let error = place.run(&args).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
