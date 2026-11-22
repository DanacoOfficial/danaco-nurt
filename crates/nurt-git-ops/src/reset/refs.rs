// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! References, reflogs and state files written through the journal.

use std::path::Path;

use gix::actor::Signature;
use gix::bstr::{BString, ByteSlice};
use gix::hash::ObjectId;
use nurt_contracts::error::{ErrorCode, Result};

use super::ident::format_time;
use super::repo::{HeadState, Repo, error, io_error};
use super::txplan::{Before, TxPlan, content_hash};

/// Name of the file with packed references.
const PACKED_REFS: &str = "packed-refs";

/// One entry of a reflog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReflogEntry {
    /// Value before the update.
    pub(crate) old: ObjectId,
    /// Value after the update.
    pub(crate) new: ObjectId,
    /// Everything after the two object names, without the line end:
    /// identity, time, tab and message, kept byte for byte.
    pub(crate) rest: BString,
}

impl ReflogEntry {
    /// Message of the entry.
    pub(crate) fn message(&self) -> String {
        self.rest
            .find_byte(b'\t')
            .map(|tab| self.rest[tab + 1..].to_str_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Time of the entry in seconds since the Unix epoch.
    pub(crate) fn seconds(&self) -> Option<i64> {
        let head = self.rest.split_str(b"\t").next()?;
        let head = head.to_str().ok()?;
        let after_mail = head.rsplit_once("> ")?.1;
        after_mail.split(' ').next()?.parse().ok()
    }

    fn to_line(&self) -> Vec<u8> {
        let mut line = Vec::with_capacity(self.rest.len() + 2 * 41 + 1);
        line.extend_from_slice(self.old.to_hex().to_string().as_bytes());
        line.push(b' ');
        line.extend_from_slice(self.new.to_hex().to_string().as_bytes());
        line.push(b' ');
        line.extend_from_slice(&self.rest);
        line.push(b'\n');
        line
    }
}

/// Reflog message as git stores it: runs of white space collapsed to one
/// space, leading and trailing white space removed.
pub(crate) fn normalize_message(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One reflog line.
pub(crate) fn reflog_line(
    old: ObjectId,
    new: ObjectId,
    who: &Signature,
    message: &str,
) -> Vec<u8> {
    format!(
        "{} {} {} <{}> {}\t{}\n",
        old.to_hex(),
        new.to_hex(),
        who.name,
        who.email,
        format_time(who.time),
        normalize_message(message)
    )
    .into_bytes()
}

/// Parses a reflog file; lines that do not parse are skipped like git does.
pub(crate) fn parse_reflog(data: &[u8]) -> Vec<ReflogEntry> {
    data.lines()
        .filter_map(|line| {
            let mut parts = line.splitn_str(3, b" ");
            let old = ObjectId::from_hex(parts.next()?).ok()?;
            let new = ObjectId::from_hex(parts.next()?).ok()?;
            let rest = BString::from(parts.next()?);
            Some(ReflogEntry { old, new, rest })
        })
        .collect()
}

/// Entries of the reflog of `name`, oldest first.
pub(crate) fn read_reflog(repo: &Repo, name: &str) -> Result<Vec<ReflogEntry>> {
    let path = repo.reflog_path(name);
    match std::fs::read(&path) {
        Ok(data) => Ok(parse_reflog(&data)),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(cause) => Err(io_error("cannot read reflog", cause)),
    }
}

/// Current value of a reference, `None` when it does not exist.
pub(crate) fn read_ref(repo: &Repo, name: &str) -> Result<Option<ObjectId>> {
    let reference = repo
        .repo
        .try_find_reference(name)
        .map_err(|cause| io_error("cannot read reference", cause))?;
    Ok(reference.and_then(|reference| reference.target().try_id().map(ToOwned::to_owned)))
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(data) => Ok(Some(data)),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(cause) => Err(io_error("cannot read repository file", cause)),
    }
}

/// Whether a lock file of `path` exists.
pub(crate) fn check_unlocked(path: &Path) -> Result<()> {
    let mut lock = path.as_os_str().to_owned();
    lock.push(".lock");
    if Path::new(&lock).exists() {
        return Err(error(
            ErrorCode::GitLocked,
            format!(
                "the lock file {} exists; another git process may be running",
                Path::new(&lock).display()
            ),
        )
        .with_detail("path", serde_json::Value::from(Path::new(&lock).to_string_lossy().into_owned())));
    }
    Ok(())
}

/// Writes a reference or state file to `value` by the lock protocol.
pub(crate) fn plan_ref_write(plan: &mut TxPlan, repo: &Repo, name: &str, value: ObjectId) -> Result<()> {
    let path = repo.ref_path(name);
    check_unlocked(&path)?;
    plan.ensure_parent_dirs(&path, &repo.common_dir)?;
    plan.locked_write(&path, Some(format!("{}\n", value.to_hex()).into_bytes()))
}

/// Appends a line to the reflog of `name`, creating the file when needed.
pub(crate) fn plan_reflog_append(plan: &mut TxPlan, repo: &Repo, name: &str, line: Vec<u8>) -> Result<()> {
    let path = repo.reflog_path(name);
    if path.is_file() {
        plan.append(&path, line)
    } else {
        plan.ensure_parent_dirs(&path, &repo.common_dir)?;
        plan.write(&path, line, &Before::Absent)
    }
}

/// Replaces the whole reflog of `name`.
pub(crate) fn plan_reflog_rewrite(
    plan: &mut TxPlan,
    repo: &Repo,
    name: &str,
    entries: &[ReflogEntry],
) -> Result<()> {
    let path = repo.reflog_path(name);
    let before = match read_optional(&path)? {
        Some(data) => Before::File(content_hash(&data)),
        None => Before::Absent,
    };
    let bytes = entries.iter().flat_map(ReflogEntry::to_line).collect();
    plan.write(&path, bytes, &before)
}

/// Deletes a reference: the loose file, its entry in `packed-refs` and its
/// reflog.
pub(crate) fn plan_ref_delete(plan: &mut TxPlan, repo: &Repo, name: &str) -> Result<()> {
    let loose = repo.ref_path(name);
    if let Some(data) = read_optional(&loose)? {
        check_unlocked(&loose)?;
        plan.remove(&loose, &Before::File(content_hash(&data)))?;
    }
    let packed_path = repo.common_dir.join(PACKED_REFS);
    if let Some(packed) = read_optional(&packed_path)? {
        if let Some(rewritten) = without_packed_ref(&packed, name) {
            check_unlocked(&packed_path)?;
            plan.locked_write(&packed_path, Some(rewritten))?;
        }
    }
    let log = repo.reflog_path(name);
    if let Some(data) = read_optional(&log)? {
        plan.remove(&log, &Before::File(content_hash(&data)))?;
    }
    Ok(())
}

/// `packed-refs` without `name` and its peeled line, or `None` when `name`
/// is not packed.
pub(crate) fn without_packed_ref(packed: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(packed.len());
    let mut found = false;
    let mut skip_peeled = false;
    for line in packed.lines_with_terminator() {
        if skip_peeled && line.starts_with(b"^") {
            continue;
        }
        skip_peeled = false;
        let is_target = !line.starts_with(b"#")
            && line
                .trim_end()
                .split_str(b" ")
                .nth(1)
                .is_some_and(|ref_name| ref_name == name.as_bytes());
        if is_target {
            found = true;
            skip_peeled = true;
            continue;
        }
        out.extend_from_slice(line);
    }
    found.then_some(out)
}

/// Moves the current branch, or a detached `HEAD`, to `new` with reflog
/// entries as `git update-ref` writes them: the branch log only when the
/// value changes, the log of a symbolic `HEAD` always, the log of a
/// detached `HEAD` only when the value changes.
pub(crate) fn plan_head_update(
    plan: &mut TxPlan,
    repo: &Repo,
    head: &HeadState,
    new: ObjectId,
    who: &Signature,
    message: &str,
) -> Result<()> {
    let null = ObjectId::null(repo.repo.object_hash());
    let old = head.id.unwrap_or(null);
    let changed = old != new;
    match &head.branch {
        Some(branch) => {
            let name = branch.as_bstr().to_str_lossy().into_owned();
            if changed {
                plan_ref_write(plan, repo, &name, new)?;
                plan_reflog_append(plan, repo, &name, reflog_line(old, new, who, message))?;
            }
            plan_reflog_append(plan, repo, "HEAD", reflog_line(old, new, who, message))
        }
        None if changed => {
            plan_ref_write(plan, repo, "HEAD", new)?;
            plan_reflog_append(plan, repo, "HEAD", reflog_line(old, new, who, message))
        }
        None => Ok(()),
    }
}

/// Deletes the given state files of the repository directory when present.
pub(crate) fn plan_state_removal(plan: &mut TxPlan, repo: &Repo, names: &[&str]) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for name in names {
        let path = repo.git_dir.join(name);
        if let Some(data) = read_optional(&path)? {
            plan.remove(&path, &Before::File(content_hash(&data)))?;
            removed.push((*name).to_owned());
        }
    }
    Ok(removed)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    #[test]
    fn normalize_collapses_white_space() {
        assert_eq!(normalize_message("  On main: a\n\nb  "), "On main: a b");
    }

    #[test]
    fn reflog_round_trips_byte_for_byte() {
        let data = format!("{A} {B} C <c@x> 1767265200 +0100\tWIP on main: x\n");
        let entries = parse_reflog(data.as_bytes());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message(), "WIP on main: x");
        assert_eq!(entries[0].seconds(), Some(1_767_265_200));
        assert_eq!(entries[0].to_line(), data.as_bytes());
    }

    #[test]
    fn reflog_line_uses_committer_identity() {
        let who = Signature {
            name: "C".into(),
            email: "c@x".into(),
            time: gix::date::Time::new(5, -3600),
        };
        let a = ObjectId::from_hex(A.as_bytes()).unwrap();
        let line = reflog_line(a, a, &who, "reset: moving to HEAD");
        assert_eq!(
            String::from_utf8(line).unwrap(),
            format!("{A} {A} C <c@x> 5 -0100\treset: moving to HEAD\n")
        );
    }

    #[test]
    fn packed_ref_and_peeled_line_are_removed() {
        let packed = format!("# pack-refs with: peeled\n{A} refs/heads/main\n{B} refs/stash\n^{A}\n");
        let out = without_packed_ref(packed.as_bytes(), "refs/stash").unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("# pack-refs with: peeled\n{A} refs/heads/main\n")
        );
        assert!(without_packed_ref(packed.as_bytes(), "refs/heads/x").is_none());
    }
}
