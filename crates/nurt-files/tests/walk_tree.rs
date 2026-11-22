// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Walker behaviour on the in-memory file system.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod support_fs;

use std::ops::ControlFlow;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::{AbsPath, FileKind, Fs, FsErrorKind};
use nurt_contracts::services::ExcludePreset;
use nurt_contracts::stage::{CancelReason, CancelToken};
use nurt_files::walk::{KindFilter, SAMPLE_LIMIT, WalkItem, WalkOptions, WalkSummary, Walker};
use support_fs::{MemFs, lossy, non_unicode_name};

/// Options delivering every kind of entry below the root, single-threaded.
fn all() -> WalkOptions {
    WalkOptions {
        min_depth: 1,
        kind: KindFilter::Any,
        threads: 1,
        ..WalkOptions::default()
    }
}

struct Outcome {
    entries: Vec<String>,
    unreadable: Vec<(String, FsErrorKind)>,
    summary: WalkSummary,
}

fn walk_roots(fs: &dyn Fs, roots: &[&str], options: WalkOptions) -> Result<Outcome, Error> {
    let walker = Walker::new(options)?;
    let roots: Vec<AbsPath> = roots.iter().map(|root| MemFs::path(root)).collect();
    let mut entries = Vec::new();
    let mut unreadable = Vec::new();
    let summary = walker.run(fs, &roots, &|| Ok(()), |item| {
        match item {
            WalkItem::Entry(entry) => {
                let relative = entry.relative_string();
                entries.push(if relative.is_empty() {
                    lossy(&entry.name)
                } else {
                    relative
                });
            }
            WalkItem::Unreadable(failure) => {
                unreadable.push((
                    support_fs::relative(&MemFs::native(""), failure.path.as_path()),
                    failure.error.kind,
                ));
            }
        }
        ControlFlow::Continue(())
    })?;
    Ok(Outcome {
        entries,
        unreadable,
        summary,
    })
}

fn walk(fs: &MemFs, options: WalkOptions) -> Outcome {
    walk_roots(fs, &["r"], options).expect("walk succeeds")
}

/// Tree used by several tests: two directories, nested files and a hidden
/// file.
fn sample_tree() -> MemFs {
    let mut fs = MemFs::new();
    fs.file("r/b.txt", "b")
        .file("r/a/z.rs", "z")
        .file("r/a/y.txt", "y")
        .file("r/a/deep/x.rs", "x")
        .file("r/.hidden", "h")
        .dir("r/c");
    fs
}

#[test]
fn delivers_entries_depth_first_with_children_sorted_by_name() {
    let outcome = walk(&sample_tree(), all());
    assert_eq!(
        outcome.entries,
        [
            ".hidden",
            "a",
            "a/deep",
            "a/deep/x.rs",
            "a/y.txt",
            "a/z.rs",
            "b.txt",
            "c"
        ]
    );
    assert!(outcome.summary.complete);
    assert_eq!(outcome.summary.entries, 8);
}

#[test]
fn order_is_identical_for_any_number_of_threads() {
    let mut fs = MemFs::new();
    for top in 0..12 {
        for inner in 0..6 {
            for file in 0..4 {
                fs.file(&format!("r/d{top:02}/e{inner}/f{file}.txt"), "");
            }
        }
    }
    let single = walk(&fs, all()).entries;
    for threads in [2, 4, 8] {
        let parallel = walk(&fs, WalkOptions { threads, ..all() }).entries;
        assert_eq!(parallel, single, "{threads} threads");
    }
    assert_eq!(single.len(), 12 + 12 * 6 + 12 * 6 * 4);
}

#[test]
fn root_is_delivered_at_depth_zero_by_its_name() {
    let fs = sample_tree();
    let options = WalkOptions {
        max_depth: Some(0),
        kind: KindFilter::Any,
        threads: 1,
        ..WalkOptions::default()
    };
    assert_eq!(
        walk_roots(&fs, &["r/b.txt"], options.clone())
            .expect("walks")
            .entries,
        ["b.txt"]
    );
    assert_eq!(
        walk_roots(&fs, &["r/a"], options).expect("walks").entries,
        ["a"]
    );
}

#[test]
fn root_file_is_listed_without_reading_a_directory() {
    let fs = sample_tree();
    let outcome = walk_roots(
        &fs,
        &["r/b.txt"],
        WalkOptions {
            threads: 1,
            ..WalkOptions::default()
        },
    )
    .expect("walks");
    assert_eq!(outcome.entries, ["b.txt"]);
}

#[test]
fn missing_root_fails_with_not_found() {
    let error = walk_roots(&sample_tree(), &["r/missing"], all())
        .err()
        .expect("root is missing");
    assert_eq!(error.code(), ErrorCode::NotFound);
}

#[test]
fn unreadable_root_fails_with_permission() {
    let mut fs = sample_tree();
    fs.deny("r");
    let error = walk_roots(&fs, &["r"], all())
        .err()
        .expect("root is unreadable");
    assert_eq!(error.code(), ErrorCode::Permission);
}

#[test]
fn roots_are_walked_in_the_given_order() {
    let fs = sample_tree();
    let outcome = walk_roots(
        &fs,
        &["r/c", "r/a/deep", "r/b.txt"],
        WalkOptions {
            min_depth: 0,
            ..all()
        },
    )
    .expect("walks");
    assert_eq!(outcome.entries, ["c", "deep", "x.rs", "b.txt"]);
}

#[test]
fn max_depth_limits_delivery_and_descent() {
    let outcome = walk(
        &sample_tree(),
        WalkOptions {
            max_depth: Some(1),
            ..all()
        },
    );
    assert_eq!(outcome.entries, [".hidden", "a", "b.txt", "c"]);
}

#[test]
fn min_depth_skips_shallow_entries_but_still_descends() {
    let outcome = walk(
        &sample_tree(),
        WalkOptions {
            min_depth: 2,
            ..all()
        },
    );
    assert_eq!(
        outcome.entries,
        ["a/deep", "a/deep/x.rs", "a/y.txt", "a/z.rs"]
    );
}

#[test]
fn kind_filter_selects_files_directories_links_or_everything() {
    let mut fs = sample_tree();
    fs.link("r/to-b", "b.txt").special("r/fifo");
    let kinds = |kind| walk(&fs, WalkOptions { kind, ..all() }).entries;
    assert_eq!(
        kinds(KindFilter::File),
        [".hidden", "a/deep/x.rs", "a/y.txt", "a/z.rs", "b.txt"]
    );
    assert_eq!(kinds(KindFilter::Directory), ["a", "a/deep", "c"]);
    assert_eq!(kinds(KindFilter::Link), ["to-b"]);
    assert!(kinds(KindFilter::Any).contains(&"fifo".to_owned()));
}

#[test]
fn hidden_false_skips_dot_entries_and_their_subtrees() {
    let mut fs = sample_tree();
    fs.file("r/.config/settings", "s");
    let outcome = walk(
        &fs,
        WalkOptions {
            hidden: false,
            ..all()
        },
    );
    assert!(
        outcome
            .entries
            .iter()
            .all(|entry| !entry.contains(".hidden") && !entry.contains(".config"))
    );
    assert_eq!(outcome.entries.len(), 7);
}

#[test]
fn glob_without_double_star_matches_top_level_only() {
    let globbed = |glob: &str| {
        walk(
            &sample_tree(),
            WalkOptions {
                globs: vec![glob.to_owned()],
                ..all()
            },
        )
        .entries
    };
    assert_eq!(globbed("*.txt"), ["b.txt"]);
    assert_eq!(globbed("**/*.rs"), ["a/deep/x.rs", "a/z.rs"]);
    assert_eq!(globbed("a/*"), ["a/deep", "a/y.txt", "a/z.rs"]);
}

#[test]
fn any_glob_of_a_list_selects_an_entry() {
    let options = WalkOptions {
        globs: vec!["*.txt".to_owned(), "a/deep/*".to_owned()],
        ..all()
    };
    assert_eq!(
        walk(&sample_tree(), options).entries,
        ["a/deep/x.rs", "b.txt"]
    );
}

#[test]
fn glob_case_sensitivity_follows_the_option() {
    let mut fs = MemFs::new();
    fs.file("r/README.md", "").file("r/readme.txt", "");
    let globbed = |case_sensitive| {
        let options = WalkOptions {
            globs: vec!["readme*".to_owned()],
            case_sensitive,
            ..all()
        };
        walk(&fs, options).entries
    };
    assert_eq!(globbed(true), ["readme.txt"]);
    assert_eq!(globbed(false), ["README.md", "readme.txt"]);
}

#[test]
fn invalid_glob_is_rejected_before_walking() {
    let options = WalkOptions {
        globs: vec!["{a,".to_owned()],
        ..all()
    };
    let error = walk_roots(&sample_tree(), &["r"], options)
        .err()
        .expect("glob is invalid");
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn exclude_patterns_follow_gitignore_rules() {
    let mut fs = MemFs::new();
    fs.file("r/app.log", "")
        .file("r/keep.log", "")
        .file("r/src/main.rs", "")
        .file("r/src/gen/out.rs", "")
        .file("r/gen/top.rs", "");
    let options = WalkOptions {
        exclude: vec![
            "*.log".to_owned(),
            "!keep.log".to_owned(),
            "/gen".to_owned(),
        ],
        kind: KindFilter::File,
        ..all()
    };
    assert_eq!(
        walk(&fs, options).entries,
        ["keep.log", "src/gen/out.rs", "src/main.rs"]
    );
}

#[test]
fn negation_does_not_reinclude_entries_below_an_excluded_directory() {
    let mut fs = MemFs::new();
    fs.file("r/build/keep.txt", "")
        .file("r/build/drop.txt", "")
        .file("r/top.txt", "");
    let options = WalkOptions {
        exclude: vec!["build/".to_owned(), "!build/keep.txt".to_owned()],
        ..all()
    };
    assert_eq!(walk(&fs, options).entries, ["top.txt"]);
}

#[test]
fn exclude_presets_skip_named_directories_only() {
    let mut fs = MemFs::new();
    fs.file("r/.git/HEAD", "")
        .file("r/node_modules/pkg/index.js", "")
        .file("r/target/debug/app", "")
        .file("r/__pycache__/m.pyc", "")
        .file("r/src/build", "a file named like a preset directory")
        .file("r/src/main.rs", "");
    let options = WalkOptions {
        exclude_presets: vec![
            ExcludePreset::Vcs,
            ExcludePreset::Deps,
            ExcludePreset::Build,
            ExcludePreset::Cache,
        ],
        ..all()
    };
    assert_eq!(
        walk(&fs, options).entries,
        ["src", "src/build", "src/main.rs"]
    );
}

#[test]
fn vcs_preset_skips_a_git_link_file() {
    let mut fs = MemFs::new();
    fs.file("r/.git", "gitdir: /elsewhere").file("r/a.txt", "");
    let options = WalkOptions {
        exclude_presets: vec![ExcludePreset::Vcs],
        ..all()
    };
    assert_eq!(walk(&fs, options).entries, ["a.txt"]);
}

/// Repository with nested ignore files: the root ignores `*.log` and
/// `tmp/`, `sub` re-includes `keep.log`.
fn repository() -> MemFs {
    let mut fs = MemFs::new();
    fs.dir("r/.git/info")
        .file("r/.gitignore", "*.log\ntmp/\n")
        .file("r/a.log", "")
        .file("r/a.rs", "")
        .file("r/tmp/t.rs", "")
        .file("r/sub/.gitignore", "!keep.log\n")
        .file("r/sub/keep.log", "")
        .file("r/sub/other.log", "");
    fs
}

fn source_files() -> WalkOptions {
    WalkOptions {
        gitignore: true,
        kind: KindFilter::File,
        exclude_presets: vec![ExcludePreset::Vcs],
        ..all()
    }
}

#[test]
fn nested_gitignore_files_apply_with_deeper_rules_first() {
    let outcome = walk(&repository(), source_files());
    assert_eq!(
        outcome.entries,
        [".gitignore", "a.rs", "sub/.gitignore", "sub/keep.log"]
    );
}

#[test]
fn gitignore_false_ignores_ignore_files() {
    let outcome = walk(
        &repository(),
        WalkOptions {
            gitignore: false,
            ..source_files()
        },
    );
    assert_eq!(outcome.entries.len(), 7);
}

#[test]
fn gitignore_outside_a_repository_is_not_applied_but_ignore_files_are() {
    let mut fs = MemFs::new();
    fs.file("r/.gitignore", "*.log\n")
        .file("r/.ignore", "*.tmp\n")
        .file("r/a.log", "")
        .file("r/a.tmp", "");
    let outcome = walk(&fs, source_files());
    assert_eq!(outcome.entries, [".gitignore", ".ignore", "a.log"]);
}

#[test]
fn ignore_file_takes_precedence_over_gitignore() {
    let mut fs = repository();
    fs.file("r/.ignore", "!a.log\n");
    let outcome = walk(&fs, source_files());
    assert!(outcome.entries.contains(&"a.log".to_owned()));
}

#[test]
fn info_exclude_and_global_file_apply_inside_the_repository() {
    let mut fs = repository();
    fs.file("r/.git/info/exclude", "a.rs\n")
        .file("home/global-ignore", "keep.log\n");
    let options = WalkOptions {
        global_ignore_file: Some(MemFs::path("home/global-ignore")),
        ..source_files()
    };
    let outcome = walk(&fs, options);
    assert_eq!(
        outcome.entries,
        [".gitignore", "sub/.gitignore", "sub/keep.log"]
    );
    let mut fs = repository();
    fs.file("home/global-ignore", "*.rs\n");
    let options = WalkOptions {
        global_ignore_file: Some(MemFs::path("home/global-ignore")),
        ..source_files()
    };
    assert_eq!(
        walk(&fs, options).entries,
        [".gitignore", "sub/.gitignore", "sub/keep.log"]
    );
}

#[test]
fn missing_global_file_is_not_an_error() {
    let options = WalkOptions {
        global_ignore_file: Some(MemFs::path("home/none")),
        ..source_files()
    };
    let outcome = walk(&repository(), options);
    assert!(outcome.unreadable.is_empty());
    assert_eq!(outcome.entries.len(), 4);
}

#[test]
fn ignore_files_of_ancestors_apply_to_a_root_inside_a_repository() {
    let fs = repository();
    let outcome = walk_roots(&fs, &["r/sub"], source_files()).expect("walks");
    assert_eq!(outcome.entries, [".gitignore", "keep.log"]);
}

#[test]
fn nested_repository_does_not_inherit_gitignore_of_the_enclosing_one() {
    let mut fs = repository();
    fs.dir("r/nested/.git").file("r/nested/n.log", "");
    let outcome = walk(&fs, source_files());
    assert!(outcome.entries.contains(&"nested/n.log".to_owned()));
}

#[test]
fn unreadable_ignore_file_is_reported_and_walk_continues() {
    let mut fs = repository();
    fs.deny("r/sub/.gitignore");
    let outcome = walk(&fs, source_files());
    assert_eq!(
        outcome.unreadable,
        [("r/sub/.gitignore".to_owned(), FsErrorKind::PermissionDenied)]
    );
    assert!(outcome.entries.contains(&"a.rs".to_owned()));
}

#[test]
fn links_are_not_followed_by_default() {
    let mut fs = sample_tree();
    fs.link("r/link", "a");
    let outcome = walk(&fs, all());
    assert!(outcome.entries.contains(&"link".to_owned()));
    assert!(
        !outcome
            .entries
            .iter()
            .any(|entry| entry.starts_with("link/"))
    );
}

#[test]
fn followed_links_are_entered_and_classified_by_target() {
    let mut fs = sample_tree();
    fs.link("r/link", "a/deep").link("r/to-file", "b.txt");
    let options = WalkOptions {
        follow_links: true,
        kind: KindFilter::File,
        ..all()
    };
    let outcome = walk(&fs, options);
    assert!(outcome.entries.contains(&"link/x.rs".to_owned()));
    assert!(outcome.entries.contains(&"to-file".to_owned()));
}

#[test]
fn link_loops_are_delivered_once_and_counted() {
    let mut fs = MemFs::new();
    fs.file("r/a/f.txt", "")
        .link("r/a/up", "..")
        .link("r/a/self", "../a");
    let options = WalkOptions {
        follow_links: true,
        ..all()
    };
    let outcome = walk(&fs, options);
    assert_eq!(outcome.entries, ["a", "a/f.txt", "a/self", "a/up"]);
    assert_eq!(outcome.summary.link_loops, 2);
    assert_eq!(
        outcome.summary.link_loop_sample,
        [MemFs::path("r/a/self"), MemFs::path("r/a/up")]
    );
}

#[test]
fn broken_links_are_delivered_as_links_when_following() {
    let mut fs = MemFs::new();
    fs.link("r/dangling", "missing").link("r/cycle", "cycle");
    let options = WalkOptions {
        follow_links: true,
        kind: KindFilter::Link,
        ..all()
    };
    let outcome = walk(&fs, options);
    assert_eq!(outcome.entries, ["cycle", "dangling"]);
    assert!(outcome.unreadable.is_empty());
}

#[test]
fn unreadable_directories_are_counted_reported_and_skipped() {
    let mut fs = MemFs::new();
    for index in 0..7 {
        fs.file(&format!("r/locked{index}/secret"), "");
    }
    fs.file("r/open/file", "");
    for index in 0..7 {
        fs.deny(&format!("r/locked{index}"));
    }
    let outcome = walk(&fs, all());
    assert_eq!(outcome.summary.permission_denied, 7);
    assert_eq!(outcome.summary.permission_denied_sample.len(), SAMPLE_LIMIT);
    assert_eq!(
        outcome.summary.permission_denied_sample[0],
        MemFs::path("r/locked0")
    );
    assert_eq!(outcome.unreadable.len(), 7);
    assert!(outcome.entries.contains(&"open/file".to_owned()));
    assert!(outcome.entries.contains(&"locked3".to_owned()));
    assert!(
        !outcome
            .entries
            .iter()
            .any(|entry| entry.ends_with("secret"))
    );
}

#[test]
fn metadata_is_read_only_when_requested() {
    let fs = sample_tree();
    let walker = Walker::new(WalkOptions {
        metadata: true,
        ..all()
    })
    .expect("options are valid");
    let mut sizes = Vec::new();
    walker
        .run(&fs, &[MemFs::path("r")], &|| Ok(()), |item| {
            if let WalkItem::Entry(entry) = item {
                sizes.push(entry.meta.map(|meta| meta.size));
            }
            ControlFlow::Continue(())
        })
        .expect("walks");
    assert!(sizes.iter().all(Option::is_some));
    let walker = Walker::new(all()).expect("options are valid");
    walker
        .run(&fs, &[MemFs::path("r")], &|| Ok(()), |item| {
            if let WalkItem::Entry(entry) = item {
                assert!(entry.meta.is_none());
            }
            ControlFlow::Continue(())
        })
        .expect("walks");
}

#[test]
fn entry_kinds_are_read_when_the_listing_omits_them() {
    let mut fs = MemFs::new().without_kinds();
    fs.file("r/a/f.txt", "").link("r/l", "a");
    let outcome = walk(
        &fs,
        WalkOptions {
            kind: KindFilter::Directory,
            ..all()
        },
    );
    assert_eq!(outcome.entries, ["a"]);
}

#[test]
fn names_that_are_not_unicode_are_delivered_and_matched() {
    let mut fs = MemFs::new();
    fs.file_named("r", non_unicode_name(".bin"))
        .file("r/plain.txt", "");
    let walker = Walker::new(WalkOptions {
        globs: vec!["*.bin".to_owned()],
        ..all()
    })
    .expect("options are valid");
    let mut names = Vec::new();
    walker
        .run(&fs, &[MemFs::path("r")], &|| Ok(()), |item| {
            if let WalkItem::Entry(entry) = item {
                names.push(entry.name);
            }
            ControlFlow::Continue(())
        })
        .expect("walks");
    assert_eq!(names, [non_unicode_name(".bin")]);
    assert!(names[0].to_str().is_none());
}

#[test]
fn decomposed_names_are_kept_as_stored() {
    let mut fs = MemFs::new();
    fs.file("r/cafe\u{301}.txt", "").file("r/caf\u{e9}.txt", "");
    let outcome = walk(&fs, all());
    assert_eq!(outcome.entries, ["cafe\u{301}.txt", "caf\u{e9}.txt"]);
}

#[test]
fn sink_can_stop_the_walk() {
    let fs = sample_tree();
    let walker = Walker::new(WalkOptions {
        threads: 3,
        ..all()
    })
    .expect("options are valid");
    let mut seen = 0;
    let summary = walker
        .run(&fs, &[MemFs::path("r")], &|| Ok(()), |_| {
            seen += 1;
            if seen == 3 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .expect("walks");
    assert_eq!(seen, 3);
    assert!(!summary.complete);
}

#[test]
fn cancellation_ends_the_walk_with_the_cancel_error() {
    let mut fs = MemFs::new();
    for index in 0..3000 {
        fs.file(&format!("r/d{}/f{index}", index % 10), "");
    }
    let cancel = CancelToken::new();
    let walker = Walker::new(WalkOptions {
        threads: 2,
        ..all()
    })
    .expect("options are valid");
    let mut delivered = 0;
    let result = walker.run(&fs, &[MemFs::path("r")], &|| cancel.check(), |_| {
        delivered += 1;
        if delivered == 10 {
            cancel.cancel(CancelReason::Requested);
        }
        ControlFlow::Continue(())
    });
    let error = result.expect_err("walk is cancelled");
    assert_eq!(error.code(), ErrorCode::Cancelled);
    assert!(delivered < 3000);
}

#[test]
fn cancelled_token_stops_before_the_first_root() {
    let cancel = CancelToken::new();
    cancel.cancel(CancelReason::Timeout);
    let walker = Walker::new(all()).expect("options are valid");
    let error = walker
        .run(
            &sample_tree(),
            &[MemFs::path("r")],
            &|| cancel.check(),
            |_| ControlFlow::Continue(()),
        )
        .expect_err("token is cancelled");
    assert_eq!(error.code(), ErrorCode::Timeout);
}

#[test]
fn entry_reports_kind_and_effective_kind() {
    let mut fs = MemFs::new();
    fs.dir("r/dir").link("r/link", "dir");
    let walker = Walker::new(WalkOptions {
        follow_links: true,
        max_depth: Some(1),
        ..all()
    })
    .expect("options are valid");
    let mut kinds = Vec::new();
    walker
        .run(&fs, &[MemFs::path("r")], &|| Ok(()), |item| {
            if let WalkItem::Entry(entry) = item {
                kinds.push((
                    entry.kind,
                    entry.effective_kind(),
                    entry.is_dir(),
                    entry.depth,
                ));
            }
            ControlFlow::Continue(())
        })
        .expect("walks");
    assert_eq!(
        kinds,
        [
            (FileKind::Directory, FileKind::Directory, true, 1),
            (FileKind::Symlink, FileKind::Directory, true, 1)
        ]
    );
}
