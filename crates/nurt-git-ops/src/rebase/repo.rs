// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Repository access for `git.rebase`: opening without launching programs,
//! effective configuration from the execution context, identities and
//! abbreviated object names.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::{Error, ErrorCode, Result, Stage};
use nurt_contracts::stage::ExecCtx;

use super::error;

/// Name of the directory or file that marks a work tree.
const DOT_GIT: &str = ".git";
/// Seconds in a minute, for time zone offsets.
const SECONDS_PER_MINUTE: i32 = 60;
/// Minutes in an hour, for time zone offsets.
const MINUTES_PER_HOUR: i32 = 60;
/// Nanoseconds in a second.
const NANOS_PER_SECOND: i128 = 1_000_000_000;
/// System configuration file on Unix systems.
#[cfg(not(windows))]
const SYSTEM_CONFIG: &str = "/etc/gitconfig";
/// System configuration file of Git for Windows.
#[cfg(windows)]
const SYSTEM_CONFIG: &str = "C:/Program Files/Git/etc/gitconfig";

/// An opened repository with a work tree.
pub(super) struct Repo {
    /// The repository handle.
    pub(super) repo: gix::Repository,
    /// Root of the work tree.
    pub(super) workdir: PathBuf,
    /// Per-work-tree Git directory (`HEAD`, `index`, `rebase-merge/`).
    pub(super) git_dir: PathBuf,
    /// Common Git directory (`refs/`, `logs/refs/`).
    pub(super) common_dir: PathBuf,
    /// Names of filter drivers configured in the repository; they are not run.
    pub(super) skipped_filters: Vec<String>,
    /// Names of merge drivers configured in the repository; they are not run.
    pub(super) skipped_merge_drivers: Vec<String>,
}

/// Identity and time of an author or committer, as written in objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Ident {
    /// Name.
    pub(super) name: BString,
    /// Email address.
    pub(super) email: BString,
    /// Seconds since the Unix epoch.
    pub(super) seconds: i64,
    /// Offset from UTC in seconds.
    pub(super) offset: i32,
}

impl Ident {
    /// The signature line: `Name <email> seconds +hhmm`.
    pub(super) fn line(&self) -> BString {
        let mut out = BString::from(self.name.as_slice());
        out.extend_from_slice(b" <");
        out.extend_from_slice(&self.email);
        out.extend_from_slice(b"> ");
        out.extend_from_slice(format!("{} {}", self.seconds, format_offset(self.offset)).as_bytes());
        out
    }
}

/// Formats a time zone offset as `+hhmm` or `-hhmm`.
pub(super) fn format_offset(offset: i32) -> String {
    let sign = if offset < 0 { '-' } else { '+' };
    let minutes = offset.abs() / SECONDS_PER_MINUTE;
    format!(
        "{sign}{:02}{:02}",
        minutes / MINUTES_PER_HOUR,
        minutes % MINUTES_PER_HOUR
    )
}

impl Repo {
    /// Opens the repository that contains `path`.
    ///
    /// The configuration is assembled from the system, global and local files
    /// located through the environment of `ctx`, never through the process
    /// environment. Filter and merge drivers are removed from the effective
    /// configuration, so no configured program is launched.
    pub(super) fn open(path: &Path, ctx: &ExecCtx) -> Result<Self> {
        let root = discover(path)?;
        let options = gix::open::Options::isolated();
        let mut repo = gix::open_opts(root.clone(), options).map_err(|cause| {
            Error::new(
                ErrorCode::GitNotRepository,
                format!("cannot open the repository at {}: {cause}", root.display()),
            )
            .with_stage(Stage::Read)
        })?;
        let workdir = repo
            .workdir()
            .map(Path::to_path_buf)
            .ok_or_else(|| error::unsupported("bare_repository", "a bare repository has no work tree to rebase"))?;
        let mut config = assemble_config(&repo, ctx)?;
        let skipped_filters = remove_driver_sections(&mut config, "filter");
        let skipped_merge_drivers = remove_driver_sections(&mut config, "merge");
        {
            let mut snapshot = repo.config_snapshot_mut();
            *snapshot = config;
            snapshot
                .commit()
                .map_err(|cause| error::io("cannot apply the repository configuration", cause))?;
        }
        let git_dir = repo.git_dir().to_path_buf();
        let common_dir = repo.common_dir().to_path_buf();
        repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
        Ok(Self {
            repo,
            workdir,
            git_dir,
            common_dir,
            skipped_filters,
            skipped_merge_drivers,
        })
    }

    /// Text value of a configuration key such as `core.commentChar`.
    pub(super) fn config_string(&self, key: &str) -> Option<BString> {
        self.repo.config_snapshot().string(key)
    }

    /// Boolean value of a configuration key; `None` when unset or invalid.
    pub(super) fn config_bool(&self, key: &str) -> Option<bool> {
        self.repo.config_snapshot().boolean(key)
    }

    /// Comment character of commit messages (`core.commentChar`, default `#`).
    pub(super) fn comment_char(&self) -> String {
        match self.config_string("core.commentChar") {
            Some(value) if !value.is_empty() && value.as_bstr() != "auto" => value.to_string(),
            _ => "#".to_owned(),
        }
    }

    /// Shortest unique abbreviation of an object name, at least `core.abbrev`
    /// digits long.
    pub(super) fn short(&self, id: ObjectId) -> String {
        id.attach(&self.repo).shorten_or_id().to_string()
    }

    /// Path of a file in the per-work-tree Git directory.
    pub(super) fn git_path(&self, name: &str) -> PathBuf {
        self.git_dir.join(name)
    }

    /// Path of a reference file: per-work-tree for `HEAD`-like names,
    /// common for names below `refs/`.
    pub(super) fn ref_path(&self, name: &str) -> PathBuf {
        if name.starts_with("refs/") {
            self.common_dir.join(name)
        } else {
            self.git_dir.join(name)
        }
    }

    /// Path of the reflog of a reference.
    pub(super) fn reflog_path(&self, name: &str) -> PathBuf {
        let base = if name.starts_with("refs/") {
            &self.common_dir
        } else {
            &self.git_dir
        };
        base.join("logs").join(name)
    }

    /// Committer identity: environment, then `committer.*`, then `user.*`,
    /// then the account of the host; time from `GIT_COMMITTER_DATE` or the
    /// clock of the context in the time zone of the host.
    pub(super) fn committer(&self, ctx: &ExecCtx) -> Result<Ident> {
        let name = env_bytes(ctx, "GIT_COMMITTER_NAME")
            .or_else(|| self.config_string("committer.name"))
            .or_else(|| self.config_string("user.name"))
            .unwrap_or_else(|| BString::from(ctx.host().info().user.as_str()));
        let email = env_bytes(ctx, "GIT_COMMITTER_EMAIL")
            .or_else(|| self.config_string("committer.email"))
            .or_else(|| self.config_string("user.email"))
            .or_else(|| env_bytes(ctx, "EMAIL"))
            .unwrap_or_else(|| {
                let info = ctx.host().info();
                BString::from(format!("{}@{}", info.user, info.hostname))
            });
        let (seconds, offset) = now(ctx, "GIT_COMMITTER_DATE")?;
        Ok(Ident {
            name: name.trim().into(),
            email: email.trim().into(),
            seconds,
            offset,
        })
    }

    /// The conflict style for content merges (`merge.conflictStyle`).
    pub(super) fn conflict_style(&self) -> gix_merge::blob::builtin_driver::text::ConflictStyle {
        use gix_merge::blob::builtin_driver::text::ConflictStyle;
        match self.config_string("merge.conflictStyle").as_ref().map(|value| value.to_str_lossy()) {
            Some(value) if value == "diff3" => ConflictStyle::Diff3,
            Some(value) if value == "zdiff3" => ConflictStyle::ZealousDiff3,
            _ => ConflictStyle::Merge,
        }
    }
}

/// Upper bound of the object cache used while picking commits.
const OBJECT_CACHE_BYTES: usize = 32 * 1024 * 1024;

/// Finds the work tree root that contains `path`.
fn discover(path: &Path) -> Result<PathBuf> {
    let mut current = Some(path);
    while let Some(dir) = current {
        if dir.join(DOT_GIT).exists() {
            return Ok(dir.to_path_buf());
        }
        current = dir.parent();
    }
    Err(Error::new(
        ErrorCode::GitNotRepository,
        format!("{} is not inside a Git work tree", path.display()),
    )
    .with_stage(Stage::Read))
}

/// Value of an environment variable of the context as bytes.
fn env_bytes(ctx: &ExecCtx, name: &str) -> Option<BString> {
    ctx.host()
        .env_var(name)
        .map(|value| BString::from(value.to_string_lossy().into_owned()))
}

/// Value of an environment variable of the context as a path, when set and
/// not empty.
fn env_path(ctx: &ExecCtx, name: &str) -> Option<PathBuf> {
    ctx.host()
        .env_var(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Current time: the date in the environment variable `variable` when set,
/// otherwise the clock of the context in the time zone of the host.
pub(super) fn now(ctx: &ExecCtx, variable: &str) -> Result<(i64, i32)> {
    if let Some(text) = ctx.host().env_var(variable) {
        let text = text.to_string_lossy().into_owned();
        let time = gix_date::parse(&text, None).map_err(|cause| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("cannot parse the date `{text}` in {variable}: {cause}"),
            )
            .with_stage(Stage::Read)
        })?;
        return Ok((time.seconds, time.offset));
    }
    let nanos = ctx.clock().now().unix_nanos();
    let seconds = i64::try_from(nanos.div_euclid(NANOS_PER_SECOND)).unwrap_or(i64::MAX);
    let offset = host_offset(ctx, seconds);
    Ok((seconds, offset))
}

/// Offset of the time zone of the host at the given instant, in seconds.
fn host_offset(ctx: &ExecCtx, seconds: i64) -> i32 {
    let zone = &ctx.host().info().timezone;
    let Ok(tz) = jiff::tz::TimeZone::get(zone) else {
        return 0;
    };
    let Ok(timestamp) = jiff::Timestamp::from_second(seconds) else {
        return 0;
    };
    tz.to_offset(timestamp).seconds()
}

/// Effective configuration: system, global, repository and the overrides
/// of `GIT_CONFIG_COUNT`, in increasing precedence.
fn assemble_config(repo: &gix::Repository, ctx: &ExecCtx) -> Result<gix_config::File<'static>> {
    let local = repo.config_snapshot().plumbing().clone();
    let mut config = gix_config::File::new(gix_config::file::Metadata::api());
    for (path, source) in config_files(ctx) {
        if !path.is_file() {
            continue;
        }
        let file = gix_config::File::from_path_no_includes(path.clone(), source).map_err(|cause| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("cannot read the configuration file {}: {cause}", path.display()),
            )
            .with_stage(Stage::Read)
        })?;
        config.append(file);
    }
    config.append(local);
    apply_env_overrides(&mut config, ctx)?;
    Ok(config)
}

/// System and global configuration files located through the environment of
/// the context, in increasing precedence.
fn config_files(ctx: &ExecCtx) -> Vec<(PathBuf, gix_config::Source)> {
    let mut files = Vec::new();
    if ctx.host().env_var("GIT_CONFIG_NOSYSTEM").is_none() {
        let system = env_path(ctx, "GIT_CONFIG_SYSTEM").unwrap_or_else(|| PathBuf::from(SYSTEM_CONFIG));
        files.push((system, gix_config::Source::System));
    }
    if let Some(global) = env_path(ctx, "GIT_CONFIG_GLOBAL") {
        files.push((global, gix_config::Source::User));
        return files;
    }
    let home = env_path(ctx, "HOME").unwrap_or_else(|| ctx.host().info().home.clone());
    let xdg = env_path(ctx, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
    files.push((xdg.join("git").join("config"), gix_config::Source::Git));
    files.push((home.join(".gitconfig"), gix_config::Source::User));
    files
}

/// Applies `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>` and `GIT_CONFIG_VALUE_<n>`.
fn apply_env_overrides(config: &mut gix_config::File<'static>, ctx: &ExecCtx) -> Result<()> {
    let Some(count) = ctx.host().env_var("GIT_CONFIG_COUNT") else {
        return Ok(());
    };
    let count: usize = count.to_string_lossy().trim().parse().unwrap_or(0);
    let mut text = String::new();
    for index in 0..count {
        let key = ctx.host().env_var(&format!("GIT_CONFIG_KEY_{index}"));
        let value = ctx.host().env_var(&format!("GIT_CONFIG_VALUE_{index}"));
        if let (Some(key), Some(value)) = (key, value) {
            text.push_str(&override_line(&key, &value));
        }
    }
    if text.is_empty() {
        return Ok(());
    }
    let file = gix_config::File::from_bytes_owned(
        &mut text.into_bytes(),
        gix_config::file::Metadata::from(gix_config::Source::Env),
        Default::default(),
    )
    .map_err(|cause| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("invalid configuration in GIT_CONFIG_KEY_<n>: {cause}"),
        )
        .with_stage(Stage::Read)
    })?;
    config.append(file);
    Ok(())
}

/// One override as configuration text: `[section "sub"]\n\tname = value\n`.
fn override_line(key: &OsString, value: &OsString) -> String {
    let key = key.to_string_lossy();
    let value = value.to_string_lossy();
    let (section, name) = match key.rsplit_once('.') {
        Some(parts) => parts,
        None => return String::new(),
    };
    let header = match section.split_once('.') {
        Some((section, sub)) => format!("[{section} \"{}\"]", sub.replace('"', "\\\"")),
        None => format!("[{section}]"),
    };
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{header}\n\t{name} = \"{escaped}\"\n")
}

/// Removes the sections `[<section> "<driver>"]` and returns the names of
/// the drivers that were configured.
fn remove_driver_sections(config: &mut gix_config::File<'static>, section: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let ids: Vec<_> = config
        .sections_and_ids()
        .filter(|(sect, _)| {
            sect.header().name().eq_ignore_ascii_case(section.as_bytes())
                && sect.header().subsection_name().is_some()
        })
        .map(|(sect, id)| {
            let name = sect
                .header()
                .subsection_name()
                .map(|name: &BStr| name.to_string())
                .unwrap_or_default();
            (name, id)
        })
        .collect();
    for (name, id) in ids {
        config.remove_section_by_id(id);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}
