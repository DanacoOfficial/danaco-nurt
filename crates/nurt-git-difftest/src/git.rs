// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-difftest::git
// Responsibility:   The `git` program and the controlled environment of its runs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `git` program and the controlled environment of its runs.

use std::ffi::{OsStr, OsString};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{DifftestError, Result};
use crate::time::ScenarioClock;

/// Oldest supported version of the `git` program.
pub const MINIMUM_VERSION: GitVersion = GitVersion {
    major: 2,
    minor: 43,
    patch: 0,
};

/// Author and committer name of both sides.
pub const IDENTITY_NAME: &str = "Nurt Test";

/// Author and committer e-mail address of both sides.
pub const IDENTITY_EMAIL: &str = "test@nurt.invalid";

/// Content of the global configuration file of both sides.
///
/// Automatic garbage collection and maintenance are switched off: `git`
/// starts them detached after a commit, and the lock files and packs they
/// create would appear and disappear while a repository is copied or its
/// state is read.
pub const GLOBAL_CONFIG: &str = "[gc]\n\tauto = 0\n[maintenance]\n\tauto = false\n";

/// Variable that stops the search for a repository above the case directory.
#[cfg(unix)]
const CEILING_VARIABLE: &str = "GIT_CEILING_DIRECTORIES";

/// Variables passed through from the environment of the test process.
#[cfg(windows)]
const INHERITED: &[&str] = &[
    "PATH",
    "SYSTEMROOT",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "TEMP",
    "TMP",
];
#[cfg(not(windows))]
const INHERITED: &[&str] = &["PATH"];

/// Version of the `git` program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion {
    /// Major version.
    pub major: u32,
    /// Minor version.
    pub minor: u32,
    /// Patch level.
    pub patch: u32,
}

impl GitVersion {
    /// Parses the output of `git --version`, for example
    /// `git version 2.53.0` or `git version 2.45.1.windows.1`.
    #[must_use]
    pub fn parse(output: &str) -> Option<Self> {
        let number = output.trim().strip_prefix("git version ")?;
        let number = number.split_whitespace().next()?;
        let mut parts = number.split('.').map(|part| part.parse::<u32>());
        let major = parts.next()?.ok()?;
        let minor = parts.next()?.ok()?;
        let patch = parts.next().and_then(Result::ok).unwrap_or(0);
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

impl std::fmt::Display for GitVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Result of locating the `git` program.
#[derive(Debug)]
pub enum Availability {
    /// A supported version is on the search path.
    Ready(GitProgram),
    /// No `git` program is on the search path.
    Missing,
    /// The version on the search path is older than [`MINIMUM_VERSION`].
    TooOld(GitVersion),
}

/// The `git` program found on the search path.
#[derive(Debug, Clone)]
pub struct GitProgram {
    program: PathBuf,
    version: GitVersion,
}

/// Exit code and output of one `git` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    /// Exit code; `-1` when the process ended by a signal.
    pub code: i32,
    /// Standard output.
    pub stdout: Vec<u8>,
    /// Standard error.
    pub stderr: Vec<u8>,
}

impl GitOutput {
    /// Standard error as text, trimmed.
    #[must_use]
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_owned()
    }

    /// Standard output as text, trimmed.
    #[must_use]
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_owned()
    }
}

impl GitProgram {
    /// Looks for `git` on the search path and checks its version.
    #[must_use]
    pub fn locate() -> Availability {
        let program = PathBuf::from("git");
        let output = Command::new(&program)
            .arg("--version")
            .stdin(Stdio::null())
            .output();
        let Ok(output) = output else {
            return Availability::Missing;
        };
        let Some(version) = GitVersion::parse(&String::from_utf8_lossy(&output.stdout)) else {
            return Availability::Missing;
        };
        if version < MINIMUM_VERSION {
            return Availability::TooOld(version);
        }
        Availability::Ready(Self { program, version })
    }

    /// Version of the program.
    #[must_use]
    pub fn version(&self) -> GitVersion {
        self.version
    }

    /// Program name or path used to start `git`.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Runs `git` with `args` in `dir` and returns its result whatever the
    /// exit code.
    ///
    /// # Errors
    ///
    /// Returns [`DifftestError::GitSpawn`] when the program cannot be started.
    pub fn run<S: AsRef<OsStr>>(&self, env: &GitEnv, dir: &Path, args: &[S]) -> Result<GitOutput> {
        self.run_with_input(env, dir, args, None)
    }

    /// Runs `git` with `bytes` on its standard input.
    ///
    /// # Errors
    ///
    /// Returns [`DifftestError::GitSpawn`] when the program cannot be started
    /// or the input cannot be written.
    pub fn run_with_input<S: AsRef<OsStr>>(
        &self,
        env: &GitEnv,
        dir: &Path,
        args: &[S],
        input: Option<&[u8]>,
    ) -> Result<GitOutput> {
        let mut command = Command::new(&self.program);
        command
            .args(args)
            .current_dir(dir)
            .env_clear()
            .envs(env.vars.iter().map(|(name, value)| (name, value)))
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(DifftestError::GitSpawn)?;
        let stdin = child.stdin.take();
        // The input is written on a second thread: a program that answers
        // while it reads (`cat-file --batch`) blocks once both pipes are full.
        let (output, written) = std::thread::scope(|scope| {
            let writer = scope.spawn(move || match (input, stdin) {
                (Some(bytes), Some(mut stdin)) => stdin.write_all(bytes),
                _ => Ok(()),
            });
            let output = child.wait_with_output();
            let written = writer
                .join()
                .unwrap_or_else(|_| Err(std::io::Error::other("the input writer stopped")));
            (output, written)
        });
        let output = output.map_err(DifftestError::GitSpawn)?;
        match written {
            Err(error) if error.kind() != std::io::ErrorKind::BrokenPipe => {
                return Err(DifftestError::GitSpawn(error));
            }
            _ => {}
        }
        Ok(GitOutput {
            code: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    /// Runs `git` and requires exit code 0; returns the standard output.
    ///
    /// # Errors
    ///
    /// Returns [`DifftestError::GitFailed`] for another exit code and
    /// [`DifftestError::GitSpawn`] when the program cannot be started.
    pub fn run_ok<S: AsRef<OsStr>>(&self, env: &GitEnv, dir: &Path, args: &[S]) -> Result<Vec<u8>> {
        let output = self.run(env, dir, args)?;
        expect_exit(&output, 0, dir, args)?;
        Ok(output.stdout)
    }
}

/// Checks the exit code of a finished run.
///
/// # Errors
///
/// Returns [`DifftestError::GitFailed`] when the code differs from `expected`.
pub fn expect_exit<S: AsRef<OsStr>>(
    output: &GitOutput,
    expected: i32,
    dir: &Path,
    args: &[S],
) -> Result<()> {
    if output.code == expected {
        return Ok(());
    }
    Err(DifftestError::GitFailed {
        args: join_args(args),
        dir: dir.to_path_buf(),
        code: output.code,
        expected,
        stderr: output.stderr_text(),
    })
}

/// Arguments joined by spaces, for messages.
pub fn join_args<S: AsRef<OsStr>>(args: &[S]) -> String {
    args.iter()
        .map(|arg| arg.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Environment of every `git` run and of the Nurt host: fixed locale, time
/// zone, identity and dates, an empty home directory and no system or global
/// configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitEnv {
    vars: Vec<(String, OsString)>,
}

impl GitEnv {
    /// Environment with home directory `home` and dates of `clock`.
    ///
    /// `home` must contain the file `.gitconfig`, with the content
    /// [`GLOBAL_CONFIG`] or empty; it is the global configuration of both
    /// sides.
    #[must_use]
    pub fn new(home: &Path, clock: ScenarioClock) -> Self {
        let date = OsString::from(clock.git_date());
        let mut vars: Vec<(String, OsString)> = INHERITED
            .iter()
            .filter_map(|name| std::env::var_os(name).map(|value| ((*name).to_owned(), value)))
            .collect();
        let fixed: [(&str, OsString); 21] = [
            ("HOME", home.into()),
            ("USERPROFILE", home.into()),
            ("XDG_CONFIG_HOME", home.join(".config").into()),
            ("LC_ALL", "C".into()),
            ("LANG", "C".into()),
            ("TZ", "UTC".into()),
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ("GIT_CONFIG_SYSTEM", home.join(".gitconfig-system").into()),
            ("GIT_CONFIG_GLOBAL", home.join(".gitconfig").into()),
            ("GIT_ATTR_NOSYSTEM", "1".into()),
            ("GIT_AUTHOR_NAME", IDENTITY_NAME.into()),
            ("GIT_AUTHOR_EMAIL", IDENTITY_EMAIL.into()),
            ("GIT_AUTHOR_DATE", date.clone()),
            ("GIT_COMMITTER_NAME", IDENTITY_NAME.into()),
            ("GIT_COMMITTER_EMAIL", IDENTITY_EMAIL.into()),
            ("GIT_COMMITTER_DATE", date),
            ("GIT_EDITOR", ":".into()),
            ("GIT_PAGER", "cat".into()),
            ("GIT_TERMINAL_PROMPT", "0".into()),
            ("GIT_MERGE_AUTOEDIT", "no".into()),
            ("GIT_ADVICE", "0".into()),
        ];
        vars.extend(
            fixed
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value)),
        );
        // A temporary directory below another repository must not make `git`
        // find that repository from a directory that has none.
        #[cfg(unix)]
        {
            if let Some(case_dir) = home.parent() {
                vars.push((CEILING_VARIABLE.to_owned(), case_dir.into()));
            }
        }
        Self { vars }
    }

    /// The same environment with dates of `clock`.
    #[must_use]
    pub fn with_clock(&self, clock: ScenarioClock) -> Self {
        let date = OsString::from(clock.git_date());
        self.with_var("GIT_AUTHOR_DATE", date.clone())
            .with_var("GIT_COMMITTER_DATE", date)
    }

    /// The same environment with one variable set.
    #[must_use]
    pub fn with_var(&self, name: &str, value: impl Into<OsString>) -> Self {
        let mut vars = self.vars.clone();
        let value = value.into();
        match vars.iter_mut().find(|(existing, _)| existing == name) {
            Some(slot) => slot.1 = value,
            None => vars.push((name.to_owned(), value)),
        }
        Self { vars }
    }

    /// Variables in setting order.
    #[must_use]
    pub fn vars(&self) -> &[(String, OsString)] {
        &self.vars
    }

    /// Value of one variable.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.vars
            .iter()
            .find(|(existing, _)| existing == name)
            .map(|(_, value)| value.as_os_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_and_vendor_versions() {
        let plain = GitVersion::parse("git version 2.53.0\n").unwrap();
        assert_eq!(plain.to_string(), "2.53.0");
        let windows = GitVersion::parse("git version 2.45.1.windows.1").unwrap();
        assert_eq!((windows.major, windows.minor, windows.patch), (2, 45, 1));
        let apple = GitVersion::parse("git version 2.39.3 (Apple Git-146)").unwrap();
        assert!(apple < MINIMUM_VERSION);
        let short = GitVersion::parse("git version 3.0").unwrap();
        assert_eq!(short.patch, 0);
        assert!(GitVersion::parse("hub version 2.14").is_none());
    }

    #[test]
    fn environment_fixes_identity_dates_and_configuration() {
        let home = Path::new("/h");
        let env = GitEnv::new(home, ScenarioClock::default());
        assert_eq!(
            env.get("GIT_AUTHOR_DATE"),
            Some(OsStr::new("@1767268800 +0000"))
        );
        assert_eq!(env.get("GIT_CONFIG_NOSYSTEM"), Some(OsStr::new("1")));
        assert_eq!(env.get("HOME"), Some(home.as_os_str()));
        let later = env.with_clock(ScenarioClock::default().advanced(60));
        assert_eq!(
            later.get("GIT_COMMITTER_DATE"),
            Some(OsStr::new("@1767268860 +0000"))
        );
        assert_eq!(later.vars().len(), env.vars().len());
        let extended = env.with_var("GIT_OPTIONAL_LOCKS", "0");
        assert_eq!(extended.vars().len(), env.vars().len() + 1);
    }

    #[test]
    fn environment_pins_locale_zone_home_and_configuration_files() {
        let home = Path::new("/case/home");
        let env = GitEnv::new(home, ScenarioClock::default());
        for (name, value) in [
            ("LC_ALL", "C"),
            ("LANG", "C"),
            ("TZ", "UTC"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_ATTR_NOSYSTEM", "1"),
            ("GIT_TERMINAL_PROMPT", "0"),
        ] {
            assert_eq!(env.get(name), Some(OsStr::new(value)), "{name}");
        }
        assert_eq!(
            env.get("GIT_CONFIG_GLOBAL"),
            Some(home.join(".gitconfig").as_os_str())
        );
        assert_eq!(
            env.get("XDG_CONFIG_HOME"),
            Some(home.join(".config").as_os_str())
        );
        let names: Vec<&str> = env.vars().iter().map(|(name, _)| name.as_str()).collect();
        let unique: std::collections::BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(names.len(), unique.len());
        for name in names {
            assert!(
                name == "PATH" || !name.starts_with("GIT_CONFIG_KEY"),
                "{name}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn environment_stops_the_repository_search_at_the_case_directory() {
        let env = GitEnv::new(Path::new("/case/home"), ScenarioClock::default());
        assert_eq!(
            env.get("GIT_CEILING_DIRECTORIES"),
            Some(OsStr::new("/case"))
        );
    }

    #[test]
    fn a_variable_set_twice_keeps_one_entry() {
        let env = GitEnv::new(Path::new("/h"), ScenarioClock::default());
        let again = env.with_var("TZ", "Europe/Warsaw").with_var("TZ", "UTC");
        assert_eq!(again.get("TZ"), Some(OsStr::new("UTC")));
        assert_eq!(again.vars().len(), env.vars().len());
    }

    #[test]
    fn joins_arguments_for_messages() {
        assert_eq!(join_args(&["commit", "-m", "x"]), "commit -m x");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
