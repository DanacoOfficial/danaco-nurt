// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-difftest::corpus::extended
// Responsibility:   Recipes of the corpus repositories for operation states, configuration variants, remotes and object forms.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Recipes of the corpus repositories for operation states, configuration
//! variants, remotes and object forms.

use std::path::Path;

use super::{
    Builder, COMMIT_INTERVAL, MERGE_CONFLICT_EXIT, fast_import_committer, fast_import_data,
    write_below,
};
use crate::error::{Result, io_error};
use crate::fsutil;

/// Size of the large file of the repository `large-file`.
const LARGE_FILE_BYTES: usize = 3 * 512 * 1024;

/// Number of directory levels of the repository `deep-tree`.
const DEEP_LEVELS: usize = 30;

/// Number of branches and of tags of the repository `heavy-branches`.
const HEAVY_REFS: u32 = 5_000;

/// Message of a commit in ISO-8859-2, with bytes above 0x7F that are not
/// valid UTF-8.
const LATIN2_MESSAGE: &[u8] = b"caf\xe9 \xb1\xb6 encoded message\n";

fn mkdir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(io_error("create directory", path))
}

pub(super) fn build_worktrees(builder: &mut Builder<'_>) -> Result<()> {
    builder.git(&["config", "worktree.useRelativePaths", "true"])?;
    builder.write("a.txt", b"shared\n")?;
    builder.commit("base")?;
    builder.git(&["worktree", "add", "-q", "-b", "wt-branch", "../wt"])?;
    let linked = builder.sibling("wt");
    write_below(&linked, "wt.txt", b"linked work tree\n")?;
    builder.commit_in(&linked, "commit in the linked work tree")?;
    builder.write("a.txt", b"shared\nmain\n")?;
    builder.commit("commit in the main work tree")
}

/// Two branches that change line 2 of `shared.txt` differently.
fn diverging_line(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("shared.txt", b"line 1\nline 2\nline 3\n")?;
    builder.write("other.txt", b"other\n")?;
    builder.commit("base")?;
    builder.git(&["switch", "-q", "-c", "topic"])?;
    builder.write("shared.txt", b"line 1\ntopic\nline 3\n")?;
    builder.commit("topic: change line 2")?;
    builder.write("other.txt", b"other\ntopic\n")?;
    builder.commit("topic: extend other")?;
    builder.git(&["switch", "-q", "main"])?;
    builder.write("shared.txt", b"line 1\nmain\nline 3\n")?;
    builder.commit("main: change line 2")
}

pub(super) fn build_rebase_in_progress(builder: &mut Builder<'_>) -> Result<()> {
    diverging_line(builder)?;
    builder.git(&["switch", "-q", "topic"])?;
    builder.git_expecting(&["rebase", "main"], MERGE_CONFLICT_EXIT)
}

pub(super) fn build_cherry_pick_in_progress(builder: &mut Builder<'_>) -> Result<()> {
    diverging_line(builder)?;
    builder.git_expecting(&["cherry-pick", "topic~1"], MERGE_CONFLICT_EXIT)
}

pub(super) fn build_revert_in_progress(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("value.txt", b"value 1\n")?;
    builder.commit("value 1")?;
    builder.write("value.txt", b"value 2\n")?;
    builder.commit("value 2")?;
    builder.write("value.txt", b"value 3\n")?;
    builder.commit("value 3")?;
    builder.git_expecting(&["revert", "--no-edit", "HEAD~1"], MERGE_CONFLICT_EXIT)
}

pub(super) fn build_octopus(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("base.txt", b"base\n")?;
    builder.commit("base")?;
    for name in ["one", "two", "three"] {
        builder.git(&["switch", "-q", "-c", name, "main"])?;
        builder.write(&format!("{name}.txt"), format!("{name}\n").as_bytes())?;
        builder.commit(&format!("add {name}"))?;
    }
    builder.git(&["switch", "-q", "main"])?;
    builder.commit_with(&["merge", "-q", "-m", "octopus merge", "one", "two", "three"])
}

pub(super) fn build_autocrlf_true(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("notes.txt", b"first\nsecond\n")?;
    builder.write("src/code.txt", b"fn\nbody\n")?;
    builder.write("data.bin", b"\0binary\nkept\n")?;
    builder.commit("add files with LF line ends")?;
    builder.git(&["config", "core.autocrlf", "true"])?;
    for path in ["notes.txt", "src/code.txt", "data.bin"] {
        builder.remove(path)?;
    }
    builder.git(&["checkout", "--", "."])?;
    builder.write("added.txt", b"untracked with LF\n")
}

pub(super) fn build_filemode_false(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("plain.sh", b"#!/bin/sh\necho plain\n")?;
    builder.write("tool.sh", b"#!/bin/sh\necho tool\n")?;
    fsutil::set_executable(&builder.repo_dir.join("tool.sh"), true)?;
    builder.commit("add scripts")?;
    builder.git(&["config", "core.fileMode", "false"])?;
    fsutil::set_executable(&builder.repo_dir.join("plain.sh"), true)?;
    fsutil::set_executable(&builder.repo_dir.join("tool.sh"), false)
}

pub(super) fn build_ignorecase(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("Readme.md", b"read me\n")?;
    builder.write("Docs/Guide.txt", b"guide\n")?;
    builder.commit("add mixed-case names")?;
    builder.git(&["config", "core.ignoreCase", "true"])?;
    let from = builder.repo_dir.join("Readme.md");
    let to = builder.repo_dir.join("README.md");
    std::fs::rename(&from, &to).map_err(io_error("rename", &from))
}

pub(super) fn build_message_forms(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("a.txt", b"a\n")?;
    builder.commit(
        "subject of a long message\n\nFirst paragraph of the body.\n\nSecond paragraph.\n\nSigned-off-by: Nurt Test <test@nurt.invalid>\nCo-authored-by: Other <other@nurt.invalid>",
    )?;
    builder.commit_with(&["commit", "-q", "--allow-empty", "-m", "empty commit"])?;
    write_below(&builder.case_dir, "message.txt", LATIN2_MESSAGE)?;
    builder.write("a.txt", b"a\nb\n")?;
    builder.git(&["add", "a.txt"])?;
    builder.commit_with(&[
        "-c",
        "i18n.commitEncoding=ISO-8859-2",
        "commit",
        "-q",
        "-F",
        "../message.txt",
    ])?;
    builder.commit_with(&["commit", "-q", "--allow-empty", "-m", "subject only"])
}

pub(super) fn build_notes(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("a.txt", b"one\n")?;
    builder.commit("first")?;
    builder.write("a.txt", b"one\ntwo\n")?;
    builder.commit("second")?;
    builder.commit_with(&["notes", "add", "-m", "note on the first commit", "HEAD~1"])?;
    builder.commit_with(&["notes", "add", "-m", "note on the second commit", "HEAD"])?;
    builder.commit_with(&[
        "notes",
        "--ref",
        "review",
        "add",
        "-m",
        "review note",
        "HEAD",
    ])
}

pub(super) fn build_alternates(builder: &mut Builder<'_>) -> Result<()> {
    let shared = builder.sibling("shared");
    builder.git_in(&shared, &["init", "-q", "-b", "main"])?;
    write_below(&shared, "common.txt", b"stored in the shared repository\n")?;
    builder.commit_in(&shared, "shared history")?;
    write_below(
        &builder.repo_dir,
        ".git/objects/info/alternates",
        b"../../../shared/.git/objects\n",
    )?;
    builder.git(&["fetch", "-q", "../shared", "main:refs/remotes/shared/main"])?;
    builder.git(&["reset", "-q", "--hard", "refs/remotes/shared/main"])?;
    builder.write("local.txt", b"stored locally\n")?;
    builder.commit("local commit")
}

pub(super) fn build_multiple_remotes(builder: &mut Builder<'_>) -> Result<()> {
    let upstream = builder.sibling("upstream");
    builder.git_in(&upstream, &["init", "-q", "-b", "main"])?;
    write_below(&upstream, "a.txt", b"upstream 1\n")?;
    builder.commit_in(&upstream, "upstream 1")?;
    let case_dir = builder.case_dir.clone();
    builder.git_in(
        &case_dir,
        &["clone", "-q", "--bare", "upstream", "origin.git"],
    )?;
    builder.git(&["remote", "add", "origin", "../origin.git"])?;
    builder.git(&["remote", "add", "upstream", "../upstream"])?;
    builder.git(&["fetch", "-q", "origin"])?;
    builder.git(&["reset", "-q", "--hard", "origin/main"])?;
    builder.git(&["branch", "-q", "--set-upstream-to=origin/main", "main"])?;
    write_below(&upstream, "a.txt", b"upstream 1\nupstream 2\n")?;
    builder.commit_in(&upstream, "upstream 2")?;
    builder.git(&["fetch", "-q", "upstream"])?;
    builder.write("local.txt", b"local\n")?;
    builder.commit("local work")
}

pub(super) fn build_empty_dirs(builder: &mut Builder<'_>) -> Result<()> {
    builder.write(".gitignore", b"*.tmp\n")?;
    builder.write("a.txt", b"a\n")?;
    builder.commit("base")?;
    mkdir(&builder.repo_dir.join("empty"))?;
    mkdir(&builder.repo_dir.join("nested/inner/empty"))?;
    builder.write("only-ignored/scratch.tmp", b"ignored\n")
}

pub(super) fn build_large_file(builder: &mut Builder<'_>) -> Result<()> {
    let mut text = String::with_capacity(LARGE_FILE_BYTES + 32);
    let mut row = 0u32;
    while text.len() < LARGE_FILE_BYTES {
        text.push_str(&format!("row {row:07} of the large file\n"));
        row += 1;
    }
    builder.write("large.txt", text.as_bytes())?;
    builder.write("small.txt", b"small\n")?;
    builder.commit("add large file")?;
    let changed = text.replacen("row 0020000 of", "row 0020000 changed in", 1);
    builder.write("large.txt", changed.as_bytes())?;
    builder.commit("change one row of the large file")
}

pub(super) fn build_detached_head(builder: &mut Builder<'_>) -> Result<()> {
    for round in 1..=3 {
        builder.write("a.txt", format!("round {round}\n").as_bytes())?;
        builder.commit(&format!("round {round}"))?;
    }
    builder.git(&["tag", "v-old", "HEAD~1"])?;
    builder.git(&["switch", "-q", "--detach", "v-old"])
}

pub(super) fn build_orphan_branch(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("main.txt", b"main\n")?;
    builder.commit("main history")?;
    builder.git(&["switch", "-q", "--orphan", "pages"])?;
    builder.write("index.html", b"<p>pages</p>\n")?;
    builder.commit("unrelated pages history")?;
    builder.git(&["switch", "-q", "main"])
}

pub(super) fn build_tag_kinds(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("a.txt", b"a\n")?;
    builder.commit("first")?;
    builder.git(&["tag", "light"])?;
    builder.git(&["tag", "-a", "inner", "-m", "annotated tag"])?;
    builder.git(&["tag", "-a", "outer", "-m", "tag of a tag", "inner"])?;
    builder.git(&["tag", "tree-tag", "HEAD^{tree}"])?;
    builder.git(&["tag", "blob-tag", "HEAD:a.txt"])?;
    builder.write("a.txt", b"a\nb\n")?;
    builder.commit("second")?;
    builder.git(&["tag", "-a", "release/1.0", "-m", "nested tag name"])
}

pub(super) fn build_deep_tree(builder: &mut Builder<'_>) -> Result<()> {
    let deep: String = (0..DEEP_LEVELS)
        .map(|level| format!("d{level:02}/"))
        .collect();
    builder.write(&format!("{deep}leaf.txt"), b"leaf\n")?;
    builder.write("top.txt", b"top\n")?;
    builder.commit("add a deep path")?;
    builder.write(&format!("{deep}leaf.txt"), b"leaf\nchanged\n")?;
    builder.commit("change the deep file")
}

pub(super) fn build_reflog_history(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("a.txt", b"one\n")?;
    builder.commit("one")?;
    builder.write("a.txt", b"one\ntwo\n")?;
    builder.commit("two")?;
    builder.commit_with(&["commit", "-q", "--amend", "-m", "two, amended"])?;
    builder.git(&["switch", "-q", "-c", "side"])?;
    builder.write("side.txt", b"side\n")?;
    builder.commit("side")?;
    builder.git(&["switch", "-q", "main"])?;
    builder.write("a.txt", b"one\ntwo\nthree\n")?;
    builder.commit("three")?;
    builder.git(&["reset", "-q", "--hard", "HEAD~1"])
}

pub(super) fn build_merge_conflict_kinds(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("content.txt", b"line 1\nline 2\nline 3\n")?;
    builder.write("modify-delete.txt", b"kept by main\n")?;
    builder.write("delete-modify.txt", b"kept by theirs\n")?;
    builder.write("renamed.txt", b"renamed by theirs\nfirst\nsecond\nthird\n")?;
    builder.commit("base")?;
    builder.git(&["switch", "-q", "-c", "theirs"])?;
    builder.write("content.txt", b"line 1\ntheirs\nline 3\n")?;
    builder.remove("modify-delete.txt")?;
    builder.write("delete-modify.txt", b"kept by theirs\nchanged\n")?;
    builder.git(&["mv", "renamed.txt", "moved.txt"])?;
    builder.write("added.txt", b"added by theirs\n")?;
    builder.commit("theirs: every kind of change")?;
    builder.git(&["switch", "-q", "main"])?;
    builder.write("content.txt", b"line 1\nmain\nline 3\n")?;
    builder.write("modify-delete.txt", b"kept by main\nchanged\n")?;
    builder.remove("delete-modify.txt")?;
    builder.remove("renamed.txt")?;
    builder.write("added.txt", b"added by main\n")?;
    builder.commit("main: conflicting changes")?;
    builder.git_expecting(&["merge", "--no-edit", "theirs"], MERGE_CONFLICT_EXIT)
}

pub(super) fn build_index_flags(builder: &mut Builder<'_>) -> Result<()> {
    for path in ["assumed.txt", "skipped.txt", "plain.txt"] {
        builder.write(path, format!("{path}\n").as_bytes())?;
    }
    builder.commit("base")?;
    builder.git(&["update-index", "--assume-unchanged", "assumed.txt"])?;
    builder.git(&["update-index", "--skip-worktree", "skipped.txt"])?;
    builder.write("intent.txt", b"intent to add\n")?;
    builder.git(&["add", "-N", "intent.txt"])
}

pub(super) fn build_with_bare_remote(builder: &mut Builder<'_>) -> Result<()> {
    let case_dir = builder.case_dir.clone();
    builder.git_in(
        &case_dir,
        &["init", "-q", "--bare", "-b", "main", "remote.git"],
    )?;
    builder.git(&["remote", "add", "origin", "../remote.git"])?;
    builder.write("a.txt", b"a\n")?;
    builder.commit("first")?;
    builder.git(&["branch", "topic"])?;
    builder.git(&["push", "-q", "-u", "origin", "main", "topic"])?;
    builder.write("a.txt", b"a\nlocal\n")?;
    builder.commit("local commit not pushed")
}

pub(super) fn build_attributes_ident(builder: &mut Builder<'_>) -> Result<()> {
    builder.write(
        ".gitattributes",
        b"*.c ident\n*.log merge=union\n*.md diff=markdown\n",
    )?;
    builder.write("source.c", b"/* $Id$ */\nint main(void) { return 0; }\n")?;
    builder.write("history.log", b"entry 1\n")?;
    builder.write("README.md", b"# Title\n")?;
    builder.commit("add files with attributes")?;
    builder.remove("source.c")?;
    builder.git(&["checkout", "--", "source.c"])
}

pub(super) fn build_typechange(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("target.txt", b"target\n")?;
    builder.write("item", b"regular file\n")?;
    builder.link("link", "target.txt")?;
    builder.commit("file and link")?;
    builder.remove("item")?;
    builder.link("item", "target.txt")?;
    builder.remove("link")?;
    builder.write("link", b"now a regular file\n")
}

pub(super) fn build_whitespace(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("clean.txt", b"clean\n")?;
    builder.write("text.md", b"first\nsecond\n")?;
    builder.commit("base")?;
    builder.write("clean.txt", b"clean\ntrailing space \n")?;
    builder.write("text.md", b"first  \nsecond")?;
    builder.write("tabs.txt", b"\t \tmixed indent\nline\r\n")
}

pub(super) fn build_merge_history(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("a.txt", b"1\n2\n3\n")?;
    builder.commit("base")?;
    builder.git(&["switch", "-q", "-c", "feature"])?;
    builder.write("f.txt", b"feature\n")?;
    builder.commit("feature work")?;
    builder.git(&["switch", "-q", "main"])?;
    builder.write("m.txt", b"main\n")?;
    builder.commit("main work")?;
    builder.commit_with(&["merge", "-q", "--no-ff", "-m", "merge feature", "feature"])?;
    builder.git(&["switch", "-q", "-c", "quick"])?;
    builder.write("q.txt", b"quick\n")?;
    builder.commit("quick fix")?;
    builder.git(&["switch", "-q", "main"])?;
    builder.git(&["merge", "-q", "--ff-only", "quick"])?;
    builder.git(&["switch", "-q", "-c", "clash"])?;
    builder.write("a.txt", b"1\nclash\n3\n")?;
    builder.commit("clash: change line 2")?;
    builder.git(&["switch", "-q", "main"])?;
    builder.write("a.txt", b"1\nmain\n3\n")?;
    builder.commit("main: change line 2")?;
    builder.git_expecting(&["merge", "--no-edit", "clash"], MERGE_CONFLICT_EXIT)?;
    builder.write("a.txt", b"1\nresolved\n3\n")?;
    builder.git(&["add", "a.txt"])?;
    builder.commit_with(&["commit", "-q", "--no-edit"])
}

pub(super) fn build_heavy_branches(builder: &mut Builder<'_>) -> Result<()> {
    let mut stream = String::from("commit refs/heads/main\nmark :1\n");
    stream.push_str(&fast_import_committer(builder.clock));
    fast_import_data(&mut stream, "base\n");
    stream.push_str("M 100644 inline a.txt\n");
    fast_import_data(&mut stream, "a\n");
    for number in 0..HEAVY_REFS {
        stream.push_str(&format!("reset refs/heads/branch/{number:05}\nfrom :1\n\n"));
        stream.push_str(&format!("reset refs/tags/tag-{number:05}\nfrom :1\n\n"));
    }
    builder.git_with_input(&["fast-import", "--quiet"], stream.as_bytes())?;
    builder.clock = builder.clock.advanced(COMMIT_INTERVAL);
    builder.git(&["pack-refs", "--all", "--prune"])?;
    builder.git(&["reset", "-q", "--hard", "main"])
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
