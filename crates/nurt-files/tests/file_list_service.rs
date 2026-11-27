// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/file_list_service.rs
// Responsibility:   The file listing service shared with other command groups.
// Dependencies:     nurt_contracts, support_fs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The file listing service shared with other command groups.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod support_fs;

use std::sync::Arc;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::services::{ExcludePreset, FileListService, WalkKind, WalkSpec};
use nurt_files::walk::FileList;
use support_fs::MemFs;

fn service(fs: MemFs) -> FileList {
    FileList::new(Arc::new(fs)).with_threads(2)
}

/// Repository with a dependency directory, an ignored file and a source
/// file.
fn project() -> MemFs {
    let mut fs = MemFs::new();
    fs.dir("p/.git")
        .file("p/.gitignore", "*.log\n")
        .file("p/debug.log", "")
        .file("p/node_modules/lib/index.js", "")
        .file("p/src/main.rs", "")
        .file("p/src/util/mod.rs", "");
    fs
}

fn listed(service: &FileList, spec: &WalkSpec) -> Vec<(String, u32, bool)> {
    service
        .walk(spec)
        .expect("root is readable")
        .map(|entry| {
            let entry = entry.expect("entry is readable");
            let relative = support_fs::relative(&MemFs::native("p"), &entry.path);
            (relative, entry.depth, entry.is_dir)
        })
        .collect()
}

#[test]
fn default_spec_lists_every_file_below_the_root() {
    let service = service(project());
    let entries = listed(&service, &WalkSpec::new(MemFs::native("p")));
    let paths: Vec<&str> = entries.iter().map(|(path, _, _)| path.as_str()).collect();
    assert_eq!(
        paths,
        [
            ".gitignore",
            "debug.log",
            "node_modules/lib/index.js",
            "src/main.rs",
            "src/util/mod.rs"
        ]
    );
}

#[test]
fn source_tree_spec_applies_ignore_files_and_presets() {
    let service = service(project());
    let entries = listed(&service, &WalkSpec::source_tree(MemFs::native("p")));
    let paths: Vec<&str> = entries.iter().map(|(path, _, _)| path.as_str()).collect();
    assert_eq!(paths, [".gitignore", "src/main.rs", "src/util/mod.rs"]);
}

#[test]
fn depth_counts_direct_children_as_one() {
    let service = service(project());
    let spec = WalkSpec {
        kind: WalkKind::Any,
        max_depth: Some(2),
        exclude_presets: vec![ExcludePreset::Vcs, ExcludePreset::Deps],
        ..WalkSpec::new(MemFs::native("p"))
    };
    assert_eq!(
        listed(&service, &spec),
        [
            (".gitignore".to_owned(), 1, false),
            ("debug.log".to_owned(), 1, false),
            ("src".to_owned(), 1, true),
            ("src/main.rs".to_owned(), 2, false),
            ("src/util".to_owned(), 2, true),
        ]
    );
}

#[test]
fn any_kind_lists_files_and_directories_but_not_links_or_devices() {
    let mut fs = MemFs::new();
    fs.file("p/a.txt", "")
        .dir("p/d")
        .link("p/to-d", "d")
        .link("p/dangling", "missing")
        .special("p/fifo");
    let spec = WalkSpec {
        kind: WalkKind::Any,
        ..WalkSpec::new(MemFs::native("p"))
    };
    let unresolved = listed(&service(fs), &spec);
    assert_eq!(
        unresolved,
        [("a.txt".to_owned(), 1, false), ("d".to_owned(), 1, true)]
    );
    let mut fs = MemFs::new();
    fs.dir("p/d").link("p/to-d", "d");
    let spec = WalkSpec {
        follow_links: true,
        ..spec
    };
    let followed = listed(&service(fs), &spec);
    assert_eq!(
        followed,
        [("d".to_owned(), 1, true), ("to-d".to_owned(), 1, true)]
    );
}

#[test]
fn directory_kind_lists_directories_only() {
    let service = service(project());
    let spec = WalkSpec {
        kind: WalkKind::Directory,
        ..WalkSpec::source_tree(MemFs::native("p"))
    };
    let entries = listed(&service, &spec);
    assert_eq!(
        entries,
        [
            ("src".to_owned(), 1, true),
            ("src/util".to_owned(), 2, true)
        ]
    );
}

#[test]
fn relative_root_is_rejected() {
    let service = service(project());
    let error = service
        .walk(&WalkSpec::new("p"))
        .err()
        .expect("root is relative");
    assert_eq!(error.code(), ErrorCode::PathRelative);
}

#[test]
fn missing_root_is_rejected() {
    let service = service(project());
    let error = service
        .walk(&WalkSpec::new(MemFs::native("absent")))
        .err()
        .expect("root is missing");
    assert_eq!(error.code(), ErrorCode::NotFound);
}

#[test]
fn file_root_is_rejected() {
    let service = service(project());
    let error = service
        .walk(&WalkSpec::new(MemFs::native("p/src/main.rs")))
        .err()
        .expect("root is a file");
    assert_eq!(error.code(), ErrorCode::Io);
}

#[test]
fn unreadable_root_is_rejected() {
    let mut fs = project();
    fs.deny("p");
    let error = service(fs)
        .walk(&WalkSpec::new(MemFs::native("p")))
        .err()
        .expect("root is unreadable");
    assert_eq!(error.code(), ErrorCode::Permission);
}

#[test]
fn invalid_pattern_is_rejected() {
    let service = service(project());
    let spec = WalkSpec {
        globs: vec!["[z-".to_owned()],
        ..WalkSpec::new(MemFs::native("p"))
    };
    let error = service.walk(&spec).err().expect("glob is invalid");
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn unreadable_directory_is_an_error_item_and_listing_continues() {
    let mut fs = project();
    fs.deny("p/src/util");
    let service = service(fs);
    let items: Vec<_> = service
        .walk(&WalkSpec::new(MemFs::native("p")))
        .expect("root is readable")
        .collect();
    let errors: Vec<ErrorCode> = items
        .iter()
        .filter_map(|item| item.as_ref().err().map(|error| error.code()))
        .collect();
    assert_eq!(errors, [ErrorCode::Permission]);
    assert_eq!(items.iter().filter(|item| item.is_ok()).count(), 4);
}

#[test]
fn dropping_the_iterator_stops_the_walk() {
    let mut fs = MemFs::new();
    for index in 0..5000 {
        fs.file(&format!("p/d{}/f{index}", index % 20), "");
    }
    let service = service(fs);
    let first: Vec<_> = service
        .walk(&WalkSpec::new(MemFs::native("p")))
        .expect("root is readable")
        .take(3)
        .collect();
    assert_eq!(first.len(), 3);
    assert!(first.iter().all(Result::is_ok));
}

#[test]
fn global_ignore_file_applies_to_gitignore_requests() {
    let mut fs = project();
    fs.file("cfg/ignore", "*.rs\n");
    let service = FileList::new(Arc::new(fs)).with_global_ignore_file(MemFs::path("cfg/ignore"));
    let entries = listed(&service, &WalkSpec::source_tree(MemFs::native("p")));
    assert_eq!(entries, [(".gitignore".to_owned(), 1, false)]);
    assert!(format!("{service:?}").contains("global_ignore_file"));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
