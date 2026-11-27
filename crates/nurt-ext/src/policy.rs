// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Paths an extension may reach through calls of the host.
//!
//! The permissions of the manifest name directories, absolute or starting
//! with a placeholder (`${project}`, `${home}`, `${temp}`, `${ext}`). For one
//! execution the placeholders are replaced by the base directory of the
//! request, the home and temporary directories of the host and the
//! extension directory; an entry whose placeholder has no value grants
//! nothing. A path is granted when it lies below a granted directory both as
//! written and after resolving symbolic links, so a link inside a granted
//! directory cannot lead outside it. Paths with `..` components are refused.

use std::path::{Component, Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Fs, FsErrorKind};
use nurt_contracts::stage::ExecCtx;

use crate::manifest::Permissions;

/// Values of the placeholders of permission paths.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlaceholderRoots {
    /// `${project}`: base directory of the request.
    pub project: Option<PathBuf>,
    /// `${home}`: home directory of the account.
    pub home: Option<PathBuf>,
    /// `${temp}`: temporary directory.
    pub temp: Option<PathBuf>,
    /// `${ext}`: directory of the extension.
    pub ext: Option<PathBuf>,
}

impl PlaceholderRoots {
    /// Values for an execution: the request base from the options, the home
    /// and temporary directories from the host facts.
    #[must_use]
    pub fn from_ctx(ctx: &ExecCtx, ext_dir: &Path) -> Self {
        let info = ctx.host().info();
        let non_empty = |path: &Path| (!path.as_os_str().is_empty()).then(|| path.to_path_buf());
        Self {
            project: ctx.options().base.as_deref().map(PathBuf::from),
            home: non_empty(&info.home),
            temp: non_empty(&info.temp),
            ext: Some(ext_dir.to_path_buf()),
        }
    }

    fn expand(&self, entry: &str) -> Option<PathBuf> {
        let placeholders = [
            ("${project}", &self.project),
            ("${home}", &self.home),
            ("${temp}", &self.temp),
            ("${ext}", &self.ext),
        ];
        for (placeholder, value) in placeholders {
            if let Some(rest) = entry.strip_prefix(placeholder) {
                let base = value.as_ref()?;
                let rest = rest.trim_start_matches('/');
                return Some(if rest.is_empty() {
                    base.clone()
                } else {
                    base.join(rest)
                });
            }
        }
        Some(PathBuf::from(entry))
    }
}

/// Directories granted to one extension for one execution.
///
/// ```
/// use nurt_ext::manifest::Permissions;
/// use nurt_ext::policy::{PathPolicy, PlaceholderRoots};
///
/// let permissions = Permissions {
///     read: vec!["${project}".to_owned()],
///     write: vec!["${project}/out".to_owned()],
///     network: Vec::new(),
/// };
/// let roots = PlaceholderRoots {
///     project: Some("/srv/app".into()),
///     ..PlaceholderRoots::default()
/// };
/// let policy = PathPolicy::new("acme", &permissions, &roots);
/// assert!(policy.grants_read(std::path::Path::new("/srv/app/src/main.rs")));
/// assert!(!policy.grants_write(std::path::Path::new("/srv/app/src/main.rs")));
/// assert!(!policy.grants_read(std::path::Path::new("/etc/passwd")));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPolicy {
    extension: String,
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
}

impl PathPolicy {
    /// Policy of the extension `extension` with `permissions` expanded by
    /// `roots`; entries that do not expand to an absolute path are dropped.
    #[must_use]
    pub fn new(extension: &str, permissions: &Permissions, roots: &PlaceholderRoots) -> Self {
        let expand = |entries: &[String]| -> Vec<PathBuf> {
            entries
                .iter()
                .filter_map(|entry| roots.expand(entry))
                .filter(|path| path.is_absolute() && is_plain(path))
                .collect()
        };
        Self {
            extension: extension.to_owned(),
            read: expand(&permissions.read),
            write: expand(&permissions.write),
        }
    }

    /// Returns `true` when `path` lies below a directory granted for
    /// reading or writing, judged on the path as written.
    #[must_use]
    pub fn grants_read(&self, path: &Path) -> bool {
        is_plain(path) && within(path, self.read.iter().chain(&self.write))
    }

    /// Returns `true` when `path` lies below a directory granted for
    /// writing, judged on the path as written.
    #[must_use]
    pub fn grants_write(&self, path: &Path) -> bool {
        is_plain(path) && within(path, self.write.iter())
    }

    /// Checks a path the extension reads.
    ///
    /// # Errors
    ///
    /// Fails with `E_PATH_RELATIVE` for a relative path and with
    /// `E_PERMISSION` when the path, as written or with links resolved, is
    /// not below a granted directory.
    pub fn check_read(&self, path: &str, fs: &dyn Fs) -> Result<AbsPath> {
        let granted: Vec<&PathBuf> = self.read.iter().chain(&self.write).collect();
        self.check(path, fs, &granted, "read")
    }

    /// Checks a path the extension writes.
    ///
    /// # Errors
    ///
    /// Fails as [`PathPolicy::check_read`] for the directories granted for
    /// writing.
    pub fn check_write(&self, path: &str, fs: &dyn Fs) -> Result<AbsPath> {
        let granted: Vec<&PathBuf> = self.write.iter().collect();
        self.check(path, fs, &granted, "write")
    }

    fn check(
        &self,
        path: &str,
        fs: &dyn Fs,
        granted: &[&PathBuf],
        access: &str,
    ) -> Result<AbsPath> {
        let written = Path::new(path);
        if !written.is_absolute() {
            return Err(Error::new(
                ErrorCode::PathRelative,
                format!(
                    "`{}` passed the relative path `{path}`; extensions use absolute paths",
                    self.extension
                ),
            ));
        }
        let lexical = is_plain(written) && within(written, granted.iter().copied());
        if !lexical || !resolved_within(written, fs, granted) {
            return Err(self.denied(path, access));
        }
        AbsPath::new(written).map_err(Error::from)
    }

    fn denied(&self, path: &str, access: &str) -> Error {
        Error::new(
            ErrorCode::Permission,
            format!(
                "extension `{}` may not {access} {path}; grant the directory in `permissions.{access}` of its manifest",
                self.extension
            ),
        )
        .with_detail("path", serde_json::Value::from(path))
        .with_detail("extension", serde_json::Value::from(self.extension.as_str()))
    }
}

fn is_plain(path: &Path) -> bool {
    !path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn within<'a>(path: &Path, roots: impl Iterator<Item = &'a PathBuf>) -> bool {
    let mut roots = roots;
    roots.any(|root| path.starts_with(root))
}

fn real(path: &Path, fs: &dyn Fs) -> Option<PathBuf> {
    let abs = AbsPath::new(path).ok()?;
    match fs.real_path(&abs) {
        Ok(real) => Some(real.as_path().to_path_buf()),
        Err(problem) if problem.kind == FsErrorKind::NotFound => {
            let parent = path.parent()?;
            let name = path.file_name()?;
            real(parent, fs).map(|parent| parent.join(name))
        }
        Err(_) => None,
    }
}

fn resolved_within(path: &Path, fs: &dyn Fs, granted: &[&PathBuf]) -> bool {
    let Some(resolved) = real(path, fs) else {
        return false;
    };
    granted.iter().any(|root| {
        let root = real(root, fs).unwrap_or_else(|| (*root).clone());
        resolved.starts_with(root)
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use nurt_contracts::error::ErrorCode;
    use nurt_testkit::clock::VirtualClock;
    use nurt_testkit::vfs::{VirtualFs, test_path};

    use super::{PathPolicy, PlaceholderRoots};
    use crate::manifest::Permissions;

    fn policy() -> PathPolicy {
        let permissions = Permissions {
            read: vec!["${project}".to_owned(), "${home}/.acme".to_owned()],
            write: vec![
                "${project}/out".to_owned(),
                nurt_testkit::paths::abs("/var/acme"),
            ],
            network: Vec::new(),
        };
        let roots = PlaceholderRoots {
            project: Some(nurt_testkit::paths::abs_path("/project")),
            ..PlaceholderRoots::default()
        };
        PathPolicy::new("acme", &permissions, &roots)
    }

    fn fs() -> VirtualFs {
        let fs = VirtualFs::new(Arc::new(VirtualClock::new()));
        fs.write_file(&test_path("/project/src/a.txt").unwrap(), b"a")
            .unwrap();
        fs.write_file(&test_path("/project/out/b.txt").unwrap(), b"b")
            .unwrap();
        fs.write_file(&test_path("/secret/key").unwrap(), b"k")
            .unwrap();
        fs
    }

    #[test]
    fn grants_follow_the_expanded_permissions() {
        let policy = policy();
        assert!(policy.grants_read(&nurt_testkit::paths::abs_path("/project/src/a.txt")));
        assert!(policy.grants_read(&nurt_testkit::paths::abs_path("/var/acme/cache")));
        assert!(policy.grants_write(&nurt_testkit::paths::abs_path("/project/out/x")));
        assert!(!policy.grants_write(&nurt_testkit::paths::abs_path("/project/src/a.txt")));
        assert!(!policy.grants_read(&nurt_testkit::paths::abs_path("/projects/other")));
        assert!(!policy.grants_read(&nurt_testkit::paths::abs_path("/project/../secret/key")));
        assert!(!policy.grants_read(&nurt_testkit::paths::abs_path("/home/user/.acme")));
    }

    #[test]
    fn checks_paths_against_the_file_system() {
        let fs = fs();
        let policy = policy();
        assert_eq!(
            policy
                .check_read(nurt_testkit::paths::abs("/project/src/a.txt").as_str(), &fs)
                .unwrap(),
            test_path("/project/src/a.txt").unwrap()
        );
        policy
            .check_write(
                nurt_testkit::paths::abs("/project/out/new.txt").as_str(),
                &fs,
            )
            .unwrap();
        let denied = policy
            .check_read(nurt_testkit::paths::abs("/secret/key").as_str(), &fs)
            .unwrap_err();
        assert_eq!(denied.code(), ErrorCode::Permission);
        assert!(denied.message.contains("permissions.read"));
        let denied = policy
            .check_write(nurt_testkit::paths::abs("/project/src/a.txt").as_str(), &fs)
            .unwrap_err();
        assert_eq!(denied.code(), ErrorCode::Permission);
        let relative = policy.check_read("project/src/a.txt", &fs).unwrap_err();
        assert_eq!(relative.code(), ErrorCode::PathRelative);
    }

    #[test]
    fn links_cannot_lead_outside_granted_directories() {
        let fs = fs();
        fs.symlink(
            nurt_testkit::paths::abs("/secret").as_str(),
            &test_path("/project/escape").unwrap(),
        )
        .unwrap();
        let policy = policy();
        let denied = policy
            .check_read(
                nurt_testkit::paths::abs("/project/escape/key").as_str(),
                &fs,
            )
            .unwrap_err();
        assert_eq!(denied.code(), ErrorCode::Permission);
        fs.symlink(
            nurt_testkit::paths::abs("/project/src").as_str(),
            &test_path("/project/inner").unwrap(),
        )
        .unwrap();
        policy
            .check_read(
                nurt_testkit::paths::abs("/project/inner/a.txt").as_str(),
                &fs,
            )
            .unwrap();
    }

    #[test]
    fn placeholders_without_values_grant_nothing() {
        let permissions = Permissions {
            read: vec!["${temp}".to_owned(), "relative".to_owned()],
            write: Vec::new(),
            network: Vec::new(),
        };
        let policy = PathPolicy::new("acme", &permissions, &PlaceholderRoots::default());
        assert!(!policy.grants_read(Path::new("/tmp/x")));
        assert!(!policy.grants_read(Path::new("relative/x")));
    }
}
