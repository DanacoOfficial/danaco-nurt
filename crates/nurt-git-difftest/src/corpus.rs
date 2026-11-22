// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Corpus of test repositories, generated deterministically with the `git`
//! program.
//!
//! Every repository is built once per test run into a cache directory as a
//! case layout (`<name>/repo`, with helper repositories beside it when the
//! recipe needs them) and copied for each scenario. Commits get dates from a
//! clock that advances by one minute per commit, so identifiers are equal on
//! every machine with the same `git` version.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::{DifftestError, Result, io_error};
use crate::fsutil;
use crate::git::{GitEnv, GitProgram, IDENTITY_EMAIL, IDENTITY_NAME, expect_exit};
use crate::scenario::Platform;
use crate::time::ScenarioClock;

mod extended;

use extended::{
    build_alternates, build_attributes_ident, build_autocrlf_true, build_cherry_pick_in_progress,
    build_deep_tree, build_detached_head, build_empty_dirs, build_filemode_false,
    build_heavy_branches, build_ignorecase, build_index_flags, build_large_file,
    build_merge_conflict_kinds, build_merge_history, build_message_forms, build_multiple_remotes,
    build_notes, build_octopus, build_orphan_branch, build_rebase_in_progress,
    build_reflog_history, build_revert_in_progress, build_tag_kinds, build_typechange,
    build_whitespace, build_with_bare_remote, build_worktrees,
};

/// Name of the repository directory inside a case directory.
pub const REPO_DIR: &str = "repo";

/// Date of the first commit of every corpus repository.
const CORPUS_EPOCH: &str = "2025-06-01T09:00:00+02:00";

/// Seconds between two commits of a corpus repository.
const COMMIT_INTERVAL: i64 = 60;

/// Exit code of `git merge` stopped by a conflict.
const MERGE_CONFLICT_EXIT: i32 = 1;

/// Number of commits of the repository `heavy-history`.
const HEAVY_COMMITS: u32 = 10_000;

/// Number of files of the repository `heavy-files`.
const HEAVY_FILES: usize = 100_000;

/// Files per directory of the repository `heavy-files`.
const FILES_PER_DIRECTORY: usize = 1_000;

/// Platforms of repositories whose content every file system can hold.
const EVERY_PLATFORM: &[Platform] = &Platform::ALL;
/// Platforms with symbolic links and executable bits.
const UNIX_PLATFORMS: &[Platform] = &[Platform::Linux, Platform::Macos];
/// Platforms with case-sensitive file names that keep the Unicode form.
const LINUX_ONLY: &[Platform] = &[Platform::Linux];

/// Recipe of one corpus repository.
#[derive(Debug, Clone, Copy)]
pub struct CorpusRepo {
    /// Name used by scenarios.
    pub name: &'static str,
    /// What the repository contains.
    pub description: &'static str,
    /// Platforms whose file systems can hold the repository.
    pub platforms: &'static [Platform],
    /// The repository is large; scenarios using it run only on request.
    pub heavy: bool,
    build: fn(&mut Builder<'_>) -> Result<()>,
}

impl CorpusRepo {
    const fn new(
        name: &'static str,
        description: &'static str,
        platforms: &'static [Platform],
        build: fn(&mut Builder<'_>) -> Result<()>,
    ) -> Self {
        Self {
            name,
            description,
            platforms,
            heavy: false,
            build,
        }
    }

    const fn heavy(mut self) -> Self {
        self.heavy = true;
        self
    }
}

/// Every corpus repository.
pub const CORPUS: &[CorpusRepo] = &[
    CorpusRepo::new(
        "empty",
        "Repository without commits on branch main.",
        EVERY_PLATFORM,
        build_empty,
    ),
    CorpusRepo::new(
        "linear-small",
        "Five commits on main touching files in nested directories, lightweight tag v1.",
        EVERY_PLATFORM,
        build_linear,
    ),
    CorpusRepo::new(
        "branchy-small",
        "Branches main, topic and feature diverging from main, annotated tag v1.0.",
        EVERY_PLATFORM,
        build_branchy,
    ),
    CorpusRepo::new(
        "criss-cross",
        "Branches left and right merged into each other, so they have two merge bases.",
        EVERY_PLATFORM,
        build_criss_cross,
    ),
    CorpusRepo::new(
        "renames",
        "History with a renamed directory and a renamed and edited file, branch before-move.",
        EVERY_PLATFORM,
        build_renames,
    ),
    CorpusRepo::new(
        "binary",
        "Binary files with NUL bytes and every byte value, changed in a second commit.",
        EVERY_PLATFORM,
        build_binary,
    ),
    CorpusRepo::new(
        "crlf",
        "Attributes text=auto, eol=crlf and eol=lf with files checked out after conversion.",
        EVERY_PLATFORM,
        build_crlf,
    ),
    CorpusRepo::new(
        "symlinks",
        "Links to a file, to a directory, across directories and a dangling link.",
        UNIX_PLATFORMS,
        build_symlinks,
    ),
    CorpusRepo::new(
        "exec-bits",
        "Scripts gaining and losing the executable bit across commits.",
        UNIX_PLATFORMS,
        build_exec_bits,
    ),
    CorpusRepo::new(
        "unicode-names",
        "Names in NFC and NFD form side by side, with spaces, quotes and non-Latin scripts.",
        LINUX_ONLY,
        build_unicode,
    ),
    CorpusRepo::new(
        "case-names",
        "Files and directories whose names differ only in letter case.",
        LINUX_ONLY,
        build_case_names,
    ),
    CorpusRepo::new(
        "packed-refs",
        "Branches and a peeled annotated tag in packed-refs, one loose branch created after packing.",
        EVERY_PLATFORM,
        build_packed_refs,
    ),
    CorpusRepo::new(
        "packed-objects",
        "Six commits whose objects are in one pack file after git gc, without loose objects.",
        EVERY_PLATFORM,
        build_packed_objects,
    ),
    CorpusRepo::new(
        "ignored-files",
        "Ignored files, an ignored directory and a negated pattern beside untracked files.",
        EVERY_PLATFORM,
        build_ignored,
    ),
    CorpusRepo::new(
        "dirty-worktree",
        "Staged, unstaged, partly staged, deleted and untracked changes on top of one commit.",
        EVERY_PLATFORM,
        build_dirty,
    ),
    CorpusRepo::new(
        "stash-present",
        "Two stash entries, the newer one with an untracked file.",
        EVERY_PLATFORM,
        build_stash,
    ),
    CorpusRepo::new(
        "submodule",
        "Submodule lib cloned from the sibling repository library and committed.",
        EVERY_PLATFORM,
        build_submodule,
    ),
    CorpusRepo::new(
        "shallow",
        "Depth 2 fetch of the four commits of the sibling repository source, remote origin.",
        EVERY_PLATFORM,
        build_shallow,
    ),
    CorpusRepo::new(
        "sparse",
        "Cone-mode sparse checkout of directory app; the other directories are skip-worktree.",
        EVERY_PLATFORM,
        build_sparse,
    ),
    CorpusRepo::new(
        "merge-in-progress",
        "Merge of branch theirs stopped by a content conflict in shared.txt, clean change in other.txt.",
        EVERY_PLATFORM,
        build_merge_in_progress,
    ),
    CorpusRepo::new(
        "many-branches",
        "Twelve branches topic/01 to topic/12, topic/01 merged into main with a merge commit.",
        EVERY_PLATFORM,
        build_many_branches,
    ),
    CorpusRepo::new(
        "heavy-history",
        "Ten thousand commits on main, each changing history.txt.",
        EVERY_PLATFORM,
        build_heavy_history,
    )
    .heavy(),
    CorpusRepo::new(
        "heavy-files",
        "One commit with one hundred thousand files in one hundred directories.",
        EVERY_PLATFORM,
        build_heavy_files,
    )
    .heavy(),
    CorpusRepo::new(
        "worktrees",
        "Linked work tree ../wt on branch wt-branch with its own commit, links with relative paths.",
        EVERY_PLATFORM,
        build_worktrees,
    ),
    CorpusRepo::new(
        "rebase-in-progress",
        "Rebase of topic onto main stopped by a conflict in shared.txt, one commit still to apply.",
        EVERY_PLATFORM,
        build_rebase_in_progress,
    ),
    CorpusRepo::new(
        "cherry-pick-in-progress",
        "Cherry-pick of the first topic commit onto main stopped by a conflict in shared.txt.",
        EVERY_PLATFORM,
        build_cherry_pick_in_progress,
    ),
    CorpusRepo::new(
        "revert-in-progress",
        "Revert of the middle of three commits stopped by a conflict in value.txt.",
        EVERY_PLATFORM,
        build_revert_in_progress,
    ),
    CorpusRepo::new(
        "octopus",
        "Octopus merge of branches one, two and three into main.",
        EVERY_PLATFORM,
        build_octopus,
    ),
    CorpusRepo::new(
        "autocrlf-true",
        "Files committed with LF and checked out with core.autocrlf=true, an untracked LF file.",
        EVERY_PLATFORM,
        build_autocrlf_true,
    ),
    CorpusRepo::new(
        "filemode-false",
        "core.fileMode=false with executable bits in the working tree that differ from the index.",
        UNIX_PLATFORMS,
        build_filemode_false,
    ),
    CorpusRepo::new(
        "ignorecase",
        "core.ignoreCase=true with Readme.md renamed to README.md in the working tree.",
        LINUX_ONLY,
        build_ignorecase,
    ),
    CorpusRepo::new(
        "message-forms",
        "Messages with body and trailers, an empty commit, an ISO-8859-2 message and a subject only.",
        EVERY_PLATFORM,
        build_message_forms,
    ),
    CorpusRepo::new(
        "notes",
        "Notes on two commits in refs/notes/commits and one in refs/notes/review.",
        EVERY_PLATFORM,
        build_notes,
    ),
    CorpusRepo::new(
        "alternates",
        "History borrowed from the sibling repository shared through objects/info/alternates.",
        EVERY_PLATFORM,
        build_alternates,
    ),
    CorpusRepo::new(
        "multiple-remotes",
        "Remotes origin (bare) and upstream, main tracking origin/main, upstream/main one commit ahead.",
        EVERY_PLATFORM,
        build_multiple_remotes,
    ),
    CorpusRepo::new(
        "empty-dirs",
        "Empty directories and a directory holding only ignored files beside one commit.",
        EVERY_PLATFORM,
        build_empty_dirs,
    ),
    CorpusRepo::new(
        "large-file",
        "Text file of 1.5 MiB committed and changed in one row.",
        EVERY_PLATFORM,
        build_large_file,
    ),
    CorpusRepo::new(
        "detached-head",
        "HEAD detached at the tag v-old, one commit behind main.",
        EVERY_PLATFORM,
        build_detached_head,
    ),
    CorpusRepo::new(
        "orphan-branch",
        "Branch pages with a history unrelated to main.",
        EVERY_PLATFORM,
        build_orphan_branch,
    ),
    CorpusRepo::new(
        "tag-kinds",
        "Lightweight and annotated tags, a tag of a tag, tags of a tree and a blob, a nested tag name.",
        EVERY_PLATFORM,
        build_tag_kinds,
    ),
    CorpusRepo::new(
        "deep-tree",
        "File thirty directory levels deep, changed in a second commit.",
        EVERY_PLATFORM,
        build_deep_tree,
    ),
    CorpusRepo::new(
        "reflog-history",
        "Reflog with an amended commit, a branch switch and a hard reset.",
        EVERY_PLATFORM,
        build_reflog_history,
    ),
    CorpusRepo::new(
        "merge-conflict-kinds",
        "Merge stopped with content, modify/delete, delete/modify, rename/delete and add/add conflicts.",
        EVERY_PLATFORM,
        build_merge_conflict_kinds,
    ),
    CorpusRepo::new(
        "index-flags",
        "Index entries with assume-unchanged, skip-worktree and intent-to-add.",
        EVERY_PLATFORM,
        build_index_flags,
    ),
    CorpusRepo::new(
        "with-bare-remote",
        "Bare remote ../remote.git with main and topic pushed, local main one commit ahead.",
        EVERY_PLATFORM,
        build_with_bare_remote,
    ),
    CorpusRepo::new(
        "attributes-ident",
        "Attributes ident, merge=union and diff=markdown, with $Id$ expanded in source.c.",
        EVERY_PLATFORM,
        build_attributes_ident,
    ),
    CorpusRepo::new(
        "typechange",
        "A file replaced by a link and a link replaced by a file in the working tree.",
        UNIX_PLATFORMS,
        build_typechange,
    ),
    CorpusRepo::new(
        "whitespace",
        "Trailing spaces, a missing final newline, mixed indentation and a CRLF line, unstaged.",
        EVERY_PLATFORM,
        build_whitespace,
    ),
    CorpusRepo::new(
        "merge-history",
        "No-fast-forward, fast-forward and conflict-resolved merges on main.",
        EVERY_PLATFORM,
        build_merge_history,
    ),
    CorpusRepo::new(
        "heavy-branches",
        "Five thousand branches and five thousand tags in packed-refs at one commit.",
        EVERY_PLATFORM,
        build_heavy_branches,
    )
    .heavy(),
];

/// Recipe of the corpus repository `name`.
#[must_use]
pub fn find(name: &str) -> Option<&'static CorpusRepo> {
    CORPUS.iter().find(|repo| repo.name == name)
}

/// Builds repositories of a recipe with the `git` program.
#[derive(Debug)]
pub struct Builder<'a> {
    git: &'a GitProgram,
    env: GitEnv,
    case_dir: PathBuf,
    repo_dir: PathBuf,
    clock: ScenarioClock,
}

impl Builder<'_> {
    /// Runs `git` in the repository with the current commit date.
    fn git(&self, args: &[&str]) -> Result<()> {
        self.git_in(&self.repo_dir, args)
    }

    /// Runs `git` in `dir`, creating it first, with the current commit date.
    fn git_in(&self, dir: &Path, args: &[&str]) -> Result<()> {
        std::fs::create_dir_all(dir).map_err(io_error("create directory", dir))?;
        let env = self.env.with_clock(self.clock);
        self.git.run_ok(&env, dir, args).map(drop)
    }

    /// Runs `git` in the repository and requires the exit code `code`.
    fn git_expecting(&self, args: &[&str], code: i32) -> Result<()> {
        let env = self.env.with_clock(self.clock);
        let output = self.git.run(&env, &self.repo_dir, args)?;
        expect_exit(&output, code, &self.repo_dir, args)
    }

    /// Runs `git` in the repository with `input` on its standard input.
    fn git_with_input(&self, args: &[&str], input: &[u8]) -> Result<()> {
        let env = self.env.with_clock(self.clock);
        let output = self
            .git
            .run_with_input(&env, &self.repo_dir, args, Some(input))?;
        expect_exit(&output, 0, &self.repo_dir, args)
    }

    /// Directory `name` beside the repository in the case directory.
    fn sibling(&self, name: &str) -> PathBuf {
        self.case_dir.join(name)
    }

    /// Writes a file of the working tree, creating its directories.
    fn write(&self, path: &str, bytes: &[u8]) -> Result<()> {
        write_below(&self.repo_dir, path, bytes)
    }

    /// Creates the symbolic link `path` of the working tree.
    fn link(&self, path: &str, target: &str) -> Result<()> {
        fsutil::make_symlink(Path::new(target), &self.repo_dir.join(path), false)
    }

    /// Stages every change and commits with `message`, then advances the
    /// clock.
    fn commit(&mut self, message: &str) -> Result<()> {
        let repo = self.repo_dir.clone();
        self.commit_in(&repo, message)
    }

    /// Stages every change of the repository in `dir` and commits it, then
    /// advances the clock.
    fn commit_in(&mut self, dir: &Path, message: &str) -> Result<()> {
        self.git_in(dir, &["add", "-A"])?;
        self.git_in(dir, &["commit", "-q", "-m", message])?;
        self.clock = self.clock.advanced(COMMIT_INTERVAL);
        Ok(())
    }

    /// Commits with a merge or another command that creates a commit.
    fn commit_with(&mut self, args: &[&str]) -> Result<()> {
        self.git(args)?;
        self.clock = self.clock.advanced(COMMIT_INTERVAL);
        Ok(())
    }

    fn remove(&self, path: &str) -> Result<()> {
        fsutil::remove_entry(&self.repo_dir.join(path))
    }
}

/// Writes the file `path` below `dir`, creating its directories.
fn write_below(dir: &Path, path: &str, bytes: &[u8]) -> Result<()> {
    let full = dir.join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(io_error("create directory", parent))?;
    }
    std::fs::write(&full, bytes).map_err(io_error("write", &full))
}

fn build_empty(_builder: &mut Builder<'_>) -> Result<()> {
    Ok(())
}

fn build_linear(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("README.md", b"# Sample\n\nA small linear history.\n")?;
    builder.write("src/lib.txt", b"one\ntwo\nthree\n")?;
    builder.commit("add readme and library")?;
    builder.write("docs/guide.md", b"Guide\n=====\n\nStep one.\n")?;
    builder.commit("add guide")?;
    builder.write("src/lib.txt", b"one\ntwo\nthree\nfour\n")?;
    builder.git(&["tag", "v1"])?;
    builder.commit("extend library")?;
    builder.write("docs/guide.md", b"Guide\n=====\n\nStep one.\nStep two.\n")?;
    builder.write("src/deep/nested/data.txt", b"nested\n")?;
    builder.commit("add nested data and second step")?;
    builder.write(
        "README.md",
        b"# Sample\n\nA small linear history with five commits.\n",
    )?;
    builder.commit("describe history")
}

fn build_branchy(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("a.txt", b"alpha\nbeta\ngamma\ndelta\n")?;
    builder.write("b.txt", b"one\ntwo\nthree\n")?;
    builder.commit("initial content")?;
    builder.write("a.txt", b"alpha\nbeta\ngamma\ndelta\nepsilon\n")?;
    builder.commit("extend a")?;
    builder.git(&["branch", "topic"])?;
    builder.git(&["branch", "feature"])?;
    builder.git(&["switch", "-q", "topic"])?;
    builder.write("b.txt", b"one\ntwo\nthree\nfour\n")?;
    builder.commit("topic: extend b")?;
    builder.write("c.txt", b"topic file\n")?;
    builder.commit("topic: add c")?;
    builder.git(&["switch", "-q", "feature"])?;
    builder.write("feature/f.txt", b"feature\n")?;
    builder.commit("feature: add f")?;
    builder.git(&["switch", "-q", "main"])?;
    builder.write("a.txt", b"ALPHA\nbeta\ngamma\ndelta\nepsilon\n")?;
    builder.commit("main: capitalize first line")?;
    builder.git(&["tag", "-a", "v1.0", "-m", "release 1.0"])
}

fn build_criss_cross(builder: &mut Builder<'_>) -> Result<()> {
    let shared = b"first\n2\n3\n4\n5\n6\n7\nlast\n";
    builder.write("l.txt", b"left 0\n")?;
    builder.write("r.txt", b"right 0\n")?;
    builder.write("shared.txt", shared)?;
    builder.commit("base")?;
    builder.git(&["branch", "left"])?;
    builder.git(&["branch", "right"])?;
    builder.git(&["switch", "-q", "left"])?;
    builder.write("l.txt", b"left 1\n")?;
    builder.commit("left 1")?;
    builder.git(&["tag", "l1"])?;
    builder.git(&["switch", "-q", "right"])?;
    builder.write("r.txt", b"right 1\n")?;
    builder.commit("right 1")?;
    builder.git(&["tag", "r1"])?;
    builder.git(&["switch", "-q", "left"])?;
    builder.commit_with(&["merge", "-q", "--no-ff", "-m", "merge r1 into left", "r1"])?;
    builder.git(&["switch", "-q", "right"])?;
    builder.commit_with(&["merge", "-q", "--no-ff", "-m", "merge l1 into right", "l1"])?;
    builder.write("shared.txt", b"first\n2\n3\n4\n5\n6\n7\nlast (right)\n")?;
    builder.commit("right 2")?;
    builder.git(&["switch", "-q", "left"])?;
    builder.write("shared.txt", b"first (left)\n2\n3\n4\n5\n6\n7\nlast\n")?;
    builder.commit("left 2")?;
    builder.git(&["switch", "-q", "main"])
}

fn build_renames(builder: &mut Builder<'_>) -> Result<()> {
    let body: Vec<u8> = (1..=20)
        .flat_map(|n| format!("line {n}\n").into_bytes())
        .collect();
    builder.write("old/one.txt", &body)?;
    builder.write("old/two.txt", b"second file\nwith two lines\n")?;
    builder.write("notes.txt", &body)?;
    builder.commit("add files")?;
    builder.git(&["branch", "before-move"])?;
    builder.git(&["mv", "old", "new"])?;
    builder.commit("rename directory old to new")?;
    builder.git(&["mv", "notes.txt", "docs-notes.txt"])?;
    let mut edited = body.clone();
    edited.extend_from_slice(b"line 21\n");
    builder.write("docs-notes.txt", &edited)?;
    builder.commit("rename and extend notes")?;
    builder.git(&["switch", "-q", "before-move"])?;
    let mut changed = body.clone();
    changed.splice(0..0, b"line 0\n".iter().copied());
    builder.write("old/one.txt", &changed)?;
    builder.commit("before-move: prepend to one")?;
    builder.git(&["switch", "-q", "main"])
}

fn build_binary(builder: &mut Builder<'_>) -> Result<()> {
    let every_byte: Vec<u8> = (0..=255u8).collect();
    let image: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0";
    builder.write(".gitattributes", b"*.dat binary\n")?;
    builder.write("bytes.dat", &every_byte)?;
    builder.write("pixel.png", image)?;
    builder.write("text.txt", b"plain text next to binary files\n")?;
    builder.commit("add binary files")?;
    let mut reversed = every_byte.clone();
    reversed.reverse();
    builder.write("bytes.dat", &reversed)?;
    builder.write("zeros.bin", &[0u8; 64])?;
    builder.commit("change binary files")
}

fn build_crlf(builder: &mut Builder<'_>) -> Result<()> {
    builder.write(
        ".gitattributes",
        b"* text=auto\n*.crlf.txt text eol=crlf\n*.sh text eol=lf\n*.bin binary\n",
    )?;
    builder.write("plain.txt", b"line one\nline two\n")?;
    builder.write("windows.crlf.txt", b"first\r\nsecond\r\n")?;
    builder.write("script.sh", b"#!/bin/sh\necho lf\n")?;
    builder.write("mixed.txt", b"lf line\ncrlf line\r\n")?;
    builder.write("raw.bin", b"keep\r\nas is\n")?;
    builder.commit("add files with line ending attributes")?;
    builder.write("windows.crlf.txt", b"first\r\nsecond\r\nthird\r\n")?;
    builder.commit("extend windows file")?;
    for path in [
        "plain.txt",
        "windows.crlf.txt",
        "script.sh",
        "mixed.txt",
        "raw.bin",
    ] {
        builder.remove(path)?;
    }
    builder.git(&["checkout", "--", "."])
}

fn build_symlinks(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("target.txt", b"link target\n")?;
    builder.write("dir/inner.txt", b"inner\n")?;
    builder.link("to-file", "target.txt")?;
    builder.link("to-dir", "dir")?;
    builder.link("dir/up", "../target.txt")?;
    builder.link("dangling", "missing.txt")?;
    builder.commit("add links")?;
    builder.remove("to-file")?;
    builder.link("to-file", "dir/inner.txt")?;
    builder.commit("retarget link")
}

fn build_exec_bits(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("run.sh", b"#!/bin/sh\necho run\n")?;
    builder.write("tool.sh", b"#!/bin/sh\necho tool\n")?;
    builder.write("data.txt", b"data\n")?;
    fsutil::set_executable(&builder.repo_dir.join("tool.sh"), true)?;
    builder.commit("add scripts")?;
    fsutil::set_executable(&builder.repo_dir.join("run.sh"), true)?;
    builder.commit("make run executable")?;
    fsutil::set_executable(&builder.repo_dir.join("tool.sh"), false)?;
    builder.write("data.txt", b"data\nmore\n")?;
    builder.commit("drop executable bit of tool")
}

fn build_unicode(builder: &mut Builder<'_>) -> Result<()> {
    builder.git(&["config", "core.quotePath", "false"])?;
    builder.write("caf\u{e9}.txt", b"composed\n")?;
    builder.write("cafe\u{301}.txt", b"decomposed\n")?;
    builder.write("na\u{ef}ve dir/\u{fc}ber.txt", b"umlaut\n")?;
    builder.write("space name.txt", b"space\n")?;
    builder.write("quote\"s.txt", b"quote\n")?;
    builder.write("\u{65e5}\u{672c}/\u{8a9e}.txt", b"cjk\n")?;
    builder.write("\u{17c}\u{f3}\u{142}w.txt", b"polish letters\n")?;
    builder.commit("add names in several scripts")?;
    builder.write("caf\u{e9}.txt", b"composed, changed\n")?;
    builder.commit("change composed name")
}

fn build_case_names(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("README", b"upper\n")?;
    builder.write("readme", b"lower\n")?;
    builder.write("Dir/a.txt", b"upper dir\n")?;
    builder.write("dir/b.txt", b"lower dir\n")?;
    builder.commit("add names differing in case")?;
    builder.write("Readme", b"mixed\n")?;
    builder.commit("add third spelling")
}

fn build_packed_refs(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("file.txt", b"one\n")?;
    builder.commit("first")?;
    for name in ["alpha", "beta", "gamma"] {
        builder.git(&["branch", name])?;
    }
    builder.git(&["tag", "-a", "v0.1", "-m", "first tag"])?;
    builder.write("file.txt", b"one\ntwo\n")?;
    builder.commit("second")?;
    builder.git(&["tag", "v0.2"])?;
    builder.git(&["pack-refs", "--all", "--prune"])?;
    builder.git(&["branch", "loose-after-pack"])
}

fn build_packed_objects(builder: &mut Builder<'_>) -> Result<()> {
    for round in 1..=6 {
        builder.write("data.txt", "row\n".repeat(round * 10).as_bytes())?;
        builder.write(
            &format!("files/f{round}.txt"),
            format!("file {round}\n").as_bytes(),
        )?;
        builder.commit(&format!("round {round}"))?;
    }
    builder.git(&["-c", "pack.threads=1", "gc", "-q", "--prune=now"])
}

fn build_ignored(builder: &mut Builder<'_>) -> Result<()> {
    builder.write(".gitignore", b"*.log\nbuild/\n!keep.log\n")?;
    builder.write("src/main.txt", b"main\n")?;
    builder.write("keep.log", b"kept\n")?;
    builder.commit("add sources and ignore rules")?;
    builder.write("debug.log", b"ignored\n")?;
    builder.write("build/out.bin", b"\x00\x01")?;
    builder.write("notes/draft.txt", b"untracked\n")?;
    builder.write("src/scratch.txt", b"untracked\n")
}

fn build_dirty(builder: &mut Builder<'_>) -> Result<()> {
    for path in ["staged.txt", "modified.txt", "deleted.txt", "both.txt"] {
        builder.write(path, b"base\n")?;
    }
    builder.commit("base")?;
    builder.write("staged.txt", b"base\nstaged\n")?;
    builder.write("both.txt", b"base\nstaged\n")?;
    builder.write("added.txt", b"new and staged\n")?;
    builder.git(&["add", "staged.txt", "both.txt", "added.txt"])?;
    builder.write("both.txt", b"base\nstaged\nunstaged\n")?;
    builder.write("modified.txt", b"base\nunstaged\n")?;
    builder.remove("deleted.txt")?;
    builder.write("untracked.txt", b"untracked\n")
}

fn build_stash(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("work.txt", b"one\n")?;
    builder.commit("base")?;
    builder.write("work.txt", b"one\nfirst stash\n")?;
    builder.commit_with(&["stash", "push", "-q", "-m", "first"])?;
    builder.write("work.txt", b"one\nsecond stash\n")?;
    builder.write("extra.txt", b"untracked in stash\n")?;
    builder.commit_with(&["stash", "push", "-q", "-u", "-m", "second"])
}

fn build_submodule(builder: &mut Builder<'_>) -> Result<()> {
    let library = builder.sibling("library");
    builder.git_in(&library, &["init", "-q", "-b", "main"])?;
    write_below(&library, "lib.txt", b"library\n")?;
    builder.commit_in(&library, "library")?;
    builder.write("app.txt", b"app\n")?;
    builder.commit("app")?;
    builder.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "-q",
        "../library",
        "lib",
    ])?;
    builder.commit("add library as submodule")
}

fn build_shallow(builder: &mut Builder<'_>) -> Result<()> {
    let source = builder.sibling("source");
    builder.git_in(&source, &["init", "-q", "-b", "main"])?;
    for round in 1..=4 {
        write_below(&source, "log.txt", "entry\n".repeat(round).as_bytes())?;
        builder.commit_in(&source, &format!("source {round}"))?;
    }
    builder.git(&["remote", "add", "origin", "../source"])?;
    builder.git(&["fetch", "-q", "--depth", "2", "origin", "main"])?;
    builder.git(&["reset", "-q", "--hard", "origin/main"])?;
    builder.git(&["branch", "-q", "--set-upstream-to=origin/main", "main"])
}

fn build_sparse(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("top.txt", b"top\n")?;
    builder.write("app/main.txt", b"app\n")?;
    builder.write("app/deep/x.txt", b"x\n")?;
    builder.write("docs/guide.txt", b"guide\n")?;
    builder.write("tools/run.txt", b"run\n")?;
    builder.commit("layout")?;
    builder.git(&["sparse-checkout", "set", "--cone", "app"])
}

fn build_merge_in_progress(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("shared.txt", b"line 1\nline 2\nline 3\n")?;
    builder.write("other.txt", b"other\n")?;
    builder.commit("base")?;
    builder.git(&["switch", "-q", "-c", "theirs"])?;
    builder.write("shared.txt", b"line 1\ntheirs\nline 3\n")?;
    builder.write("other.txt", b"other\ntheirs\n")?;
    builder.commit("theirs: change line 2")?;
    builder.git(&["switch", "-q", "main"])?;
    builder.write("shared.txt", b"line 1\nours\nline 3\n")?;
    builder.commit("ours: change line 2")?;
    builder.git_expecting(&["merge", "--no-edit", "theirs"], MERGE_CONFLICT_EXIT)
}

fn build_many_branches(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("base.txt", b"base\n")?;
    builder.commit("base")?;
    for number in 1..=12 {
        let name = format!("topic/{number:02}");
        builder.git(&["switch", "-q", "-c", &name, "main"])?;
        builder.write(
            &format!("topics/{number:02}.txt"),
            format!("topic {number}\n").as_bytes(),
        )?;
        builder.commit(&format!("topic {number}"))?;
    }
    builder.git(&["switch", "-q", "main"])?;
    builder.commit_with(&["merge", "-q", "--no-ff", "-m", "merge topic/01", "topic/01"])
}

/// Appends one `data` command of a `git fast-import` stream.
fn fast_import_data(stream: &mut String, text: &str) {
    stream.push_str(&format!("data {}\n{text}\n", text.len()));
}

/// Committer line of a `git fast-import` stream at `clock`.
fn fast_import_committer(clock: ScenarioClock) -> String {
    let date = clock.git_date();
    format!(
        "committer {IDENTITY_NAME} <{IDENTITY_EMAIL}> {}\n",
        date.trim_start_matches('@')
    )
}

fn build_heavy_history(builder: &mut Builder<'_>) -> Result<()> {
    let mut stream = String::new();
    for number in 0..HEAVY_COMMITS {
        let clock = builder.clock.advanced(COMMIT_INTERVAL * i64::from(number));
        stream.push_str(&format!("commit refs/heads/main\nmark :{}\n", number + 1));
        stream.push_str(&fast_import_committer(clock));
        fast_import_data(&mut stream, &format!("commit {number}\n"));
        if number > 0 {
            stream.push_str(&format!("from :{number}\n"));
        }
        stream.push_str("M 100644 inline history.txt\n");
        fast_import_data(&mut stream, &format!("revision {number}\n"));
    }
    builder.git_with_input(&["fast-import", "--quiet"], stream.as_bytes())?;
    builder.git(&["reset", "-q", "--hard", "main"])
}

fn build_heavy_files(builder: &mut Builder<'_>) -> Result<()> {
    let mut stream = String::from("commit refs/heads/main\n");
    stream.push_str(&fast_import_committer(builder.clock));
    fast_import_data(&mut stream, "many files\n");
    for number in 0..HEAVY_FILES {
        let directory = number / FILES_PER_DIRECTORY;
        stream.push_str(&format!(
            "M 100644 inline d{directory:03}/f{number:06}.txt\n"
        ));
        fast_import_data(&mut stream, &format!("file {number}\n"));
    }
    builder.git_with_input(&["fast-import", "--quiet"], stream.as_bytes())?;
    builder.git(&["reset", "-q", "--hard", "main"])
}

/// Case directory of one repository, built once; the error is kept
/// as text so that every scenario using the repository reports it.
type BuildCell = Arc<OnceLock<Result<PathBuf, String>>>;

/// Corpus repositories built on demand and cached for the test run.
#[derive(Debug)]
pub struct Corpus {
    git: GitProgram,
    root: PathBuf,
    built: Mutex<HashMap<&'static str, BuildCell>>,
}

impl Corpus {
    /// Corpus cached below `root`.
    #[must_use]
    pub fn new(git: GitProgram, root: PathBuf) -> Self {
        Self {
            git,
            root,
            built: Mutex::new(HashMap::new()),
        }
    }

    /// Copies the case directory of repository `name` to `destination`,
    /// building it first when needed.
    ///
    /// # Errors
    ///
    /// Fails with [`DifftestError::Corpus`] when the name is unknown or the
    /// recipe fails, and with [`DifftestError::Io`] when the copy fails.
    pub fn materialize(&self, name: &str, destination: &Path) -> Result<()> {
        let built = self.build(name)?;
        fsutil::copy_tree(&built, destination)
    }

    /// Case directory of repository `name`, built once.
    ///
    /// # Errors
    ///
    /// Fails with [`DifftestError::Corpus`] when the name is unknown or the
    /// recipe fails.
    pub fn build(&self, name: &str) -> Result<PathBuf> {
        let recipe = find(name).ok_or_else(|| DifftestError::Corpus {
            name: name.to_owned(),
            message: "unknown repository, use a name from the corpus table".to_owned(),
        })?;
        let cell = {
            let mut built = self
                .built
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Arc::clone(built.entry(recipe.name).or_default())
        };
        cell.get_or_init(|| self.run_recipe(recipe).map_err(|error| error.to_string()))
            .clone()
            .map_err(|message| DifftestError::Corpus {
                name: name.to_owned(),
                message,
            })
    }

    fn run_recipe(&self, recipe: &CorpusRepo) -> Result<PathBuf> {
        let case_dir = self.root.join(recipe.name);
        let repo_dir = case_dir.join(REPO_DIR);
        let home = self.root.join(".home");
        std::fs::create_dir_all(&home).map_err(io_error("create directory", &home))?;
        let config = home.join(".gitconfig");
        std::fs::write(&config, b"").map_err(io_error("write", &config))?;
        let clock =
            ScenarioClock::parse(CORPUS_EPOCH).map_err(|message| DifftestError::Corpus {
                name: recipe.name.to_owned(),
                message,
            })?;
        let mut builder = Builder {
            git: &self.git,
            env: GitEnv::new(&home, clock),
            case_dir: case_dir.clone(),
            repo_dir,
            clock,
        };
        builder.git(&["init", "-q", "-b", "main"])?;
        (recipe.build)(&mut builder)?;
        Ok(case_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_found() {
        let mut names: Vec<&str> = CORPUS.iter().map(|repo| repo.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CORPUS.len());
        assert!(find("empty").is_some());
        assert!(find("no-such-repository").is_none());
    }

    #[test]
    fn every_recipe_has_a_description_and_platforms() {
        for repo in CORPUS {
            assert!(repo.description.ends_with('.'), "{}", repo.name);
            assert!(!repo.platforms.is_empty(), "{}", repo.name);
        }
        assert!(find("heavy-history").is_some_and(|repo| repo.heavy));
        assert!(find("linear-small").is_some_and(|repo| !repo.heavy));
    }

    #[test]
    fn the_corpus_has_fifty_repositories_three_of_them_heavy() {
        assert_eq!(CORPUS.len(), 50);
        assert_eq!(CORPUS.iter().filter(|repo| repo.heavy).count(), 3);
    }

    #[test]
    fn fast_import_commands_carry_lengths_and_raw_dates() {
        let mut stream = String::new();
        fast_import_data(&mut stream, "abc\n");
        assert_eq!(stream, "data 4\nabc\n\n");
        let line = fast_import_committer(ScenarioClock::default());
        assert_eq!(
            line,
            format!("committer {IDENTITY_NAME} <{IDENTITY_EMAIL}> 1767268800 +0000\n")
        );
    }
}
