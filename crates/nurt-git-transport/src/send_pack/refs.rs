// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::send_pack::refs
// Responsibility:   Reference updates with reflog entries, planned as journaled file changes.
// Dependencies:     bstr, gix, nurt_contracts, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reference updates with reflog entries, planned as journaled file changes.
//!
//! Loose reference files, `packed-refs` and the files under `logs/` are
//! written in the formats of `git`. An update replaces the loose reference
//! file, which must not be locked by another `git` process; a deletion removes
//! the loose file, rewrites `packed-refs` without the reference and removes its
//! reflog. The value read when the plan is made is checked again before the
//! files are replaced.

use std::path::{Path, PathBuf};

use bstr::{BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::ContentHash;
use nurt_contracts::journal::Expect;
use sha2::{Digest, Sha256};

use super::file_plan::FilePlan;

/// Name of the packed references file.
const PACKED_REFS: &str = "packed-refs";

/// When reflog entries are created for references without a reflog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogPolicy {
    /// Only existing reflogs are extended (`core.logAllRefUpdates=false`).
    Existing,
    /// Branches, remote-tracking references, notes and `HEAD` get a reflog
    /// (`true`, the default of repositories with a working tree).
    Standard,
    /// Every reference gets a reflog (`always`).
    Always,
}

impl LogPolicy {
    /// Policy of `core.logAllRefUpdates` in the configuration of `repo`.
    #[must_use]
    pub fn of(repo: &gix::Repository) -> Self {
        let config = repo.config_snapshot();
        match config
            .string("core.logAllRefUpdates")
            .as_deref()
            .map(|value| value.as_bytes())
        {
            Some(b"always") => Self::Always,
            Some(_) => match config.boolean("core.logAllRefUpdates") {
                Some(true) => Self::Standard,
                _ => Self::Existing,
            },
            None if repo.is_bare() => Self::Existing,
            None => Self::Standard,
        }
    }

    fn creates_log_for(self, name: &[u8]) -> bool {
        match self {
            Self::Existing => false,
            Self::Always => true,
            Self::Standard => {
                name == b"HEAD"
                    || [&b"refs/heads/"[..], b"refs/remotes/", b"refs/notes/"]
                        .iter()
                        .any(|prefix| name.starts_with(prefix))
            }
        }
    }
}

/// Identity and time written into reflog entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Name.
    pub name: BString,
    /// E-mail address.
    pub email: BString,
    /// Seconds since the Unix epoch.
    pub seconds: i64,
    /// Offset from UTC in seconds.
    pub offset: i32,
}

impl Identity {
    fn signature(&self) -> Vec<u8> {
        let sign = if self.offset < 0 { '-' } else { '+' };
        let minutes = self.offset.unsigned_abs() / 60;
        let mut out = Vec::new();
        out.extend_from_slice(&self.name);
        out.extend_from_slice(b" <");
        out.extend_from_slice(&self.email);
        out.extend_from_slice(
            format!(
                "> {} {sign}{:02}{:02}",
                self.seconds,
                minutes / 60,
                minutes % 60
            )
            .as_bytes(),
        );
        out
    }

    /// Formats one reflog line.
    #[must_use]
    pub fn reflog_line(&self, old: ObjectId, new: ObjectId, message: &str) -> Vec<u8> {
        let mut line = Vec::new();
        line.extend_from_slice(old.to_hex().to_string().as_bytes());
        line.push(b' ');
        line.extend_from_slice(new.to_hex().to_string().as_bytes());
        line.push(b' ');
        line.extend_from_slice(&self.signature());
        line.push(b'\t');
        line.extend_from_slice(message.as_bytes());
        line.push(b'\n');
        line
    }
}

/// One planned reference change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefEdit {
    /// Full reference name.
    pub name: BString,
    /// Value the reference has when the plan is made; `None` when absent.
    pub old: Option<ObjectId>,
    /// New value; `None` deletes the reference.
    pub new: Option<ObjectId>,
    /// Reflog message.
    pub message: String,
}

/// Current value of the reference `name` in the repository at `git_dir`.
///
/// # Errors
///
/// Fails with `E_GIT_REF` when the loose file holds a symbolic or malformed
/// reference and with `E_IO` when a file cannot be read.
pub fn read_ref(git_dir: &Path, name: &[u8]) -> Result<Option<ObjectId>> {
    let path = ref_path(git_dir, name)?;
    match std::fs::read(&path) {
        Ok(bytes) => parse_loose(name, &bytes).map(Some),
        Err(error) if is_absent(&error) => packed_value(git_dir, name),
        Err(error) => Err(read_error(&path, &error)),
    }
}

/// Adds the changes of `edits` to `plan`; references, `packed-refs` and logs
/// are in `git_dir`.
///
/// # Errors
///
/// Fails with `E_GIT_REF` for an invalid or conflicting name, `E_GIT_LOCKED`
/// when a lock file exists and `E_IO` when the repository cannot be read.
pub fn plan_ref_edits(
    git_dir: &Path,
    edits: &[RefEdit],
    identity: &Identity,
    policy: LogPolicy,
    plan: &mut FilePlan,
) -> Result<()> {
    plan_ref_edits_in(git_dir, git_dir, edits, identity, policy, plan)
}

/// Adds the changes of `edits` to `plan` for a repository whose working tree
/// has its own directory: references, `packed-refs` and their logs are in
/// `common_dir`, while `HEAD` and its log are in `git_dir`. The two are equal
/// except in a linked working tree.
///
/// # Errors
///
/// As [`plan_ref_edits`].
pub fn plan_ref_edits_in(
    common_dir: &Path,
    git_dir: &Path,
    edits: &[RefEdit],
    identity: &Identity,
    policy: LogPolicy,
    plan: &mut FilePlan,
) -> Result<()> {
    let head_target = symbolic_head(git_dir);
    let head_dir = git_dir;
    let git_dir = common_dir;
    let mut packed_removals = Vec::new();
    for edit in edits {
        let path = ref_path(git_dir, &edit.name)?;
        let zero = ObjectId::null(gix::hash::Kind::Sha1);
        match edit.new {
            Some(new) => {
                let content = format!("{}\n", new.to_hex()).into_bytes();
                plan.replace_locked(git_dir, &path, content)?;
                let line = identity.reflog_line(edit.old.unwrap_or(zero), new, &edit.message);
                append_log(git_dir, &edit.name, &line, policy, plan)?;
                if head_target.as_ref() == Some(&edit.name) {
                    append_log(head_dir, b"HEAD", &line, policy, plan)?;
                }
            }
            None => {
                if let Ok(bytes) = std::fs::read(&path) {
                    plan.delete(&path, Expect::Hash(content_hash(&bytes)))?;
                }
                if packed_value(git_dir, &edit.name)?.is_some() {
                    packed_removals.push(edit.name.clone());
                }
                let log = log_path(git_dir, &edit.name)?;
                if log.is_file() {
                    plan.delete(&log, Expect::Unchecked)?;
                }
            }
        }
        add_value_check(git_dir, edit, plan);
    }
    if !packed_removals.is_empty() {
        let packed = git_dir.join(PACKED_REFS);
        let bytes = std::fs::read(&packed).map_err(|error| read_error(&packed, &error))?;
        plan.replace_locked(git_dir, &packed, without_packed(&bytes, &packed_removals))?;
    }
    Ok(())
}

fn add_value_check(git_dir: &Path, edit: &RefEdit, plan: &mut FilePlan) {
    let git_dir = git_dir.to_path_buf();
    let name = edit.name.clone();
    let expected = edit.old;
    plan.check_when_locked(Box::new(move || {
        let actual = read_ref(&git_dir, &name)?;
        if actual == expected {
            return Ok(());
        }
        Err(Error::new(
            ErrorCode::Stale,
            format!(
                "reference `{}` changed while the push ran; fetch and push again",
                name.as_bstr()
            ),
        )
        .with_detail(
            "ref",
            serde_json::Value::String(name.to_str_lossy().into_owned()),
        ))
    }));
}

fn append_log(
    git_dir: &Path,
    name: &[u8],
    line: &[u8],
    policy: LogPolicy,
    plan: &mut FilePlan,
) -> Result<()> {
    let log = log_path(git_dir, name)?;
    if log.is_file() || policy.creates_log_for(name) {
        plan.append(git_dir, &log, line.to_vec())?;
    }
    Ok(())
}

/// Path of the loose reference file of `name`.
///
/// # Errors
///
/// Fails with `E_GIT_REF` when `name` is not a valid reference name.
pub fn ref_path(git_dir: &Path, name: &[u8]) -> Result<PathBuf> {
    validate_name(name)?;
    Ok(join_name(git_dir, name))
}

fn log_path(git_dir: &Path, name: &[u8]) -> Result<PathBuf> {
    validate_name(name)?;
    Ok(join_name(&git_dir.join("logs"), name))
}

fn join_name(base: &Path, name: &[u8]) -> PathBuf {
    let mut path = base.to_path_buf();
    for component in name.split_str(b"/") {
        path.push(component.to_str_lossy().as_ref());
    }
    path
}

/// Checks that `name` is a valid full reference name or `HEAD`.
///
/// # Errors
///
/// Fails with `E_GIT_REF` and problem `invalid_name` otherwise.
pub fn validate_name(name: &[u8]) -> Result<()> {
    if name == b"HEAD" {
        return Ok(());
    }
    let valid = name.starts_with(b"refs/")
        && gix::refs::FullName::try_from(name.as_bstr()).is_ok()
        && name.to_str().is_ok();
    if valid {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::GitRef,
            format!(
                "`{}` is not a valid reference name; use a name under refs/ without `..`, spaces or control characters",
                name.as_bstr()
            ),
        )
        .with_problem("invalid_name"))
    }
}

fn parse_loose(name: &[u8], bytes: &[u8]) -> Result<ObjectId> {
    let text = bytes.trim();
    ObjectId::from_hex(text).map_err(|_| {
        Error::new(
            ErrorCode::GitRef,
            format!(
                "reference `{}` is symbolic or malformed and cannot be updated by push",
                name.as_bstr()
            ),
        )
        .with_problem("not_a_direct_reference")
    })
}

fn packed_value(git_dir: &Path, name: &[u8]) -> Result<Option<ObjectId>> {
    let packed = git_dir.join(PACKED_REFS);
    let bytes = match std::fs::read(&packed) {
        Ok(bytes) => bytes,
        Err(error) if is_absent(&error) => return Ok(None),
        Err(error) => return Err(read_error(&packed, &error)),
    };
    Ok(bytes.lines().find_map(|line| {
        let (id, ref_name) = line.split_once_str(b" ")?;
        if ref_name == name {
            ObjectId::from_hex(id).ok()
        } else {
            None
        }
    }))
}

fn without_packed(bytes: &[u8], names: &[BString]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut skip_peeled = false;
    for line in bytes.lines_with_terminator() {
        if line.starts_with(b"^") && skip_peeled {
            continue;
        }
        skip_peeled = false;
        let trimmed = line.trim_end();
        let removed = trimmed
            .split_once_str(b" ")
            .is_some_and(|(_, name)| names.iter().any(|candidate| candidate == name));
        if removed {
            skip_peeled = true;
            continue;
        }
        out.extend_from_slice(line);
    }
    out
}

fn symbolic_head(git_dir: &Path) -> Option<BString> {
    let bytes = std::fs::read(git_dir.join("HEAD")).ok()?;
    bytes
        .trim()
        .strip_prefix(b"ref: ")
        .map(|target| target.trim().into())
}

/// SHA-256 of `bytes` as used by the journal for expected states.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> ContentHash {
    ContentHash(Sha256::digest(bytes).into())
}

fn is_absent(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

fn read_error(path: &Path, error: &std::io::Error) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("reading `{}` failed: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send_pack::file_plan::preserve_and_apply;
    use crate::send_pack::testing::RecordingTx;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    fn id(hex: &str) -> ObjectId {
        ObjectId::from_hex(hex.as_bytes()).unwrap()
    }

    fn identity() -> Identity {
        Identity {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            seconds: 1_700_000_000,
            offset: -90 * 60,
        }
    }

    fn git_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        std::fs::create_dir_all(git.join("refs").join("heads")).unwrap();
        std::fs::write(git.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
        (dir, git)
    }

    #[test]
    fn formats_reflog_lines_like_git() {
        let line = identity().reflog_line(id(A), id(B), "update by push");
        assert_eq!(
            line,
            format!("{A} {B} Ada <ada@example.com> 1700000000 -0130\tupdate by push\n")
                .into_bytes()
        );
    }

    #[test]
    fn creates_updates_and_undoes_a_branch_with_head_log() {
        let (_dir, git) = git_dir();
        let mut plan = FilePlan::new();
        let edit = RefEdit {
            name: "refs/heads/main".into(),
            old: None,
            new: Some(id(A)),
            message: "push".to_owned(),
        };
        plan_ref_edits(&git, &[edit], &identity(), LogPolicy::Standard, &mut plan).unwrap();
        let mut tx = RecordingTx::new();
        preserve_and_apply(&mut tx, &[&plan]).unwrap();
        assert_eq!(read_ref(&git, b"refs/heads/main").unwrap(), Some(id(A)));
        assert!(git.join("logs/refs/heads/main").is_file());
        assert!(git.join("logs/HEAD").is_file());
        tx.undo();
        assert_eq!(read_ref(&git, b"refs/heads/main").unwrap(), None);
        assert!(!git.join("logs").exists());
    }

    #[test]
    fn skips_logs_for_bare_policy_and_deletes_packed_refs() {
        let (_dir, git) = git_dir();
        let packed = format!(
            "# pack-refs with: peeled fully-peeled sorted \n{A} refs/tags/v1\n^{B}\n{B} refs/tags/v2\n"
        );
        std::fs::write(git.join(PACKED_REFS), &packed).unwrap();
        std::fs::create_dir_all(git.join("logs/refs/tags")).unwrap();
        std::fs::write(git.join("logs/refs/tags/v1"), b"x\n").unwrap();
        std::fs::write(git.join("refs/heads/topic"), format!("{A}\n")).unwrap();
        assert_eq!(read_ref(&git, b"refs/tags/v1").unwrap(), Some(id(A)));
        let edits = [
            RefEdit {
                name: "refs/tags/v1".into(),
                old: Some(id(A)),
                new: None,
                message: String::new(),
            },
            RefEdit {
                name: "refs/heads/topic".into(),
                old: Some(id(A)),
                new: None,
                message: String::new(),
            },
            RefEdit {
                name: "refs/tags/v3".into(),
                old: None,
                new: Some(id(B)),
                message: "push".to_owned(),
            },
        ];
        let mut plan = FilePlan::new();
        plan_ref_edits(&git, &edits, &identity(), LogPolicy::Existing, &mut plan).unwrap();
        let mut tx = RecordingTx::new();
        preserve_and_apply(&mut tx, &[&plan]).unwrap();
        assert_eq!(read_ref(&git, b"refs/tags/v1").unwrap(), None);
        assert_eq!(read_ref(&git, b"refs/tags/v2").unwrap(), Some(id(B)));
        assert_eq!(read_ref(&git, b"refs/heads/topic").unwrap(), None);
        assert!(!git.join("logs/refs/tags/v1").exists());
        assert!(!git.join("logs/refs/tags/v3").exists());
        tx.undo();
        assert_eq!(
            std::fs::read_to_string(git.join(PACKED_REFS)).unwrap(),
            packed
        );
        assert_eq!(read_ref(&git, b"refs/heads/topic").unwrap(), Some(id(A)));
    }

    #[test]
    fn detects_a_reference_changed_after_planning() {
        let (_dir, git) = git_dir();
        let mut plan = FilePlan::new();
        let edit = RefEdit {
            name: "refs/heads/main".into(),
            old: None,
            new: Some(id(A)),
            message: "push".to_owned(),
        };
        plan_ref_edits(&git, &[edit], &identity(), LogPolicy::Always, &mut plan).unwrap();
        std::fs::write(git.join("refs/heads/main"), format!("{B}\n")).unwrap();
        let mut tx = RecordingTx::new();
        let error = preserve_and_apply(&mut tx, &[&plan]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        tx.undo();
        assert_eq!(read_ref(&git, b"refs/heads/main").unwrap(), Some(id(B)));
    }

    #[test]
    fn rejects_invalid_and_symbolic_references() {
        let (_dir, git) = git_dir();
        for name in [
            &b"main"[..],
            b"refs/heads/a..b",
            b"refs/heads/x y",
            b"refs/heads/",
        ] {
            assert_eq!(
                validate_name(name).unwrap_err().problem.as_deref(),
                Some("invalid_name")
            );
        }
        std::fs::write(git.join("refs/heads/sym"), b"ref: refs/heads/main\n").unwrap();
        let error = read_ref(&git, b"refs/heads/sym").unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("not_a_direct_reference"));
        assert!(validate_name(b"HEAD").is_ok());
    }

    #[test]
    fn reads_the_log_policy_from_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let repo = crate::send_pack::testing::init_isolated(dir.path(), false);
        assert_eq!(LogPolicy::of(&repo), LogPolicy::Standard);
        let bare = crate::send_pack::testing::init_isolated(&dir.path().join("bare.git"), true);
        assert_eq!(LogPolicy::of(&bare), LogPolicy::Existing);
        let config = dir.path().join(".git").join("config");
        let mut text = std::fs::read_to_string(&config).unwrap();
        text.push_str("[core]\n\tlogAllRefUpdates = always\n");
        std::fs::write(&config, text).unwrap();
        let repo = crate::send_pack::testing::open_isolated(dir.path());
        assert_eq!(LogPolicy::of(&repo), LogPolicy::Always);
        assert!(LogPolicy::Always.creates_log_for(b"refs/tags/v1"));
        assert!(!LogPolicy::Standard.creates_log_for(b"refs/tags/v1"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
