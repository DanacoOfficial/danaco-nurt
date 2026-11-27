// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of `http.request` against a test server on the loopback interface.
//!
//! No test reaches a host outside the loopback interface: names resolve
//! through a table resolver, and unreachable addresses are closed local
//! ports.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a failed step ends the test"
)]

#[path = "support/http_server.rs"]
mod http_server;

use std::ffi::OsString;
use std::io::{self, Seek, SeekFrom, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use flate2::Compression;
use flate2::write::GzEncoder;
use indexmap::IndexMap;
use nurt_contracts::config::Config;
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, Durability, ExtendedMeta, FsResult, MetaTransferReport, WriteHandle,
};
use nurt_contracts::journal::{
    AfterState, Applied, BeforeState, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle,
    OpId, PreserveMode, Preserved, RollbackReason, RollbackReport, TargetIndex, TargetPath,
    TargetSpec,
};
use nurt_contracts::record::{Change, ChangeAction, HttpResponse, Row};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_contracts::spec::{CatalogFile, StageSpec};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, CheckCtx, EffectPlan, EffectStage, Event,
    EventSink, ExecCtx, Extensions, ReadStage, StageData, StageImpl, StageInput, StageOutput,
    StaticInfo,
};
use nurt_contracts::value::{self, Record, Size, Value};
use nurt_net::client::{ConnectGate, ConnectRequest, NameResolver};
use nurt_net::http::request::{COMMAND, HttpRequest, RESULTS_DIR, body_etag};

use http_server::{MockDns, Reply, Request, TestServer};

const MIB: usize = 1024 * 1024;

/// Page with navigation, a footer, three sections, a link and an image.
const ARTICLE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Packages</title><meta name=\"description\" content=\"Package guide\"></head><body>\
<nav>Site menu</nav><main><h1>Packages</h1><p>Introduction to packages.</p>\
<h2>Installation</h2><p>Package installation uses the install command.</p>\
<h2>API</h2><p>The programming interface.</p>\
<p>Read the <a href=\"/docs\">documentation</a> first. <img src=\"/a.png\" alt=\"Diagram\"></p>\
</main><footer>Page footer</footer></body></html>";

/// Page with five paragraphs, the second of which mentions a package.
const PARAGRAPHS: &str = "<html><body><main><p>Alpha line.</p><p>Beta package line.</p>\
<p>Gamma line.</p><p>Delta line.</p><p>Epsilon line.</p></main></body></html>";

/// Page with three tables of two data rows each.
const TABLES: &str = "<html><body><main>\
<table><tr><th>Name</th><th>Size</th></tr><tr><td>a</td><td>1</td></tr><tr><td>b</td><td>2</td></tr></table>\
<table><tr><th>Name</th><th>Size</th></tr><tr><td>c</td><td>3</td></tr><tr><td>d</td><td>4</td></tr></table>\
<table><tr><th>Name</th><th>Size</th></tr><tr><td>e</td><td>5</td></tr><tr><td>f</td><td>6</td></tr></table>\
</main></body></html>";

/// Start of a PNG file.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR";

/// Central European letters encoded differently by windows-1250 and
/// ISO-8859-2.
const LETTERS: &str = "\u{105}\u{107}\u{119}\u{142}\u{144}\u{f3}\u{15b}\u{17a}\u{17c}";

struct SystemClock {
    origin: Instant,
}

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        let since_epoch = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        WallTime::from_unix_nanos(i128::try_from(since_epoch).unwrap_or(i128::MAX))
    }

    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: self.monotonic(),
            since_boot_active: self.monotonic(),
        }
    }

    fn sleep(
        &self,
        duration: Duration,
        cancel: &CancelToken,
    ) -> std::result::Result<(), CancelReason> {
        match cancel.wait_timeout(duration) {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}

/// Host without environment variables, so that no proxy of the machine
/// running the tests applies.
struct FixedHost(HostInfo);

impl Host for FixedHost {
    fn info(&self) -> &HostInfo {
        &self.0
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

fn host() -> FixedHost {
    FixedHost(HostInfo {
        os: Os::Linux,
        os_version: "6.8.0".to_owned(),
        architecture: "x86_64".to_owned(),
        hostname: "host".to_owned(),
        user: "user".to_owned(),
        home: "/home/user".into(),
        temp: std::env::temp_dir(),
        path_style: PathStyle::Posix,
        timezone: "UTC".to_owned(),
        locale: None,
        cpus: 1,
    })
}

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);

impl EventSink for Events {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    fn logs(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::Log(log) => Some(log.message.clone()),
                _ => None,
            })
            .collect()
    }

    fn phases(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::Progress(progress) => Some(progress.phase.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Connection gate that records every request and refuses redirects.
#[derive(Default)]
struct DenyRedirects {
    seen: Mutex<Vec<(u32, Vec<IpAddr>)>>,
}

impl ConnectGate for DenyRedirects {
    fn check(&self, request: &ConnectRequest<'_>) -> std::result::Result<(), String> {
        self.seen
            .lock()
            .unwrap()
            .push((request.hop, request.addresses.to_vec()));
        if request.hop > 0 {
            Err("redirects are not allowed".to_owned())
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct Setup {
    dns: Option<Arc<MockDns>>,
    gate: Option<Arc<dyn ConnectGate>>,
    config: Config,
    max_chars: Option<u64>,
    cancel: Option<CancelToken>,
}

struct Harness {
    ctx: ExecCtx,
    events: Arc<Events>,
    state: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        Self::with(Setup::default())
    }

    fn with(setup: Setup) -> Self {
        let state = tempfile::tempdir().unwrap();
        let events = Arc::new(Events::default());
        let mut extensions = Extensions::new();
        if let Some(dns) = setup.dns {
            extensions.insert::<dyn NameResolver>(dns);
        }
        if let Some(gate) = setup.gate {
            extensions.insert::<dyn ConnectGate>(gate);
        }
        let options = RequestOptions {
            max_chars: setup.max_chars,
            ..RequestOptions::default()
        };
        let clock = Arc::new(SystemClock {
            origin: Instant::now(),
        });
        let mut builder = ExecCtx::builder(clock, Arc::new(host()))
            .events(Arc::clone(&events) as Arc<dyn EventSink>)
            .config(Arc::new(setup.config))
            .options(Arc::new(options))
            .state_dir(state.path().to_path_buf())
            .extensions(extensions);
        if let Some(cancel) = setup.cancel {
            builder = builder.cancel(cancel);
        }
        Self {
            ctx: builder.build(),
            events,
            state,
        }
    }

    fn run(&self, values: &[(&str, Value)]) -> Result<Vec<Record>> {
        let stage = HttpRequest;
        let bound = args(values);
        let read: &dyn ReadStage = stage.as_read().unwrap();
        let output = read.execute(&bound, StageInput::none(), &self.ctx)?;
        collect(output)
    }

    fn one(&self, values: &[(&str, Value)]) -> HttpResponse {
        let mut records = self.run(values).unwrap();
        assert_eq!(records.len(), 1, "one response is expected");
        HttpResponse::try_from(records.remove(0)).unwrap()
    }

    fn fail(&self, values: &[(&str, Value)]) -> Error {
        match self.run(values) {
            Ok(records) => panic!("the request succeeded with {} records", records.len()),
            Err(error) => error,
        }
    }

    fn plan(&self, values: &[(&str, Value)]) -> Result<EffectPlan> {
        let stage = HttpRequest;
        let bound = args(values);
        let effect: &dyn EffectStage = stage.as_effect().unwrap();
        effect.plan(&bound, StageInput::none(), &self.ctx)
    }

    fn apply(&self, values: &[(&str, Value)], tx: &mut FileTx) -> Result<Outcome> {
        let stage = HttpRequest;
        let bound = args(values);
        let effect: &dyn EffectStage = stage.as_effect().unwrap();
        let plan = effect.plan(&bound, StageInput::none(), &self.ctx)?;
        let outcome = effect.apply(&bound, plan, tx, &self.ctx)?;
        let changes = outcome.changes.clone();
        let irreversible = outcome.irreversible.clone();
        let responses = collect(outcome.output)?
            .into_iter()
            .map(|record| HttpResponse::try_from(record).unwrap())
            .collect();
        Ok(Outcome {
            responses,
            changes,
            irreversible,
        })
    }
}

/// Result of a stage run as the effect of a program.
struct Outcome {
    responses: Vec<HttpResponse>,
    changes: Vec<Change>,
    irreversible: Vec<IrreversibleEntry>,
}

/// Transaction that writes staged bodies to files of a test directory.
#[derive(Default)]
struct FileTx {
    targets: Vec<PathBuf>,
    committed: Vec<PathBuf>,
    irreversible: Vec<IrreversibleEntry>,
}

impl FileTx {
    fn target(&self, index: TargetIndex) -> PathBuf {
        self.targets[usize::try_from(index.0).unwrap()].clone()
    }
}

fn staged_path(target: &FsPath) -> PathBuf {
    target.with_extension("staged")
}

struct StagedFile {
    path: AbsPath,
    file: std::fs::File,
}

impl Write for StagedFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for StagedFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

impl WriteHandle for StagedFile {
    fn path(&self) -> &AbsPath {
        &self.path
    }

    fn sync(&mut self, _durability: Durability) -> FsResult<()> {
        Ok(())
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        self.file.set_len(length).unwrap();
        Ok(())
    }

    fn apply_meta(&mut self, _meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        Ok(MetaTransferReport::default())
    }
}

impl MutationTx for FileTx {
    fn op(&self) -> Option<OpId> {
        None
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        let mut preserved = Vec::new();
        for spec in targets {
            let TargetPath::Fs(path) = &spec.path else {
                panic!("only file targets are expected");
            };
            let index = TargetIndex(u32::try_from(self.targets.len()).unwrap());
            let existing = std::fs::metadata(path.as_path()).ok();
            self.targets.push(path.as_path().to_path_buf());
            preserved.push(Preserved {
                index,
                volume: None,
                mode: PreserveMode::Moved,
                bytes_held: 0,
                before: BeforeState {
                    exists: existing.is_some(),
                    size: existing.map(|meta| meta.len()),
                    ..BeforeState::default()
                },
                irreversible: None,
                notable: Vec::new(),
            });
        }
        Ok(preserved)
    }

    fn stage(&mut self, target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        let path = staged_path(&self.target(target));
        let file = std::fs::File::create(&path).unwrap();
        Ok(Box::new(StagedFile {
            path: AbsPath::new(path).unwrap(),
            file,
        }))
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        assert!(
            matches!(mutation, Mutation::CommitStaged),
            "only staged writes are expected"
        );
        let path = self.target(target);
        std::fs::rename(staged_path(&path), &path).unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        self.committed.push(path);
        Ok(Applied {
            index: target,
            after: AfterState {
                exists: true,
                size: Some(size),
                ..AfterState::default()
            },
        })
    }

    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        Ok(Vec::new())
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.irreversible.push(marker);
    }

    fn record_exempt(&mut self, _write: ExemptWrite) {}

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        Err(Error::new(
            ErrorCode::Internal,
            "the test transaction is finished by the test",
        ))
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        Err(Error::new(
            ErrorCode::Internal,
            "the test transaction is finished by the test",
        ))
    }
}

fn collect(output: StageOutput<'_>) -> Result<Vec<Record>> {
    match output.data {
        StageData::Records(stream) => stream.collect(),
        other => panic!("expected records, got {other:?}"),
    }
}

fn args(values: &[(&str, Value)]) -> BoundArgs {
    values
        .iter()
        .fold(BoundArgs::new(COMMAND), |bound, (name, value)| {
            bound.with(*name, ArgValue::Value(value.clone()), false)
        })
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn list(items: &[String]) -> Value {
    Value::List(items.iter().map(|item| text(item)).collect())
}

fn map(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

fn regex(pattern: &str) -> Value {
    Value::Regex(value::Regex {
        pattern: pattern.to_owned(),
        flags: value::RegexFlags::default(),
    })
}

fn millis(count: i64) -> Value {
    Value::Duration(value::Duration::from_millis(count))
}

fn count_of(value: usize) -> u64 {
    u64::try_from(value).unwrap()
}

fn catalog_spec() -> StageSpec {
    let path = format!(
        "{}/../../catalog/network.toml",
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR"))
    );
    let mut file: CatalogFile = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    file.resolve_groups();
    file.command(COMMAND).unwrap().clone()
}

fn check(values: &[(&str, Value)], config: &Config) -> Result<StaticInfo> {
    let spec = catalog_spec();
    let host = host();
    HttpRequest.check(&args(values), &CheckCtx::new(&spec, &host, config))
}

fn entries(info: &StaticInfo) -> Vec<IndexMap<String, Value>> {
    let Value::List(items) = &info.details["network"] else {
        panic!("the network details are not a list");
    };
    items
        .iter()
        .map(|item| match item {
            Value::Map(entry) => entry.clone(),
            other => panic!("a network entry is {other:?}"),
        })
        .collect()
}

fn first_address(entry: &IndexMap<String, Value>) -> IndexMap<String, Value> {
    match &entry["addresses"] {
        Value::List(items) => match items.first() {
            Some(Value::Map(address)) => address.clone(),
            other => panic!("no address in {other:?}"),
        },
        other => panic!("the addresses are {other:?}"),
    }
}

fn problem(error: &Error) -> Option<&str> {
    error.data().problem.as_deref()
}

fn html_server(page: impl Into<String>) -> TestServer {
    let page = page.into();
    TestServer::start(move |_: &Request| {
        Reply::body(200, "text/html; charset=utf-8", page.clone().into_bytes())
    })
}

fn closed_port() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.local_addr().unwrap().port()
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// Page of more than 40 000 characters in 40 sections.
fn long_page() -> String {
    let mut html = String::from(
        "<!doctype html><html><head><title>Manual</title></head><body><main><h1>Manual</h1>",
    );
    let detail = "Detail text follows here. ".repeat(8);
    for part in 1..=40 {
        html.push_str(&format!("<h2>Part {part}</h2>"));
        for line in 1..=4 {
            html.push_str(&format!(
                "<p>Paragraph {line} of part {part} describes one step of the procedure. {detail}</p>"
            ));
        }
    }
    html.push_str("</main></body></html>");
    html
}

/// Page with six paragraphs about package installation and two unrelated
/// ones.
fn steps_page() -> String {
    let mut html = String::from("<html><body><main><h1>Setup</h1>");
    for step in 1..=6 {
        html.push_str(&format!(
            "<p>Item {step}: package installation step {step} for the server.</p>"
        ));
    }
    html.push_str("<p>Unrelated weather note.</p><p>Another unrelated remark.</p>");
    html.push_str("</main></body></html>");
    html
}

fn flaky_server(retry_after: Option<&'static str>) -> TestServer {
    let calls = AtomicUsize::new(0);
    TestServer::start(move |_: &Request| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let reply = Reply::status(503);
            match retry_after {
                Some(seconds) => reply.header("retry-after", seconds),
                None => reply,
            }
        } else {
            Reply::body(200, "text/plain", "recovered")
        }
    })
}

#[test]
fn tf_01_markdown_keeps_main_content() {
    let server = html_server(ARTICLE);
    let harness = Harness::new();
    let url = text(&server.url("/guide"));
    let main = harness.one(&[("url", url.clone()), ("as", text("markdown"))]);
    assert!(
        main.body
            .contains("Package installation uses the install command.")
    );
    assert!(main.body.contains("Diagram"));
    assert!(!main.body.contains("Site menu"));
    assert!(!main.body.contains("Page footer"));
    let whole = harness.one(&[
        ("url", url),
        ("as", text("markdown")),
        ("main_only", Value::Bool(false)),
    ]);
    assert!(whole.body.contains("Site menu"));
    assert!(whole.body.contains("Page footer"));
}

#[test]
fn tf_02_link_list_and_dropped_images() {
    let server = html_server(ARTICLE);
    let response = Harness::new().one(&[
        ("url", text(&server.url("/guide"))),
        ("as", text("markdown")),
        ("links", text("list")),
        ("images", text("drop")),
    ]);
    assert!(
        response
            .body
            .contains(&format!("[1]: {}", server.url("/docs")))
    );
    assert!(!response.body.contains("Diagram"));
}

#[test]
fn tf_03_long_page_returns_outline_with_result_file() {
    let server = html_server(long_page());
    let harness = Harness::with(Setup {
        max_chars: Some(1000),
        ..Setup::default()
    });
    let url = text(&server.url("/manual"));
    let outline = harness.one(&[("url", url.clone()), ("as", text("markdown"))]);
    assert!(outline.body.contains("title: Manual"));
    assert!(outline.body.contains("sections (chars):"));
    assert!(outline.body.contains("Part 40 ("));
    let path = outline
        .body
        .lines()
        .find_map(|line| line.strip_prefix("full content: "))
        .unwrap();
    let full = std::fs::read_to_string(path).unwrap();
    assert!(full.chars().count() > 40_000);
    assert!(full.contains("Paragraph 4 of part 40"));
    let never = harness.one(&[
        ("url", url),
        ("as", text("markdown")),
        ("outline", text("never")),
    ]);
    assert!(never.body.chars().count() > 40_000);
    assert!(!never.body.contains("sections (chars):"));
}

#[test]
fn tf_04_section_read_from_cache_without_request() {
    let server = html_server(ARTICLE);
    let harness = Harness::new();
    let url = text(&server.url("/guide"));
    let first = harness.one(&[
        ("url", url.clone()),
        ("as", text("markdown")),
        ("cache", text("use")),
    ]);
    assert_eq!(first.from_cache, Some(false));
    let section = harness.one(&[
        ("url", url),
        ("as", text("markdown")),
        ("section", text("Installation")),
        ("cache", text("use")),
    ]);
    assert!(section.body.contains("Package installation uses"));
    assert!(!section.body.contains("Introduction to packages"));
    assert_eq!(section.from_cache, Some(true));
    assert_eq!(server.request_count(), 1);
}

#[test]
fn tf_05_query_returns_best_paragraphs_within_limit() {
    let server = html_server(steps_page());
    let harness = Harness::new();
    let url = text(&server.url("/setup"));
    let query = text("package installation");
    let best = harness.one(&[
        ("url", url.clone()),
        ("as", text("text")),
        ("query", query.clone()),
    ]);
    let found = best.body.matches("Item ").count();
    assert!((1..=5).contains(&found), "{found} paragraphs");
    assert!(!best.body.contains("Unrelated"));
    let two = harness.one(&[
        ("url", url.clone()),
        ("as", text("text")),
        ("query", query.clone()),
        ("query_limit", Value::Int(2)),
    ]);
    let found = two.body.matches("Item ").count();
    assert!((1..=2).contains(&found), "{found} paragraphs");
    let error = harness.fail(&[
        ("url", url),
        ("as", text("text")),
        ("query", query),
        ("query_limit", Value::Int(21)),
    ]);
    assert_eq!(error.code(), ErrorCode::Arg);
}

#[test]
fn tf_06_offset_continues_and_rejects_past_end() {
    let server = html_server(long_page());
    let harness = Harness::new();
    let url = text(&server.url("/manual"));
    let full = harness.one(&[("url", url.clone()), ("as", text("text"))]);
    let total = full.body.chars().count();
    assert_eq!(full.total_chars, Some(count_of(total)));
    let rest = harness.one(&[
        ("url", url.clone()),
        ("as", text("text")),
        ("offset", Value::Int(1000)),
    ]);
    assert_eq!(rest.body, full.body.chars().skip(1000).collect::<String>());
    assert_eq!(rest.offset, Some(1000));
    assert_eq!(rest.total_chars, Some(count_of(total)));
    let past = i64::try_from(total + 1).unwrap();
    let error = harness.fail(&[
        ("url", url),
        ("as", text("text")),
        ("offset", Value::Int(past)),
    ]);
    assert_eq!(error.code(), ErrorCode::Arg);
    assert_eq!(error.data().actual, Some(serde_json::json!(total)));
}

#[test]
fn tf_07_list_with_unreachable_address() {
    let server = html_server(ARTICLE);
    let closed = closed_port();
    let unreachable = format!("http://127.0.0.1:{closed}/c");
    let urls = list(&[server.url("/a"), server.url("/b"), unreachable.clone()]);
    let harness = Harness::new();
    let error = harness.fail(&[("url", urls.clone())]);
    assert_eq!(error.code(), ErrorCode::Network);
    let records = harness
        .run(&[("url", urls), ("on_error", text("record"))])
        .unwrap();
    assert_eq!(records.len(), 2);
    assert!(
        harness
            .events
            .logs()
            .iter()
            .any(|message| message.contains(&unreachable) && message.contains("E_NETWORK"))
    );
}

#[test]
fn tf_08_parallel_requests_are_limited() {
    let server = TestServer::start(|_: &Request| {
        Reply::body(200, "text/plain", "ok").delayed(Duration::from_millis(100))
    });
    let urls: Vec<String> = (0..12)
        .map(|index| server.url(&format!("/{index}")))
        .collect();
    let records = Harness::new().run(&[("url", list(&urls))]).unwrap();
    assert_eq!(records.len(), 12);
    assert_eq!(server.request_count(), 12);
    assert!(
        server.peak_connections() <= 4,
        "{} connections",
        server.peak_connections()
    );
}

#[test]
fn tf_09_head_returns_status_and_headers_without_body() {
    let server = html_server(ARTICLE);
    let response = Harness::new().one(&[
        ("url", text(&server.url("/guide"))),
        ("method", text("head")),
        ("as", text("markdown")),
    ]);
    assert_eq!(response.status, 200);
    assert!(response.headers.contains_key("content-type"));
    assert!(response.body.is_empty());
}

#[test]
fn tf_10_headers_and_metadata() {
    let server = html_server(ARTICLE);
    let harness = Harness::new();
    let url = text(&server.url("/guide"));
    let headers = harness.one(&[("url", url.clone()), ("extract", text("headers"))]);
    assert!(headers.body.is_empty());
    assert_eq!(
        headers.headers["content-type"],
        text("text/html; charset=utf-8")
    );
    assert_eq!(headers.content_type.as_deref(), Some("text/html"));
    assert_eq!(headers.charset.as_deref(), Some("utf-8"));
    let meta = harness.one(&[
        ("url", url),
        ("as", text("text")),
        ("extract", text("meta")),
    ]);
    assert!(meta.body.contains("title: Packages"));
    assert!(meta.body.contains("description: Package guide"));
    assert!(meta.body.contains("language: en"));
}

#[test]
fn tf_11_tables_as_rows() {
    let server = html_server(TABLES);
    let harness = Harness::new();
    let url = text(&server.url("/tables"));
    let all = harness
        .run(&[("url", url.clone()), ("extract", text("tables"))])
        .unwrap();
    assert_eq!(all.len(), 6);
    let second: Vec<Row> = harness
        .run(&[
            ("url", url.clone()),
            ("extract", text("tables")),
            ("table", Value::Int(2)),
        ])
        .unwrap()
        .into_iter()
        .map(|record| Row::try_from(record).unwrap())
        .collect();
    assert_eq!(second.len(), 2);
    assert!(second.iter().all(|row| row.table.as_deref() == Some("2")));
    assert_eq!(second[0].cells["Name"], text("c"));
    assert_eq!(second[1].no, 2);
    let error = harness.fail(&[
        ("url", url),
        ("extract", text("tables")),
        ("table", Value::Int(9)),
    ]);
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert_eq!(error.data().actual, Some(serde_json::json!(3)));
}

#[test]
fn tf_12_json_body_returned_as_text() {
    let json = "{\"items\":[{\"name\":\"first\"}]}";
    let server = TestServer::start(move |_: &Request| Reply::body(200, "application/json", json));
    let response = Harness::new().one(&[("url", text(&server.url("/items")))]);
    assert_eq!(response.body, json);
    assert_eq!(response.content_type.as_deref(), Some("application/json"));
}

#[test]
fn tf_13_find_with_context_and_invalid_pattern() {
    let server = html_server(PARAGRAPHS);
    let harness = Harness::new();
    let url = text(&server.url("/lines"));
    let found = harness.one(&[
        ("url", url.clone()),
        ("as", text("text")),
        ("find", regex("package")),
        ("find_context", Value::Int(1)),
    ]);
    assert!(found.body.contains("Alpha line."));
    assert!(found.body.contains("Beta package line."));
    assert!(found.body.contains("Gamma line."));
    assert!(!found.body.contains("Epsilon line."));
    let error = harness.fail(&[("url", url), ("as", text("text")), ("find", regex("("))]);
    assert_eq!(error.code(), ErrorCode::Regex);
}

#[test]
fn tf_14_markdown_section_ignores_headings_in_code() {
    let document = "# Guide\n\nIntro text.\n\n```sh\n# API is not a heading here\n```\n\n\
## API\n\nEndpoint list.\n\n## Other\n\nOther text.\n";
    let server = TestServer::start(move |_: &Request| Reply::body(200, "text/markdown", document));
    let response = Harness::new().one(&[
        ("url", text(&server.url("/guide.md"))),
        ("as", text("markdown")),
        ("section", text("API")),
    ]);
    assert!(response.body.contains("Endpoint list."));
    assert!(!response.body.contains("Intro text."));
    assert!(!response.body.contains("Other text."));
}

#[test]
fn tf_15_compression_and_character_sets() {
    let compressed = gzip(b"compressed body");
    let server = TestServer::start(move |request: &Request| match request.route() {
        "/gzip" => Reply::body(200, "text/plain; charset=utf-8", compressed.clone())
            .header("content-encoding", "gzip"),
        "/meta" => {
            let page = format!(
                "<html><head><meta charset=\"windows-1250\"><title>Letters</title></head>\
                 <body><main><p>Letters: {LETTERS}</p></main></body></html>"
            );
            let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode(&page);
            Reply::body(200, "text/html", bytes.into_owned())
        }
        "/latin2" => {
            let (bytes, _, _) = encoding_rs::ISO_8859_2.encode(LETTERS);
            Reply::body(200, "text/plain; charset=windows-1250", bytes.into_owned())
        }
        _ => Reply::body(200, "text/plain", vec![b'a'; 3 * MIB]),
    });
    let harness = Harness::new();
    let unpacked = harness.one(&[("url", text(&server.url("/gzip")))]);
    assert_eq!(unpacked.body, "compressed body");
    let declared = harness.one(&[("url", text(&server.url("/meta"))), ("as", text("text"))]);
    assert!(declared.body.contains(LETTERS));
    assert_eq!(declared.charset.as_deref(), Some("windows-1250"));
    let forced = harness.one(&[
        ("url", text(&server.url("/latin2"))),
        ("charset", text("iso-8859-2")),
    ]);
    assert_eq!(forced.body, LETTERS);
    assert_eq!(forced.charset.as_deref(), Some("iso-8859-2"));
    let limit = Size::from_bytes(count_of(2 * MIB));
    let truncated = harness.one(&[
        ("url", text(&server.url("/large"))),
        ("max_size", Value::Size(limit)),
    ]);
    assert_eq!(truncated.truncated, Some(true));
    assert_eq!(truncated.bytes, limit);
    assert!(
        harness
            .events
            .logs()
            .iter()
            .any(|message| message.starts_with("body_truncated"))
    );
}

#[test]
fn tf_16_cache_hit_revalidation_bypass_and_only() {
    let server = TestServer::start(|request: &Request| {
        if request.header("if-none-match") == Some("\"v1\"") {
            Reply::status(304).header("etag", "\"v1\"")
        } else {
            Reply::body(200, "text/plain", "cached body").header("etag", "\"v1\"")
        }
    });
    let dns = MockDns::loopback(&["cache.test"]);
    let harness = Harness::with(Setup {
        dns: Some(Arc::clone(&dns)),
        ..Setup::default()
    });
    let url = text(&format!("http://cache.test:{}/c", server.port()));
    let first = harness.one(&[("url", url.clone()), ("cache", text("use"))]);
    assert_eq!(first.from_cache, Some(false));
    let hit = harness.one(&[("url", url.clone()), ("cache", text("use"))]);
    assert_eq!(hit.from_cache, Some(true));
    assert_eq!(hit.body, "cached body");
    assert_eq!(server.request_count(), 1);
    let revalidated = harness.one(&[("url", url.clone()), ("cache", text("revalidate"))]);
    assert_eq!(revalidated.from_cache, Some(true));
    assert_eq!(revalidated.body, "cached body");
    assert_eq!(server.request_count(), 2);
    assert_eq!(server.requests()[1].header("if-none-match"), Some("\"v1\""));
    let bypass = harness.one(&[("url", url.clone()), ("cache", text("bypass"))]);
    assert_eq!(bypass.from_cache, Some(false));
    assert_eq!(server.request_count(), 3);
    let empty_dns = MockDns::loopback(&["cache.test"]);
    let offline = Harness::with(Setup {
        dns: Some(Arc::clone(&empty_dns)),
        ..Setup::default()
    });
    let error = offline.fail(&[("url", url), ("cache", text("only"))]);
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert_eq!(empty_dns.lookups(), 0);
    assert_eq!(server.request_count(), 3);
}

#[test]
fn tf_17_own_headers_bypass_cache() {
    let server = html_server(ARTICLE);
    let harness = Harness::new();
    let values = [
        ("url", text(&server.url("/guide"))),
        ("headers", map(&[("Accept-Language", text("en"))])),
        ("cache", text("use")),
    ];
    let first = harness.one(&values);
    let second = harness.one(&values);
    assert_eq!(first.from_cache, Some(false));
    assert_eq!(second.from_cache, Some(false));
    assert_eq!(server.request_count(), 2);
    assert_eq!(server.requests()[0].header("accept-language"), Some("en"));
}

fn redirect_server() -> TestServer {
    TestServer::start(|request: &Request| match request.route() {
        "/r1" => Reply::redirect(301, "/r2"),
        "/r2" => Reply::redirect(302, "/final"),
        "/final" => Reply::body(200, "text/plain", "done"),
        "/bad" => Reply::redirect(302, "http://[zz]/"),
        route => match route
            .strip_prefix("/loop/")
            .and_then(|step| step.parse::<u32>().ok())
        {
            Some(step) => Reply::redirect(302, &format!("/loop/{}", step + 1)),
            None => Reply::status(404),
        },
    })
}

#[test]
fn tf_18_redirect_chain_limits_and_bad_location() {
    let server = redirect_server();
    let harness = Harness::new();
    let followed = harness.one(&[("url", text(&server.url("/r1")))]);
    assert_eq!(followed.status, 200);
    assert_eq!(followed.final_url, server.url("/final"));
    assert_eq!(followed.redirects.map(|steps| steps.len()), Some(2));
    let stopped = harness.one(&[
        ("url", text(&server.url("/r1"))),
        ("follow_redirects", Value::Bool(false)),
    ]);
    assert_eq!(stopped.status, 301);
    assert_eq!(stopped.headers["location"], text("/r2"));
    assert_eq!(stopped.final_url, server.url("/r1"));
    let limited = harness.fail(&[
        ("url", text(&server.url("/loop/0"))),
        ("max_redirects", Value::Int(5)),
    ]);
    assert_eq!(limited.code(), ErrorCode::Network);
    assert_eq!(problem(&limited), Some("redirect_limit"));
    let bad = harness.fail(&[("url", text(&server.url("/bad")))]);
    assert_eq!(bad.code(), ErrorCode::Network);
    assert_eq!(problem(&bad), Some("bad_location"));
}

#[test]
fn tf_19_statuses_are_data_unless_failing_is_requested() {
    let server = TestServer::start(|request: &Request| {
        let status = request
            .route()
            .trim_start_matches('/')
            .parse()
            .unwrap_or(500);
        Reply::body(status, "text/plain", "status body")
    });
    let harness = Harness::new();
    for status in [403_u16, 404, 429, 500, 504, 418] {
        let url = text(&server.url(&format!("/{status}")));
        let response = harness.one(&[("url", url.clone())]);
        assert_eq!(response.status, u64::from(status));
        let error = harness.fail(&[("url", url), ("fail_on_status", Value::Bool(true))]);
        assert_eq!(error.code(), ErrorCode::Network);
        assert_eq!(problem(&error), Some("status"));
        assert_eq!(error.data().actual, Some(serde_json::json!(status)));
    }
}

#[test]
fn tf_20_retries_after_transient_status() {
    let server = flaky_server(None);
    let harness = Harness::new();
    let response = harness.one(&[
        ("url", text(&server.url("/flaky"))),
        ("retries", Value::Int(1)),
    ]);
    assert_eq!(response.status, 200);
    assert_eq!(server.request_count(), 2);
    assert!(harness.events.phases().iter().any(|phase| phase == "retry"));

    let waiting = flaky_server(Some("1"));
    let started = Instant::now();
    let response = harness.one(&[
        ("url", text(&waiting.url("/flaky"))),
        ("retries", Value::Int(1)),
    ]);
    assert_eq!(response.status, 200);
    assert!(started.elapsed() >= Duration::from_millis(900));

    let posted = flaky_server(None);
    let applied = harness
        .apply(
            &[
                ("url", text(&posted.url("/flaky"))),
                ("method", text("post")),
                ("retries", Value::Int(1)),
            ],
            &mut FileTx::default(),
        )
        .unwrap();
    assert_eq!(applied.responses[0].status, 503);
    assert_eq!(posted.request_count(), 1);
}

#[test]
fn tf_21_timeout_and_cancellation() {
    let slow = TestServer::start(|_: &Request| {
        Reply::body(200, "text/plain", "late").delayed(Duration::from_secs(5))
    });
    let started = Instant::now();
    let error = Harness::new().fail(&[("url", text(&slow.url("/"))), ("timeout", millis(1000))]);
    assert_eq!(error.code(), ErrorCode::Timeout);
    assert!(started.elapsed() < Duration::from_secs(4));

    let stalled = TestServer::start(|_: &Request| {
        Reply::body(200, "text/plain", vec![b'x'; 64 * 1024]).stalled(16, Duration::from_secs(5))
    });
    let cancel = CancelToken::new();
    let harness = Harness::with(Setup {
        cancel: Some(cancel.clone()),
        ..Setup::default()
    });
    let stopper = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        cancel.cancel(CancelReason::Requested);
    });
    let error = harness.fail(&[("url", text(&stalled.url("/")))]);
    stopper.join().unwrap();
    assert_eq!(error.code(), ErrorCode::Cancelled);
    assert!(stalled.wait_aborted(1, Duration::from_secs(3)));
}

#[test]
fn tf_22_unchanged_since_known_etag() {
    let body = Arc::new(Mutex::new("first version".to_owned()));
    let shared = Arc::clone(&body);
    let server = TestServer::start(move |_: &Request| {
        Reply::body(
            200,
            "text/plain",
            shared.lock().unwrap().clone().into_bytes(),
        )
    });
    let harness = Harness::new();
    let url = text(&server.url("/doc"));
    let first = harness.one(&[("url", url.clone())]);
    let etag = first.etag.clone().unwrap();
    assert_eq!(etag, body_etag("first version"));
    let same = harness.one(&[("url", url.clone()), ("unchanged_since", text(&etag))]);
    assert_eq!(same.unchanged, Some(true));
    assert!(same.body.is_empty());
    assert_eq!(same.etag.as_deref(), Some(etag.as_str()));
    *body.lock().unwrap() = "second version".to_owned();
    let changed = harness.one(&[("url", url), ("unchanged_since", text(&etag))]);
    assert_eq!(changed.unchanged, Some(false));
    assert_eq!(changed.body, "second version");
    assert_ne!(changed.etag, Some(etag));
}

#[test]
fn tf_23_loopback_address_is_contacted() {
    let server = html_server(ARTICLE);
    let url = text(&server.url("/"));
    let response = Harness::new().one(&[("url", url.clone())]);
    assert_eq!(response.status, 200);
    assert_eq!(response.remote_address, Some(server.address().to_string()));
    let info = check(&[("url", url)], &Config::default()).unwrap();
    let entry = &entries(&info)[0];
    assert_eq!(entry["host_kind"], text("ipv4"));
    assert_eq!(first_address(entry)["class"], text("loopback"));
}

#[test]
fn tf_24_check_normalizes_hosts() {
    let inputs = [
        "http://2130706433/".to_owned(),
        "http://0x7f.1/".to_owned(),
        "http://[::ffff:127.0.0.1]/".to_owned(),
        "https://b\u{fc}cher.example/".to_owned(),
    ];
    let info = check(&[("url", list(&inputs))], &Config::default()).unwrap();
    let found = entries(&info);
    assert_eq!(found.len(), 4);
    for entry in &found[..2] {
        assert_eq!(entry["host"], text("127.0.0.1"));
        assert_eq!(first_address(entry)["class"], text("loopback"));
    }
    let mapped = first_address(&found[2]);
    assert_eq!(mapped["class"], text("ipv4_mapped"));
    assert_eq!(
        mapped["embedded"],
        map(&[("ip", text("127.0.0.1")), ("class", text("loopback"))])
    );
    assert_eq!(found[3]["host"], text("xn--bcher-kva.example"));
    assert_eq!(found[3]["url"], text("https://xn--bcher-kva.example/"));
    assert_eq!(found[3]["url_input"], text(&inputs[3]));
    assert_eq!(found[3]["port"], Value::Int(443));
}

#[test]
fn tf_25_pinned_address_wins_over_resolver() {
    let server = html_server(ARTICLE);
    let dns = Arc::new(MockDns::default());
    dns.set("pinned.test", &[IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))]);
    let harness = Harness::with(Setup {
        dns: Some(Arc::clone(&dns)),
        ..Setup::default()
    });
    let port = server.port();
    let authority = format!("pinned.test:{port}");
    let values = [
        ("url", text(&format!("http://pinned.test:{port}/a"))),
        (
            "resolve",
            map(&[(authority.as_str(), list(&["127.0.0.1".to_owned()]))]),
        ),
    ];
    let response = harness.one(&values);
    assert_eq!(response.status, 200);
    assert_eq!(response.remote_address, Some(format!("127.0.0.1:{port}")));
    assert_eq!(dns.lookups(), 0);
    let info = check(&values, &Config::default()).unwrap();
    let entry = &entries(&info)[0];
    assert_eq!(entry["pinned"], Value::Bool(true));
    assert_eq!(first_address(entry)["ip"], text("127.0.0.1"));
}

#[test]
fn tf_26_connect_gate_refuses_redirect_target() {
    let landing = html_server(ARTICLE);
    let target = landing.url("/landing");
    let start = TestServer::start(move |_: &Request| Reply::redirect(302, &target));
    let gate = Arc::new(DenyRedirects::default());
    let guarded = Harness::with(Setup {
        gate: Some(Arc::clone(&gate) as Arc<dyn ConnectGate>),
        ..Setup::default()
    });
    let url = text(&start.url("/go"));
    let error = guarded.fail(&[("url", url.clone())]);
    assert_eq!(error.code(), ErrorCode::Network);
    assert_eq!(problem(&error), Some("denied"));
    assert!(error.data().message.contains("redirects are not allowed"));
    let seen = gate.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1], (1, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]));
    assert_eq!(landing.request_count(), 0);
    let open = Harness::new().one(&[("url", url)]);
    assert_eq!(open.status, 200);
    assert_eq!(landing.request_count(), 1);
}

#[test]
fn tf_27_configured_proxy_and_direct_connection() {
    let proxy = format!("http://127.0.0.1:{}", closed_port());
    let mut config = Config::default();
    config.http.proxy = Some(proxy.clone());
    let remote = text("http://example.test/");
    let through = check(&[("url", remote.clone())], &config).unwrap();
    let Value::Text(used) = &entries(&through)[0]["proxy"] else {
        panic!("no proxy in the network entry");
    };
    assert!(used.starts_with(&proxy), "{used}");
    let direct = check(&[("url", remote), ("proxy", text("none"))], &config).unwrap();
    assert_eq!(entries(&direct)[0]["proxy"], Value::Null);
    let server = html_server(ARTICLE);
    let harness = Harness::with(Setup {
        config,
        ..Setup::default()
    });
    let response = harness.one(&[("url", text(&server.url("/"))), ("proxy", text("none"))]);
    assert_eq!(response.status, 200);
}

#[test]
fn tf_28_address_without_scheme_uses_https() {
    let info = check(&[("url", text("example.test/a"))], &Config::default()).unwrap();
    let entry = &entries(&info)[0];
    assert_eq!(entry["url"], text("https://example.test/a"));
    assert_eq!(entry["scheme_assumed"], Value::Bool(true));
    assert_eq!(entry["url_input"], text("example.test/a"));
    let server = html_server(ARTICLE);
    let harness = Harness::new();
    let address = format!("127.0.0.1:{}/a", server.port());
    let result = harness.run(&[("url", text(&address)), ("timeout", millis(2000))]);
    assert!(result.is_err(), "a TLS request to a plain server succeeded");
    assert!(
        harness
            .events
            .logs()
            .iter()
            .any(|message| message.starts_with("scheme_assumed"))
    );
}

#[test]
fn tf_29_binary_body_is_saved_not_decoded() {
    let server = TestServer::start(|_: &Request| Reply::body(200, "image/png", PNG));
    let harness = Harness::new();
    let url = text(&server.url("/x.png"));
    let error = harness.fail(&[("url", url.clone()), ("as", text("text"))]);
    assert_eq!(error.code(), ErrorCode::Binary);
    let target = harness.state.path().join("x.png");
    // The language form of the path, as the response names it.
    let target_text = target.to_str().unwrap().replace('\\', "/");
    let mut tx = FileTx::default();
    let applied = harness
        .apply(
            &[
                ("url", url),
                ("as", text("raw")),
                ("save", Value::Path(target_text.clone())),
            ],
            &mut tx,
        )
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), PNG);
    assert_eq!(
        applied.responses[0].saved.as_ref().map(value::Path::as_str),
        Some(target_text.as_str())
    );
    assert_eq!(applied.changes.len(), 1);
    assert_eq!(applied.changes[0].action, ChangeAction::Create);
    assert_eq!(applied.changes[0].path.as_str(), target_text);
    assert_eq!(
        applied.changes[0].size_after,
        Some(Size::from_bytes(count_of(PNG.len())))
    );
    assert_eq!(tx.committed, vec![target]);
}

#[test]
fn an_error_answer_is_not_saved_over_the_file() {
    let server = TestServer::start(|_: &Request| Reply::body(404, "text/html", "not found page"));
    let harness = Harness::new();
    let target = harness.state.path().join("kept.txt");
    std::fs::write(&target, b"kept").unwrap();
    let mut tx = FileTx::default();
    let applied = harness
        .apply(
            &[
                ("url", text(&server.url("/missing"))),
                ("as", text("raw")),
                ("save", Value::Path(target.to_str().unwrap().to_owned())),
            ],
            &mut tx,
        )
        .unwrap();
    assert_eq!(applied.responses[0].status, 404);
    assert_eq!(applied.responses[0].saved, None);
    assert_eq!(std::fs::read(&target).unwrap(), b"kept");
    assert!(applied.changes.is_empty());
    assert!(tx.committed.is_empty());
}

#[test]
fn request_no_policy_private_address_allowed() {
    let server = html_server(ARTICLE);
    let url = text(&server.url("/private"));
    let response = Harness::new().one(&[("url", url.clone())]);
    assert_eq!(response.status, 200);
    let info = check(&[("url", url)], &Config::default()).unwrap();
    let entry = &entries(&info)[0];
    for key in ["allowed", "blocked", "denied", "policy"] {
        assert!(!entry.contains_key(key), "the entry has the key `{key}`");
    }
}

#[test]
fn request_check_network_entry_normalized() {
    let info = check(
        &[
            ("url", text("HTTP://Example.TEST:80/a?b=1")),
            ("headers", map(&[("Accept-Language", text("en"))])),
            ("json", map(&[("k", Value::Int(1))])),
        ],
        &Config::default(),
    )
    .unwrap();
    let entry = &entries(&info)[0];
    let expected = [
        ("command", text(COMMAND)),
        ("method", text("get")),
        ("url", text("http://example.test/a?b=1")),
        ("scheme", text("http")),
        ("scheme_assumed", Value::Bool(false)),
        ("host", text("example.test")),
        ("host_kind", text("name")),
        ("port", Value::Int(80)),
        ("dynamic", Value::Bool(false)),
        ("header_names", Value::List(vec![text("accept-language")])),
        ("body", text("json")),
        ("cache", text("bypass")),
        ("network_access", text("yes")),
        ("resolved", Value::Bool(false)),
    ];
    for (key, value) in expected {
        assert_eq!(entry[key], value, "field `{key}`");
    }
    assert_eq!(info.network.len(), 1);
    assert_eq!(info.network[0].host, "example.test");
    assert_eq!(info.network[0].port, Some(80));
}

#[test]
fn request_resolve_pins_after_redirect() {
    let server = TestServer::start(|request: &Request| match request.route() {
        "/start" => Reply::redirect(302, "/end"),
        _ => Reply::body(200, "text/plain", "end"),
    });
    let dns = Arc::new(MockDns::default());
    let harness = Harness::with(Setup {
        dns: Some(Arc::clone(&dns)),
        ..Setup::default()
    });
    let port = server.port();
    let authority = format!("pinned.test:{port}");
    let response = harness.one(&[
        ("url", text(&format!("http://pinned.test:{port}/start"))),
        (
            "resolve",
            map(&[(authority.as_str(), list(&["127.0.0.1".to_owned()]))]),
        ),
    ]);
    assert_eq!(response.status, 200);
    assert_eq!(response.final_url, format!("http://pinned.test:{port}/end"));
    assert_eq!(server.request_count(), 2);
    assert_eq!(dns.lookups(), 0);
}

#[test]
fn request_cache_only_no_socket() {
    let server = html_server(ARTICLE);
    let dns = MockDns::loopback(&["offline.test"]);
    let harness = Harness::with(Setup {
        dns: Some(Arc::clone(&dns)),
        ..Setup::default()
    });
    let values = [
        (
            "url",
            text(&format!("http://offline.test:{}/", server.port())),
        ),
        ("cache", text("only")),
    ];
    let error = harness.fail(&values);
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert_eq!(server.request_count(), 0);
    assert_eq!(dns.lookups(), 0);
    let info = check(&values, &Config::default()).unwrap();
    assert_eq!(entries(&info)[0]["network_access"], text("no"));
}

#[test]
fn request_fail_on_status() {
    let server = TestServer::start(|_: &Request| Reply::body(404, "text/plain", "missing"));
    let harness = Harness::new();
    let url = text(&server.url("/missing"));
    assert_eq!(harness.one(&[("url", url.clone())]).status, 404);
    let error = harness.fail(&[("url", url), ("fail_on_status", Value::Bool(true))]);
    assert_eq!(error.code(), ErrorCode::Network);
    assert_eq!(problem(&error), Some("status"));
    assert_eq!(error.data().actual, Some(serde_json::json!(404)));
}

#[test]
fn request_on_error_record_partial() {
    let server = html_server(ARTICLE);
    let harness = Harness::new();
    let first = format!("http://127.0.0.1:{}/", closed_port());
    let second = format!("http://127.0.0.1:{}/", closed_port());
    let partial = harness
        .run(&[
            ("url", list(&[first.clone(), server.url("/ok")])),
            ("on_error", text("record")),
        ])
        .unwrap();
    assert_eq!(partial.len(), 1);
    let error = harness.fail(&[
        ("url", list(&[first, second])),
        ("on_error", text("record")),
    ]);
    assert_eq!(error.code(), ErrorCode::Network);
}

#[test]
fn request_offset_total_chars() {
    let server = TestServer::start(|_: &Request| Reply::body(200, "text/plain", "0123456789"));
    let harness = Harness::new();
    let url = text(&server.url("/digits"));
    let rest = harness.one(&[("url", url.clone()), ("offset", Value::Int(4))]);
    assert_eq!(rest.body, "456789");
    assert_eq!(rest.offset, Some(4));
    assert_eq!(rest.total_chars, Some(10));
    let end = harness.one(&[("url", url.clone()), ("offset", Value::Int(10))]);
    assert!(end.body.is_empty());
    let error = harness.fail(&[("url", url), ("offset", Value::Int(11))]);
    assert_eq!(error.code(), ErrorCode::Arg);
}

#[test]
fn request_query_limit_bounds_param_conflict() {
    let config = Config::default();
    let url = ("url", text("https://example.test/"));
    let alone = check(&[url.clone(), ("query_limit", Value::Int(3))], &config).unwrap_err();
    assert_eq!(alone.code(), ErrorCode::Arg);
    assert_eq!(problem(&alone), Some("param_conflict"));
    let query = [
        url.clone(),
        ("as", text("text")),
        ("query", text("install")),
    ];
    for limit in [0, 21] {
        let mut values = query.to_vec();
        values.push(("query_limit", Value::Int(limit)));
        let error = check(&values, &config).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Arg);
        assert_eq!(
            error.data().expected,
            Some(serde_json::json!({"min": 1, "max": 20}))
        );
    }
    let raw = check(&[url, ("query", text("install"))], &config).unwrap_err();
    assert_eq!(problem(&raw), Some("param_conflict"));
}

#[test]
fn request_markdown_outline_auto_result_file() {
    let server = html_server(long_page());
    let harness = Harness::with(Setup {
        max_chars: Some(1000),
        ..Setup::default()
    });
    let url = text(&server.url("/manual"));
    let outline = harness.one(&[("url", url.clone()), ("as", text("markdown"))]);
    let path = outline
        .body
        .lines()
        .find_map(|line| line.strip_prefix("full content: "))
        .map(PathBuf::from)
        .unwrap();
    assert!(path.starts_with(harness.state.path().join(RESULTS_DIR)));
    assert_eq!(path.extension().and_then(|ext| ext.to_str()), Some("md"));
    let full = harness.one(&[
        ("url", url),
        ("as", text("markdown")),
        ("outline", text("never")),
    ]);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), full.body);
}

#[test]
fn request_tables_index_not_found() {
    let server = html_server(TABLES);
    let harness = Harness::new();
    let error = harness.fail(&[
        ("url", text(&server.url("/tables"))),
        ("extract", text("tables")),
        ("table", Value::Int(4)),
    ]);
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert_eq!(error.data().actual, Some(serde_json::json!(3)));
    let conflict = check(
        &[
            ("url", text(&server.url("/tables"))),
            ("table", Value::Int(1)),
        ],
        &Config::default(),
    )
    .unwrap_err();
    assert_eq!(problem(&conflict), Some("param_conflict"));
}

#[test]
fn request_save_via_mutation_tx() {
    let server = TestServer::start(|_: &Request| Reply::body(200, "text/plain", "new content"));
    let harness = Harness::new();
    let target = harness.state.path().join("page.txt");
    std::fs::write(&target, "old").unwrap();
    // The language form of the path, as the response names it.
    let target_text = target.to_str().unwrap().replace('\\', "/");
    let values = [
        ("url", text(&server.url("/page"))),
        ("save", Value::Path(target_text.clone())),
    ];
    let error = harness.run(&values).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Arg);
    let plan = harness.plan(&values).unwrap();
    assert_eq!(plan.changes.len(), 1);
    assert!(plan.irreversible.is_empty());
    let mut tx = FileTx::default();
    let applied = harness.apply(&values, &mut tx).unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new content");
    let change = &applied.changes[0];
    assert_eq!(change.action, ChangeAction::Modify);
    assert_eq!(change.size_before, Some(Size::from_bytes(3)));
    assert_eq!(change.size_after, Some(Size::from_bytes(11)));
    assert_eq!(applied.responses[0].bytes, Size::from_bytes(11));
    assert!(applied.irreversible.is_empty());
    assert_eq!(tx.committed, vec![target]);
}

#[test]
fn request_post_irreversible_network() {
    let server = TestServer::start(|request: &Request| {
        Reply::body(201, "application/json", request.body.clone())
    });
    let harness = Harness::new();
    let url = server.url("/items");
    let values = [
        ("url", text(&url)),
        ("method", text("post")),
        ("json", map(&[("name", text("demo"))])),
    ];
    let error = harness.run(&values).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Arg);
    let plan = harness.plan(&values).unwrap();
    assert_eq!(plan.irreversible.len(), 1);
    assert_eq!(plan.irreversible[0].reason, Irreversible::Network);
    assert_eq!(plan.irreversible[0].detail, Some(format!("post {url}")));
    let mut tx = FileTx::default();
    let applied = harness.apply(&values, &mut tx).unwrap();
    assert_eq!(applied.responses[0].status, 201);
    assert_eq!(applied.responses[0].body, "{\"name\":\"demo\"}");
    assert!(applied.changes.is_empty());
    assert_eq!(applied.irreversible.len(), 1);
    assert_eq!(tx.irreversible.len(), 1);
    let received = &server.requests()[0];
    assert_eq!(received.method, "POST");
    assert_eq!(received.header("content-type"), Some("application/json"));
}
