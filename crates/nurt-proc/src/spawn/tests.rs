// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of program resolution, arguments and environment.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::Launcher;
use nurt_contracts::services::EnvVar;

use super::{
    ChildEnv, InheritEnv, Platform, check_cwd, prepare_launch, program_text, record_path, resolve,
    text_literal,
};

fn command(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn vars(items: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    items
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

fn set(name: &str, value: &str) -> EnvVar {
    EnvVar {
        name: name.to_owned(),
        value: Some(value.to_owned()),
    }
}

fn env_with_path(platform: Platform, path: &Path, extra: &[(&str, &str)]) -> ChildEnv {
    let mut inherited = vars(extra);
    inherited.push((OsString::from("PATH"), path.as_os_str().to_owned()));
    ChildEnv::build(platform, inherited, InheritEnv::All, [], &[]).unwrap()
}

/// Creates an empty file; on Unix with execute permission when `executable`.
fn touch(path: &Path, executable: bool) {
    std::fs::write(path, b"").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o755 } else { 0o644 };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = executable;
}

fn problem(error: &Error) -> Option<&str> {
    error.data().problem.as_deref()
}

mod resolution {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn finds_the_program_in_path_of_the_child_environment() {
        let bin = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        touch(&bin.path().join("tool"), true);
        let env = env_with_path(Platform::Unix, bin.path(), &[]);
        let resolved = resolve(&command(&["tool"]), cwd.path(), &env).unwrap();
        assert_eq!(resolved.path, bin.path().join("tool"));
        assert_eq!(resolved.launcher, Launcher::Direct);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_given_in_env_replaces_the_inherited_path() {
        let inherited = tempfile::tempdir().unwrap();
        let given = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        touch(&given.path().join("tool"), true);
        let env = ChildEnv::build(
            Platform::Unix,
            vars(&[("PATH", &inherited.path().to_string_lossy())]),
            InheritEnv::All,
            [],
            &[set("PATH", &given.path().to_string_lossy())],
        )
        .unwrap();
        let resolved = resolve(&command(&["tool"]), cwd.path(), &env).unwrap();
        assert_eq!(resolved.path, given.path().join("tool"));
    }

    #[cfg(unix)]
    #[test]
    fn skips_files_without_execute_permission() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        touch(&first.path().join("tool"), false);
        touch(&second.path().join("tool"), true);
        let joined = format!("{}::{}", first.path().display(), second.path().display());
        let env = env_with_path(Platform::Unix, Path::new(&joined), &[]);
        let resolved = resolve(&command(&["tool"]), first.path(), &env).unwrap();
        assert_eq!(resolved.path, second.path().join("tool"));
    }

    #[cfg(unix)]
    #[test]
    fn a_relative_entry_of_path_is_taken_against_the_working_directory() {
        let cwd = tempfile::tempdir().unwrap();
        std::fs::create_dir(cwd.path().join("bin")).unwrap();
        touch(&cwd.path().join("bin").join("tool"), true);
        let env = env_with_path(Platform::Unix, Path::new("bin"), &[]);
        let resolved = resolve(&command(&["tool"]), cwd.path(), &env).unwrap();
        assert_eq!(resolved.path, cwd.path().join("bin").join("tool"));
        assert!(resolved.path.is_absolute());
    }

    #[cfg(unix)]
    #[test]
    fn empty_entries_of_path_are_skipped() {
        let bin = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        touch(&cwd.path().join("tool"), true);
        touch(&bin.path().join("tool"), true);
        let joined = format!(":{}:", bin.path().display());
        let env = env_with_path(Platform::Unix, Path::new(&joined), &[]);
        let resolved = resolve(&command(&["tool"]), cwd.path(), &env).unwrap();
        assert_eq!(resolved.path, bin.path().join("tool"));
    }

    #[test]
    fn tries_the_extensions_of_pathext_in_order() {
        let bin = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        touch(&bin.path().join("tool.CMD"), true);
        touch(&bin.path().join("tool.EXE"), true);
        let env = env_with_path(
            Platform::Windows,
            bin.path(),
            &[("PATHEXT", ".COM;.EXE;.BAT;.CMD")],
        );
        let resolved = resolve(&command(&["tool"]), cwd.path(), &env).unwrap();
        assert_eq!(resolved.path, bin.path().join("tool.EXE"));
        assert_eq!(resolved.launcher, Launcher::Direct);
    }

    #[test]
    fn a_name_with_an_extension_is_tried_as_written_first() {
        let bin = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        touch(&bin.path().join("tool.cmd"), true);
        touch(&bin.path().join("tool.exe"), true);
        let env = env_with_path(Platform::Windows, bin.path(), &[("PATHEXT", ".EXE;.CMD")]);
        let resolved = resolve(&command(&["tool.cmd"]), cwd.path(), &env).unwrap();
        assert_eq!(resolved.path, bin.path().join("tool.cmd"));
        assert_eq!(resolved.launcher, Launcher::Cmd);
    }

    #[test]
    fn batch_files_start_through_the_command_interpreter() {
        let bin = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        touch(&bin.path().join("build.BAT"), true);
        let env = env_with_path(
            Platform::Windows,
            bin.path(),
            &[("PATHEXT", ".EXE;.BAT"), ("SystemRoot", r"D:\Win")],
        );
        let launch = prepare_launch(&command(&["build", "a&b"]), cwd.path(), &env).unwrap();
        assert_eq!(launch.launcher, Launcher::Cmd);
        assert_eq!(launch.program, bin.path().join("build.BAT"));
        assert_eq!(launch.executable, PathBuf::from(r"D:\Win\System32\cmd.exe"));
        assert!(launch.args.is_empty());
        let script = bin.path().join("build.BAT");
        assert_eq!(
            launch.command_line,
            Some(OsString::from(format!(
                "cmd.exe /e:ON /v:OFF /d /c \"\"{}\" \"a&b\"\"",
                script.display()
            )))
        );
        assert_eq!(launch.argv().len(), 1);
    }

    #[test]
    fn variable_names_are_compared_without_case_on_windows() {
        let env = ChildEnv::build(
            Platform::Windows,
            vars(&[("Path", "C:/bin")]),
            InheritEnv::All,
            [],
            &[set("PATH", "D:/bin")],
        )
        .unwrap();
        assert_eq!(env.get("path"), Some(std::ffi::OsStr::new("D:/bin")));
        assert_eq!(env.names().count(), 1);
    }

    #[test]
    fn lists_similar_names_of_the_search_path_as_candidates() {
        let bin = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        for name in ["cargo", "cargo-fmt", "ls", "carp"] {
            touch(&bin.path().join(name), true);
        }
        let env = env_with_path(Platform::Unix, bin.path(), &[]);
        let error = resolve(&command(&["cargoo", "build"]), cwd.path(), &env).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ProgramNotFound);
        assert_eq!(problem(&error), Some("not_in_path"));
        let candidates: Vec<&str> = error
            .data()
            .candidates
            .iter()
            .map(|candidate| candidate.program.as_str())
            .collect();
        assert_eq!(candidates.first(), Some(&r#"run ["cargo", "build"]"#));
        assert!(!candidates.iter().any(|program| program.contains("\"ls\"")));
        assert!(candidates.len() <= 5);
        assert_eq!(
            error.data().fix.first().map(String::as_str),
            Some(r#"run ["cargo", "build"]"#)
        );
    }

    #[test]
    fn compares_candidate_names_without_case_and_extension_on_windows() {
        let bin = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        touch(&bin.path().join("Cargo.EXE"), true);
        touch(&bin.path().join("readme.txt"), true);
        let env = env_with_path(Platform::Windows, bin.path(), &[("PATHEXT", ".EXE")]);
        let error = resolve(&command(&["CARGOO"]), cwd.path(), &env).unwrap_err();
        let candidates: Vec<&str> = error
            .data()
            .candidates
            .iter()
            .map(|candidate| candidate.program.as_str())
            .collect();
        assert_eq!(candidates, vec![r#"run ["Cargo"]"#]);
    }

    #[cfg(unix)]
    #[test]
    fn suggests_the_file_of_the_working_directory() {
        let bin = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        touch(&cwd.path().join("build.sh"), true);
        let env = env_with_path(Platform::Unix, bin.path(), &[]);
        let error = resolve(&command(&["build.sh", "all"]), cwd.path(), &env).unwrap_err();
        assert_eq!(
            error.data().fix,
            vec![r#"run ["./build.sh", "all"]"#.to_owned()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_with_a_directory_is_taken_from_the_working_directory() {
        let cwd = tempfile::tempdir().unwrap();
        std::fs::create_dir(cwd.path().join("bin")).unwrap();
        touch(&cwd.path().join("bin/tool"), true);
        let env = env_with_path(Platform::Unix, Path::new("/nonexistent"), &[]);
        let resolved = resolve(&command(&["./bin/tool"]), cwd.path(), &env).unwrap();
        assert_eq!(resolved.path, cwd.path().join("./bin/tool"));
    }

    #[test]
    fn a_directory_or_a_missing_file_cannot_be_started() {
        let cwd = tempfile::tempdir().unwrap();
        std::fs::create_dir(cwd.path().join("dir")).unwrap();
        let env = env_with_path(Platform::Unix, Path::new("/nonexistent"), &[]);
        let directory = resolve(&command(&["./dir"]), cwd.path(), &env).unwrap_err();
        assert_eq!(directory.code(), ErrorCode::Spawn);
        assert_eq!(problem(&directory), Some("program_is_directory"));
        let missing = resolve(&command(&["./missing"]), cwd.path(), &env).unwrap_err();
        assert_eq!(missing.code(), ErrorCode::ProgramNotFound);
        assert_eq!(problem(&missing), Some("missing_file"));
    }

    #[cfg(unix)]
    #[test]
    fn a_file_without_execute_permission_is_a_spawn_error() {
        let cwd = tempfile::tempdir().unwrap();
        touch(&cwd.path().join("script"), false);
        let env = env_with_path(Platform::Unix, Path::new("/nonexistent"), &[]);
        let error = resolve(&command(&["./script"]), cwd.path(), &env).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Spawn);
        assert_eq!(problem(&error), Some("not_executable"));
    }
}

mod environment {
    use super::*;

    #[test]
    fn applies_inherited_variables_then_defaults_then_changes() {
        let env = ChildEnv::build(
            Platform::Unix,
            vars(&[("HOME", "/home/user"), ("TERM", "xterm"), ("KEEP", "1")]),
            InheritEnv::All,
            [("TERM", "dumb"), ("NO_COLOR", "1")],
            &[
                set("NO_COLOR", "0"),
                EnvVar {
                    name: "HOME".to_owned(),
                    value: None,
                },
                set("EMPTY", ""),
            ],
        )
        .unwrap();
        assert_eq!(
            env.into_vars(),
            vars(&[
                ("TERM", "dumb"),
                ("KEEP", "1"),
                ("NO_COLOR", "0"),
                ("EMPTY", "")
            ])
        );
    }

    #[test]
    fn an_empty_environment_holds_only_defaults_and_changes_on_unix() {
        let env = ChildEnv::build(
            Platform::Unix,
            vars(&[("HOME", "/home/user"), ("PATH", "/usr/bin")]),
            InheritEnv::None,
            [("TERM", "dumb")],
            &[set("A", "1")],
        )
        .unwrap();
        assert_eq!(env.into_vars(), vars(&[("TERM", "dumb"), ("A", "1")]));
    }

    #[test]
    fn an_empty_environment_keeps_the_variables_windows_requires() {
        let env = ChildEnv::build(
            Platform::Windows,
            vars(&[
                ("SYSTEMROOT", r"C:\Windows"),
                ("ComSpec", r"C:\Windows\System32\cmd.exe"),
                ("PATHEXT", ".EXE"),
                ("USERPROFILE", r"C:\Users\user"),
            ]),
            InheritEnv::None,
            [],
            &[],
        )
        .unwrap();
        let names: Vec<String> = env
            .names()
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["SYSTEMROOT", "ComSpec", "PATHEXT"]);
    }

    #[test]
    fn rejects_names_and_values_a_program_cannot_receive() {
        for (name, value, expected) in [
            ("", "x", "empty_environment_name"),
            ("A=B", "x", "equals_in_environment_name"),
            ("A\0B", "x", "nul_in_environment"),
            ("A", "x\0y", "nul_in_environment"),
        ] {
            let error = ChildEnv::build(
                Platform::Unix,
                Vec::new(),
                InheritEnv::All,
                [],
                &[set(name, value)],
            )
            .unwrap_err();
            assert_eq!(error.code(), ErrorCode::Arg);
            assert_eq!(problem(&error), Some(expected));
        }
    }
}

mod arguments {
    use super::*;

    #[test]
    fn rejects_a_nul_character_with_its_position_and_a_fix() {
        let cwd = tempfile::tempdir().unwrap();
        let env = env_with_path(Platform::Unix, cwd.path(), &[]);
        let error =
            prepare_launch(&command(&["tool", "ok", "a\0b"]), cwd.path(), &env).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Arg);
        assert_eq!(problem(&error), Some("nul_in_argument"));
        assert_eq!(
            error.data().details.get("index"),
            Some(&serde_json::Value::from(2))
        );
        assert_eq!(
            error.data().fix,
            vec![r#"run ["tool", "ok"] stdin: "a\0b""#.to_owned()]
        );
    }

    #[test]
    fn rejects_a_line_break_in_an_argument_of_a_batch_file() {
        let bin = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        touch(&bin.path().join("job.CMD"), true);
        let env = env_with_path(Platform::Windows, bin.path(), &[("PATHEXT", ".CMD")]);
        let error = prepare_launch(&command(&["job", "two\nlines"]), cwd.path(), &env).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Arg);
        assert_eq!(problem(&error), Some("line_break_in_batch_argument"));
        assert_eq!(
            error.data().details.get("index"),
            Some(&serde_json::Value::from(1))
        );
        assert_eq!(
            error.data().fix,
            vec![r#"run ["job"] stdin: "two\nlines""#.to_owned()]
        );
    }

    #[test]
    fn a_line_break_reaches_a_program_that_is_not_a_batch_file() {
        let bin = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        touch(&bin.path().join("tool.EXE"), true);
        let env = env_with_path(Platform::Windows, bin.path(), &[("PATHEXT", ".EXE")]);
        let launch = prepare_launch(&command(&["tool", "two\nlines"]), cwd.path(), &env).unwrap();
        assert_eq!(launch.args, vec![OsString::from("two\nlines")]);
        assert!(launch.command_line.is_none());
    }

    #[test]
    fn an_empty_command_is_rejected() {
        let cwd = tempfile::tempdir().unwrap();
        let env = env_with_path(Platform::Unix, cwd.path(), &[]);
        let error = prepare_launch(&[], cwd.path(), &env).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Arg);
        assert_eq!(problem(&error), Some("empty_command"));
    }

    #[test]
    fn writes_commands_as_program_text() {
        assert_eq!(
            program_text(&command(&["printf", "a\"b", "c\\d", "tab\t", "\u{7}"])),
            r#"run ["printf", "a\"b", "c\\d", "tab\t", "\u{7}"]"#
        );
        assert_eq!(text_literal("line\r\n"), r#""line\r\n""#);
    }
}

mod paths {
    use super::*;

    #[test]
    fn writes_windows_paths_in_the_canonical_form() {
        assert_eq!(
            record_path(Path::new(r"c:\Users\user\app"), Platform::Windows).as_str(),
            "C:/Users/user/app"
        );
        assert_eq!(
            record_path(Path::new(r"\\?\d:\work"), Platform::Windows).as_str(),
            "D:/work"
        );
        assert_eq!(
            record_path(Path::new(r"\\server\share\x"), Platform::Windows).as_str(),
            "//server/share/x"
        );
        assert_eq!(
            record_path(Path::new("/home/user/a"), Platform::Unix).as_str(),
            "/home/user/a"
        );
    }

    #[test]
    fn writes_extended_length_network_paths_as_network_paths() {
        assert_eq!(
            record_path(Path::new(r"\\?\UNC\server\share\x"), Platform::Windows).as_str(),
            "//server/share/x"
        );
        assert_eq!(
            record_path(Path::new(r"\\?\unc\server\share"), Platform::Windows).as_str(),
            "//server/share"
        );
        assert_eq!(
            record_path(Path::new(r"\\?\C:\work\unc\a"), Platform::Windows).as_str(),
            "C:/work/unc/a"
        );
        assert_eq!(
            record_path(Path::new("d:/a/b"), Platform::Windows).as_str(),
            "D:/a/b"
        );
    }

    #[test]
    fn the_working_directory_must_be_an_existing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "x").unwrap();
        assert!(check_cwd(dir.path()).is_ok());
        let not_directory = check_cwd(&file).unwrap_err();
        assert_eq!(not_directory.code(), ErrorCode::Spawn);
        assert_eq!(problem(&not_directory), Some("cwd_not_directory"));
        let missing = check_cwd(&dir.path().join("missing")).unwrap_err();
        assert_eq!(missing.code(), ErrorCode::Spawn);
        assert_eq!(
            missing.data().cause.as_ref().map(|cause| cause.code()),
            Some(ErrorCode::NotFound)
        );
    }
}
