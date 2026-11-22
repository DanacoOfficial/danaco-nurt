// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use globset::{Candidate, GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{FileKind, FileMeta};
use nurt_contracts::services::ExcludePreset;
use unicode_normalization::{UnicodeNormalization, is_nfc};

use super::KindFilter;

/// Directories excluded by the `vcs` preset. A `.git` file, which links a
/// worktree or a submodule to its repository, is excluded as well.
pub const VCS_DIRS: &[&str] = &[".git", ".hg", ".svn"];

/// Directories excluded by the `deps` preset.
pub const DEPS_DIRS: &[&str] = &["node_modules", "vendor", ".venv", "venv"];

/// Directories excluded by the `build` preset.
pub const BUILD_DIRS: &[&str] = &["target", "dist", "build", "out"];

/// Directories excluded by the `cache` preset.
pub const CACHE_DIRS: &[&str] = &[".cache", "__pycache__", ".pytest_cache"];

/// Name of the git metadata entry.
pub(super) const GIT_DIR: &str = ".git";

/// `FILE_ATTRIBUTE_HIDDEN` of the Windows file attributes.
const WINDOWS_HIDDEN_ATTRIBUTE: u32 = 0x2;

/// Directory names excluded by a preset.
///
/// ```
/// use nurt_contracts::services::ExcludePreset;
/// use nurt_files::walk::preset_names;
///
/// assert!(preset_names(ExcludePreset::Deps).contains(&"node_modules"));
/// ```
#[must_use]
pub fn preset_names(preset: ExcludePreset) -> &'static [&'static str] {
    match preset {
        ExcludePreset::Vcs => VCS_DIRS,
        ExcludePreset::Deps => DEPS_DIRS,
        ExcludePreset::Build => BUILD_DIRS,
        ExcludePreset::Cache => CACHE_DIRS,
        _ => &[],
    }
}

pub(super) fn preset_list(presets: &[ExcludePreset]) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = presets
        .iter()
        .flat_map(|preset| preset_names(*preset).iter().copied())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Whether a preset excludes the entry. Presets name directories; the only
/// file they exclude is a `.git` file under the `vcs` preset.
pub(super) fn excluded_by_preset(presets: &[&str], name: &OsStr, is_dir: bool) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    if !presets.contains(&name) {
        return false;
    }
    is_dir || name == GIT_DIR
}

/// Whether an entry is hidden: its name starts with `.`, or, on Windows, it
/// carries the hidden attribute.
pub(super) fn is_hidden(name: &OsStr, meta: Option<&FileMeta>) -> bool {
    if name.as_encoded_bytes().first() == Some(&b'.') {
        return true;
    }
    cfg!(windows)
        && meta.is_some_and(|meta| meta.windows_attributes & WINDOWS_HIDDEN_ATTRIBUTE != 0)
}

/// Whether reading metadata is needed to decide [`is_hidden`].
pub(super) fn hidden_needs_metadata(hidden: bool) -> bool {
    !hidden && cfg!(windows)
}

pub(super) fn kind_matches(filter: KindFilter, kind: FileKind, target: Option<FileKind>) -> bool {
    let is_link = matches!(kind, FileKind::Symlink | FileKind::Junction);
    let effective = target.unwrap_or(kind);
    match filter {
        KindFilter::File => effective == FileKind::File,
        KindFilter::Directory => effective == FileKind::Directory,
        KindFilter::Link => is_link && target.is_none(),
        KindFilter::Any => true,
    }
}

/// Compiles glob patterns into one set: `*` and `?` do not match `/`, `\`
/// escapes, letter case as `case_sensitive`. Patterns are compiled in NFC,
/// the form [`glob_matches`] gives the paths.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` for a pattern that is not valid; `found`
/// holds the pattern as given.
pub(crate) fn compile_globs(patterns: &[String], case_sensitive: bool) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let composed = nfc_text(pattern);
        let glob = GlobBuilder::new(&composed)
            .literal_separator(true)
            .backslash_escape(true)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|error| invalid_pattern("glob", pattern, &error.kind().to_string()))?;
        builder.add(glob);
    }
    builder
        .build()
        .map(Some)
        .map_err(|error| invalid_pattern("glob", &patterns.join(", "), &error.to_string()))
}

/// Compiles exclusion patterns into one matcher rooted at the empty path, so
/// that it matches paths relative to any root. Patterns are compiled in NFC,
/// the form [`excluded_by_patterns`] gives the paths.
pub(super) fn compile_exclude(
    patterns: &[String],
    case_sensitive: bool,
) -> Result<Option<Gitignore>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GitignoreBuilder::new("");
    builder
        .case_insensitive(!case_sensitive)
        .map_err(|error| invalid_pattern("exclude pattern", "", &error.to_string()))?;
    for pattern in patterns {
        builder
            .add_line(None, &nfc_text(pattern))
            .map_err(|error| invalid_pattern("exclude pattern", pattern, &error.to_string()))?;
    }
    builder.build().map(Some).map_err(|error| {
        invalid_pattern("exclude pattern", &patterns.join(", "), &error.to_string())
    })
}

/// Whether an exclusion pattern excludes the relative path, compared in NFC.
pub(super) fn excluded_by_patterns(
    exclude: Option<&Gitignore>,
    relative: &Path,
    is_dir: bool,
) -> bool {
    exclude.is_some_and(|matcher| matcher.matched(&*nfc_path(relative), is_dir).is_ignore())
}

/// Whether the relative path matches one of the globs, compared in NFC;
/// `None` matches every path.
pub(crate) fn glob_matches(globs: Option<&GlobSet>, relative: &Path) -> bool {
    globs.is_none_or(|set| {
        let composed = nfc_path(relative);
        set.is_match_candidate(&Candidate::new(&*composed))
    })
}

/// Text in Unicode normalization form C; borrowed when already composed.
fn nfc_text(text: &str) -> Cow<'_, str> {
    if is_nfc(text) {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.nfc().collect())
    }
}

/// Path in Unicode normalization form C, so that names stored decomposed,
/// as on macOS volumes, match patterns written composed. A path that is not
/// valid Unicode is returned unchanged.
fn nfc_path(path: &Path) -> Cow<'_, Path> {
    match path.to_str() {
        Some(text) if !is_nfc(text) => Cow::Owned(PathBuf::from(text.nfc().collect::<String>())),
        _ => Cow::Borrowed(path),
    }
}

fn invalid_pattern(what: &str, pattern: &str, reason: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("{what} `{pattern}` is not valid: {reason}"),
    )
    .with_found(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn globs(patterns: &[&str], case_sensitive: bool) -> GlobSet {
        let patterns: Vec<String> = patterns.iter().map(|p| (*p).to_owned()).collect();
        compile_globs(&patterns, case_sensitive)
            .expect("patterns compile")
            .expect("set is present")
    }

    #[test]
    fn glob_without_double_star_matches_top_level_only() {
        let set = globs(&["*.rs"], true);
        assert!(glob_matches(Some(&set), Path::new("lib.rs")));
        assert!(!glob_matches(Some(&set), &Path::new("src").join("lib.rs")));
    }

    #[test]
    fn double_star_glob_matches_any_depth() {
        let set = globs(&["**/*.rs"], true);
        assert!(glob_matches(Some(&set), Path::new("lib.rs")));
        assert!(glob_matches(
            Some(&set),
            &Path::new("a").join("b").join("c.rs")
        ));
    }

    #[test]
    fn glob_is_case_sensitive_unless_disabled() {
        assert!(!glob_matches(
            Some(&globs(&["*.RS"], true)),
            Path::new("a.rs")
        ));
        assert!(glob_matches(
            Some(&globs(&["*.RS"], false)),
            Path::new("a.rs")
        ));
    }

    #[test]
    fn empty_glob_list_matches_everything() {
        assert!(compile_globs(&[], true).expect("compiles").is_none());
        assert!(glob_matches(None, Path::new("anything")));
    }

    #[test]
    fn invalid_glob_reports_pattern() {
        let error = compile_globs(&["[a-".to_owned()], true).expect_err("pattern is invalid");
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.found.as_deref(), Some("[a-"));
    }

    #[test]
    fn invalid_decomposed_glob_reports_pattern_as_given() {
        let given = "e\u{301}[a-";
        let error = compile_globs(&[given.to_owned()], true).expect_err("pattern is invalid");
        assert_eq!(error.found.as_deref(), Some(given));
    }

    #[test]
    fn decomposed_names_match_composed_globs() {
        let set = globs(&["caf\u{e9}.txt", "r\u{e9}sum\u{e9}/*"], true);
        assert!(glob_matches(Some(&set), Path::new("cafe\u{301}.txt")));
        assert!(glob_matches(
            Some(&set),
            &Path::new("re\u{301}sume\u{301}").join("cv.txt")
        ));
        assert!(glob_matches(Some(&set), Path::new("caf\u{e9}.txt")));
        assert!(!glob_matches(Some(&set), Path::new("cafe.txt")));
    }

    #[test]
    fn composed_names_match_decomposed_globs() {
        let set = globs(&["cafe\u{301}.*"], true);
        assert!(glob_matches(Some(&set), Path::new("caf\u{e9}.md")));
        assert!(glob_matches(Some(&set), Path::new("cafe\u{301}.md")));
    }

    #[test]
    fn decomposed_names_match_composed_exclusions() {
        let patterns = vec!["caf\u{e9}/".to_owned(), "*.\u{e9}t\u{e9}".to_owned()];
        let matcher = compile_exclude(&patterns, true).expect("compiles");
        assert!(excluded_by_patterns(
            matcher.as_ref(),
            Path::new("cafe\u{301}"),
            true
        ));
        assert!(excluded_by_patterns(
            matcher.as_ref(),
            Path::new("notes.e\u{301}te\u{301}"),
            false
        ));
        assert!(!excluded_by_patterns(
            matcher.as_ref(),
            Path::new("cafe"),
            true
        ));
        let decomposed = compile_exclude(&["cafe\u{301}.log".to_owned()], true).expect("compiles");
        assert!(excluded_by_patterns(
            decomposed.as_ref(),
            Path::new("caf\u{e9}.log"),
            false
        ));
    }

    #[test]
    fn exclude_last_matching_pattern_decides() {
        let patterns = vec!["*.log".to_owned(), "!keep.log".to_owned()];
        let matcher = compile_exclude(&patterns, true).expect("compiles");
        assert!(excluded_by_patterns(
            matcher.as_ref(),
            Path::new("a.log"),
            false
        ));
        assert!(!excluded_by_patterns(
            matcher.as_ref(),
            Path::new("keep.log"),
            false
        ));
        assert!(!excluded_by_patterns(
            matcher.as_ref(),
            Path::new("a.txt"),
            false
        ));
    }

    #[test]
    fn exclude_directory_pattern_ignores_files_of_that_name() {
        let matcher = compile_exclude(&["out/".to_owned()], true).expect("compiles");
        assert!(excluded_by_patterns(
            matcher.as_ref(),
            Path::new("out"),
            true
        ));
        assert!(!excluded_by_patterns(
            matcher.as_ref(),
            Path::new("out"),
            false
        ));
    }

    #[test]
    fn anchored_exclude_matches_only_at_root() {
        let matcher = compile_exclude(&["/target".to_owned()], true).expect("compiles");
        assert!(excluded_by_patterns(
            matcher.as_ref(),
            Path::new("target"),
            true
        ));
        assert!(!excluded_by_patterns(
            matcher.as_ref(),
            &Path::new("a").join("target"),
            true
        ));
    }

    #[test]
    fn presets_exclude_directories_and_git_file_only() {
        let presets = preset_list(&[ExcludePreset::Vcs, ExcludePreset::Build]);
        assert!(excluded_by_preset(&presets, OsStr::new(".git"), true));
        assert!(excluded_by_preset(&presets, OsStr::new(".git"), false));
        assert!(excluded_by_preset(&presets, OsStr::new("target"), true));
        assert!(!excluded_by_preset(&presets, OsStr::new("build"), false));
        assert!(!excluded_by_preset(&presets, OsStr::new("src"), true));
    }

    #[test]
    fn preset_list_is_sorted_without_duplicates() {
        let presets = preset_list(&[ExcludePreset::Cache, ExcludePreset::Cache]);
        assert_eq!(presets, vec![".cache", ".pytest_cache", "__pycache__"]);
    }

    #[test]
    fn dot_names_are_hidden() {
        assert!(is_hidden(OsStr::new(".env"), None));
        assert!(!is_hidden(OsStr::new("env"), None));
    }

    #[test]
    fn kind_filter_uses_target_kind_of_resolved_links() {
        assert!(kind_matches(
            KindFilter::File,
            FileKind::Symlink,
            Some(FileKind::File)
        ));
        assert!(!kind_matches(
            KindFilter::Link,
            FileKind::Symlink,
            Some(FileKind::File)
        ));
        assert!(kind_matches(KindFilter::Link, FileKind::Symlink, None));
        assert!(kind_matches(
            KindFilter::Directory,
            FileKind::Directory,
            None
        ));
        assert!(!kind_matches(KindFilter::File, FileKind::Special, None));
        assert!(kind_matches(KindFilter::Any, FileKind::Special, None));
    }
}
