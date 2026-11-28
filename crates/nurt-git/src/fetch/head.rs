// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::fetch::head
// Responsibility:   Content of `FETCH_HEAD` and the address shown in it.
// Dependencies:     gix
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Content of `FETCH_HEAD` and the address shown in it.
//!
//! Every line is `<id>\t<flag>\t<description>`: the flag is empty for heads
//! to merge and `not-for-merge` for the others, heads to merge come first,
//! and the description names the remote reference and the address, for
//! example `branch 'main' of https://example.com/app`. The address has no
//! credentials, no trailing `/` and no trailing `.git`.

use gix::ObjectId;

use super::FetchedHead;

/// Flag of the heads that are not merged by a pull.
const NOT_FOR_MERGE: &str = "not-for-merge";

/// Suffix removed from the address shown in `FETCH_HEAD`.
const GIT_SUFFIX: &str = ".git";

/// One fetched remote reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// Object the remote reference points to.
    pub(crate) id: ObjectId,
    /// Full name on the remote, or the identifier for a fetch by
    /// identifier.
    pub(crate) remote_name: String,
    /// Position of the explicit refspec that selected the reference; `None`
    /// for a followed tag.
    pub(crate) spec: Option<usize>,
    /// The head is merged by a pull.
    pub(crate) merge: bool,
}

/// Marks the heads to merge as `git fetch` does.
///
/// With refspecs given by the caller every head of an explicit refspec is
/// merged. Otherwise the heads named by `branch.<current>.merge` are merged
/// when the current branch follows the remote, and without such
/// configuration the head of the first refspec when it has no pattern.
pub(crate) fn mark_merge(
    entries: &mut [Entry],
    explicit: bool,
    merge_refs: &[String],
    first_is_exact: bool,
) {
    if explicit {
        for entry in entries.iter_mut() {
            entry.merge = entry.spec.is_some();
        }
        return;
    }
    if !merge_refs.is_empty() {
        for entry in entries.iter_mut() {
            entry.merge = merge_refs.contains(&entry.remote_name);
        }
        return;
    }
    if first_is_exact && let Some(entry) = entries.iter_mut().find(|entry| entry.spec == Some(0)) {
        entry.merge = true;
    }
}

/// Content of `FETCH_HEAD` and the heads marked for merging.
pub(crate) fn render(entries: &[Entry], url: &str) -> (Vec<u8>, Vec<FetchedHead>) {
    let mut content = Vec::new();
    let mut for_merge = Vec::new();
    let ordered = entries
        .iter()
        .filter(|entry| entry.merge)
        .chain(entries.iter().filter(|entry| !entry.merge));
    for entry in ordered {
        let description = format!("{}{url}", note(&entry.remote_name));
        let flag = if entry.merge { "" } else { NOT_FOR_MERGE };
        content.extend_from_slice(format!("{}\t{flag}\t{description}\n", entry.id).as_bytes());
        if entry.merge {
            for_merge.push(FetchedHead {
                id: entry.id,
                description,
                remote_ref: entry.remote_name.clone(),
            });
        }
    }
    (content, for_merge)
}

/// Kind and short name of a remote reference, as written before the
/// address.
fn note(remote_name: &str) -> String {
    if remote_name == "HEAD" {
        return String::new();
    }
    let kinds = [
        ("refs/heads/", "branch"),
        ("refs/tags/", "tag"),
        ("refs/remotes/", "remote-tracking branch"),
    ];
    for (prefix, kind) in kinds {
        if let Some(short) = remote_name.strip_prefix(prefix) {
            return format!("{kind} '{short}' of ");
        }
    }
    format!("'{remote_name}' of ")
}

/// Address as shown in `FETCH_HEAD`: without credentials, trailing `/` and
/// trailing `.git`.
///
/// # Examples
///
/// ```text
/// assert_eq!(display_url("https://u:p@example.com/app.git/"), "https://example.com/app");
/// ```
pub(crate) fn display_url(url: &str) -> String {
    let anonymous = anonymize(url);
    let trimmed = anonymous.trim_end_matches('/');
    let end = trimmed.len().saturating_sub(1);
    if end > GIT_SUFFIX.len() && trimmed.ends_with(GIT_SUFFIX) {
        trimmed[..trimmed.len() - GIT_SUFFIX.len()].to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Address without the user name and password, as `git` shows it.
pub(crate) fn anonymize(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let rest = &url[scheme_end + 3..];
        let host_end = rest.find('/').unwrap_or(rest.len());
        return match rest[..host_end].rfind('@') {
            Some(at) => format!("{}{}", &url[..scheme_end + 3], &rest[at + 1..]),
            None => url.to_owned(),
        };
    }
    match (url.find('@'), url.find(':')) {
        (Some(at), Some(colon)) if at < colon && !url[..at].contains('/') => {
            url[at + 1..].to_owned()
        }
        _ => url.to_owned(),
    }
}

/// `text` with the credentials of every address in it removed.
pub(crate) fn scrub(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("://") {
        let (before, after) = rest.split_at(start + 3);
        out.push_str(before);
        let end = after
            .find(|c: char| c == '/' || c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(after.len());
        match after[..end].rfind('@') {
            Some(at) => out.push_str(&after[at + 1..end]),
            None => out.push_str(&after[..end]),
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> ObjectId {
        ObjectId::from_bytes_or_panic(&[byte; 20])
    }

    fn entry(name: &str, spec: Option<usize>, byte: u8) -> Entry {
        Entry {
            id: id(byte),
            remote_name: name.to_owned(),
            spec,
            merge: false,
        }
    }

    #[test]
    fn display_url_drops_credentials_slash_and_git_suffix() {
        assert_eq!(
            display_url("https://user:secret@example.com/app.git/"),
            "https://example.com/app"
        );
        assert_eq!(display_url("/srv/up"), "/srv/up");
        assert_eq!(display_url("x.git"), "x.git");
        assert_eq!(display_url("git@host:org/r.git"), "host:org/r");
    }

    #[test]
    fn scrub_removes_credentials_inside_text() {
        assert_eq!(
            scrub("cannot reach https://u:pw@h.example/r: refused"),
            "cannot reach https://h.example/r: refused"
        );
        assert_eq!(scrub("plain text"), "plain text");
    }

    #[test]
    fn merge_heads_follow_branch_configuration() {
        let mut entries = vec![
            entry("refs/heads/feature", Some(0), 1),
            entry("refs/heads/main", Some(0), 2),
            entry("refs/tags/v1", None, 3),
        ];
        mark_merge(&mut entries, false, &["refs/heads/main".to_owned()], false);
        let (content, merged) = render(&entries, "/srv/up");
        let text = String::from_utf8(content).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], format!("{}\t\tbranch 'main' of /srv/up", id(2)));
        assert_eq!(
            lines[1],
            format!("{}\tnot-for-merge\tbranch 'feature' of /srv/up", id(1))
        );
        assert_eq!(
            lines[2],
            format!("{}\tnot-for-merge\ttag 'v1' of /srv/up", id(3))
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].remote_ref, "refs/heads/main");
    }

    #[test]
    fn explicit_refspecs_merge_every_head_but_tags() {
        let mut entries = vec![
            entry("refs/heads/a", Some(0), 1),
            entry("refs/tags/t", None, 2),
        ];
        mark_merge(&mut entries, true, &[], false);
        assert!(entries[0].merge);
        assert!(!entries[1].merge);
    }

    #[test]
    fn first_exact_refspec_is_merged_without_configuration() {
        let mut entries = vec![entry("HEAD", Some(0), 1)];
        mark_merge(&mut entries, false, &[], true);
        let (content, _) = render(&entries, "https://example.com/r");
        assert_eq!(
            String::from_utf8(content).unwrap(),
            format!("{}\t\thttps://example.com/r\n", id(1))
        );
    }

    #[test]
    fn notes_describe_reference_kinds() {
        assert_eq!(note("refs/remotes/o/x"), "remote-tracking branch 'o/x' of ");
        assert_eq!(note("refs/pull/1/head"), "'refs/pull/1/head' of ");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
