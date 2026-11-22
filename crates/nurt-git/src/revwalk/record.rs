// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Records of commits: identities mapped through `.mailmap`, subject,
//! trailers, names of references, signature format and changed files.

use std::collections::{BTreeMap, HashMap};

use bstr::{BStr, ByteSlice};
use gix::ObjectId;
use gix::objs::tree::EntryKind;
use indexmap::IndexMap;
use nurt_contracts::error::Error;
use nurt_contracts::record::{Commit, CommitFile, CommitSignature, CommitStats, SignatureFormat, SignatureStatus};
use nurt_contracts::stage::FieldDemand;
use nurt_contracts::value::Instant;

use super::changes::{FileChange, Side};
use super::lines;
use super::repo::OpenRepo;

/// A person as recorded in a commit, after `.mailmap`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Person {
    /// Name.
    pub(crate) name: String,
    /// E-mail address.
    pub(crate) email: String,
    /// Time in seconds since the Unix epoch.
    pub(crate) seconds: i64,
    /// Offset of the local time from UTC, in seconds.
    pub(crate) offset: i32,
}

/// Builds `commit` records for one repository.
pub(crate) struct CommitRecords<'r> {
    repo: &'r OpenRepo,
    mailmap: gix::mailmap::Snapshot,
    decorations: Option<HashMap<ObjectId, Vec<String>>>,
    signatures: bool,
}

impl<'r> CommitRecords<'r> {
    /// Prepares records with the fields of `demand`: names of references
    /// only when used, the signature only when asked for by name.
    pub(crate) fn new(repo: &'r OpenRepo, demand: &FieldDemand) -> Self {
        let decorations = demand.wants("refs").then(|| decorations(repo));
        let signatures = matches!(demand, FieldDemand::Only(fields) if fields.contains("signature"));
        Self {
            repo,
            mailmap: repo.repo.open_mailmap(),
            decorations,
            signatures,
        }
    }

    /// The repository of the records.
    pub(crate) fn repo(&self) -> &'r OpenRepo {
        self.repo
    }

    /// Author of a commit.
    pub(crate) fn author(&self, id: ObjectId) -> Result<Person, Error> {
        let object = self.object(id)?;
        let commit = self.decode(id, &object)?;
        let author = commit.author().map_err(|err| self.repo.read_error(format_args!("commit {id}"), err))?;
        Ok(self.person(author))
    }

    /// The record of commit `id`, without changed files.
    pub(crate) fn commit(&self, id: ObjectId) -> Result<Commit, Error> {
        let object = self.object(id)?;
        let commit = self.decode(id, &object)?;
        let invalid = |err: gix::objs::decode::Error| self.repo.read_error(format_args!("commit {id}"), err);
        let author = self.person(commit.author().map_err(invalid)?);
        let committer = self.person(commit.committer().map_err(invalid)?);
        let raw = commit.message;
        let parsed = gix::objs::commit::MessageRef::from_bytes(raw);
        let trailers = trailers(&parsed);
        Ok(Commit {
            id: id.to_string(),
            short_id: self.repo.short_id(id),
            author: author.name,
            time: instant(author.seconds),
            message: raw.to_str_lossy().trim_end_matches('\n').to_owned(),
            subject: parsed.summary().to_str_lossy().into_owned(),
            parents: commit.parents().map(|parent| parent.to_string()).collect(),
            author_email: Some(author.email),
            committer: Some(committer.name),
            committer_email: Some(committer.email),
            commit_time: Some(instant(committer.seconds)),
            refs: self.decorations.as_ref().map(|all| all.get(&id).cloned().unwrap_or_default()),
            signature: self.signatures.then(|| signature(&commit)),
            stats: None,
            trailers: (!trailers.is_empty()).then_some(trailers),
            tree: Some(commit.tree().to_string()),
            files: None,
        })
    }

    fn object(&self, id: ObjectId) -> Result<gix::Object<'r>, Error> {
        self.repo
            .repo
            .find_object(id)
            .map_err(|err| self.repo.read_error(format_args!("commit {id}"), err))
    }

    fn decode<'a>(&self, id: ObjectId, object: &'a gix::Object<'r>) -> Result<gix::objs::CommitRef<'a>, Error> {
        gix::objs::CommitRef::from_bytes(&object.data, self.repo.repo.object_hash())
            .map_err(|err| self.repo.read_error(format_args!("commit {id}"), err))
    }

    fn person(&self, signature: gix::actor::SignatureRef<'_>) -> Person {
        let signature = signature.trim();
        let seconds = signature.seconds();
        let offset = signature.time().map_or(0, |time| time.offset);
        let mapped = self.mailmap.resolve(signature);
        Person {
            name: mapped.name.to_str_lossy().into_owned(),
            email: mapped.email.to_str_lossy().into_owned(),
            seconds,
            offset,
        }
    }

    /// Changed files and their totals; binary files and submodules count no
    /// lines.
    pub(crate) fn file_stats(&self, changes: &[FileChange]) -> Result<(Vec<CommitFile>, CommitStats), Error> {
        let mut files = Vec::with_capacity(changes.len());
        let mut stats = CommitStats {
            files: 0,
            added: 0,
            removed: 0,
        };
        for change in changes {
            let old = self.content(change.old)?;
            let new = self.content(change.new)?;
            let binary = lines::is_binary(&old) || lines::is_binary(&new);
            let (added, removed) = if binary { (None, None) } else {
                let (added, removed) = lines::line_counts(&old, &new);
                (Some(added), Some(removed))
            };
            stats.files += 1;
            stats.added += added.unwrap_or(0);
            stats.removed += removed.unwrap_or(0);
            files.push(CommitFile {
                path: change.path.to_str_lossy().into_owned(),
                status: change.status,
                old_path: change.old_path.as_ref().map(|path| path.to_str_lossy().into_owned()),
                added,
                removed,
                binary: Some(binary),
            });
        }
        Ok((files, stats))
    }

    /// Content of one side of a change; a submodule is the line `git` shows
    /// for it.
    pub(crate) fn content(&self, side: Option<Side>) -> Result<Vec<u8>, Error> {
        match side {
            None => Ok(Vec::new()),
            Some(side) if side.mode.kind() == EntryKind::Commit => Ok(format!("Subproject commit {}\n", side.id).into_bytes()),
            Some(side) => self.repo.blob(side.id),
        }
    }
}

/// Instant of a time in seconds since the Unix epoch.
pub(crate) fn instant(seconds: i64) -> Instant {
    Instant::from_unix_seconds(seconds).unwrap_or(Instant::UNIX_EPOCH)
}

fn trailers(message: &gix::objs::commit::MessageRef<'_>) -> IndexMap<String, String> {
    let mut out: IndexMap<String, String> = IndexMap::new();
    let Some(body) = message.body() else {
        return out;
    };
    for trailer in body.trailers() {
        let token = trailer.token.to_str_lossy().into_owned();
        let value = trailer.value.to_str_lossy().into_owned();
        out.entry(token)
            .and_modify(|existing| {
                existing.push_str(", ");
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    out
}

fn signature(commit: &gix::objs::CommitRef<'_>) -> CommitSignature {
    let header = commit
        .extra_headers()
        .find("gpgsig")
        .or_else(|| commit.extra_headers().find("gpgsig-sha256"));
    let Some(text) = header else {
        return CommitSignature {
            status: SignatureStatus::None,
            format: None,
            key: None,
            signer: None,
        };
    };
    CommitSignature {
        status: SignatureStatus::Unsupported,
        format: Some(signature_format(text)),
        key: None,
        signer: None,
    }
}

fn signature_format(text: &BStr) -> SignatureFormat {
    if text.starts_with(b"-----BEGIN SSH SIGNATURE") {
        SignatureFormat::Ssh
    } else if text.starts_with(b"-----BEGIN SIGNED MESSAGE") {
        SignatureFormat::X509
    } else {
        SignatureFormat::Openpgp
    }
}

/// Names of the references that point to each commit, as `git log
/// --decorate` writes them: `HEAD -> main` for the current branch, `HEAD`
/// when detached, `tag: v1` for tags.
fn decorations(repo: &OpenRepo) -> HashMap<ObjectId, Vec<String>> {
    let mut by_commit: HashMap<ObjectId, BTreeMap<(u8, String), String>> = HashMap::new();
    let current = repo.repo.head_name().ok().flatten();
    if let Ok(platform) = repo.repo.references() {
        if let Ok(all) = platform.all() {
            for mut reference in all.flatten() {
                let full = reference.name().as_bstr().to_str_lossy().into_owned();
                let Ok(target) = reference.peel_to_id() else {
                    continue;
                };
                let Some(commit) = repo.peel_to_commit(target.detach()) else {
                    continue;
                };
                let short = reference.name().shorten().to_str_lossy().into_owned();
                let (rank, label) = if current.as_ref().is_some_and(|name| name.as_bstr() == full.as_bytes()) {
                    (0, format!("HEAD -> {short}"))
                } else if full.starts_with("refs/tags/") {
                    (3, format!("tag: {short}"))
                } else if full.starts_with("refs/heads/") {
                    (1, short)
                } else {
                    (2, short)
                };
                by_commit.entry(commit).or_default().insert((rank, full), label);
            }
        }
    }
    if current.is_none() {
        if let Ok(head) = repo.repo.head_id() {
            by_commit
                .entry(head.detach())
                .or_default()
                .insert((0, "HEAD".to_owned()), "HEAD".to_owned());
        }
    }
    by_commit
        .into_iter()
        .map(|(id, names)| (id, names.into_values().collect()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revwalk::changes::{Detect, tree_changes};
    use crate::revwalk::fixture::Fixture;
    use nurt_contracts::record::GitFileState;
    use std::collections::BTreeSet;

    fn all() -> FieldDemand {
        FieldDemand::All
    }

    #[test]
    fn builds_commit_records_with_subject_and_trailers() {
        let mut fixture = Fixture::new();
        let id = fixture.commit(
            "feat: add parser\n\nLonger body.\n\nSigned-off-by: A <a@example.com>\nSigned-off-by: B <b@example.com>\n",
            &[("a.txt", Some("a\n"))],
        );
        fixture.tag("v1", id);
        let repo = fixture.open();
        let records = CommitRecords::new(&repo, &all());
        let record = records.commit(id).unwrap();
        assert_eq!(record.id, id.to_string());
        assert_eq!(record.subject, "feat: add parser");
        assert!(record.message.ends_with("B <b@example.com>"));
        assert_eq!(record.author, "Author");
        assert_eq!(record.author_email.as_deref(), Some("author@example.com"));
        assert!(record.parents.is_empty());
        assert_eq!(record.time, instant(fixture.time_of(id)));
        let trailers = record.trailers.unwrap();
        assert_eq!(trailers["Signed-off-by"], "A <a@example.com>, B <b@example.com>");
        assert_eq!(record.refs.unwrap(), ["HEAD -> main", "tag: v1"]);
        assert!(record.signature.is_none());
    }

    #[test]
    fn maps_identities_through_mailmap() {
        let mut fixture = Fixture::new();
        fixture.write_file(".mailmap", "Proper Name <proper@example.com> <author@example.com>\n");
        let id = fixture.commit("first", &[("a.txt", Some("a\n"))]);
        let repo = fixture.open();
        let records = CommitRecords::new(&repo, &all());
        let record = records.commit(id).unwrap();
        assert_eq!(record.author, "Proper Name");
        assert_eq!(record.author_email.as_deref(), Some("proper@example.com"));
        assert_eq!(records.author(id).unwrap().name, "Proper Name");
    }

    #[test]
    fn reports_signature_format_only_when_asked_for() {
        let mut fixture = Fixture::new();
        let id = fixture.commit_signed("signed", "-----BEGIN SSH SIGNATURE-----\nabc\n-----END SSH SIGNATURE-----");
        let plain = fixture.commit("plain", &[("a", Some("a\n"))]);
        let repo = fixture.open();
        let demand = FieldDemand::Only(BTreeSet::from(["signature".to_owned()]));
        let records = CommitRecords::new(&repo, &demand);
        let signature = records.commit(id).unwrap().signature.unwrap();
        assert_eq!(signature.status, SignatureStatus::Unsupported);
        assert_eq!(signature.format, Some(SignatureFormat::Ssh));
        assert_eq!(records.commit(plain).unwrap().signature.unwrap().status, SignatureStatus::None);
        assert!(records.commit(plain).unwrap().refs.is_none());
        assert_eq!(signature_format(b"-----BEGIN PGP SIGNATURE".as_bstr()), SignatureFormat::Openpgp);
        assert_eq!(signature_format(b"-----BEGIN SIGNED MESSAGE".as_bstr()), SignatureFormat::X509);
    }

    #[test]
    fn counts_lines_of_changed_files() {
        let mut fixture = Fixture::new();
        let first = fixture.commit("first", &[("a.txt", Some("a\nb\n")), ("bin", Some("x\0y"))]);
        let second = fixture.commit("second", &[("a.txt", Some("a\nB\nc\n")), ("bin", Some("x\0z"))]);
        let repo = fixture.open();
        let records = CommitRecords::new(&repo, &all());
        let changes = tree_changes(&repo, Some(repo.tree_of(first).unwrap()), repo.tree_of(second).unwrap(), Detect::Renames, None).unwrap();
        let (files, stats) = records.file_stats(&changes).unwrap();
        assert_eq!((stats.files, stats.added, stats.removed), (2, 2, 1));
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(files[0].status, GitFileState::Modified);
        assert_eq!(files[1].binary, Some(true));
        assert_eq!(files[1].added, None);
    }

    #[test]
    fn detached_head_is_named_head() {
        let mut fixture = Fixture::new();
        let id = fixture.commit("first", &[("a", Some("a\n"))]);
        fixture.detach_head(id);
        let repo = fixture.open();
        let records = CommitRecords::new(&repo, &all());
        assert_eq!(records.commit(id).unwrap().refs.unwrap(), ["HEAD", "main"]);
    }
}
