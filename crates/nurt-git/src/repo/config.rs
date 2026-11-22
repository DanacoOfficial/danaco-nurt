// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Configuration of a repository without the configuration discovery of the
//! `git` installation.
//!
//! The repository library opens a repository with the repository-local
//! configuration only and reads no environment variable. The system, global
//! and environment layers are loaded here from the variables of the execution
//! context and merged into the repository configuration with the precedence
//! of `git`: a value of the system or global layer is applied only when no
//! repository-local layer defines the same key; values from
//! `GIT_CONFIG_COUNT` override every file.

use std::path::{Path, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use nurt_contracts::error::Error;

use super::env::GitEnv;
use super::error;

/// Configuration overrides of every repository opened by the console.
///
/// The local transport and SSH are implemented in process, so the library
/// must never start the `git-upload-pack` or `ssh` programs itself.
pub const OPEN_OVERRIDES: [&str; 3] = [
    "protocol.file.allow=never",
    "protocol.ssh.allow=never",
    "protocol.ext.allow=never",
];

/// Configuration keys holding paths in which `git` expands a leading `~`.
const PATH_KEYS: [(&str, &str); 4] = [
    ("core", "excludesFile"),
    ("core", "attributesFile"),
    ("core", "hooksPath"),
    ("commit", "template"),
];

/// Repository extensions understood by the implementation, in lower case.
const SUPPORTED_EXTENSIONS: [&str; 7] = [
    "noop",
    "noop-v1",
    "objectformat",
    "partialclone",
    "preciousobjects",
    "worktreeconfig",
    "relativeworktrees",
];

/// Loads the system and global configuration files named by `env`, with
/// includes followed relative to the home directory of `env`.
///
/// Files that do not exist are skipped. `git_dir` and `branch` evaluate the
/// `includeIf` conditions.
///
/// # Errors
///
/// Fails with `E_IO` when a file exists but cannot be read or parsed.
pub fn load_outer_layers(
    env: &GitEnv,
    git_dir: &Path,
    branch: Option<&gix::refs::FullNameRef>,
) -> Result<gix::config::File, Error> {
    let mut metas: Vec<gix::config::file::Metadata> = Vec::new();
    if let Some(system) = env.system_config_file() {
        metas.push(metadata(gix::config::Source::System, system));
    }
    for global in env.global_config_files() {
        metas.push(metadata(gix::config::Source::User, global));
    }
    let options = gix::config::file::init::Options {
        includes: gix::config::file::includes::Options::follow(
            gix::config::path::interpolate::Context {
                git_install_dir: None,
                home_dir: env.home(),
                home_for_user: None,
            },
            gix::config::file::includes::conditional::Context {
                git_dir: Some(git_dir),
                branch_name: branch,
            },
        ),
        lossy: false,
        ignore_io_errors: false,
    };
    let mut buf = Vec::new();
    let err_on_missing_files = false;
    gix::config::File::from_paths_metadata_buf(
        &mut metas.into_iter(),
        &mut buf,
        err_on_missing_files,
        options,
    )
    .map(Option::unwrap_or_default)
    .map_err(|source| error::from_gix("read the global git configuration", &source))
}

fn metadata(source: gix::config::Source, path: PathBuf) -> gix::config::file::Metadata {
    gix::config::file::Metadata::from(source)
        .at(path)
        .with(gix::sec::Trust::Full)
}

/// Overrides that apply the outer layers and the environment to a
/// configuration whose repository-local layers are `local`.
///
/// Each item is a `key=value` text (or a bare `key` for an implicit `true`)
/// in the order in which it must be appended.
#[must_use]
pub fn merge_overrides(
    local: &gix::config::File,
    outer: &gix::config::File,
    env: &GitEnv,
) -> MergePlan {
    let mut outer_values = Vec::new();
    for section in outer.sections() {
        let header = section.header();
        let name = header.name();
        let subsection = header.subsection_name();
        for value_name in section.body().value_names() {
            if local.raw_value_by(&*name.to_str_lossy(), subsection, value_name.as_str()).is_ok() {
                continue;
            }
            for value in section.body().values(&value_name) {
                outer_values.push(override_text(name, subsection, &value_name, Some(value.as_bstr())));
            }
        }
    }
    let env_values = env
        .config_pairs()
        .iter()
        .map(|(key, value)| BString::from(format!("{key}={value}")))
        .collect();
    let mut plan = MergePlan {
        outer: outer_values,
        env: env_values,
        paths: Vec::new(),
    };
    plan.paths = path_fixes(local, outer, env);
    plan
}

/// Overrides computed by [`merge_overrides`], grouped by the source they are
/// recorded with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergePlan {
    /// Values of the system and global layers not defined locally.
    pub outer: Vec<BString>,
    /// Values from `GIT_CONFIG_COUNT`.
    pub env: Vec<BString>,
    /// Path values with `~` expanded and default files of the XDG directory.
    pub paths: Vec<BString>,
}

impl MergePlan {
    /// Appends the overrides to the configuration of `repo`.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when a value is rejected by the configuration
    /// parser.
    pub fn apply(self, repo: &mut gix::Repository) -> Result<(), Error> {
        if self.outer.is_empty() && self.env.is_empty() && self.paths.is_empty() {
            return Ok(());
        }
        let mut snapshot = repo.config_snapshot_mut();
        let apply = |snapshot: &mut gix::config::SnapshotMut<'_>, values: Vec<BString>, source| {
            if values.is_empty() {
                return Ok(());
            }
            snapshot
                .append_config(values.iter().map(AsRef::<BStr>::as_ref), source)
                .map(|_| ())
                .map_err(|source| error::from_gix("apply the git configuration", &source))
        };
        apply(&mut snapshot, self.outer, gix::config::Source::User)?;
        apply(&mut snapshot, self.env, gix::config::Source::Env)?;
        apply(&mut snapshot, self.paths, gix::config::Source::Api)?;
        snapshot
            .commit()
            .map(|_| ())
            .map_err(|source| error::from_gix("apply the git configuration", &source))
    }
}

fn override_text(
    section: &BStr,
    subsection: Option<&BStr>,
    value_name: &str,
    value: Option<&BStr>,
) -> BString {
    let mut text = BString::from(section.to_vec());
    if let Some(subsection) = subsection {
        text.push(b'.');
        text.extend_from_slice(subsection);
    }
    text.push(b'.');
    text.extend_from_slice(value_name.as_bytes());
    if let Some(value) = value {
        text.push(b'=');
        text.extend_from_slice(value);
    }
    text
}

fn path_fixes(local: &gix::config::File, outer: &gix::config::File, env: &GitEnv) -> Vec<BString> {
    let mut fixes = Vec::new();
    for (section, name) in PATH_KEYS {
        let key = format!("{section}.{name}");
        let winner = env
            .config_pairs()
            .iter()
            .rev()
            .find(|(pair_key, _)| pair_key.eq_ignore_ascii_case(&key))
            .map(|(_, value)| BString::from(value.as_str()))
            .or_else(|| local.string(key.as_str()))
            .or_else(|| outer.string(key.as_str()));
        match winner {
            Some(value) => {
                if let Some(expanded) = env.expand_home(&value.to_str_lossy()) {
                    fixes.push(BString::from(format!("{key}={}", expanded.display())));
                }
            }
            None => {
                if let Some(default) = xdg_default(section, name, env) {
                    fixes.push(BString::from(format!("{key}={}", default.display())));
                }
            }
        }
    }
    fixes
}

fn xdg_default(section: &str, name: &str, env: &GitEnv) -> Option<PathBuf> {
    let file = match (section, name) {
        ("core", "excludesFile") => "ignore",
        ("core", "attributesFile") => "attributes",
        _ => return None,
    };
    env.xdg_git_dir()
        .map(|dir| dir.join(file))
        .filter(|path| path.is_file())
}

/// Checks the repository extensions declared in `config`.
///
/// # Errors
///
/// Fails with `E_GIT_UNSUPPORTED` for `extensions.refStorage=reftable`, for
/// `extensions.compatObjectFormat` and for extensions the implementation
/// does not know.
pub fn check_extensions(config: &gix::config::File) -> Result<(), Error> {
    let Some(sections) = config.sections_by_name("extensions") else {
        return Ok(());
    };
    for section in sections {
        for name in section.body().value_names() {
            let lower = name.to_ascii_lowercase();
            let value = section.body().value(&name).unwrap_or_default();
            if lower == "refstorage" {
                if !value.eq_ignore_ascii_case(b"files") {
                    return Err(error::unsupported(
                        &format!("extensions.refStorage={value}"),
                        "only the files reference backend is implemented",
                    ));
                }
                continue;
            }
            if !SUPPORTED_EXTENSIONS.contains(&lower.as_str()) {
                return Err(error::unsupported(
                    &format!("extensions.{name}"),
                    "the extension is not implemented",
                ));
            }
        }
    }
    Ok(())
}

/// Whether `core.fsmonitor` names a program or the built-in daemon; the
/// monitor is never used, so the caller reports it as ignored.
#[must_use]
pub fn fsmonitor_configured(config: &gix::config::File) -> bool {
    match config.string("core.fsmonitor") {
        None => false,
        Some(value) => {
            let value = value.trim();
            !(value.is_empty()
                || ["false", "no", "off", "0"]
                    .iter()
                    .any(|off| value.eq_ignore_ascii_case(off.as_bytes())))
        }
    }
}

/// Whether `safe.directory` in the outer layers and the environment lists
/// `repo`, as `git` checks it for a repository of another account.
///
/// An empty value clears the entries before it; `*` trusts every
/// repository; a value ending in `/*` trusts every repository below the
/// leading part.
#[must_use]
pub fn is_safe_directory(outer: &gix::config::File, env: &GitEnv, repo: &Path) -> bool {
    let mut entries: Vec<String> = outer
        .strings("safe.directory")
        .unwrap_or_default()
        .iter()
        .map(|value| value.to_str_lossy().into_owned())
        .collect();
    entries.extend(
        env.config_pairs()
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("safe.directory"))
            .map(|(_, value)| value.clone()),
    );
    let mut safe = false;
    let repo = normalize(repo);
    for entry in entries {
        if entry.is_empty() {
            safe = false;
            continue;
        }
        if entry == "*" {
            safe = true;
            continue;
        }
        let expanded = env
            .expand_home(&entry)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or(entry);
        let pattern = normalize(Path::new(&expanded));
        if let Some(prefix) = pattern.strip_suffix("/*") {
            if repo.starts_with(&format!("{prefix}/")) {
                safe = true;
            }
        } else if pattern == repo {
            safe = true;
        }
    }
    safe
}

fn normalize(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    text.trim_end_matches('/').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TestHost;

    fn parse(text: &str) -> gix::config::File {
        gix::config::File::from_bytes_owned(
            &mut text.as_bytes().to_vec(),
            gix::config::file::Metadata::from(gix::config::Source::Local),
            gix::config::file::init::Options::default(),
        )
        .expect("valid configuration")
    }

    #[test]
    fn outer_values_defined_locally_are_skipped() {
        let local = parse("[user]\n\tname = Local\n");
        let outer = parse("[user]\n\tname = Global\n\temail = g@example.com\n[core]\n\tautocrlf\n");
        let env = GitEnv::from_host(&TestHost::new("/home/a"));
        let plan = merge_overrides(&local, &outer, &env);
        assert_eq!(
            plan.outer,
            vec![BString::from("user.email=g@example.com"), BString::from("core.autocrlf")]
        );
    }

    #[test]
    fn subsections_are_kept_in_override_keys() {
        let local = parse("");
        let outer = parse("[url \"https://example.com/\"]\n\tinsteadOf = ex:\n");
        let env = GitEnv::from_host(&TestHost::new("/home/a"));
        let plan = merge_overrides(&local, &outer, &env);
        assert_eq!(plan.outer, vec![BString::from("url.https://example.com/.insteadOf=ex:")]);
    }

    #[test]
    fn tilde_in_path_values_is_expanded_with_the_context_home() {
        let local = parse("[core]\n\texcludesFile = ~/.ignore\n");
        let env = GitEnv::from_host(&TestHost::new("/home/a"));
        let plan = merge_overrides(&local, &parse(""), &env);
        assert_eq!(plan.paths, vec![BString::from("core.excludesFile=/home/a/.ignore")]);
    }

    #[test]
    fn environment_pairs_follow_the_files() {
        let host = TestHost::new("/home/a")
            .with_var("GIT_CONFIG_COUNT", "1")
            .with_var("GIT_CONFIG_KEY_0", "core.abbrev")
            .with_var("GIT_CONFIG_VALUE_0", "10");
        let plan = merge_overrides(&parse(""), &parse(""), &GitEnv::from_host(&host));
        assert_eq!(plan.env, vec![BString::from("core.abbrev=10")]);
    }

    #[test]
    fn reftable_is_rejected() {
        let config = parse("[extensions]\n\trefStorage = reftable\n");
        let error = check_extensions(&config).expect_err("reftable is unsupported");
        assert_eq!(error.code, nurt_contracts::error::ErrorCode::GitUnsupported);
        assert_eq!(error.found.as_deref(), Some("extensions.refStorage=reftable"));
    }

    #[test]
    fn known_extensions_are_accepted_and_unknown_rejected() {
        let ok = parse("[extensions]\n\tobjectFormat = sha256\n\tworktreeConfig = true\n\trefStorage = files\n");
        assert!(check_extensions(&ok).is_ok());
        let unknown = parse("[extensions]\n\tsomethingNew = 1\n");
        assert!(check_extensions(&unknown).is_err());
    }

    #[test]
    fn fsmonitor_detection_ignores_false_values() {
        assert!(!fsmonitor_configured(&parse("")));
        assert!(!fsmonitor_configured(&parse("[core]\n\tfsmonitor = false\n")));
        assert!(fsmonitor_configured(&parse("[core]\n\tfsmonitor = true\n")));
        assert!(fsmonitor_configured(&parse("[core]\n\tfsmonitor = .git/hooks/query\n")));
    }

    #[test]
    fn safe_directory_follows_git_rules() {
        let env = GitEnv::from_host(&TestHost::new("/home/a"));
        let repo = Path::new("/srv/repos/app");
        assert!(!is_safe_directory(&parse(""), &env, repo));
        assert!(is_safe_directory(&parse("[safe]\n\tdirectory = /srv/repos/app/\n"), &env, repo));
        assert!(is_safe_directory(&parse("[safe]\n\tdirectory = /srv/*\n"), &env, repo));
        assert!(is_safe_directory(&parse("[safe]\n\tdirectory = *\n"), &env, repo));
        assert!(!is_safe_directory(
            &parse("[safe]\n\tdirectory = *\n\tdirectory =\n"),
            &env,
            repo
        ));
    }
}
