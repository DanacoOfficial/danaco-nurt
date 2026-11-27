// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-types::path
// Responsibility:   Lexical operations on paths in canonical form.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Lexical operations on paths in canonical form.
//!
//! A canonical path uses `/` as the separator and starts with a root: `/`,
//! a drive (`C:/`), a share (`//server/share`) or a provider prefix
//! (`env:/`). The operations never consult the file system.

const UNC_PREFIX: &str = "//";
const ROOT_SEPARATOR: &str = ":/";

/// Length of the root of `path` in bytes; 0 for a relative path.
pub(crate) fn root_length(path: &str) -> usize {
    if let Some(rest) = path.strip_prefix(UNC_PREFIX) {
        let server = rest.find('/').unwrap_or(rest.len());
        let after_server = rest.get(server..).unwrap_or("");
        let share = after_server
            .get(1..)
            .and_then(|share| share.find('/'))
            .map_or(after_server.len(), |end| end + 1);
        return UNC_PREFIX.len() + server + share;
    }
    if path.starts_with('/') {
        return 1;
    }
    match path.find(ROOT_SEPARATOR) {
        Some(index)
            if index > 0
                && path.get(..index).is_some_and(|prefix| {
                    prefix.bytes().all(|byte| byte.is_ascii_alphanumeric())
                }) =>
        {
            index + ROOT_SEPARATOR.len()
        }
        _ => 0,
    }
}

/// Parent of `path`; `None` for a root and for a relative path of one
/// segment.
pub(crate) fn parent(path: &str) -> Option<String> {
    let root = root_length(path);
    let trimmed = path.trim_end_matches('/');
    if trimmed.len() <= root {
        return None;
    }
    let cut = trimmed.rfind('/')?;
    let end = if cut < root { root } else { cut };
    path.get(..end).map(str::to_owned)
}

/// Joins `segment` to `base` and removes `.` and `..` segments lexically;
/// a `segment` with a root replaces `base`.
pub(crate) fn join(base: &str, segment: &str) -> String {
    if root_length(segment) > 0 {
        return normalize(segment);
    }
    let mut joined = base.trim_end_matches('/').to_owned();
    if joined.len() < root_length(base) {
        joined = base.to_owned();
    } else {
        joined.push('/');
    }
    joined.push_str(segment);
    normalize(&joined)
}

fn normalize(path: &str) -> String {
    let root_end = root_length(path);
    let (root, rest) = path.split_at(root_end);
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            ".." if root.is_empty() => parts.push(".."),
            ".." => {}
            other => parts.push(other),
        }
    }
    let mut text = root.to_owned();
    if !parts.is_empty() && !text.is_empty() && !text.ends_with('/') {
        text.push('/');
    }
    text.push_str(&parts.join("/"));
    if text.is_empty() {
        ".".to_owned()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::{join, parent, root_length};

    #[test]
    fn root_length_recognises_every_root_form() {
        assert_eq!(root_length("/srv/app"), 1);
        assert_eq!(root_length("C:/proj"), 3);
        assert_eq!(root_length("//server/share/dir"), 14);
        assert_eq!(root_length("//server"), 8);
        assert_eq!(root_length("env:/PATH"), 5);
        assert_eq!(root_length("relative/dir"), 0);
        assert_eq!(root_length(":/x"), 0);
    }

    #[test]
    fn parent_stops_at_the_root() {
        assert_eq!(parent("/srv/app/src").as_deref(), Some("/srv/app"));
        assert_eq!(parent("/srv").as_deref(), Some("/"));
        assert_eq!(parent("/"), None);
        assert_eq!(parent("C:/proj").as_deref(), Some("C:/"));
        assert_eq!(parent("C:/"), None);
        assert_eq!(
            parent("//server/share/dir").as_deref(),
            Some("//server/share")
        );
        assert_eq!(parent("//server/share"), None);
        assert_eq!(parent("env:/PATH").as_deref(), Some("env:/"));
        assert_eq!(parent("/srv/app/").as_deref(), Some("/srv"));
        assert_eq!(parent("a/b").as_deref(), Some("a"));
        assert_eq!(parent("a"), None);
    }

    #[test]
    fn join_resolves_dot_segments_lexically() {
        assert_eq!(join("/srv/app", "src/main.rs"), "/srv/app/src/main.rs");
        assert_eq!(join("/srv/app/", "../lib"), "/srv/lib");
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(join("/", "../.."), "/");
        assert_eq!(join("C:/", "Users"), "C:/Users");
        assert_eq!(join("//server/share", "dir/./f"), "//server/share/dir/f");
        assert_eq!(join("/srv", "/etc/hosts"), "/etc/hosts");
        assert_eq!(join("a", "../.."), "..");
        assert_eq!(join("a", ".."), ".");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
