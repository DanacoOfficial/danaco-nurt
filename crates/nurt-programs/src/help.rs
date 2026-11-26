// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Help text of a program, read on explicit request.
//!
//! `program.describe ... help: true` is the only place where the command
//! group starts a program. The program runs once through the
//! [`ProcessHost`](nurt_contracts::services::ProcessHost) of the context
//! with the argument list `[path, "--help"]` (`/?` for programs in the
//! Windows system directory), without a shell, with empty input, in an
//! empty temporary working directory, with a time limit and a limit of
//! output bytes. Programs with effects outside the computer, scripts
//! without an interpreter line and files outside the search path are never
//! started. A failure never fails the command; it is reported in
//! `help_status`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nurt_contracts::env::Os;
use nurt_contracts::services::{EnvVar, ProgramInput, ProgramSpec};
use nurt_contracts::stage::ExecCtx;
use sha2::{Digest, Sha256};

use crate::source::HostEnv;
use crate::text::decode;

/// Largest help text kept, in bytes.
pub const HELP_MAX_BYTES: u64 = 64 * 1024;
/// Argument that asks a program for its help.
pub const HELP_ARGUMENT: &str = "--help";
/// Argument that asks a Windows system program for its help.
pub const WINDOWS_HELP_ARGUMENT: &str = "/?";
/// Programs never started for their help: effects outside the computer or
/// on whole devices.
pub const REFUSED_PROGRAMS: &[&str] = &[
    "shutdown", "reboot", "halt", "poweroff", "init", "telinit", "dd", "format", "fdisk", "sfdisk",
    "parted", "wipefs", "shred", "mkswap",
];
/// Name prefixes of programs never started for their help.
pub const REFUSED_PREFIXES: &[&str] = &["mkfs"];
const ELF_MAGIC: &[u8] = b"\x7fELF";
const SCRIPT_MAGIC: &[u8] = b"#!";
const MACHO_MAGICS: &[[u8; 4]] = &[
    [0xcf, 0xfa, 0xed, 0xfe],
    [0xce, 0xfa, 0xed, 0xfe],
    [0xca, 0xfe, 0xba, 0xbe],
];
const WINDOWS_SYSTEM_DIR: &str = "\\windows\\system32\\";
const WINDOWS_EXECUTABLES: &[&str] = &[".exe", ".com"];
const TEMP_PREFIX: &str = "nurt-help-";
/// Hexadecimal digits of the name of a working directory.
const SCRATCH_NAME_DIGITS: usize = 32;

/// Outcome of reading the help of a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpStatus {
    /// Help text read.
    Ok,
    /// The program exceeded the time limit.
    Timeout,
    /// The program was not started.
    Refused,
    /// The program could not be started or ended without output.
    Failed,
    /// The program wrote nothing.
    Empty,
}

impl HelpStatus {
    /// Name used in records.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Timeout => "timeout",
            Self::Refused => "refused",
            Self::Failed => "failed",
            Self::Empty => "empty",
        }
    }
}

/// Help text and how it was obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpResult {
    /// Text, when any.
    pub text: Option<String>,
    /// Outcome.
    pub status: HelpStatus,
    /// Command line that was run.
    pub executed: Option<Vec<String>>,
    /// Why the program was refused or failed.
    pub reason: Option<String>,
    /// Files the program changed in its working directory.
    pub changed: Vec<PathBuf>,
}

impl HelpResult {
    fn refused(reason: impl Into<String>) -> Self {
        Self {
            text: None,
            status: HelpStatus::Refused,
            executed: None,
            reason: Some(reason.into()),
            changed: Vec::new(),
        }
    }
}

/// Reason not to start the program at host path `path`, when there is one.
///
/// `patterns` are the configured `run.irreversible_patterns`; a pattern
/// refuses the program when its first token names it (`*` matches any
/// name, `a|b` lists alternatives).
#[must_use]
pub fn refusal(env: &HostEnv, path: &Path, patterns: &[String]) -> Option<String> {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Some("the file name is missing or not valid Unicode".to_owned());
    };
    let name = command_name_of(env, file_name);
    // A link under another name starts the program it points to, so the
    // refusals apply to the name of the target as well.
    let target_name = std::fs::canonicalize(env.sys_path(path))
        .ok()
        .and_then(|real| {
            let target = real
                .file_name()?
                .to_str()
                .map(|text| command_name_of(env, text))?;
            (target != name).then_some(target)
        });
    for name in std::iter::once(&name).chain(target_name.as_ref()) {
        if REFUSED_PROGRAMS.contains(&name.as_str())
            || REFUSED_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
        {
            return Some(format!("{name} has effects outside the computer"));
        }
        let configured = patterns.iter().any(|pattern| {
            pattern.split_whitespace().next().is_some_and(|first| {
                first
                    .split('|')
                    .any(|item| item == "*" || item.eq_ignore_ascii_case(name))
            })
        });
        if configured {
            return Some(format!("{name} matches run.irreversible_patterns"));
        }
    }
    if !in_search_path(env, path) {
        return Some("the file is not in the search path".to_owned());
    }
    executable_format(env, path).err()
}

/// Lower-case command name of a file name, without the executable extension
/// of the host.
fn command_name_of(env: &HostEnv, file_name: &str) -> String {
    env.command_name(file_name)
        .unwrap_or_else(|| file_name.to_owned())
        .to_lowercase()
}

fn in_search_path(env: &HostEnv, path: &Path) -> bool {
    path.parent()
        .is_some_and(|dir| env.path_dirs().iter().any(|known| known == dir))
}

/// Checks that the file is a native executable or a script with an
/// interpreter line.
fn executable_format(env: &HostEnv, path: &Path) -> Result<(), String> {
    if env.os() == Os::Windows {
        let lower = path.to_string_lossy().to_lowercase();
        return if WINDOWS_EXECUTABLES
            .iter()
            .any(|extension| lower.ends_with(extension))
        {
            Ok(())
        } else {
            Err("only .exe and .com programs are started for their help".to_owned())
        };
    }
    let mut head = [0u8; 4];
    let read = std::fs::File::open(env.sys_path(path))
        .and_then(|mut file| std::io::Read::read(&mut file, &mut head))
        .map_err(|error| format!("cannot read the program file: {error}"))?;
    let head = &head[..read];
    let native =
        head.starts_with(ELF_MAGIC) || MACHO_MAGICS.iter().any(|magic| head == magic.as_slice());
    if native || head.starts_with(SCRIPT_MAGIC) {
        Ok(())
    } else {
        Err("script without an interpreter line".to_owned())
    }
}

/// Command line used for the help of the program at `path`.
#[must_use]
pub fn help_command(env: &HostEnv, path: &Path) -> Vec<String> {
    let text = path.to_string_lossy().into_owned();
    let system = env.os() == Os::Windows && text.to_lowercase().contains(WINDOWS_SYSTEM_DIR);
    let argument = if system {
        WINDOWS_HELP_ARGUMENT
    } else {
        HELP_ARGUMENT
    };
    vec![text, argument.to_owned()]
}

/// Locales of the languages whose region differs from the language code or
/// that are common; other languages use [`FALLBACK_LOCALE`].
const LOCALES: &[(&str, &str)] = &[
    ("ar", "ar_SA"),
    ("bg", "bg_BG"),
    ("cs", "cs_CZ"),
    ("da", "da_DK"),
    ("de", "de_DE"),
    ("el", "el_GR"),
    ("es", "es_ES"),
    ("et", "et_EE"),
    ("fi", "fi_FI"),
    ("fr", "fr_FR"),
    ("he", "he_IL"),
    ("hi", "hi_IN"),
    ("hr", "hr_HR"),
    ("hu", "hu_HU"),
    ("it", "it_IT"),
    ("ja", "ja_JP"),
    ("ko", "ko_KR"),
    ("lt", "lt_LT"),
    ("lv", "lv_LV"),
    ("nb", "nb_NO"),
    ("nl", "nl_NL"),
    ("pl", "pl_PL"),
    ("pt", "pt_PT"),
    ("ro", "ro_RO"),
    ("ru", "ru_RU"),
    ("sk", "sk_SK"),
    ("sl", "sl_SI"),
    ("sr", "sr_RS"),
    ("sv", "sv_SE"),
    ("tr", "tr_TR"),
    ("uk", "uk_UA"),
    ("vi", "vi_VN"),
    ("zh", "zh_CN"),
];
/// Locale of English and of languages without an entry in [`LOCALES`].
const FALLBACK_LOCALE: &str = "C.UTF-8";

/// Locale variables for the preferred language of the environment.
fn locale_vars(env: &HostEnv) -> Vec<EnvVar> {
    let language = env.languages().first().map_or("en", String::as_str);
    let locale = LOCALES
        .iter()
        .find(|(code, _)| *code == language)
        .map_or_else(
            || FALLBACK_LOCALE.to_owned(),
            |(_, locale)| format!("{locale}.UTF-8"),
        );
    ["LC_ALL", "LANG"]
        .into_iter()
        .map(|name| EnvVar {
            name: name.to_owned(),
            value: Some(locale.clone()),
        })
        .collect()
}

/// Attempts to create a private directory with a name that cannot be guessed.
const SCRATCH_ATTEMPTS: usize = 8;

/// Working directory of a started program, removed when dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ignored = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates an empty directory below `parent` that only the account can
/// enter. The name derives from the process, the time, a counter and the
/// thread; the directory is created exclusively, so an existing name, also a
/// link planted by another account, is never reused.
fn private_directory(parent: &Path, nanos: i128) -> std::io::Result<Scratch> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(parent)?;
    let mut last = std::io::Error::other("no directory name was tried");
    for _ in 0..SCRATCH_ATTEMPTS {
        let mut hasher = Sha256::new();
        hasher.update(std::process::id().to_le_bytes());
        hasher.update(nanos.to_le_bytes());
        hasher.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
        hasher.update(format!("{:?}", std::thread::current().id()).as_bytes());
        let digest = hex::encode(hasher.finalize());
        let directory = parent.join(format!("{TEMP_PREFIX}{}", &digest[..SCRATCH_NAME_DIGITS]));
        match create_private(&directory) {
            Ok(()) => return Ok(Scratch(directory)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => last = error,
            Err(error) => return Err(error),
        }
    }
    Err(last)
}

#[cfg(unix)]
fn create_private(directory: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    const OWNER_ONLY: u32 = 0o700;
    std::fs::DirBuilder::new()
        .mode(OWNER_ONLY)
        .create(directory)
}

#[cfg(not(unix))]
fn create_private(directory: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().create(directory)
}

/// Runs the program at host path `path` once for its help.
#[must_use]
pub fn read_help(ctx: &ExecCtx, env: &HostEnv, path: &Path, timeout: Duration) -> HelpResult {
    if let Some(reason) = refusal(env, path, &ctx.config().run.irreversible_patterns) {
        return HelpResult::refused(reason);
    }
    let Some(host) = ctx.process() else {
        return HelpResult::refused("program launching is not available");
    };
    let scratch = match private_directory(&ctx.host().info().temp, ctx.clock().now().unix_nanos()) {
        Ok(scratch) => scratch,
        Err(error) => {
            return HelpResult::refused(format!("cannot create a working directory: {error}"));
        }
    };
    let command = help_command(env, path);
    let mut spec = ProgramSpec::new(command[0].clone(), command[1..].to_vec(), scratch.0.clone());
    spec.env = locale_vars(env);
    spec.stdin = ProgramInput::Null;
    spec.timeout_ms = Some(u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX));
    spec.output_limit = Some(HELP_MAX_BYTES);
    spec.detect_changes = true;
    let outcome = host.run_program(&spec, ctx.cancel());
    drop(scratch);
    let mut result = HelpResult {
        text: None,
        status: HelpStatus::Failed,
        executed: Some(command),
        reason: None,
        changed: Vec::new(),
    };
    match outcome {
        Ok(outcome) => {
            result.changed = outcome.changed_paths;
            let bytes = if outcome
                .stdout
                .iter()
                .any(|byte| !byte.is_ascii_whitespace())
            {
                outcome.stdout
            } else {
                outcome.stderr
            };
            let text = decode(&bytes).trim_end().to_owned();
            result.status = if outcome.timed_out {
                HelpStatus::Timeout
            } else if outcome.cancelled {
                HelpStatus::Failed
            } else if text.trim().is_empty() {
                HelpStatus::Empty
            } else {
                HelpStatus::Ok
            };
            result.text = (!text.trim().is_empty()).then_some(text);
        }
        Err(error) => result.reason = Some(error.message.clone()),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_listed_and_configured_programs() {
        let env = HostEnv::new(Os::Linux).with_var("PATH", "/usr/bin");
        assert!(refusal(&env, Path::new("/usr/sbin/reboot"), &[]).is_some());
        assert!(refusal(&env, Path::new("/usr/bin/mkfs.ext4"), &[]).is_some());
        let patterns = vec!["deploy-tool publish".to_owned()];
        assert!(
            refusal(&env, Path::new("/usr/bin/deploy-tool"), &patterns)
                .unwrap()
                .contains("irreversible")
        );
        assert!(
            refusal(&env, Path::new("/opt/x/tool"), &[])
                .unwrap()
                .contains("search path")
        );
    }

    fn locale_of(language: &str) -> String {
        let env = HostEnv::new(Os::Linux).with_languages(vec![language.to_owned()]);
        let vars = locale_vars(&env);
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].value, vars[1].value);
        vars[0].value.clone().unwrap()
    }

    #[test]
    fn locale_names_are_valid_and_never_invented() {
        assert_eq!(locale_of("en"), "C.UTF-8");
        assert_eq!(locale_of("pl"), "pl_PL.UTF-8");
        assert_eq!(locale_of("cs"), "cs_CZ.UTF-8");
        assert_eq!(locale_of("uk"), "uk_UA.UTF-8");
        assert_eq!(locale_of("ja"), "ja_JP.UTF-8");
        assert_eq!(locale_of("xx"), "C.UTF-8");
        for (code, locale) in LOCALES {
            assert!(locale.starts_with(code), "{locale}");
        }
    }

    #[test]
    fn private_directories_are_unique_exclusive_and_removed() {
        let parent = tempfile::tempdir().unwrap();
        let first = private_directory(parent.path(), 7).unwrap();
        let second = private_directory(parent.path(), 7).unwrap();
        assert_ne!(first.0, second.0);
        assert!(first.0.is_dir());
        assert!(first.0.starts_with(parent.path()));
        let name = first.0.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name.len(), TEMP_PREFIX.len() + SCRATCH_NAME_DIGITS);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&first.0).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        assert!(create_private(&first.0).is_err());
        let path = first.0.clone();
        drop(first);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_link_that_points_to_a_listed_program() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("shutdown");
        std::fs::write(&target, b"#!/bin/sh\n").unwrap();
        let link = dir.path().join("harmless");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let env = HostEnv::new(Os::Linux).with_var("PATH", dir.path().to_str().unwrap());
        let reason = refusal(&env, &link, &[]).unwrap();
        assert!(reason.contains("shutdown"), "{reason}");
        let patterns = vec!["shutdown".to_owned()];
        let reason = refusal(&env, &link, &patterns).unwrap();
        assert!(reason.contains("shutdown"), "{reason}");
    }

    #[test]
    fn refuses_path_without_file_name() {
        let env = HostEnv::new(Os::Linux).with_var("PATH", "/usr/bin");
        let reason = refusal(&env, Path::new("/"), &[]).unwrap();
        assert!(reason.contains("file name"));
        assert!(refusal(&env, Path::new("/usr/bin/.."), &[]).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_file_name_that_is_not_unicode() {
        use std::os::unix::ffi::OsStrExt;

        let env = HostEnv::new(Os::Linux).with_var("PATH", "/usr/bin");
        let path = Path::new(std::ffi::OsStr::from_bytes(b"/usr/bin/tool\xff"));
        let reason = refusal(&env, path, &[]).unwrap();
        assert!(reason.contains("not valid Unicode"));
    }

    #[test]
    fn windows_command_line() {
        let env = HostEnv::new(Os::Windows);
        assert_eq!(
            help_command(&env, Path::new("C:\\Windows\\System32\\where.exe"))[1],
            "/?"
        );
        assert_eq!(
            help_command(&env, Path::new("C:\\Tools\\git.exe"))[1],
            "--help"
        );
        assert!(executable_format(&env, Path::new("C:\\Tools\\x.bat")).is_err());
        assert_eq!(HelpStatus::Timeout.as_str(), "timeout");
    }
}
