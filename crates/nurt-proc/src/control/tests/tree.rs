// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The process tree of the operating system, used without the supervisor,
//! and start errors reported before a program runs.

use super::*;

#[test]
fn missing_input_file_is_a_spawn_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut missing = request(&["count-input"], limits());
    missing.stdin = StdinSource::File(dir.path().join("absent.txt"));
    let error = super::super::start(missing, Arc::new(Collect::default()), CancelToken::new())
        .err()
        .map(|error| error.code());
    assert_eq!(error, Some(ErrorCode::Spawn));
}

#[cfg(unix)]
#[test]
fn a_tree_reports_the_exit_code_and_usage_of_its_program() {
    use crate::os::{Exit, Tree};

    let (tree, streams) = Tree::spawn(&request(&["exit-with", "3"], limits())).unwrap();
    assert!(streams.stdin.is_none());
    assert!(streams.stderr.is_some());
    let mut stdout = String::new();
    std::io::Read::read_to_string(&mut { streams.stdout }, &mut stdout).unwrap();
    assert_eq!(stdout, "exit 3\n");
    tree.wait_exit().unwrap();
    assert!(tree.pid() > 0);
    let finished = tree.finish().unwrap();
    assert!(matches!(finished.exit, Exit::Code(3)));
    assert_eq!(finished.limit_hit, None);
}

#[cfg(unix)]
#[test]
fn a_tree_whose_leader_could_not_be_collected_is_not_signalled_again() {
    use crate::os::Tree;

    let (tree, streams) = Tree::spawn(&request(&["exit-with", "3"], limits())).unwrap();
    let mut stdout = String::new();
    std::io::Read::read_to_string(&mut { streams.stdout }, &mut stdout).unwrap();
    tree.wait_exit().unwrap();
    let pid = libc::pid_t::try_from(tree.pid()).unwrap();
    // SAFETY: `pid` is the exited child of this process; collecting it here
    // makes the later collection by the tree fail.
    let collected = unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0) };
    assert_eq!(collected, pid);
    assert!(!tree.is_collected());
    assert!(tree.finish().is_err());
    assert!(tree.is_collected());
    tree.kill().unwrap();
}

#[cfg(unix)]
#[test]
fn a_tree_with_merged_streams_has_no_error_stream() {
    use crate::os::Tree;

    let mut merged = request(&["print-both"], limits());
    merged.merge_stderr = true;
    merged.stdin = StdinSource::Bytes(b"unused".to_vec());
    let (tree, streams) = Tree::spawn(&merged).unwrap();
    assert!(streams.stderr.is_none());
    assert!(streams.stdin.is_some());
    let mut output = String::new();
    std::io::Read::read_to_string(&mut { streams.stdout }, &mut output).unwrap();
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort_unstable();
    assert_eq!(lines, ["err", "out"]);
    tree.wait_exit().unwrap();
    tree.finish().unwrap();
}
