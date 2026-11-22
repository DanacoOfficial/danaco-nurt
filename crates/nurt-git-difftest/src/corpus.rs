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

use crate::error::{HarnessError, Result, io_error};
use crate::fsutil;
use crate::git::{GitEnv, GitProgram};
use crate::scenario::Platform;
use crate::time::ScenarioClock;

/// Name of the repository directory inside a case directory.
pub const REPO_DIR: &str = "repo";

/// Date of the first commit of every corpus repository.
const CORPUS_EPOCH: &str = "2025-06-01T09:00:00+02:00";

/// Seconds between two commits of a corpus repository.
const COMMIT_INTERVAL: i64 = 60;

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

/// Every corpus repository, in name order of their groups.
pub const CORPUS: &[CorpusRepo] = &[
    CorpusRepo {
        name: "empty",
        description: "Repository without commits on branch main.",
        platforms: EVERY_PLATFORM,
        heavy: false,
        build: build_empty,
    },
    CorpusRepo {
        name: "linear-small",
        description: "Five commits on main touching files in nested directories, lightweight tag v1.",
        platforms: EVERY_PLATFORM,
        heavy: false,
        build: build_linear,
    },
    CorpusRepo {
        name: "branchy-small",
        description: "Branches main, topic and feature diverging from main, annotated tag v1.0.",
        platforms: EVERY_PLATFORM,
        heavy: false,
        build: build_branchy,
    },
    CorpusRepo {
        name: "criss-cross",
        description: "Branches left and right merged into each other, so they have two merge bases.",
        platforms: EVERY_PLATFORM,
        heavy: false,
        build: build_criss_cross,
    },
    CorpusRepo {
        name: "renames",
        description: "History with a renamed directory and a renamed and edited file, branch before-move.",
        platforms: EVERY_PLATFORM,
        heavy: false,
        build: build_renames,
    },
    CorpusRepo {
        name: "binary",
        description: "Binary files with NUL bytes and every byte value, changed in a second commit.",
        platforms: EVERY_PLATFORM,
        heavy: false,
        build: build_binary,
    },
    CorpusRepo {
        name: "crlf",
        description: "Attributes text=auto, eol=crlf and eol=lf with files checked out after conversion.",
        platforms: EVERY_PLATFORM,
        heavy: false,
        build: build_crlf,
    },
    CorpusRepo {
        name: "symlinks",
        description: "Links to a file, to a directory, across directories and a dangling link.",
        platforms: UNIX_PLATFORMS,
        heavy: false,
        build: build_symlinks,
    },
    CorpusRepo {
        name: "exec-bits",
        description: "Scripts gaining and losing the executable bit across commits.",
        platforms: UNIX_PLATFORMS,
        heavy: false,
        build: build_exec_bits,
    },
    CorpusRepo {
        name: "unicode-names",
        description: "Names in NFC and NFD form side by side, with spaces, quotes and non-Latin scripts.",
        platforms: LINUX_ONLY,
        heavy: false,
        build: build_unicode,
    },
    CorpusRepo {
        name: "case-names",
        description: "Files and directories whose names differ only in letter case.",
        platforms: LINUX_ONLY,
        heavy: false,
        build: build_case_names,
    },
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

    fn git_in(&self, dir: &Path, args: &[&str]) -> Result<()> {
        let env = self.env.with_clock(self.clock);
        self.git.run_ok(&env, dir, args).map(drop)
    }

    /// Writes a file of the working tree, creating its directories.
    fn write(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let full = self.repo_dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(io_error("create directory", parent))?;
        }
        std::fs::write(&full, bytes).map_err(io_error("write", &full))
    }

    /// Stages every change and commits with `message`, then advances the
    /// clock.
    fn commit(&mut self, message: &str) -> Result<()> {
        self.git(&["add", "-A"])?;
        self.git(&["commit", "-q", "-m", message])?;
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

fn build_empty(builder: &mut Builder<'_>) -> Result<()> {
    let _ = builder;
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
    builder.write("README.md", b"# Sample\n\nA small linear history with five commits.\n")?;
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
    let body: Vec<u8> = (1..=20).flat_map(|n| format!("line {n}\n").into_bytes()).collect();
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
    for path in ["plain.txt", "windows.crlf.txt", "script.sh", "mixed.txt", "raw.bin"] {
        builder.remove(path)?;
    }
    builder.git(&["checkout", "--", "."])
}

fn build_symlinks(builder: &mut Builder<'_>) -> Result<()> {
    builder.write("target.txt", b"link target\n")?;
    builder.write("dir/inner.txt", b"inner\n")?;
    let link = |path: &str, target: &str| {
        fsutil::make_symlink(Path::new(target), &builder.repo_dir.join(path), false)
    };
    link("to-file", "target.txt")?;
    link("to-dir", "dir")?;
    link("dir/up", "../target.txt")?;
    link("dangling", "missing.txt")?;
    builder.commit("add links")?;
    builder.remove("to-file")?;
    link("to-file", "dir/inner.txt")?;
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

/// Corpus repositories built on demand and cached for the test run.
#[derive(Debug)]
pub struct Corpus {
    git: GitProgram,
    root: PathBuf,
    built: Mutex<HashMap<&'static str, Arc<OnceLock<Result<PathBuf, String>>>>>,
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
    /// Fails with [`HarnessError::Corpus`] when the name is unknown or the
    /// recipe fails, and with [`HarnessError::Io`] when the copy fails.
    pub fn materialize(&self, name: &str, destination: &Path) -> Result<()> {
        let built = self.build(name)?;
        fsutil::copy_tree(&built, destination)
    }

    /// Case directory of repository `name`, built once.
    ///
    /// # Errors
    ///
    /// Fails with [`HarnessError::Corpus`] when the name is unknown or the
    /// recipe fails.
    pub fn build(&self, name: &str) -> Result<PathBuf> {
        let recipe = find(name).ok_or_else(|| HarnessError::Corpus {
            name: name.to_owned(),
            message: "unknown repository, use a name from the corpus table".to_owned(),
        })?;
        let cell = {
            let mut built = self.built.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            Arc::clone(built.entry(recipe.name).or_default())
        };
        cell.get_or_init(|| self.run_recipe(recipe).map_err(|error| error.to_string()))
            .clone()
            .map_err(|message| HarnessError::Corpus {
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
        std::fs::create_dir_all(&repo_dir).map_err(io_error("create directory", &repo_dir))?;
        let clock = ScenarioClock::parse(CORPUS_EPOCH).map_err(|message| HarnessError::Corpus {
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
        let _ = &builder.case_dir;
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
    }
}
