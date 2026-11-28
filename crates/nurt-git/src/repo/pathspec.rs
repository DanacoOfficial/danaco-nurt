// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::repo::pathspec
// Responsibility:   Path selection of the `git` commands: the `paths` and `exclude` parameters, and the paths a working tree may contain.
// Dependencies:     bstr, nurt_contracts, nurt_testkit, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Path selection of the `git` commands: the `paths` and `exclude`
//! parameters, and the paths a working tree may contain.
//!
//! Paths given to a command are relative to the root of the working tree,
//! whatever the current directory is; an absolute path inside the working
//! tree is accepted and made relative. A path without glob characters
//! selects the file of that name and everything below the directory of that
//! name, as a `git` pathspec does. A path with `*`, `?` or `[` is a glob in
//! which `*` also matches `/`, as in `git`.
//!
//! An `exclude` pattern follows the syntax of `.gitignore`: a pattern
//! without `/` matches the name of a file or directory at any depth, a
//! pattern with `/` is anchored at the root, `**` matches any number of
//! directories, and an excluded directory excludes everything below it.

use std::path::{Component, Path};

use bstr::{BStr, BString, ByteSlice};
use nurt_contracts::error::{Error, ErrorCode};
use serde_json::Value;

/// Characters that make a path a glob.
const GLOB_CHARS: [u8; 3] = *b"*?[";

/// Code points that macOS ignores when it compares file names, which make a
/// name equal to `.git` on HFS+.
const HFS_IGNORABLE: [char; 16] = [
    '\u{200C}', '\u{200D}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
    '\u{202E}', '\u{206A}', '\u{206B}', '\u{206C}', '\u{206D}', '\u{206E}', '\u{206F}', '\u{FEFF}',
];

/// Short name that Windows may give to `.git` on NTFS.
const NTFS_SHORT_NAME: &str = "git~1";

/// Stream of an NTFS name, after the first `:`, that opens a directory.
const NTFS_DIRECTORY_STREAM: &str = ":$index_allocation";

/// Converts a path given to a command into a path relative to the working
/// tree root `root`, with `/` separators and without `.` components.
///
/// `\` is accepted as a separator. The empty path and `.` are the root
/// itself and give an empty path.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` (problem `outside_repository`) when the path
/// is absolute and not below `root`, or when `..` leaves the root.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use nurt_git::repo::pathspec::relative_path;
///
/// # let drive = if cfg!(windows) { "C:" } else { "" };
/// let root_text = format!("{drive}/work/app");
/// let root = Path::new(&root_text);
/// assert_eq!(relative_path(root, "src/./main.rs").unwrap(), "src/main.rs");
/// assert_eq!(relative_path(root, &format!("{drive}/work/app/docs/")).unwrap(), "docs");
/// assert!(relative_path(root, "../other").is_err());
/// ```
pub fn relative_path(root: &Path, path: &str) -> Result<BString, Error> {
    let text = path.replace('\\', "/");
    let relative = if Path::new(&text).is_absolute() || is_windows_absolute(&text) {
        let root_text = root.to_string_lossy().replace('\\', "/");
        let root_text = root_text.trim_end_matches('/');
        match text.strip_prefix(root_text) {
            Some("") => String::new(),
            Some(rest) if rest.starts_with('/') => rest.to_owned(),
            _ => return Err(outside(root, path)),
        }
    } else {
        text
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(outside(root, path));
                }
            }
            part => parts.push(part),
        }
    }
    Ok(BString::from(parts.join("/")))
}

fn is_windows_absolute(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn outside(root: &Path, path: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!(
            "path {path} is outside the working tree {}; pass a path relative to the repository root",
            root.display()
        ),
    )
    .with_problem("outside_repository")
    .with_found(path)
    .with_detail("root", Value::from(root.to_string_lossy().into_owned()))
}

/// One entry of the `paths` parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Include {
    /// The file or directory of this name.
    Literal(BString),
    /// A glob in which `*` matches `/`.
    Glob(BString),
}

/// Selection of paths by the `paths` and `exclude` parameters.
///
/// An empty `paths` list selects every path; `exclude` removes paths from
/// the selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathFilter {
    includes: Vec<Include>,
    excludes: Vec<BString>,
}

impl PathFilter {
    /// Filter that selects every path.
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// Filter of the `paths` and `exclude` parameters of a command working
    /// in the working tree rooted at `root`.
    ///
    /// # Errors
    ///
    /// Fails as [`relative_path`] for a literal path outside the working
    /// tree.
    pub fn new(root: &Path, paths: &[String], exclude: &[String]) -> Result<Self, Error> {
        let mut includes = Vec::with_capacity(paths.len());
        for path in paths {
            if path.bytes().any(|byte| GLOB_CHARS.contains(&byte)) {
                let glob = path.replace('\\', "/");
                includes.push(Include::Glob(BString::from(glob.trim_start_matches("./"))));
                continue;
            }
            let relative = relative_path(root, path)?;
            if relative.is_empty() {
                return Ok(Self {
                    includes: Vec::new(),
                    excludes: exclude_patterns(exclude),
                });
            }
            includes.push(Include::Literal(relative));
        }
        Ok(Self {
            includes,
            excludes: exclude_patterns(exclude),
        })
    }

    /// Whether the filter selects every path.
    #[must_use]
    pub fn is_all(&self) -> bool {
        self.includes.is_empty() && self.excludes.is_empty()
    }

    /// Literal paths of the `paths` parameter, in the given order.
    pub fn literal_paths(&self) -> impl Iterator<Item = &BStr> {
        self.includes.iter().filter_map(|include| match include {
            Include::Literal(path) => Some(path.as_bstr()),
            Include::Glob(_) => None,
        })
    }

    /// Whether the file `path` (relative to the root) is selected.
    #[must_use]
    pub fn matches(&self, path: &BStr) -> bool {
        self.included(path) && !self.excluded(path)
    }

    /// Whether the directory `dir` may contain selected paths, so that a
    /// walk of the working tree descends into it.
    #[must_use]
    pub fn may_contain(&self, dir: &BStr) -> bool {
        if self.excluded(dir) {
            return false;
        }
        if self.includes.is_empty() {
            return true;
        }
        self.includes.iter().any(|include| match include {
            Include::Literal(path) => {
                is_below(path.as_bstr(), dir) || is_below(dir, path.as_bstr())
            }
            Include::Glob(glob) => {
                let prefix = literal_dir_prefix(glob.as_bstr());
                prefix.is_empty() || is_below(prefix, dir) || is_below(dir, prefix)
            }
        })
    }

    /// Whether an `exclude` pattern matches `path` or one of its parent
    /// directories.
    #[must_use]
    pub fn excluded(&self, path: &BStr) -> bool {
        self.excludes
            .iter()
            .any(|pattern| exclude_matches(pattern.as_bstr(), path))
    }

    fn included(&self, path: &BStr) -> bool {
        self.includes.is_empty()
            || self.includes.iter().any(|include| match include {
                Include::Literal(literal) => is_below(path, literal.as_bstr()),
                Include::Glob(glob) => {
                    gix::glob::wildmatch(glob.as_bstr(), path, gix::glob::wildmatch::Mode::empty())
                }
            })
    }
}

fn exclude_patterns(exclude: &[String]) -> Vec<BString> {
    exclude
        .iter()
        .map(|pattern| pattern.replace('\\', "/"))
        .map(|pattern| pattern.trim_end_matches('/').to_owned())
        .filter(|pattern| !pattern.is_empty())
        .map(BString::from)
        .collect()
}

/// Whether `path` is `ancestor` or lies below it.
fn is_below(path: &BStr, ancestor: &BStr) -> bool {
    path == ancestor
        || (path.starts_with(ancestor.as_bytes()) && path.get(ancestor.len()) == Some(&b'/'))
}

/// Directory part of a glob before its first glob character.
fn literal_dir_prefix(glob: &BStr) -> &BStr {
    let first = glob
        .iter()
        .position(|byte| GLOB_CHARS.contains(byte))
        .unwrap_or(glob.len());
    match glob[..first].rfind_byte(b'/') {
        Some(slash) => glob[..slash].as_bstr(),
        None => b"".as_bstr(),
    }
}

fn exclude_matches(pattern: &BStr, path: &BStr) -> bool {
    let anchored = pattern.contains(&b'/');
    let pattern = pattern.strip_prefix(b"/").unwrap_or(pattern).as_bstr();
    let mode = gix::glob::wildmatch::Mode::NO_MATCH_SLASH_LITERAL;
    let mut end = 0;
    loop {
        let next = path[end..].find_byte(b'/').map(|offset| end + offset);
        let candidate = path[..next.unwrap_or(path.len())].as_bstr();
        let subject = if anchored {
            candidate
        } else {
            candidate
                .rfind_byte(b'/')
                .map_or(candidate, |slash| candidate[slash + 1..].as_bstr())
        };
        if gix::glob::wildmatch(pattern, subject, mode) {
            return true;
        }
        match next {
            Some(slash) => end = slash + 1,
            None => return false,
        }
    }
}

/// Whether `component` may be a name in a working tree: not empty, not `.`
/// or `..`, without `/`, `\` or NUL, and not a name that a file system of
/// any supported system treats as `.git` (case-insensitive comparison,
/// trailing dots and spaces of NTFS, the NTFS short name `git~1`, and code
/// points that HFS+ ignores).
///
/// # Examples
///
/// ```
/// use nurt_git::repo::pathspec::is_safe_component;
///
/// assert!(is_safe_component("src".into()));
/// assert!(!is_safe_component(".GIT".into()));
/// assert!(!is_safe_component(".git. ".into()));
/// assert!(!is_safe_component("..".into()));
/// ```
#[must_use]
pub fn is_safe_component(component: &BStr) -> bool {
    if component.is_empty() || component == "." || component == ".." {
        return false;
    }
    if component
        .iter()
        .any(|byte| matches!(byte, b'/' | b'\\' | 0))
    {
        return false;
    }
    let text = component.to_str_lossy();
    let folded: String = text
        .chars()
        .filter(|ch| !HFS_IGNORABLE.contains(ch))
        .collect::<String>()
        .to_ascii_lowercase();
    let (stem, stream) = match folded.split_once(':') {
        Some((stem, stream)) => (stem, Some(stream)),
        None => (folded.as_str(), None),
    };
    let trimmed = stem.trim_end_matches(['.', ' ']);
    // `.git::$INDEX_ALLOCATION` names the directory `.git` on NTFS.
    let names_directory =
        stream.is_none_or(|stream| stream.trim_end_matches(['.', ' ']) == NTFS_DIRECTORY_STREAM);
    !((trimmed == ".git" && names_directory) || trimmed == NTFS_SHORT_NAME)
}

/// Whether every component of the relative path `path` passes
/// [`is_safe_component`], so that writing it cannot leave the working tree
/// or reach the repository directory.
#[must_use]
pub fn is_safe_path(path: &BStr) -> bool {
    !path.is_empty()
        && path
            .split_str("/")
            .all(|component| is_safe_component(component.as_bstr()))
}

/// Whether the file system path `path` has a component that is a parent
/// directory reference, so that it cannot be joined to a root safely.
#[must_use]
pub fn has_parent_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(paths: &[&str], exclude: &[&str]) -> PathFilter {
        let owned = |items: &[&str]| {
            items
                .iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        };
        PathFilter::new(Path::new("/r"), &owned(paths), &owned(exclude)).unwrap()
    }

    #[test]
    fn relative_paths_are_normalized_against_the_root() {
        let root = &nurt_testkit::paths::abs_path("/r");
        assert_eq!(relative_path(root, "a/b/../c").unwrap(), "a/c");
        assert_eq!(relative_path(root, r"a\b").unwrap(), "a/b");
        assert_eq!(relative_path(root, ".").unwrap(), "");
        assert_eq!(
            relative_path(root, nurt_testkit::paths::abs("/r").as_str()).unwrap(),
            ""
        );
        assert_eq!(
            relative_path(root, nurt_testkit::paths::abs("/r/x").as_str()).unwrap(),
            "x"
        );
    }

    #[test]
    fn paths_outside_the_root_are_rejected() {
        let root = &nurt_testkit::paths::abs_path("/r");
        let error = relative_path(root, nurt_testkit::paths::abs("/rx/a").as_str()).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidValue);
        assert_eq!(error.problem.as_deref(), Some("outside_repository"));
        assert!(relative_path(root, "a/../../b").is_err());
    }

    #[test]
    fn literal_paths_select_files_and_directory_contents() {
        let selection = filter(&["src", "README.md"], &[]);
        assert!(selection.matches("src/main.rs".into()));
        assert!(selection.matches("README.md".into()));
        assert!(!selection.matches("srcx/a".into()));
        assert!(!selection.matches("docs/a.md".into()));
    }

    #[test]
    fn globs_match_across_directories() {
        let selection = filter(&["src/*.rs"], &[]);
        assert!(selection.matches("src/a.rs".into()));
        assert!(selection.matches("src/deep/b.rs".into()));
        assert!(!selection.matches("lib/a.rs".into()));
    }

    #[test]
    fn root_path_selects_everything() {
        let selection = filter(&["."], &[]);
        assert!(selection.is_all());
        assert!(selection.matches("any/path".into()));
    }

    #[test]
    fn exclude_patterns_follow_gitignore_rules() {
        let selection = filter(&[], &[".env", "build/", "/docs/*.md", "**/*.lock"]);
        assert!(selection.excluded(".env".into()));
        assert!(selection.excluded("app/.env".into()));
        assert!(selection.excluded("build/out/a.o".into()));
        assert!(selection.excluded("docs/a.md".into()));
        assert!(!selection.excluded("app/docs/a.md".into()));
        assert!(selection.excluded("x/y/Cargo.lock".into()));
        assert!(!selection.excluded("src/main.rs".into()));
        assert!(!selection.matches("app/.env".into()));
    }

    #[test]
    fn walks_descend_only_where_selected_paths_may_be() {
        let selection = filter(&["src/net", "docs/*.md"], &["target"]);
        assert!(selection.may_contain("src".into()));
        assert!(selection.may_contain("src/net/http".into()));
        assert!(!selection.may_contain("src/fs".into()));
        assert!(selection.may_contain("docs".into()));
        assert!(!selection.may_contain("target".into()));
        assert!(filter(&["*.rs"], &[]).may_contain("anything".into()));
    }

    #[test]
    fn literal_paths_are_listed_in_order() {
        let selection = filter(&["b", "*.x", "a"], &[]);
        let listed: Vec<&BStr> = selection.literal_paths().collect();
        assert_eq!(listed, vec![BStr::new("b"), BStr::new("a")]);
    }

    #[test]
    fn repository_directory_names_are_unsafe_on_every_file_system() {
        assert!(!is_safe_component(".Git".into()));
        assert!(!is_safe_component("GIT~1".into()));
        assert!(!is_safe_component(".git...".into()));
        assert!(!is_safe_component(".g\u{200C}it".into()));
        assert!(!is_safe_component(".git::$INDEX_ALLOCATION".into()));
        assert!(!is_safe_component(".GIT.::$index_allocation ".into()));
        assert!(is_safe_component(".git:stream".into()));
        assert!(!is_safe_component("a\\b".into()));
        assert!(is_safe_component(".gitignore".into()));
        assert!(is_safe_path("src/.gitignore".into()));
        assert!(!is_safe_path("src/../x".into()));
        assert!(!is_safe_path("a/.git/config".into()));
        assert!(!is_safe_path("".into()));
    }

    #[test]
    fn parent_components_are_detected() {
        assert!(has_parent_component(Path::new("a/../b")));
        assert!(!has_parent_component(Path::new("a/b")));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
