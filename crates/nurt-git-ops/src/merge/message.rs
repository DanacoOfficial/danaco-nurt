// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::merge::message
// Responsibility:   Messages of merge commits, of `MERGE_MSG` and of the reflog, formatted as by `git fmt-merge-msg` and `git merge`.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Messages of merge commits, of `MERGE_MSG` and of the reflog, formatted
//! as by `git fmt-merge-msg` and `git merge`.

/// Destination branches whose name is left out of merge messages when
/// `merge.suppressDest` is not set.
pub const DEFAULT_SUPPRESSED_DESTINATIONS: &[&str] = &["main", "master"];

/// What was merged, as named in the merge message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeSource {
    /// A local branch: `Merge branch '<name>'`.
    Branch(String),
    /// An ancestor of a local branch named with `^` suffixes:
    /// `Merge branch '<name>' (early part)`.
    EarlyPart(String),
    /// A remote-tracking branch, as `<remote>/<branch>`:
    /// `Merge remote-tracking branch '<name>'`.
    RemoteBranch(String),
    /// A tag: `Merge tag '<name>'`, followed by the message of an annotated
    /// tag.
    Tag {
        /// Tag name.
        name: String,
        /// Message of the annotated tag.
        message: Option<String>,
    },
    /// A commit given by its identifier: `Merge commit '<id>'`.
    Commit(String),
    /// A fetched branch of a remote repository, as `git pull` describes it:
    /// `Merge branch '<branch>' of <url>`.
    Fetched {
        /// Branch on the remote.
        branch: String,
        /// URL of the remote.
        url: String,
    },
}

impl MergeSource {
    fn title(&self) -> String {
        match self {
            Self::Branch(name) => format!("Merge branch '{name}'"),
            Self::EarlyPart(name) => format!("Merge branch '{name}' (early part)"),
            Self::RemoteBranch(name) => format!("Merge remote-tracking branch '{name}'"),
            Self::Tag { name, .. } => format!("Merge tag '{name}'"),
            Self::Commit(id) => format!("Merge commit '{id}'"),
            Self::Fetched { branch, url } => format!("Merge branch '{branch}' of {url}"),
        }
    }
}

/// Message of a merge commit of `source` into `destination` (the current
/// branch name without `refs/heads/`, or `None` for a detached `HEAD`).
///
/// ` into <destination>` is appended unless the destination matches one of
/// `suppressed` (glob patterns of `merge.suppressDest`). The message ends
/// with a newline.
///
/// # Examples
///
/// ```
/// use nurt_git_ops::merge::{merge_message, MergeSource, DEFAULT_SUPPRESSED_DESTINATIONS};
///
/// let source = MergeSource::Branch("topic".to_owned());
/// assert_eq!(
///     merge_message(&source, Some("feature"), DEFAULT_SUPPRESSED_DESTINATIONS),
///     "Merge branch 'topic' into feature\n"
/// );
/// assert_eq!(
///     merge_message(&source, Some("main"), DEFAULT_SUPPRESSED_DESTINATIONS),
///     "Merge branch 'topic'\n"
/// );
/// ```
#[must_use]
pub fn merge_message<S: AsRef<str>>(
    source: &MergeSource,
    destination: Option<&str>,
    suppressed: &[S],
) -> String {
    let mut out = source.title();
    let destination = destination.unwrap_or("HEAD");
    if !suppressed
        .iter()
        .any(|pattern| glob_match(pattern.as_ref(), destination))
    {
        out.push_str(" into ");
        out.push_str(destination);
    }
    out.push('\n');
    if let MergeSource::Tag {
        message: Some(message),
        ..
    } = source
    {
        let body = message.trim_end();
        if !body.is_empty() {
            out.push('\n');
            out.push_str(body);
            out.push('\n');
        }
    }
    out
}

/// Appends the `# Conflicts:` section that `git merge` writes into
/// `MERGE_MSG` when it stops with conflicts.
///
/// # Examples
///
/// ```
/// use nurt_git_ops::merge::with_conflicts_section;
///
/// assert_eq!(
///     with_conflicts_section("Merge branch 'topic'\n", ["f.txt"], '#'),
///     "Merge branch 'topic'\n\n# Conflicts:\n#\tf.txt\n"
/// );
/// ```
#[must_use]
pub fn with_conflicts_section<I, P>(message: &str, paths: I, comment: char) -> String
where
    I: IntoIterator<Item = P>,
    P: AsRef<str>,
{
    let mut out = message.to_owned();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push(comment);
    out.push_str(" Conflicts:\n");
    for path in paths {
        out.push(comment);
        out.push('\t');
        out.push_str(path.as_ref());
        out.push('\n');
    }
    out
}

/// How a commit message is cleaned up (`commit.cleanup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cleanup {
    /// Remove comment lines, trailing whitespace and surplus blank lines,
    /// as after editing a message.
    Strip,
    /// Remove trailing whitespace and surplus blank lines only, as for a
    /// message given on the command line.
    Whitespace,
}

/// Cleans up `message` like `git stripspace`, removing lines that start
/// with `comment` in [`Cleanup::Strip`] mode. The result is empty or ends
/// with one newline.
///
/// # Examples
///
/// ```
/// use nurt_git_ops::merge::{clean_message, Cleanup};
///
/// let text = "Merge branch 'topic'\n\n# Conflicts:\n#\tf\n";
/// assert_eq!(clean_message(text, Cleanup::Strip, '#'), "Merge branch 'topic'\n");
/// ```
#[must_use]
pub fn clean_message(message: &str, cleanup: Cleanup, comment: char) -> String {
    let mut out = String::with_capacity(message.len());
    let mut pending_blank = false;
    for line in message.lines() {
        if cleanup == Cleanup::Strip && line.starts_with(comment) {
            continue;
        }
        let line = line.trim_end();
        if line.is_empty() {
            pending_blank = !out.is_empty();
            continue;
        }
        if pending_blank {
            out.push('\n');
            pending_blank = false;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Reflog message of a fast-forward merge started by `action` (for example
/// `merge topic` or `pull`).
#[must_use]
pub fn reflog_fast_forward(action: &str) -> String {
    format!("{action}: Fast-forward")
}

/// Reflog message of a merge commit created by `action`.
#[must_use]
pub fn reflog_merge(action: &str) -> String {
    format!("{action}: Merge made by the 'ort' strategy.")
}

/// Reflog message of a commit that concludes a stopped merge, from the
/// first line of its message.
#[must_use]
pub fn reflog_commit_merge(message: &str) -> String {
    let subject = message.lines().next().unwrap_or_default();
    format!("commit (merge): {subject}")
}

/// Matches `text` against a pattern with `*` and `?`; the position after the
/// last `*` is retried, so the time is bounded by the product of the lengths.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut at_pattern, mut at_text) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while let Some(current) = text.get(at_text) {
        match pattern.get(at_pattern) {
            Some('*') => {
                star = Some((at_pattern, at_text));
                at_pattern += 1;
            }
            Some(expected) if *expected == '?' || expected == current => {
                at_pattern += 1;
                at_text += 1;
            }
            _ => match star {
                Some((star_pattern, star_text)) => {
                    star = Some((star_pattern, star_text + 1));
                    at_pattern = star_pattern + 1;
                    at_text = star_text + 1;
                }
                None => return false,
            },
        }
    }
    pattern
        .get(at_pattern..)
        .is_some_and(|rest| rest.iter().all(|c| *c == '*'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: &[&str] = &[];

    #[test]
    fn names_every_kind_of_source_like_fmt_merge_msg() {
        let cases = [
            (
                MergeSource::Branch("dev".into()),
                "Merge branch 'dev' into feat\n",
            ),
            (
                MergeSource::EarlyPart("dev".into()),
                "Merge branch 'dev' (early part) into feat\n",
            ),
            (
                MergeSource::RemoteBranch("origin/dev".into()),
                "Merge remote-tracking branch 'origin/dev' into feat\n",
            ),
            (
                MergeSource::Tag {
                    name: "v1".into(),
                    message: None,
                },
                "Merge tag 'v1' into feat\n",
            ),
            (
                MergeSource::Commit("fab8025".into()),
                "Merge commit 'fab8025' into feat\n",
            ),
            (
                MergeSource::Fetched {
                    branch: "main".into(),
                    url: "https://example.org/r.git".into(),
                },
                "Merge branch 'main' of https://example.org/r.git into feat\n",
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(merge_message(&source, Some("feat"), NONE), expected);
        }
    }

    #[test]
    fn appends_annotated_tag_message() {
        let source = MergeSource::Tag {
            name: "v2".into(),
            message: Some("ann tag\n".into()),
        };
        assert_eq!(
            merge_message(&source, Some("main"), DEFAULT_SUPPRESSED_DESTINATIONS),
            "Merge tag 'v2'\n\nann tag\n"
        );
    }

    #[test]
    fn names_detached_head_as_destination() {
        let source = MergeSource::Branch("dev".into());
        assert_eq!(
            merge_message(&source, None, DEFAULT_SUPPRESSED_DESTINATIONS),
            "Merge branch 'dev' into HEAD\n"
        );
    }

    #[test]
    fn suppresses_destinations_matching_globs() {
        let source = MergeSource::Branch("dev".into());
        assert_eq!(
            merge_message(&source, Some("release/1.0"), &["release/*"]),
            "Merge branch 'dev'\n"
        );
        assert_eq!(
            merge_message(&source, Some("rel"), &["re?"]),
            "Merge branch 'dev'\n"
        );
        assert_eq!(
            merge_message(&source, Some("main"), &["feat"]),
            "Merge branch 'dev' into main\n"
        );
    }

    #[test]
    fn glob_matching_terminates_on_many_wildcards() {
        let text = "a".repeat(60);
        assert!(!glob_match("*a*a*a*a*a*a*a*a*a*a*a*a*a*b", &text));
        assert!(glob_match("*a*a*a*a*a*a*a*a*a*a*a*a*a*", &text));
        assert!(glob_match("", ""));
        assert!(!glob_match("", "a"));
        assert!(glob_match("a*", "a"));
        assert!(glob_match("*ä?", "żółwiä!"));
        assert!(!glob_match("a?", "a"));
    }

    #[test]
    fn writes_conflicts_section_with_comment_character() {
        assert_eq!(
            with_conflicts_section("Merge x", ["a", "b/c"], ';'),
            "Merge x\n\n; Conflicts:\n;\ta\n;\tb/c\n"
        );
    }

    #[test]
    fn whitespace_cleanup_keeps_comments_and_collapses_blank_lines() {
        let text = "\n\nsubject  \n\n\n\nbody\n# kept\n\n";
        assert_eq!(
            clean_message(text, Cleanup::Whitespace, '#'),
            "subject\n\nbody\n# kept\n"
        );
        assert_eq!(clean_message("\n \n", Cleanup::Strip, '#'), "");
    }

    #[test]
    fn formats_reflog_messages_like_git() {
        assert_eq!(reflog_fast_forward("merge dev"), "merge dev: Fast-forward");
        assert_eq!(
            reflog_merge("pull"),
            "pull: Merge made by the 'ort' strategy."
        );
        assert_eq!(
            reflog_commit_merge("Merge branch 'topic'\n\nbody\n"),
            "commit (merge): Merge branch 'topic'"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
