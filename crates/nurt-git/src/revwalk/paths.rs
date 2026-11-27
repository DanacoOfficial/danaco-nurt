// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::revwalk::paths
// Responsibility:   Literal path limits of the history commands and comparison of two trees.
// Dependencies:     bstr, gix
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Literal path limits of the history commands and comparison of two trees
//! within them.

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use gix::objs::tree::{EntryKind, EntryMode};

use super::repo::OpenRepo;

/// Repository-relative paths that limit a history; a path names a file or a
/// directory with everything below it, the empty path the whole tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathFilter {
    paths: Vec<BString>,
}

impl PathFilter {
    /// Filter of the given paths; duplicates and paths below another given
    /// path are dropped.
    pub(crate) fn new(mut paths: Vec<BString>) -> Self {
        paths.sort();
        paths.dedup();
        let mut kept: Vec<BString> = Vec::with_capacity(paths.len());
        for path in paths {
            if !kept
                .iter()
                .any(|outer| covers(outer.as_ref(), path.as_ref()))
            {
                kept.push(path);
            }
        }
        Self { paths: kept }
    }

    /// Whether the file at `path` is within the filter.
    pub(crate) fn matches(&self, path: &BStr) -> bool {
        self.paths.iter().any(|outer| covers(outer.as_ref(), path))
    }

    /// Whether two trees hold the same entries at every path of the filter;
    /// a missing tree is the empty tree.
    pub(crate) fn same_in(
        &self,
        repo: &OpenRepo,
        old: Option<ObjectId>,
        new: ObjectId,
    ) -> Result<bool, String> {
        for path in &self.paths {
            let before = match old {
                Some(tree) => entry_at(repo, tree, path.as_ref())?,
                None => None,
            };
            if before != entry_at(repo, new, path.as_ref())? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Whether a path matches one of the glob patterns; a pattern without `/`
/// matches the file name at any depth, as in `.gitignore`.
pub(crate) fn excluded(patterns: &[BString], path: &BStr) -> bool {
    let name = path
        .rfind_byte(b'/')
        .map_or(path, |slash| &path[slash + 1..]);
    patterns.iter().any(|pattern| {
        let target = if pattern.contains(&b'/') { path } else { name };
        gix::glob::wildmatch(
            pattern.as_bstr(),
            target,
            gix::glob::wildmatch::Mode::NO_MATCH_SLASH_LITERAL,
        )
    })
}

fn covers(outer: &BStr, path: &BStr) -> bool {
    outer.is_empty()
        || path == outer
        || (path.len() > outer.len()
            && path.starts_with(outer.as_bytes())
            && path[outer.len()] == b'/')
}

/// Mode and object of the entry at `path` in `tree`; the empty path is the
/// tree itself.
pub(crate) fn entry_at(
    repo: &OpenRepo,
    tree: ObjectId,
    path: &BStr,
) -> Result<Option<(EntryMode, ObjectId)>, String> {
    if path.is_empty() {
        return Ok(Some((EntryMode::from(EntryKind::Tree), tree)));
    }
    let mut current = tree;
    let mut found = None;
    for name in path.split_str("/") {
        if let Some((mode, _)) = found
            && !EntryMode::is_tree(&mode)
        {
            return Ok(None);
        }
        let object = repo
            .repo
            .find_tree(current)
            .map_err(|err| err.to_string())?;
        let Some(entry) = object.find_entry(name) else {
            return Ok(None);
        };
        let id = entry.object_id();
        found = Some((entry.mode(), id));
        current = id;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revwalk::fixture::Fixture;

    fn filter(paths: &[&str]) -> PathFilter {
        PathFilter::new(paths.iter().map(|path| BString::from(*path)).collect())
    }

    #[test]
    fn directory_paths_cover_their_contents_only() {
        let limit = filter(&["src", "src/lib.rs", "doc/a.md"]);
        let kept = PathFilter {
            paths: vec![BString::from("doc/a.md"), BString::from("src")],
        };
        assert_eq!(limit, kept);
        assert!(limit.matches("src/lib.rs".into()));
        assert!(limit.matches("src".into()));
        assert!(!limit.matches("src2/lib.rs".into()));
        assert!(!limit.matches("doc/a.mdx".into()));
        assert!(filter(&[""]).matches("anything".into()));
    }

    #[test]
    fn compares_trees_within_the_paths() {
        let mut fixture = Fixture::new();
        let first = fixture.commit(
            "first",
            &[("src/a.rs", Some("a\n")), ("b.txt", Some("b\n"))],
        );
        let second = fixture.commit("second", &[("b.txt", Some("changed\n"))]);
        let repo = fixture.open();
        let first_tree = repo.tree_of(first).unwrap();
        let second_tree = repo.tree_of(second).unwrap();
        assert!(
            filter(&["src"])
                .same_in(&repo, Some(first_tree), second_tree)
                .unwrap()
        );
        assert!(
            !filter(&["b.txt"])
                .same_in(&repo, Some(first_tree), second_tree)
                .unwrap()
        );
        assert!(
            !filter(&["src/a.rs"])
                .same_in(&repo, None, second_tree)
                .unwrap()
        );
        assert!(
            filter(&["missing"])
                .same_in(&repo, None, second_tree)
                .unwrap()
        );
        assert!(
            filter(&["b.txt/below"])
                .same_in(&repo, None, second_tree)
                .unwrap()
        );
        assert!(
            !filter(&[""])
                .same_in(&repo, Some(first_tree), second_tree)
                .unwrap()
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
