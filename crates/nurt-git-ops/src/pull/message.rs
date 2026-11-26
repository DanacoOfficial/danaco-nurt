// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Title of the merge commit created by a pull, built from the descriptions
//! of the fetched heads in `FETCH_HEAD` the way `git fmt-merge-msg` builds it.

/// Branches whose name is left out of the merge title when
/// `merge.suppressDest` is not configured.
const DEFAULT_SUPPRESSED: [&str; 2] = ["main", "master"];

/// One fetched head, parsed from its `FETCH_HEAD` description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeSource {
    /// Kind of the head.
    pub kind: SourceKind,
    /// Quoted name of the head (`'main'`); empty for [`SourceKind::Whole`].
    pub name: String,
    /// Repository the head was fetched from (URL, path or `.`).
    pub origin: String,
}

/// Kind of a fetched head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceKind {
    /// The `HEAD` of the remote, described by the URL alone.
    Whole,
    /// A branch.
    Branch,
    /// A remote-tracking branch.
    RemoteBranch,
    /// A tag.
    Tag,
    /// Any other reference or commit.
    Generic,
}

impl MergeSource {
    /// Parses a description such as `branch 'main' of https://example.com/r`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_ops::pull::{MergeSource, SourceKind};
    ///
    /// let source = MergeSource::parse("tag 'v1.0' of ../upstream");
    /// assert_eq!(source.kind, SourceKind::Tag);
    /// assert_eq!(source.name, "'v1.0'");
    /// assert_eq!(source.origin, "../upstream");
    /// ```
    #[must_use]
    pub fn parse(description: &str) -> Self {
        const PREFIXES: [(&str, SourceKind); 3] = [
            ("branch ", SourceKind::Branch),
            ("remote-tracking branch ", SourceKind::RemoteBranch),
            ("tag ", SourceKind::Tag),
        ];
        let Some((what, origin)) = description.split_once("' of ") else {
            return Self {
                kind: SourceKind::Whole,
                name: String::new(),
                origin: description.to_owned(),
            };
        };
        let quoted = format!("{what}'");
        for (prefix, kind) in PREFIXES {
            if let Some(name) = quoted.strip_prefix(prefix) {
                return Self {
                    kind,
                    name: name.to_owned(),
                    origin: origin.to_owned(),
                };
            }
        }
        Self {
            kind: SourceKind::Generic,
            name: quoted,
            origin: origin.to_owned(),
        }
    }
}

fn joined(singular: &str, plural: &str, names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => format!("{singular}{one}"),
        [rest @ .., last] => format!("{plural}{} and {last}", rest.join(", ")),
    }
}

fn origin_part(origin: &str, sources: &[&MergeSource]) -> String {
    if sources
        .iter()
        .any(|source| source.kind == SourceKind::Whole)
    {
        return origin.to_owned();
    }
    let groups = [
        (SourceKind::Branch, "branch ", "branches "),
        (
            SourceKind::RemoteBranch,
            "remote-tracking branch ",
            "remote-tracking branches ",
        ),
        (SourceKind::Tag, "tag ", "tags "),
        (SourceKind::Generic, "commit ", "commits "),
    ];
    let parts: Vec<String> = groups
        .iter()
        .filter_map(|(kind, singular, plural)| {
            let names: Vec<&str> = sources
                .iter()
                .filter(|source| source.kind == *kind)
                .map(|source| source.name.as_str())
                .collect();
            (!names.is_empty()).then(|| joined(singular, plural, &names))
        })
        .collect();
    let mut part = parts.join(", ");
    if origin != "." {
        part.push_str(" of ");
        part.push_str(origin);
    }
    part
}

/// Matches `name` against a `wildmatch` pattern with `*` and `?`; the
/// position after the last `*` is retried, so the time is bounded by the
/// product of the lengths.
fn glob_matches(pattern: &[u8], name: &[u8]) -> bool {
    let (mut at_pattern, mut at_name) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while let Some(found) = name.get(at_name) {
        match pattern.get(at_pattern) {
            Some(b'*') => {
                star = Some((at_pattern, at_name));
                at_pattern += 1;
            }
            Some(expected) if *expected == b'?' || expected == found => {
                at_pattern += 1;
                at_name += 1;
            }
            _ => match star {
                Some((star_pattern, star_name)) => {
                    star = Some((star_pattern, star_name + 1));
                    at_pattern = star_pattern + 1;
                    at_name = star_name + 1;
                }
                None => return false,
            },
        }
    }
    pattern
        .get(at_pattern..)
        .is_some_and(|rest| rest.iter().all(|byte| *byte == b'*'))
}

/// Whether the merge title leaves out ` into <branch>`.
///
/// `suppress_dest` holds the values of `merge.suppressDest` in configuration
/// order; an empty value clears the earlier ones, and without any value the
/// branches `main` and `master` are left out.
fn suppressed(branch: &str, suppress_dest: Option<&[String]>) -> bool {
    match suppress_dest {
        None => DEFAULT_SUPPRESSED.contains(&branch),
        Some(values) => {
            let start = values
                .iter()
                .rposition(String::is_empty)
                .map_or(0, |index| index + 1);
            values[start..]
                .iter()
                .any(|pattern| glob_matches(pattern.as_bytes(), branch.as_bytes()))
        }
    }
}

/// Title of the merge commit of a pull.
///
/// `descriptions` are the `FETCH_HEAD` descriptions of the heads to merge,
/// `branch` is the current branch (`None` when `HEAD` is detached) and
/// `suppress_dest` the values of `merge.suppressDest`, `None` when the key
/// is not set.
///
/// # Examples
///
/// ```
/// use nurt_git_ops::pull::merge_message;
///
/// let heads = ["branch 'main' of ../upstream".to_owned()];
/// assert_eq!(merge_message(&heads, Some("main"), None), "Merge branch 'main' of ../upstream");
/// assert_eq!(
///     merge_message(&heads, Some("topic"), None),
///     "Merge branch 'main' of ../upstream into topic"
/// );
/// ```
#[must_use]
pub fn merge_message(
    descriptions: &[String],
    branch: Option<&str>,
    suppress_dest: Option<&[String]>,
) -> String {
    let sources: Vec<MergeSource> = descriptions
        .iter()
        .map(|text| MergeSource::parse(text))
        .collect();
    let mut origins: Vec<&str> = Vec::new();
    for source in &sources {
        if !origins.contains(&source.origin.as_str()) {
            origins.push(&source.origin);
        }
    }
    let parts: Vec<String> = origins
        .iter()
        .map(|origin| {
            let group: Vec<&MergeSource> = sources
                .iter()
                .filter(|source| source.origin == *origin)
                .collect();
            origin_part(origin, &group)
        })
        .collect();
    let mut title = format!("Merge {}", parts.join("; "));
    let destination = branch.unwrap_or("HEAD");
    if !suppressed(destination, suppress_dest) {
        title.push_str(" into ");
        title.push_str(destination);
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heads(descriptions: &[&str]) -> Vec<String> {
        descriptions.iter().map(|text| (*text).to_owned()).collect()
    }

    #[test]
    fn titles_a_branch_of_a_remote() {
        let fetched = heads(&["branch 'main' of /srv/up"]);
        assert_eq!(
            merge_message(&fetched, Some("main"), None),
            "Merge branch 'main' of /srv/up"
        );
        assert_eq!(
            merge_message(&fetched, Some("master"), None),
            "Merge branch 'main' of /srv/up"
        );
        assert_eq!(
            merge_message(&fetched, Some("develop"), None),
            "Merge branch 'main' of /srv/up into develop"
        );
    }

    #[test]
    fn leaves_out_the_origin_of_the_repository_itself() {
        let fetched = heads(&["branch 'main' of ."]);
        assert_eq!(
            merge_message(&fetched, Some("feat"), None),
            "Merge branch 'main' into feat"
        );
    }

    #[test]
    fn joins_several_heads_like_git() {
        let fetched = heads(&[
            "branch 'a' of /srv/up",
            "branch 'b' of /srv/up",
            "branch 'c' of /srv/up",
            "tag 'v1' of /srv/up",
        ]);
        assert_eq!(
            merge_message(&fetched, Some("main"), None),
            "Merge branches 'a', 'b' and 'c', tag 'v1' of /srv/up"
        );
        let two_origins = heads(&[
            "branch 'a' of /srv/one",
            "remote-tracking branch 'origin/b' of /srv/two",
        ]);
        assert_eq!(
            merge_message(&two_origins, Some("main"), None),
            "Merge branch 'a' of /srv/one; remote-tracking branch 'origin/b' of /srv/two"
        );
    }

    #[test]
    fn titles_whole_remotes_and_other_references() {
        assert_eq!(
            merge_message(&heads(&["https://example.com/r"]), Some("main"), None),
            "Merge https://example.com/r"
        );
        assert_eq!(
            merge_message(
                &heads(&["'refs/pull/1/head' of /srv/up"]),
                Some("main"),
                None
            ),
            "Merge commit 'refs/pull/1/head' of /srv/up"
        );
    }

    #[test]
    fn names_a_detached_head_as_destination() {
        let fetched = heads(&["branch 'dev' of /srv/up"]);
        assert_eq!(
            merge_message(&fetched, None, None),
            "Merge branch 'dev' of /srv/up into HEAD"
        );
    }

    #[test]
    fn follows_merge_suppress_dest() {
        let fetched = heads(&["branch 'dev' of /srv/up"]);
        let patterns = vec!["release/*".to_owned()];
        assert_eq!(
            merge_message(&fetched, Some("release/1.0"), Some(&patterns)),
            "Merge branch 'dev' of /srv/up"
        );
        assert_eq!(
            merge_message(&fetched, Some("main"), Some(&patterns)),
            "Merge branch 'dev' of /srv/up into main"
        );
        let cleared = vec!["main".to_owned(), String::new(), "t?pic".to_owned()];
        assert!(!suppressed("main", Some(&cleared)));
        assert!(suppressed("topic", Some(&cleared)));
        assert!(!suppressed("toppic", Some(&cleared)));
    }

    #[test]
    fn matches_wildcards() {
        assert!(glob_matches(b"*", b""));
        assert!(glob_matches(b"a*c", b"abbbc"));
        assert!(!glob_matches(b"a*c", b"abd"));
        assert!(!glob_matches(b"?", b""));
        assert!(glob_matches(b"", b""));
        assert!(!glob_matches(b"", b"a"));
        assert!(!glob_matches(b"a?", b"a"));
    }

    #[test]
    fn wildcard_matching_terminates_on_many_stars() {
        let name = "a".repeat(60);
        assert!(!glob_matches(
            b"*a*a*a*a*a*a*a*a*a*a*a*a*a*b",
            name.as_bytes()
        ));
        assert!(glob_matches(
            b"*a*a*a*a*a*a*a*a*a*a*a*a*a*",
            name.as_bytes()
        ));
    }
}
