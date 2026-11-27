// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Privilege elevation through the engine: the choice of the backend, the
//! refusals, `auto`, `as_user`, the references to elevated journal entries
//! and the hidden execution of a request with the identity of the
//! administrator.

// Tests stop at the first unexpected value by unwrapping or panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::config::Config;
use nurt_contracts::envelope::{Language, Privilege, Request, RequestOptions, Response, Verbosity};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::report::{Report, ReportStatus};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, ReadStage, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::Value;
use nurt_core::config::{ConfigSources, toml_table};
use nurt_core::frontend::{Compiled, Corrected, Frontend};
use nurt_core::{Engine, EngineConfig};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use nurt_testkit::journal::RecordingJournal;
use nurt_testkit::vfs::VirtualFs;
use serde_json::json;
use support::{Find, Programs, Remove, find_stage, program};

/// Front end of the support module that also corrects text, so that the
/// engine can send a syntax tree to a backend.
struct Elevating(Programs);

fn tree() -> AstDocument {
    serde_json::from_value(json!({"nurt": "1.0", "program": {"statements": []}})).unwrap()
}

impl Frontend for Elevating {
    fn parse(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<AstDocument> {
        self.0.parse(text, variant, config)
    }

    fn correct(&self, _: &str, _: Option<SyntaxVariant>, _: &Config) -> Result<Corrected> {
        Ok(Corrected {
            ast: tree(),
            corrections: Vec::new(),
            ran: None,
        })
    }

    fn bind(&self, document: &AstDocument, config: &Config) -> Result<BoundDocument> {
        self.0.bind(document, config)
    }

    fn print(&self, document: &AstDocument, variant: SyntaxVariant) -> Result<String> {
        self.0.print(document, variant)
    }

    fn describe(&self, subject: &str, language: Language) -> Result<serde_json::Value> {
        self.0.describe(subject, language)
    }

    fn help(&self, topic: &str, language: Language) -> Result<String> {
        self.0.help(topic, language)
    }

    fn compile(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<Compiled> {
        self.0.compile(text, variant, config)
    }
}

/// `find` that the account may not run on `/denied`.
struct DeniedFind;

impl StageImpl for DeniedFind {
    fn descriptor(&self) -> StageDescriptor {
        Find.descriptor()
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for DeniedFind {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        if matches!(args.value("path"), Some(Value::Path(path)) if path.ends_with("/needs-root")) {
            return Err(Error::new(ErrorCode::Privilege, "the operation needs root"));
        }
        if matches!(args.value("path"), Some(Value::Path(path)) if path.ends_with("/denied")) {
            return Err(Error::new(ErrorCode::Permission, "access denied"));
        }
        Find.execute(args, input, ctx)
    }
}

fn deny_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "find", "effect": "read", "phase": "source",
        "args": [{"name": "path", "value": {"t": "path", "v": nurt_testkit::paths::abs("/denied")}}]
    })
}

/// `stage` with the call parameter `name` as the bound front end writes it.
fn with_call(mut stage: serde_json::Value, name: &str, value: &str) -> serde_json::Value {
    stage["args"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": name, "value": {"t": "text", "v": value}}));
    stage
}

fn root_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "find", "effect": "read", "phase": "source",
        "args": [{"name": "path", "value": {"t": "path", "v": nurt_testkit::paths::abs("/needs-root")}}]
    })
}

/// Two pipelines of one program that give different `privilege`.
fn two_pipelines() -> BoundDocument {
    serde_json::from_value(json!({
        "nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
        "program": {"statements": [
            {"t": "pipeline", "head": with_call(find_stage(), "privilege", "elevated"), "stages": []},
            {"t": "pipeline", "head": find_stage(), "stages": []}
        ]}
    }))
    .unwrap()
}

fn undo_stage(id: i64) -> serde_json::Value {
    json!({
        "t": "stage", "name": "undo", "effect": "local", "phase": "source",
        "args": [{"name": "id", "value": {"t": "int", "v": id}}]
    })
}

fn programs() -> Programs {
    Programs::default()
        .with("find /data", program(find_stage(), Vec::new()))
        .with("undo 7", program(undo_stage(7), Vec::new()))
        .with(
            "undo 1000000007",
            program(undo_stage(1_000_000_007), Vec::new()),
        )
        .with("two pipelines", two_pipelines())
        .with("deny", program(deny_stage(), Vec::new()))
        .with("needs-root", program(root_stage(), Vec::new()))
        .with(
            "find /data privilege: account",
            program(with_call(find_stage(), "privilege", "account"), Vec::new()),
        )
        .with(
            "find /data privilege: elevated",
            program(with_call(find_stage(), "privilege", "elevated"), Vec::new()),
        )
}

struct Setup {
    engine: Engine,
    state: PathBuf,
    _temp: tempfile::TempDir,
}

fn setup(
    host: TestHost,
    administrator: bool,
    search_path: Option<&Path>,
    overrides: &str,
) -> Setup {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let clock = Arc::new(VirtualClock::new());
    let vfs = Arc::new(VirtualFs::new(clock.clone()));
    let journal = Arc::new(RecordingJournal::keeping(vfs.clone()));
    let mut config = EngineConfig::new(clock, Arc::new(host))
        .with_sources(
            ConfigSources::isolated().with_overrides(toml_table(overrides, "test").unwrap()),
        )
        .with_state_dir(&state)
        .with_fs(vfs)
        .with_journal(journal)
        .with_frontend(Arc::new(Elevating(programs())))
        .with_stage(Arc::new(DeniedFind))
        .with_stage(Arc::new(Remove))
        .with_administrator(administrator)
        .with_elevation_executable("/opt/nurt/bin/nurt");
    config = config.with_elevation_search_path(
        search_path.map_or_else(std::ffi::OsString::new, |path| path.as_os_str().to_owned()),
    );
    Setup {
        engine: Engine::open(config).unwrap(),
        state,
        _temp: temp,
    }
}

fn options(privilege: Option<Privilege>, as_user: Option<&str>) -> RequestOptions {
    RequestOptions {
        privilege,
        as_user: as_user.map(str::to_owned),
        verbosity: Some(Verbosity::Detailed),
        ..RequestOptions::default()
    }
}

fn call(setup: &Setup, text: &str, options: RequestOptions) -> Response {
    let response = setup
        .engine
        .run(&Request::program(text).with_options(options));
    response.validate().unwrap();
    response
}

fn error_of(response: &Response) -> &Error {
    assert_eq!(response.report.status, ReportStatus::Failed, "{response:?}");
    response.report.error.as_ref().unwrap()
}

#[test]
fn account_runs_are_not_marked_elevated() {
    let setup = setup(TestHost::linux(), true, None, "");
    let response = call(&setup, "find /data", options(None, None));
    assert!(!response.report.elevated);
    assert!(response.report.backend.is_none());
}

#[test]
fn administrator_runs_an_elevated_call_in_process() {
    let setup = setup(TestHost::linux(), true, None, "");
    let response = call(
        &setup,
        "find /data",
        options(Some(Privilege::Elevated), None),
    );
    assert_eq!(response.report.status, ReportStatus::Ok);
    assert!(response.report.elevated);
    assert_eq!(response.report.backend.as_deref(), Some("self"));
    assert!(response.report.as_user.is_none());
}

#[test]
fn configured_default_applies_when_the_call_gives_no_privilege() {
    let setup = setup(
        TestHost::linux(),
        true,
        None,
        "[privilege]\ndefault = \"elevated\"\n",
    );
    let response = call(&setup, "find /data", options(None, None));
    assert!(response.report.elevated);
    let account = call(
        &setup,
        "find /data",
        options(Some(Privilege::Account), None),
    );
    assert!(!account.report.elevated);
}

#[test]
fn a_stage_reads_the_effective_privilege_from_the_context() {
    let setup = setup(
        TestHost::linux(),
        true,
        None,
        "[privilege]\ndefault = \"auto\"\n",
    );
    let ctx = setup
        .engine
        .context(&Request::program("find /data"))
        .unwrap();
    assert_eq!(ctx.options().privilege(), Privilege::Auto);
    let given = Request::program("find /data").with_options(options(None, Some("deploy")));
    let ctx = setup.engine.context(&given).unwrap();
    assert_eq!(ctx.options().effective_privilege(), Privilege::Auto);
    assert_eq!(ctx.options().as_user.as_deref(), Some("deploy"));
}

#[test]
fn configuration_can_forbid_elevation() {
    let setup = setup(
        TestHost::linux(),
        true,
        None,
        "[privilege]\nallowed = false\n",
    );
    for privilege in [Privilege::Elevated, Privilege::Auto] {
        let response = call(&setup, "find /data", options(Some(privilege), None));
        let error = error_of(&response);
        assert_eq!(error.code(), ErrorCode::Privilege);
        assert_eq!(
            error.details.get("reason"),
            Some(&json!("disabled_by_configuration"))
        );
        assert!(!error.next_steps.is_empty());
    }
    let account = call(&setup, "find /data", options(None, None));
    assert_eq!(account.report.status, ReportStatus::Ok);
}

#[test]
fn without_a_backend_the_error_lists_every_refusal() {
    let setup = setup(TestHost::linux(), false, None, "");
    let response = call(
        &setup,
        "find /data",
        options(Some(Privilege::Elevated), None),
    );
    let error = error_of(&response);
    assert_eq!(error.code(), ErrorCode::Privilege);
    assert_eq!(
        error.details.get("backend"),
        Some(&json!([
            {"backend": "self", "result": "not_administrator"},
            {"backend": "broker", "result": "not_installed"},
            {"backend": "sudo", "result": "not_installed"},
            {"backend": "doas", "result": "not_installed"},
            {"backend": "run0", "result": "not_installed"},
        ]))
    );
    assert!(!error.next_steps.is_empty());
}

#[test]
fn configured_backends_limit_the_attempts() {
    let setup = setup(
        TestHost::linux(),
        false,
        None,
        "[privilege]\nbackends = [\"sudo\"]\n",
    );
    let response = call(
        &setup,
        "find /data",
        options(Some(Privilege::Elevated), None),
    );
    assert_eq!(
        error_of(&response).details.get("backend"),
        Some(&json!([{"backend": "sudo", "result": "not_installed"}]))
    );
}

#[test]
fn windows_has_no_as_user() {
    let setup = setup(TestHost::windows(), true, None, "");
    let response = call(&setup, "find /data", options(None, Some("deploy")));
    let error = error_of(&response);
    assert_eq!(error.code(), ErrorCode::Unsupported);
    assert!(
        error
            .next_steps
            .iter()
            .any(|step| step.contains("privilege: elevated"))
    );
}

#[test]
fn an_elevated_windows_console_runs_in_process_with_the_self_backend() {
    let setup = setup(TestHost::windows(), true, None, "");
    let response = call(
        &setup,
        "find /data",
        options(Some(Privilege::Elevated), None),
    );
    assert_eq!(response.report.status, ReportStatus::Ok);
    assert!(response.report.elevated);
    assert_eq!(response.report.backend.as_deref(), Some("self"));
}

#[test]
fn a_windows_console_without_elevation_lists_self_and_broker() {
    let setup = setup(TestHost::windows(), false, None, "");
    let response = call(
        &setup,
        "find /data",
        options(Some(Privilege::Elevated), None),
    );
    let error = error_of(&response);
    assert_eq!(error.code(), ErrorCode::Privilege);
    assert_eq!(
        error.details.get("backend"),
        Some(&json!([
            {"backend": "self", "result": "not_administrator"},
            {"backend": "broker", "result": "not_installed"},
        ]))
    );
}

#[test]
fn elevated_exec_refuses_a_process_that_is_not_the_administrator() {
    let setup = setup(TestHost::linux(), false, None, "");
    let request =
        Request::program("find /data").with_options(options(Some(Privilege::Elevated), None));
    let response = setup.engine.elevated_exec(&request);
    assert_eq!(error_of(&response).code(), ErrorCode::Privilege);
}

#[test]
fn elevated_exec_runs_in_the_process_and_marks_the_report() {
    let setup = setup(TestHost::linux(), true, None, "");
    let request =
        Request::program("find /data").with_options(options(Some(Privilege::Elevated), None));
    let response = setup.engine.elevated_exec(&request);
    assert_eq!(response.report.status, ReportStatus::Ok);
    assert!(response.report.elevated);
    assert_eq!(response.report.backend.as_deref(), Some("self"));
}

#[test]
fn stage_call_parameters_override_the_options_and_the_default() {
    let setup = setup(
        TestHost::linux(),
        true,
        None,
        "[privilege]\ndefault = \"elevated\"\n",
    );
    let response = call(&setup, "find /data privilege: account", options(None, None));
    assert_eq!(response.report.status, ReportStatus::Ok, "{response:?}");
    assert!(!response.report.elevated);
    let elevated = call(
        &setup,
        "find /data privilege: elevated",
        options(Some(Privilege::Account), None),
    );
    assert!(elevated.report.elevated, "{elevated:?}");
    assert_eq!(elevated.report.backend.as_deref(), Some("self"));
}

#[test]
fn undo_runs_the_kept_inverse_programs_once() {
    let setup = setup(TestHost::linux(), false, None, "");
    let line = json!({
        "op": 7,
        "kind": "start",
        "programs": ["find /data"],
        "undone": false
    });
    std::fs::write(
        setup.state.join("inverse-operations.jsonl"),
        format!("{line}\n"),
    )
    .unwrap();
    let first = call(&setup, "undo 7", options(None, None));
    assert_eq!(first.report.status, ReportStatus::Ok, "{first:?}");
    assert_eq!(first.report.kind, "find");
    let second = call(&setup, "undo 7", options(None, None));
    assert_ne!(second.report.kind, "find", "the entry is undone once");
    let kept = std::fs::read_to_string(setup.state.join("inverse-operations.jsonl")).unwrap();
    assert!(kept.contains("\"undone\":true"), "{kept}");
}

#[test]
fn a_preview_of_undo_keeps_the_inverse_programs() {
    let setup = setup(TestHost::linux(), false, None, "");
    let line = json!({"op": 7, "kind": "start", "programs": ["find /data"], "undone": false});
    std::fs::write(
        setup.state.join("inverse-operations.jsonl"),
        format!("{line}\n"),
    )
    .unwrap();
    let preview = RequestOptions {
        preview: Some(true),
        ..options(None, None)
    };
    let _ = call(&setup, "undo 7", preview);
    let kept = std::fs::read_to_string(setup.state.join("inverse-operations.jsonl")).unwrap();
    assert!(!kept.contains("\"undone\":true"), "{kept}");
}

#[test]
fn system_journal_directory_follows_the_host() {
    let linux = setup(TestHost::linux(), true, None, "");
    assert_eq!(
        linux.engine.system_state_dir(),
        PathBuf::from("/var/lib/nurt")
    );
    let macos = setup(TestHost::macos(), true, None, "");
    assert_eq!(
        macos.engine.system_state_dir(),
        PathBuf::from("/Library/Application Support/Nurt")
    );
}

#[test]
fn auto_without_a_backend_reports_the_refusal_and_the_denial() {
    let setup = setup(TestHost::linux(), false, None, "");
    let response = call(&setup, "deny", options(Some(Privilege::Auto), None));
    let error = error_of(&response);
    assert_eq!(error.code(), ErrorCode::Privilege);
    assert_eq!(
        error.cause.as_ref().map(Error::code),
        Some(ErrorCode::Permission)
    );
}

#[test]
fn a_privilege_error_of_a_stage_names_elevation_and_the_backends() {
    let setup = setup(TestHost::linux(), false, None, "");
    let response = call(&setup, "needs-root", options(None, None));
    let error = error_of(&response);
    assert_eq!(error.code(), ErrorCode::Privilege);
    assert!(
        error
            .next_steps
            .iter()
            .any(|step| step.contains("privilege: elevated"))
    );
    let listed = error.details.get("backend").unwrap().as_array().unwrap();
    assert!(listed.contains(&json!({"backend": "self", "result": "not_administrator"})));
    assert!(listed.contains(&json!({"backend": "sudo", "result": "not_installed"})));
}

#[test]
fn different_rights_on_two_pipelines_are_refused() {
    let setup = setup(TestHost::linux(), true, None, "");
    let mut with_script = options(None, None);
    with_script.script = Some(true);
    let response = call(&setup, "two pipelines", with_script);
    let error = error_of(&response);
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert!(!error.next_steps.is_empty());
}

#[test]
fn account_privilege_keeps_the_denial() {
    let setup = setup(TestHost::linux(), false, None, "");
    let response = call(&setup, "deny", options(None, None));
    assert_eq!(error_of(&response).code(), ErrorCode::Permission);
}

#[test]
fn auto_does_not_repeat_a_stage_that_did_not_fail_by_denial() {
    let setup = setup(TestHost::linux(), false, None, "");
    let response = call(&setup, "find /data", options(Some(Privilege::Auto), None));
    assert_eq!(response.report.status, ReportStatus::Ok);
    assert!(!response.report.elevated);
}

#[cfg(unix)]
mod fake_programs {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A directory with a program `name` that records its arguments and its
    /// standard input and then runs `body`.
    struct Fake {
        dir: tempfile::TempDir,
    }

    impl Fake {
        fn new(name: &str, body: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let program = dir.path().join(name);
            let script = format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{dir}/argv'\ncat > '{dir}/request.json'\n{body}\n",
                dir = dir.path().display()
            );
            std::fs::write(&program, script).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self { dir }
        }

        fn answering(name: &str, response: &Response) -> Self {
            let fake = Self::new(name, "cat \"$0.response\"");
            let path = fake.dir.path().join(format!("{name}.response"));
            std::fs::write(path, serde_json::to_vec(response).unwrap()).unwrap();
            fake
        }

        fn argv(&self) -> String {
            std::fs::read_to_string(self.dir.path().join("argv"))
                .unwrap()
                .trim()
                .to_owned()
        }

        fn request(&self) -> serde_json::Value {
            let bytes = std::fs::read(self.dir.path().join("request.json")).unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }
    }

    fn canned(op: Option<u64>) -> Response {
        let mut report = Report::new(ReportStatus::Ok, "write");
        report.op = op;
        Response::new(report)
    }

    #[test]
    fn sudo_receives_the_syntax_tree_on_standard_input() {
        let fake = Fake::answering("sudo", &canned(None));
        let setup = setup(TestHost::linux(), false, Some(fake.dir.path()), "");
        let response = call(
            &setup,
            "find /data",
            options(Some(Privilege::Elevated), None),
        );
        assert_eq!(response.report.status, ReportStatus::Ok);
        assert!(response.report.elevated);
        assert_eq!(response.report.backend.as_deref(), Some("sudo"));
        assert_eq!(fake.argv(), "-n -- /opt/nurt/bin/nurt elevated-exec");
        let request = fake.request();
        assert!(request.get("program").is_none(), "{request}");
        assert!(request.get("ast").is_some(), "{request}");
        assert_eq!(request["options"]["privilege"], json!("elevated"));
    }

    #[test]
    fn as_user_switches_the_account_and_implies_elevation() {
        let fake = Fake::answering("sudo", &canned(None));
        let setup = setup(TestHost::linux(), false, Some(fake.dir.path()), "");
        let response = call(&setup, "find /data", options(None, Some("deploy")));
        assert!(response.report.elevated);
        assert_eq!(response.report.as_user.as_deref(), Some("deploy"));
        assert_eq!(
            fake.argv(),
            "-n -u deploy -- /opt/nurt/bin/nurt elevated-exec"
        );
        assert_eq!(fake.request()["options"]["as_user"], json!("deploy"));
    }

    #[test]
    fn as_user_skips_the_self_backend_even_for_the_administrator() {
        let fake = Fake::answering("sudo", &canned(None));
        let setup = setup(TestHost::linux(), true, Some(fake.dir.path()), "");
        let response = call(&setup, "find /data", options(None, Some("deploy")));
        assert_eq!(response.report.backend.as_deref(), Some("sudo"));
    }

    #[test]
    fn a_password_request_is_reported_as_password_required() {
        let fake = Fake::new("sudo", "echo 'sudo: a password is required' >&2\nexit 1");
        let setup = setup(TestHost::linux(), false, Some(fake.dir.path()), "");
        let response = call(
            &setup,
            "find /data",
            options(Some(Privilege::Elevated), None),
        );
        let error = error_of(&response);
        assert_eq!(error.code(), ErrorCode::Privilege);
        let listed = error.details.get("backend").unwrap();
        assert!(
            listed
                .as_array()
                .unwrap()
                .contains(&json!({"backend": "sudo", "result": "password_required"})),
            "{listed}"
        );
        assert!(
            error
                .next_steps
                .iter()
                .any(|step| step.contains("NOPASSWD"))
        );
    }

    #[test]
    fn the_next_backend_is_tried_after_a_refusal() {
        let refusing = Fake::new("sudo", "echo 'sudo: a password is required' >&2\nexit 1");
        let answering = Fake::answering("doas", &canned(None));
        let path = std::env::join_paths([refusing.dir.path(), answering.dir.path()]).unwrap();
        let setup = setup(TestHost::linux(), false, Some(Path::new(&path)), "");
        let response = call(
            &setup,
            "find /data",
            options(Some(Privilege::Elevated), None),
        );
        assert_eq!(response.report.backend.as_deref(), Some("doas"));
    }

    #[test]
    fn auto_repeats_a_denied_stage_once_with_elevated_rights() {
        let fake = Fake::answering("sudo", &canned(None));
        let setup = setup(TestHost::linux(), false, Some(fake.dir.path()), "");
        let response = call(&setup, "deny", options(Some(Privilege::Auto), None));
        assert_eq!(response.report.status, ReportStatus::Ok);
        assert!(response.report.elevated);
        assert_eq!(
            response.report.elevation_reason.as_deref(),
            Some("permission_denied")
        );
        assert_eq!(fake.request()["options"]["privilege"], json!("elevated"));
    }

    #[test]
    fn elevated_entries_are_remembered_and_undone_through_the_backend() {
        let fake = Fake::answering("sudo", &canned(Some(7)));
        let setup = setup(TestHost::linux(), false, Some(fake.dir.path()), "");
        let response = call(
            &setup,
            "find /data",
            options(Some(Privilege::Elevated), None),
        );
        assert_eq!(response.report.op, Some(1_000_000_007));
        let history = setup
            .engine
            .history(&nurt_contracts::journal::HistoryQuery::default())
            .unwrap();
        assert!(
            history.iter().any(|record| record.id == 1_000_000_007),
            "{history:?}"
        );
        let references = setup.engine.elevated_references();
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].op, 7);
        assert_eq!(references[0].backend, "sudo");
        assert_eq!(references[0].journal, "/var/lib/nurt");
        assert!(setup.state.join("elevated-operations.jsonl").is_file());

        let own = call(&setup, "undo 7", options(None, None));
        assert!(
            !own.report.elevated,
            "the number 7 is an entry of the account"
        );
        let undone = call(&setup, "undo 1000000007", options(None, None));
        assert!(undone.report.elevated);
        assert_eq!(
            undone.report.elevation_reason.as_deref(),
            Some("system_journal_entry")
        );
        assert_eq!(fake.request()["options"]["privilege"], json!("elevated"));
    }

    #[test]
    fn undo_of_an_unknown_entry_stays_with_the_account() {
        let fake = Fake::answering("sudo", &canned(None));
        let setup = setup(TestHost::linux(), false, Some(fake.dir.path()), "");
        let response = call(&setup, "undo 7", options(None, None));
        assert!(!response.report.elevated);
    }
}
