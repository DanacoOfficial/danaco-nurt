// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Checks of the test runner with the `git` program: the corpus, the
//! normalizer, the HTTP and SSH test servers, the output comparison and the
//! runner with stage implementations that follow or break the journal
//! protocol.
//!
//! Every test returns without checks when no supported `git` program is on
//! the search path; the scenario test binary reports that case.

#[cfg(test)]
mod checks {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use nurt_contracts::error::Result;
    use nurt_contracts::fs::AbsPath;
    use nurt_contracts::journal::{
        Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec,
    };
    use nurt_contracts::stage::{
        BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
        RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
    };
    use nurt_contracts::value::{Record, RecordKind, Value};
    use nurt_git_difftest::compare::compare;
    use nurt_git_difftest::corpus::{CORPUS, Corpus, REPO_DIR};
    use nurt_git_difftest::git::{Availability, GitEnv, GitProgram};
    use nurt_git_difftest::normalize::Normalizer;
    use nurt_git_difftest::runner::{Outcome, RunOptions, Runner, Status};
    use nurt_git_difftest::scenario::{Platform, Scenario, Section, StateSelector};
    use nurt_git_difftest::server::HttpServer;
    use nurt_git_difftest::ssh::SshServer;
    use nurt_git_difftest::stage::CommandTable;
    use nurt_git_difftest::time::ScenarioClock;
    use russh::keys::PrivateKey;
    use russh::keys::ssh_key::private::Ed25519Keypair;

    const BRANCH_CREATE: &str = r#"description = "Create branch topic at HEAD."
corpus = "linear-small"
[git]
args = ["branch", "topic"]
[nurt]
command = "git.branch"
params = { create = "topic" }
"#;

    const REVIEW: &str = r#"description = "Review reports the missing final newline of notes.txt."
corpus = "linear-small"
files = { "notes.txt" = "no final newline" }
[git]
args = ["status", "--short"]
[nurt]
command = "git.review"
record_count = 1
[[nurt.records]]
kind = "finding"
check = "final_newline"
path = "$ROOT/repo/notes.txt"
"#;

    const HTTP_FETCH: &str = r#"description = "Fetch from a bare clone served over smart HTTP."
corpus = "linear-small"
transport = "http"
setup = [
  { git = ["clone", "-q", "--bare", "repo", "upstream.git"], cwd = ".." },
  ["remote", "add", "origin", "$SERVER/upstream.git"],
]
[git]
args = ["fetch", "-q", "origin"]
[nurt]
command = "git.fetch"
"#;

    /// Reflog message of `git branch <name>` on branch `main`.
    const CREATED_FROM_MAIN: &str = "branch: Created from main";

    fn git() -> Option<GitProgram> {
        match GitProgram::locate() {
            Availability::Ready(git) => Some(git),
            Availability::Missing | Availability::TooOld(_) => None,
        }
    }

    fn home(dir: &Path) -> PathBuf {
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".gitconfig"), b"").unwrap();
        home
    }

    fn scenario(id: &str, body: &str) -> Scenario {
        let text = format!("id = \"{id}\"\n{body}");
        Scenario::parse(&text, Path::new(&format!("{id}.toml")), id).unwrap()
    }

    fn run(
        stages: Vec<Arc<dyn StageImpl>>,
        options: RunOptions,
        scenario: &Scenario,
    ) -> Option<Outcome> {
        let git = git()?;
        let mut table = CommandTable::new();
        for stage in stages {
            table.insert(stage);
        }
        let runner = Runner::new(git, table, options).unwrap();
        Some(runner.run(scenario, 0))
    }

    fn failures(outcome: &Outcome) -> String {
        match &outcome.status {
            Status::Failed(findings) => findings.join("\n"),
            other => panic!("{} did not fail: {other:?}", outcome.id),
        }
    }

    fn repo_dir(args: &BoundArgs) -> PathBuf {
        match args.value("repo") {
            Some(Value::Path(path)) => PathBuf::from(path),
            Some(Value::List(items)) => match items.first() {
                Some(Value::Path(path)) => PathBuf::from(path),
                other => panic!("repo is {other:?}"),
            },
            other => panic!("repo is {other:?}"),
        }
    }

    fn head_commit(git_dir: &Path) -> String {
        let head = std::fs::read_to_string(git_dir.join("HEAD")).unwrap();
        let reference = head.trim().strip_prefix("ref: ").unwrap();
        std::fs::read_to_string(git_dir.join(reference))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn created(path: &Path) -> TargetSpec {
        TargetSpec {
            path: TargetPath::Fs(AbsPath::new(path).unwrap()),
            action: TargetAction::Create,
            expect: Expect::Absent,
            size_hint: None,
        }
    }

    /// Creates the branch named by `create` at `HEAD` with the reflog message
    /// `message`, through the transaction or, when `journaled` is false, by
    /// direct writes.
    struct CreateBranch {
        message: &'static str,
        journaled: bool,
    }

    impl StageImpl for CreateBranch {
        fn descriptor(&self) -> StageDescriptor {
            StageDescriptor {
                command: "git.branch".into(),
                source: true,
                input: InputKinds::None,
            }
        }

        fn as_effect(&self) -> Option<&dyn EffectStage> {
            Some(self)
        }
    }

    impl EffectStage for CreateBranch {
        fn plan<'a>(
            &'a self,
            _args: &'a BoundArgs,
            _input: StageInput<'a>,
            _ctx: &'a ExecCtx,
        ) -> Result<EffectPlan> {
            Ok(EffectPlan::new())
        }

        fn apply<'a>(
            &'a self,
            args: &'a BoundArgs,
            _plan: EffectPlan,
            tx: &mut dyn MutationTx,
            _ctx: &'a ExecCtx,
        ) -> Result<EffectOutcome<'a>> {
            let Some(Value::Text(name)) = args.value("create") else {
                panic!("create is not given");
            };
            let git_dir = repo_dir(args).join(".git");
            let head = head_commit(&git_dir);
            let reference = git_dir.join("refs/heads").join(name);
            let log = git_dir.join("logs/refs/heads").join(name);
            let zero = "0".repeat(head.len());
            let line = format!(
                "{zero} {head} Nurt Test <test@nurt.invalid> 0 +0000\t{}\n",
                self.message
            );
            if self.journaled {
                tx.preserve(&[created(&reference), created(&log)])?;
                tx.apply(
                    TargetIndex(0),
                    Mutation::WriteBytes(format!("{head}\n").into_bytes()),
                )?;
                tx.apply(TargetIndex(1), Mutation::WriteBytes(line.into_bytes()))?;
            } else {
                std::fs::write(&reference, format!("{head}\n")).unwrap();
                std::fs::write(&log, line).unwrap();
            }
            Ok(EffectOutcome::new(StageOutput::none()))
        }
    }

    /// Reports one missing final newline in `notes.txt`.
    struct Review;

    impl StageImpl for Review {
        fn descriptor(&self) -> StageDescriptor {
            StageDescriptor {
                command: "git.review".into(),
                source: true,
                input: InputKinds::None,
            }
        }

        fn as_read(&self) -> Option<&dyn ReadStage> {
            Some(self)
        }
    }

    impl ReadStage for Review {
        fn execute<'a>(
            &'a self,
            args: &'a BoundArgs,
            _input: StageInput<'a>,
            _ctx: &'a ExecCtx,
        ) -> Result<StageOutput<'a>> {
            let path = repo_dir(args).join("notes.txt");
            let record = Record::new(RecordKind::Finding)
                .with("check", Value::from("final_newline"))
                .with("path", Value::Text(path.to_string_lossy().into_owned()));
            Ok(StageOutput::records(RecordStream::new(std::iter::once(
                Ok(record),
            ))))
        }
    }

    #[test]
    fn a_supported_git_program_is_present_when_the_run_requires_one() {
        let required = std::env::var("NURT_DIFFTEST_REQUIRE_GIT").is_ok_and(|value| value == "1");
        if required {
            assert!(
                git().is_some(),
                "NURT_DIFFTEST_REQUIRE_GIT=1 but no supported git program is on the search path"
            );
        }
    }

    #[test]
    fn the_child_environment_does_not_leak_the_account_of_the_test_process() {
        let Some(git) = git() else {
            return;
        };
        let work = tempfile::tempdir().unwrap();
        let env = GitEnv::new(&home(work.path()), ScenarioClock::default());
        let output = git
            .run_ok(&env, work.path(), &["var", "GIT_COMMITTER_IDENT"])
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output).trim(),
            "Nurt Test <test@nurt.invalid> 1767268800 +0000"
        );
        let configured = git
            .run(&env, work.path(), &["config", "--global", "--list"])
            .unwrap();
        assert_eq!(configured.code, 0, "{}", configured.stderr_text());
        assert!(configured.stdout.is_empty());
        let system = git
            .run(&env, work.path(), &["config", "--system", "--list"])
            .unwrap();
        assert!(system.stdout.is_empty());
    }

    #[test]
    fn corpus_repositories_build_identically_twice_and_pass_fsck() {
        let Some(git) = git() else {
            return;
        };
        let work = tempfile::tempdir().unwrap();
        let first = Corpus::new(git.clone(), work.path().join("first"));
        let second = Corpus::new(git.clone(), work.path().join("second"));
        let env = GitEnv::new(&home(work.path()), ScenarioClock::default());
        let platform = Platform::current();
        for repo in CORPUS
            .iter()
            .filter(|repo| !repo.heavy && repo.platforms.contains(&platform))
        {
            let one = first.build(repo.name).unwrap().join(REPO_DIR);
            let two = second.build(repo.name).unwrap().join(REPO_DIR);
            let refs = |dir: &Path| {
                git.run_ok(
                    &env,
                    dir,
                    &["for-each-ref", "--format=%(refname) %(objectname)"],
                )
                .unwrap()
            };
            assert_eq!(refs(&one), refs(&two), "{}", repo.name);
            let fsck = git
                .run(&env, &one, &["fsck", "--strict", "--no-dangling"])
                .unwrap();
            assert_eq!(fsck.code, 0, "{}: {}", repo.name, fsck.stderr_text());
        }
    }

    #[test]
    fn copies_of_a_corpus_repository_have_equal_normalized_states() {
        let Some(git) = git() else {
            return;
        };
        let work = tempfile::tempdir().unwrap();
        let corpus = Corpus::new(git.clone(), work.path().join("corpus"));
        let env = GitEnv::new(&home(work.path()), ScenarioClock::default());
        let selectors: Vec<StateSelector> = Section::STANDARD
            .into_iter()
            .chain([Section::Fsck])
            .map(StateSelector::main)
            .collect();
        let normalizer = Normalizer::new(&git, &env, false);
        for name in [
            "branchy-small",
            "dirty-worktree",
            "merge-in-progress",
            "stash-present",
            "sparse",
        ] {
            let left = work.path().join(format!("{name}-left"));
            let right = work.path().join(format!("{name}-right"));
            corpus.materialize(name, &left).unwrap();
            corpus.materialize(name, &right).unwrap();
            let baseline = normalizer
                .baseline(&left.join(REPO_DIR), &selectors)
                .unwrap();
            let a = normalizer
                .snapshot(&left, &left.join(REPO_DIR), &selectors, &baseline)
                .unwrap();
            let b = normalizer
                .snapshot(&right, &right.join(REPO_DIR), &selectors, &baseline)
                .unwrap();
            let differences = compare(&a, &b, &selectors, &[]);
            assert!(differences.is_empty(), "{name}: {differences:?}");
            let fsck = a.section(&StateSelector::main(Section::Fsck)).unwrap();
            assert_eq!(fsck["result"], "ok", "{name}");
        }
    }

    #[test]
    fn http_server_serves_fetch_and_push() {
        let Some(git) = git() else {
            return;
        };
        let work = tempfile::tempdir().unwrap();
        let env = GitEnv::new(&home(work.path()), ScenarioClock::default());
        let source = work.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        let commit = |name: &str| {
            std::fs::write(source.join(name), name.as_bytes()).unwrap();
            git.run_ok(&env, &source, &["add", name]).unwrap();
            git.run_ok(&env, &source, &["commit", "-q", "-m", name])
                .unwrap();
        };
        git.run_ok(&env, &source, &["init", "-q", "-b", "main"])
            .unwrap();
        commit("a.txt");
        git.run_ok(
            &env,
            work.path(),
            &["clone", "-q", "--bare", "source", "served.git"],
        )
        .unwrap();
        let server = HttpServer::start(&git, &env, work.path()).unwrap();
        let url = format!("{}/served.git", server.base_url());
        let listing = git.run_ok(&env, &source, &["ls-remote", &url]).unwrap();
        assert!(String::from_utf8_lossy(&listing).contains("refs/heads/main"));
        commit("b.txt");
        git.run_ok(&env, &source, &["push", "-q", &url, "main"])
            .unwrap();
        let pushed = git
            .run_ok(
                &env,
                &work.path().join("served.git"),
                &["rev-parse", "main"],
            )
            .unwrap();
        assert_eq!(
            pushed,
            git.run_ok(&env, &source, &["rev-parse", "main"]).unwrap()
        );
        let missing = format!("{}/missing.git", server.base_url());
        assert_ne!(
            git.run(&env, &source, &["ls-remote", &missing])
                .unwrap()
                .code,
            0
        );
    }

    #[test]
    fn journaled_branch_creation_matches_git_and_is_undone() {
        let stage = CreateBranch {
            message: CREATED_FROM_MAIN,
            journaled: true,
        };
        let scenario = scenario("common/branch-create", BRANCH_CREATE);
        let Some(outcome) = run(vec![Arc::new(stage)], RunOptions::default(), &scenario) else {
            return;
        };
        assert_eq!(outcome.status, Status::Passed, "{outcome:?}");
    }

    #[test]
    fn differences_name_the_section_key_and_both_values() {
        let stage = CreateBranch {
            message: "branch: created",
            journaled: true,
        };
        let scenario = scenario("common/branch-create", BRANCH_CREATE);
        let Some(outcome) = run(vec![Arc::new(stage)], RunOptions::default(), &scenario) else {
            return;
        };
        let text = failures(&outcome);
        assert!(text.contains("states differ after the operation"), "{text}");
        assert!(text.contains("[reflog] logs/refs/heads/topic"), "{text}");
        assert!(text.contains(CREATED_FROM_MAIN), "{text}");
    }

    #[test]
    fn writes_outside_the_transaction_fail_the_scenario() {
        let stage = CreateBranch {
            message: CREATED_FROM_MAIN,
            journaled: false,
        };
        let scenario = scenario("common/branch-create", BRANCH_CREATE);
        let Some(outcome) = run(vec![Arc::new(stage)], RunOptions::default(), &scenario) else {
            return;
        };
        let text = failures(&outcome);
        assert!(
            text.contains("write outside the transaction: repo/.git/refs/heads/topic"),
            "{text}"
        );
    }

    #[test]
    fn commands_without_implementation_run_the_git_side_only() {
        let scenario = scenario("common/branch-create", BRANCH_CREATE);
        let Some(outcome) = run(Vec::new(), RunOptions::default(), &scenario) else {
            return;
        };
        assert_eq!(outcome.status, Status::GitOnly);
        let required = RunOptions {
            require_nurt: true,
            ..RunOptions::default()
        };
        let outcome = run(Vec::new(), required, &scenario).unwrap();
        assert!(failures(&outcome).contains("no implementation"));
    }

    #[test]
    fn a_failing_git_side_fails_the_scenario() {
        let body = BRANCH_CREATE.replace("[\"branch\", \"topic\"]", "[\"branch\", \"bad..name\"]");
        let scenario = scenario("common/branch-invalid", &body);
        let Some(outcome) = run(Vec::new(), RunOptions::default(), &scenario) else {
            return;
        };
        assert!(failures(&outcome).contains("git side"));
    }

    #[test]
    fn expected_records_of_a_read_command_are_checked() {
        let passing = scenario("common/review", REVIEW);
        let Some(outcome) = run(vec![Arc::new(Review)], RunOptions::default(), &passing) else {
            return;
        };
        assert_eq!(outcome.status, Status::Passed, "{outcome:?}");
        let failing = scenario(
            "common/review",
            &REVIEW.replace("repo/notes.txt", "repo/other.txt"),
        );
        let outcome = run(vec![Arc::new(Review)], RunOptions::default(), &failing).unwrap();
        assert!(failures(&outcome).contains("no record matching finding"));
    }

    #[test]
    fn http_scenarios_reach_the_server_of_their_side() {
        let scenario = scenario("common/http-fetch", HTTP_FETCH);
        let Some(outcome) = run(Vec::new(), RunOptions::default(), &scenario) else {
            return;
        };
        assert_eq!(outcome.status, Status::GitOnly, "{outcome:?}");
    }

    const SSH_FETCH: &str = r#"description = "Fetch from a bare clone served over SSH."
corpus = "linear-small"
transport = "ssh"
setup = [
  { git = ["clone", "-q", "--bare", "repo", "upstream.git"], cwd = ".." },
  ["remote", "add", "origin", "$SERVER/upstream.git"],
  ["ls-remote", "origin"],
]
[git]
args = ["fetch", "-q", "origin"]
[nurt]
command = "git.fetch"
"#;

    const STATUS_OUTPUT: &str = r#"description = "Status lists the untracked notes.txt."
corpus = "linear-small"
files = { "notes.txt" = "notes\n" }
[git]
args = ["status"]
[nurt]
command = "git.status"
[compare]
output = "status"
"#;

    /// Reports `path` as the only untracked path.
    struct StatusMock {
        path: &'static str,
    }

    impl StageImpl for StatusMock {
        fn descriptor(&self) -> StageDescriptor {
            StageDescriptor {
                command: "git.status".into(),
                source: true,
                input: InputKinds::None,
            }
        }

        fn as_read(&self) -> Option<&dyn ReadStage> {
            Some(self)
        }
    }

    impl ReadStage for StatusMock {
        fn execute<'a>(
            &'a self,
            _args: &'a BoundArgs,
            _input: StageInput<'a>,
            _ctx: &'a ExecCtx,
        ) -> Result<StageOutput<'a>> {
            let record = Record::new(RecordKind::GitStatus)
                .with("path", Value::from(self.path))
                .with("kind", Value::from("untracked"));
            Ok(StageOutput::records(RecordStream::new(std::iter::once(
                Ok(record),
            ))))
        }
    }

    struct TrustingClient;

    impl russh::client::Handler for TrustingClient {
        type Error = russh::Error;

        async fn check_server_key(
            &mut self,
            _key: &russh::keys::PublicKeyOrCertificate,
        ) -> std::result::Result<bool, Self::Error> {
            Ok(true)
        }
    }

    /// Exit status and standard output of `command` run over SSH with
    /// `key`; `None` when the server refuses the key. A flush packet ends the
    /// request once the server has sent one.
    fn ssh_exec(port: u16, key: PrivateKey, command: &str) -> Option<(Option<u32>, Vec<u8>)> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let config = Arc::new(russh::client::Config::default());
            let mut session = russh::client::connect(config, ("127.0.0.1", port), TrustingClient)
                .await
                .unwrap();
            let key = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), None);
            let auth = session.authenticate_publickey("nurt", key).await.unwrap();
            if !auth.success() {
                return None;
            }
            let mut channel = session.channel_open_session().await.unwrap();
            channel
                .set_env(false, "GIT_PROTOCOL", "version=2")
                .await
                .unwrap();
            channel.exec(true, command).await.unwrap();
            let mut stdout = Vec::new();
            let mut status = None;
            let mut flushed = false;
            while let Some(message) = channel.wait().await {
                match message {
                    russh::ChannelMsg::Data { data } => {
                        stdout.extend_from_slice(&data);
                        if !flushed && stdout.ends_with(b"0000") {
                            channel.data(&b"0000"[..]).await.unwrap();
                            channel.eof().await.unwrap();
                            flushed = true;
                        }
                    }
                    russh::ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                    russh::ChannelMsg::Failure => break,
                    _ => {}
                }
            }
            Some((status, stdout))
        })
    }

    #[test]
    fn ssh_server_runs_git_programs_for_the_client_key_only() {
        let Some(git) = git() else {
            return;
        };
        let work = tempfile::tempdir().unwrap();
        let home = home(work.path());
        let env = GitEnv::new(&home, ScenarioClock::default());
        git.run_ok(
            &env,
            work.path(),
            &["init", "-q", "--bare", "-b", "main", "served.git"],
        )
        .unwrap();
        let server = SshServer::start(&git, &env, work.path(), &home).unwrap();
        let base = server.base_url();
        assert!(base.starts_with("ssh://nurt@127.0.0.1:"), "{base}");
        let port: u16 = base.rsplit(':').next().unwrap().parse().unwrap();
        let identity = std::fs::read_to_string(home.join(".ssh/id_ed25519")).unwrap();
        let key = PrivateKey::from_openssh(&identity).unwrap();
        let (status, stdout) =
            ssh_exec(port, key.clone(), "git-upload-pack '/served.git'").unwrap();
        assert_eq!(status, Some(0));
        assert!(String::from_utf8_lossy(&stdout).contains("version 2"));
        let (status, _) = ssh_exec(port, key.clone(), "git-upload-pack '/missing.git'").unwrap();
        assert!(status.is_some_and(|code| code != 0), "{status:?}");
        let (status, stdout) = ssh_exec(port, key, "sh -c id").unwrap();
        assert_eq!((status, stdout.len()), (None, 0));
        let stranger = PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]));
        assert!(ssh_exec(port, stranger, "git-upload-pack '/served.git'").is_none());
    }

    #[test]
    fn ssh_scenarios_run_the_git_side_without_an_ssh_client() {
        let scenario = scenario("common/ssh-fetch", SSH_FETCH);
        let Some(outcome) = run(Vec::new(), RunOptions::default(), &scenario) else {
            return;
        };
        assert_eq!(outcome.status, Status::GitOnly, "{outcome:?}");
    }

    #[test]
    fn output_records_are_compared_with_the_git_output() {
        let scenario = scenario("common/status-output", STATUS_OUTPUT);
        let matching = StatusMock { path: "notes.txt" };
        let Some(outcome) = run(vec![Arc::new(matching)], RunOptions::default(), &scenario) else {
            return;
        };
        assert_eq!(outcome.status, Status::Passed, "{outcome:?}");
        let other = StatusMock { path: "other.txt" };
        let outcome = run(vec![Arc::new(other)], RunOptions::default(), &scenario).unwrap();
        let text = failures(&outcome);
        assert!(text.contains("output differs from `git status"), "{text}");
        assert!(text.contains("git only: ?? notes.txt"), "{text}");
        assert!(text.contains("nurt only: ?? other.txt"), "{text}");
    }
}
