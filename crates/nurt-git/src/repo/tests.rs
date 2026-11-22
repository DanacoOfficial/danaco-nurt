// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use super::*;
use crate::testutil::{RepoBuilder, TestHost, test_ctx, test_ctx_with};
use nurt_contracts::error::ErrorCode;

#[test]
fn discover_finds_the_root_from_a_subdirectory() {
    let fixture = RepoBuilder::new()
        .file("src/deep/a.txt", "1\n")
        .commit("one")
        .build();
    let repo = GitRepo::discover(&fixture.path().join("src/deep"), &test_ctx()).unwrap();
    assert_eq!(repo.root(), fixture.path());
    assert_eq!(repo.workdir(), Some(fixture.path()));
    assert!(!repo.is_bare());
}

#[test]
fn open_accepts_the_root_and_the_git_directory() {
    let fixture = RepoBuilder::new()
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let ctx = test_ctx();
    let by_root = GitRepo::open(fixture.path(), &ctx).unwrap();
    let by_git_dir = GitRepo::open(&fixture.path().join(".git"), &ctx).unwrap();
    assert_eq!(by_root.git_dir(), by_git_dir.git_dir());
    assert_eq!(by_root.head_id().unwrap(), by_git_dir.head_id().unwrap());
}

#[test]
fn bare_repository_has_its_directory_as_root() {
    let fixture = RepoBuilder::bare()
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
    assert!(repo.is_bare());
    assert_eq!(repo.root(), fixture.path());
    assert!(repo.head_id().unwrap().is_some());
}

#[test]
fn separate_git_directory_is_followed_through_the_git_file() {
    let fixture = RepoBuilder::new()
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let moved = fixture.temp_dir().join("store.git");
    std::fs::rename(fixture.path().join(".git"), &moved).unwrap();
    std::fs::write(
        fixture.path().join(".git"),
        format!("gitdir: {}\n", moved.display()),
    )
    .unwrap();
    let mut config = std::fs::read_to_string(moved.join("config")).unwrap();
    config.push_str(&format!(
        "[core]\n\tworktree = {}\n",
        fixture.path().display()
    ));
    std::fs::write(moved.join("config"), config).unwrap();
    let repo = GitRepo::discover(fixture.path(), &test_ctx()).unwrap();
    assert_eq!(repo.git_dir(), moved.as_path());
    assert!(repo.head_id().unwrap().is_some());
}

#[test]
fn unborn_branch_has_no_head_commit() {
    let fixture = RepoBuilder::new().build();
    let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
    assert_eq!(repo.head_id().unwrap(), None);
    assert_eq!(
        repo.head_ref()
            .unwrap()
            .map(|name| name.as_bstr().to_string()),
        Some("refs/heads/main".to_owned())
    );
    assert!(repo.index_file().unwrap().entries().is_empty());
}

#[test]
fn directory_outside_a_repository_lists_nearby_repositories() {
    let fixture = RepoBuilder::new().build();
    let error = GitRepo::open(fixture.temp_dir(), &test_ctx()).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitNotRepository);
    assert_eq!(error.candidates.len(), 1);
    assert_eq!(
        error.candidates[0].program,
        format!("git.status {}", error::quote_path(fixture.path()))
    );
}

#[test]
fn missing_path_is_reported_as_path_not_found() {
    let error =
        GitRepo::discover(Path::new("/nonexistent/path/for/test"), &test_ctx()).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitNotRepository);
    assert_eq!(error.problem.as_deref(), Some("path_not_found"));
}

#[test]
fn reftable_repository_is_unsupported() {
    let fixture = RepoBuilder::new()
        .config("core.repositoryFormatVersion", "1")
        .config("extensions.refStorage", "reftable")
        .build();
    let error = GitRepo::open(fixture.path(), &test_ctx()).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitUnsupported);
}

#[test]
fn reftable_directory_is_unsupported_without_the_extension() {
    let fixture = RepoBuilder::new().build();
    std::fs::create_dir(fixture.git_dir().join("reftable")).unwrap();
    let error = GitRepo::open(fixture.path(), &test_ctx()).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitUnsupported);
}

#[test]
fn fsmonitor_is_ignored_with_a_note() {
    let fixture = RepoBuilder::new().config("core.fsmonitor", "true").build();
    let ctx = test_ctx();
    GitRepo::open(fixture.path(), &ctx).unwrap();
    let notes = ctx.take_notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].code, Notable::FsmonitorIgnored);
}

fn home_with_gitconfig(text: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join(".gitconfig"), text).unwrap();
    home
}

#[test]
fn global_configuration_comes_from_the_context_home() {
    let home = home_with_gitconfig("[user]\n\tname = Global Name\n\temail = g@example.com\n");
    let fixture = RepoBuilder::new()
        .config("user.email", "local@example.com")
        .build();
    let ctx = test_ctx_with(TestHost::new(home.path()));
    let repo = GitRepo::open(fixture.path(), &ctx).unwrap();
    assert_eq!(
        repo.config_string("user.name").as_deref(),
        Some("Global Name")
    );
    assert_eq!(
        repo.config_string("user.email").as_deref(),
        Some("local@example.com")
    );
}

#[test]
fn xdg_configuration_has_lower_precedence_than_gitconfig() {
    let home = home_with_gitconfig("[core]\n\tabbrev = 9\n");
    let xdg = home.path().join("xdg");
    std::fs::create_dir_all(xdg.join("git")).unwrap();
    std::fs::write(
        xdg.join("git/config"),
        "[core]\n\tabbrev = 12\n[pull]\n\trebase = true\n",
    )
    .unwrap();
    let fixture = RepoBuilder::new()
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let host = TestHost::new(home.path()).with_var("XDG_CONFIG_HOME", xdg.as_os_str());
    let repo = GitRepo::open(fixture.path(), &test_ctx_with(host)).unwrap();
    assert_eq!(repo.config_string("core.abbrev").as_deref(), Some("9"));
    assert_eq!(repo.config_bool("pull.rebase"), Some(true));
    let head = repo.head_id().unwrap().unwrap();
    assert_eq!(repo.short_id(&head).len(), 9);
}

#[test]
fn environment_pairs_override_the_repository_configuration() {
    let fixture = RepoBuilder::new().config("core.abbrev", "8").build();
    let host = TestHost::new("/nonexistent-home")
        .with_var("GIT_CONFIG_COUNT", "1")
        .with_var("GIT_CONFIG_KEY_0", "core.abbrev")
        .with_var("GIT_CONFIG_VALUE_0", "11");
    let repo = GitRepo::open(fixture.path(), &test_ctx_with(host)).unwrap();
    assert_eq!(repo.config_string("core.abbrev").as_deref(), Some("11"));
}

#[test]
fn global_includes_are_resolved_with_the_context_home() {
    let home = home_with_gitconfig("[include]\n\tpath = ~/extra.inc\n");
    std::fs::write(
        home.path().join("extra.inc"),
        "[merge]\n\tconflictStyle = zdiff3\n",
    )
    .unwrap();
    let fixture = RepoBuilder::new().build();
    let repo = GitRepo::open(fixture.path(), &test_ctx_with(TestHost::new(home.path()))).unwrap();
    assert_eq!(
        repo.config_string("merge.conflictStyle").as_deref(),
        Some("zdiff3")
    );
}

#[test]
fn git_config_global_replaces_the_home_files() {
    let home = home_with_gitconfig("[user]\n\tname = Home\n");
    let custom = home.path().join("custom");
    std::fs::write(&custom, "[user]\n\tname = Custom\n").unwrap();
    let fixture = RepoBuilder::new().build();
    let host = TestHost::new(home.path()).with_var("GIT_CONFIG_GLOBAL", custom.as_os_str());
    let repo = GitRepo::open(fixture.path(), &test_ctx_with(host)).unwrap();
    assert_eq!(repo.config_string("user.name").as_deref(), Some("Custom"));
}

#[test]
fn system_configuration_is_read_from_git_config_system() {
    let dir = tempfile::tempdir().unwrap();
    let system = dir.path().join("gitconfig");
    std::fs::write(&system, "[core]\n\tautocrlf = input\n").unwrap();
    let fixture = RepoBuilder::new().build();
    let host = TestHost::new("/nonexistent-home")
        .with_var("GIT_CONFIG_NOSYSTEM", "0")
        .with_var("GIT_CONFIG_SYSTEM", system.as_os_str());
    let repo = GitRepo::open(fixture.path(), &test_ctx_with(host)).unwrap();
    assert_eq!(
        repo.config_string("core.autocrlf").as_deref(),
        Some("input")
    );
}

#[test]
fn transports_that_start_programs_are_disabled() {
    let fixture = RepoBuilder::new().build();
    let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
    assert_eq!(
        repo.config_string("protocol.file.allow").as_deref(),
        Some("never")
    );
    assert_eq!(
        repo.config_string("protocol.ssh.allow").as_deref(),
        Some("never")
    );
}

#[test]
fn ceiling_directories_stop_discovery() {
    let fixture = RepoBuilder::new()
        .file("sub/a.txt", "1\n")
        .commit("one")
        .build();
    let host = TestHost::new("/nonexistent-home")
        .with_var("GIT_CEILING_DIRECTORIES", fixture.path().as_os_str());
    let error = GitRepo::discover(&fixture.path().join("sub"), &test_ctx_with(host)).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitNotRepository);
}
