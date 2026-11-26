// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use nurt_contracts::fs::{AbsPath, FileKind, Follow, Fs, FsError, FsErrorKind};

use super::filter::{GIT_DIR, nfc_path, nfc_text};

/// Ignore file read in every directory.
pub(super) const IGNORE_FILE: &str = ".ignore";

/// Ignore file read in directories of a git repository.
pub(super) const GITIGNORE_FILE: &str = ".gitignore";

/// Exclude file of a repository, relative to its `.git` directory.
const INFO_EXCLUDE: [&str; 2] = ["info", "exclude"];

/// Git repository whose working tree contains the walked entries.
pub(super) struct Repo {
    dir: AbsPath,
    exclude: Option<Gitignore>,
}

/// Ignore rules in effect in one directory, linked to the rules of its
/// parent.
pub(super) struct Frame {
    parent: Option<Arc<Frame>>,
    ignore: Option<Gitignore>,
    gitignore: Option<Gitignore>,
    repo: Option<Arc<Repo>>,
    repo_root: bool,
}

/// Kind of the `.git` entry of a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GitMarker {
    /// A repository directory; `info/exclude` is read from it.
    Directory,
    /// A link file of a worktree or a submodule.
    File,
}

/// Ignore-related names present in a directory.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Markers {
    pub(super) git: Option<GitMarker>,
    pub(super) ignore: bool,
    pub(super) gitignore: bool,
}

impl Markers {
    pub(super) fn is_empty(&self) -> bool {
        self.git.is_none() && !self.ignore && !self.gitignore
    }
}

/// Builds the rules of `dir`, given the rules of its parent. Returns the
/// parent rules unchanged when `dir` adds none.
pub(super) fn frame_for(
    fs: &dyn Fs,
    parent: Option<&Arc<Frame>>,
    dir: &AbsPath,
    markers: Markers,
    problems: &mut Vec<(AbsPath, FsError)>,
) -> Option<Arc<Frame>> {
    if markers.is_empty() {
        return parent.cloned();
    }
    let repo_root = markers.git.is_some();
    let repo = if repo_root {
        let exclude = if markers.git == Some(GitMarker::Directory) {
            child_path(dir, &[GIT_DIR, INFO_EXCLUDE[0], INFO_EXCLUDE[1]])
                .and_then(|path| read_rules(fs, dir, &path, problems))
        } else {
            None
        };
        Some(Arc::new(Repo {
            dir: dir.clone(),
            exclude,
        }))
    } else {
        parent.and_then(|frame| frame.repo.clone())
    };
    let ignore = markers
        .ignore
        .then(|| child_path(dir, &[IGNORE_FILE]))
        .flatten()
        .and_then(|path| read_rules(fs, dir, &path, problems));
    let gitignore = (markers.gitignore && repo.is_some())
        .then(|| child_path(dir, &[GITIGNORE_FILE]))
        .flatten()
        .and_then(|path| read_rules(fs, dir, &path, problems));
    if ignore.is_none() && gitignore.is_none() && !repo_root {
        return parent.cloned();
    }
    Some(Arc::new(Frame {
        parent: parent.cloned(),
        ignore,
        gitignore,
        repo,
        repo_root,
    }))
}

/// Rules of the ancestors of `root`, from the file system root down to the
/// parent of `root`.
pub(super) fn ancestor_frames(
    fs: &dyn Fs,
    root: &AbsPath,
    problems: &mut Vec<(AbsPath, FsError)>,
) -> Option<Arc<Frame>> {
    let mut ancestors = Vec::new();
    let mut current = root.parent();
    while let Some(dir) = current {
        current = dir.parent();
        ancestors.push(dir);
    }
    let mut frame = None;
    for dir in ancestors.iter().rev() {
        let markers = probe_markers(fs, dir);
        frame = frame_for(fs, frame.as_ref(), dir, markers, problems);
    }
    frame
}

/// Finds the ignore-related names of a directory that was not listed.
fn probe_markers(fs: &dyn Fs, dir: &AbsPath) -> Markers {
    let exists = |name: &str| {
        dir.join_name(name)
            .ok()
            .and_then(|path| fs.metadata(&path, Follow::NoFollow).ok())
    };
    let git = exists(GIT_DIR).map(|meta| {
        if meta.kind == FileKind::Directory {
            GitMarker::Directory
        } else {
            GitMarker::File
        }
    });
    Markers {
        git,
        ignore: exists(IGNORE_FILE).is_some(),
        gitignore: exists(GITIGNORE_FILE).is_some(),
    }
}

impl Frame {
    /// Whether the rules exclude `path`, which lies below the directory of
    /// this frame.
    ///
    /// Names are compared in Unicode normalization form C, as the patterns
    /// are stored.
    pub(super) fn is_ignored(&self, path: &Path, is_dir: bool, global: Option<&Gitignore>) -> bool {
        let composed = nfc_path(path);
        let path = &*composed;
        for frame in self.chain() {
            if let Some(ignored) = frame
                .ignore
                .as_ref()
                .and_then(|m| decision(m.matched(path, is_dir)))
            {
                return ignored;
            }
        }
        for frame in self.chain() {
            if let Some(ignored) = frame
                .gitignore
                .as_ref()
                .and_then(|m| decision(m.matched(path, is_dir)))
            {
                return ignored;
            }
            if frame.repo_root {
                break;
            }
        }
        let Some(repo) = &self.repo else {
            return false;
        };
        if let Some(ignored) = repo
            .exclude
            .as_ref()
            .and_then(|m| decision(m.matched(path, is_dir)))
        {
            return ignored;
        }
        let repo_dir = nfc_path(repo.dir.as_path());
        let relative = path.strip_prefix(&*repo_dir).unwrap_or(path);
        global.is_some_and(|matcher| matcher.matched(relative, is_dir).is_ignore())
    }

    fn chain(&self) -> impl Iterator<Item = &Frame> {
        std::iter::successors(Some(self), |frame| frame.parent.as_deref())
    }
}

/// Decision of one matcher: `Some(true)` to exclude, `Some(false)` to keep,
/// `None` when no pattern matched.
fn decision<T>(matched: Match<T>) -> Option<bool> {
    match matched {
        Match::Ignore(_) => Some(true),
        Match::Whitelist(_) => Some(false),
        Match::None => None,
    }
}

/// Reads the global exclude file; the matcher is rooted at the empty path
/// and is matched against paths relative to the repository root.
pub(super) fn read_global(fs: &dyn Fs, path: &AbsPath) -> Result<Option<Gitignore>, FsError> {
    match read_all(fs, path) {
        Ok(bytes) => Ok(build_rules(Path::new(""), path, &bytes)),
        Err(error) if error.kind == FsErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_rules(
    fs: &dyn Fs,
    dir: &AbsPath,
    path: &AbsPath,
    problems: &mut Vec<(AbsPath, FsError)>,
) -> Option<Gitignore> {
    match read_all(fs, path) {
        Ok(bytes) => build_rules(dir.as_path(), path, &bytes),
        Err(error) if error.kind == FsErrorKind::NotFound => None,
        Err(error) => {
            problems.push((path.clone(), error));
            None
        }
    }
}

/// Byte order mark of UTF-8 text.
const UTF8_MARK: &[u8] = b"\xEF\xBB\xBF";

fn build_rules(root: &Path, source: &AbsPath, bytes: &[u8]) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(nfc_path(root));
    // Git skips a byte order mark at the start of an exclude file; kept, it
    // would become part of the first pattern.
    let bytes = bytes.strip_prefix(UTF8_MARK).unwrap_or(bytes);
    for line in String::from_utf8_lossy(bytes).lines() {
        // Git skips a pattern it cannot parse and applies the remaining
        // ones, so the error of a single line is dropped here as well.
        let _ = builder.add_line(Some(source.as_path().to_path_buf()), &nfc_text(line));
    }
    builder.build().ok().filter(|rules| !rules.is_empty())
}

fn read_all(fs: &dyn Fs, path: &AbsPath) -> Result<Vec<u8>, FsError> {
    let mut handle = fs.open_read(path)?;
    let mut bytes = Vec::new();
    handle.read_to_end(&mut bytes).map_err(|error| {
        let mut failure = FsError::new(FsErrorKind::Io, "read", Some(path.as_path()));
        failure.os_code = error.raw_os_error();
        failure
    })?;
    Ok(bytes)
}

fn child_path(dir: &AbsPath, names: &[&str]) -> Option<AbsPath> {
    names
        .iter()
        .try_fold(dir.clone(), |path, name| path.join_name(name).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPOSED: &str = "caf\u{e9}.txt";
    const DECOMPOSED: &str = "cafe\u{301}.txt";

    fn frame(root: &str, source: &str, rules: &str) -> Frame {
        let source = AbsPath::new(source).expect("absolute source");
        Frame {
            parent: None,
            ignore: build_rules(Path::new(root), &source, rules.as_bytes()),
            gitignore: None,
            repo: None,
            repo_root: false,
        }
    }

    #[test]
    fn composed_rule_ignores_decomposed_name() {
        let frame = frame("/work", "/work/.ignore", &format!("{COMPOSED}\n"));
        let path = Path::new("/work").join(DECOMPOSED);
        assert!(frame.is_ignored(&path, false, None));
    }

    #[test]
    fn decomposed_rule_ignores_composed_name() {
        let frame = frame("/work", "/work/.ignore", &format!("{DECOMPOSED}\n"));
        let path = Path::new("/work").join(COMPOSED);
        assert!(frame.is_ignored(&path, false, None));
    }

    #[test]
    fn decomposed_root_directory_matches_relative_rule() {
        let root = format!("/{DECOMPOSED}");
        let frame = frame(&root, &format!("{root}/.ignore"), "*.log\n");
        let path = Path::new(&root).join("a.log");
        assert!(frame.is_ignored(&path, false, None));
    }

    #[test]
    fn a_byte_order_mark_is_not_part_of_the_first_pattern() {
        let source = AbsPath::new("/work/.ignore").expect("absolute source");
        let rules = build_rules(Path::new("/work"), &source, b"\xEF\xBB\xBF*.log\nbuild/\n");
        let frame = Frame {
            parent: None,
            ignore: rules,
            gitignore: None,
            repo: None,
            repo_root: false,
        };
        assert!(frame.is_ignored(Path::new("/work/a.log"), false, None));
        assert!(frame.is_ignored(Path::new("/work/build"), true, None));
    }

    #[test]
    fn unrelated_name_stays_visible() {
        let frame = frame("/work", "/work/.ignore", &format!("{COMPOSED}\n"));
        assert!(!frame.is_ignored(Path::new("/work/cafe.txt"), false, None));
    }

    #[test]
    fn global_rules_match_names_in_either_form() {
        let source = AbsPath::new("/config/ignore").expect("absolute source");
        let global = build_rules(Path::new(""), &source, format!("{COMPOSED}\n").as_bytes());
        let repo = Arc::new(Repo {
            dir: AbsPath::new("/work").expect("absolute repository"),
            exclude: None,
        });
        let frame = Frame {
            parent: None,
            ignore: None,
            gitignore: None,
            repo: Some(repo),
            repo_root: true,
        };
        let path = Path::new("/work").join(DECOMPOSED);
        assert!(frame.is_ignored(&path, false, global.as_ref()));
    }
}
