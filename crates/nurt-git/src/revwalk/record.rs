// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::revwalk::record
// Responsibility:   Records of commits: identities mapped through `.mailmap`, subject.
// Dependencies:     bstr, gix, indexmap, nurt_contracts, nurt_git_transport
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Records of commits: identities mapped through `.mailmap`, subject,
//! trailers, names of references, signature format and changed files.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};

use bstr::{BStr, ByteSlice};
use gix::ObjectId;
use gix::objs::tree::EntryKind;
use indexmap::IndexMap;
use nurt_contracts::error::Error;
use nurt_contracts::record::{
    Commit, CommitFile, CommitSignature, CommitStats, SignatureFormat, SignatureStatus,
};
use nurt_contracts::stage::FieldDemand;
use nurt_contracts::value::Instant;

use super::changes::{FileChange, Side};
use super::lines;
use super::repo::OpenRepo;
use nurt_git_transport::sign::{verify_commit, verify_tag};

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
        let signatures =
            matches!(demand, FieldDemand::Only(fields) if fields.contains("signature"));
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

    /// Whether the signature of commits and tags was asked for.
    pub(crate) fn signatures(&self) -> bool {
        self.signatures
    }

    /// Author of a commit.
    pub(crate) fn author(&self, id: ObjectId) -> Result<Person, Error> {
        let object = self.object(id)?;
        let commit = self.decode(id, &object)?;
        let author = commit
            .author()
            .map_err(|err| self.repo.read_error(format_args!("commit {id}"), err))?;
        Ok(self.person(author))
    }

    /// The record of commit `id`, without changed files.
    pub(crate) fn commit(&self, id: ObjectId) -> Result<Commit, Error> {
        let object = self.object(id)?;
        let commit = self.decode(id, &object)?;
        let author = self.person(
            commit
                .author()
                .map_err(|err| self.repo.read_error(format_args!("commit {id}"), err))?,
        );
        let committer = self.person(
            commit
                .committer()
                .map_err(|err| self.repo.read_error(format_args!("commit {id}"), err))?,
        );
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
            refs: self
                .decorations
                .as_ref()
                .map(|all| all.get(&id).cloned().unwrap_or_default()),
            signature: self.signatures.then(|| {
                signature_of(
                    self.repo,
                    SignedKind::Commit,
                    &object.data,
                    commit
                        .extra_headers()
                        .find("gpgsig")
                        .or_else(|| commit.extra_headers().find("gpgsig-sha256")),
                )
            }),
            stats: None,
            trailers: (!trailers.is_empty()).then_some(trailers),
            tree: Some(commit.tree().to_string()),
            files: None,
            branch: None,
        })
    }

    fn object(&self, id: ObjectId) -> Result<gix::Object<'r>, Error> {
        self.repo
            .repo
            .find_object(id)
            .map_err(|err| self.repo.read_error(format_args!("commit {id}"), err))
    }

    fn decode<'a>(
        &self,
        id: ObjectId,
        object: &'a gix::Object<'r>,
    ) -> Result<gix::objs::CommitRef<'a>, Error> {
        gix::objs::CommitRef::from_bytes(&object.data, self.repo.repo.object_hash())
            .map_err(|err| self.repo.read_error(format_args!("commit {id}"), err))
    }

    /// A recorded identity with its time, after `.mailmap`.
    pub(crate) fn person(&self, signature: gix::actor::SignatureRef<'_>) -> Person {
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
    pub(crate) fn file_stats(
        &self,
        changes: &[FileChange],
    ) -> Result<(Vec<CommitFile>, CommitStats), Error> {
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
            let (added, removed) = if binary {
                (None, None)
            } else {
                let (added, removed) = lines::line_counts(&old, &new);
                (Some(added), Some(removed))
            };
            stats.files += 1;
            stats.added += added.unwrap_or(0);
            stats.removed += removed.unwrap_or(0);
            files.push(CommitFile {
                path: change.path.to_str_lossy().into_owned(),
                status: change.status,
                old_path: change
                    .old_path
                    .as_ref()
                    .map(|path| path.to_str_lossy().into_owned()),
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
            Some(side) if side.mode.kind() == EntryKind::Commit => {
                Ok(format!("Subproject commit {}\n", side.id).into_bytes())
            }
            Some(side) => self.repo.blob(side.id),
        }
    }
}

/// Instant of a time in seconds since the Unix epoch.
pub(crate) fn instant(seconds: i64) -> Instant {
    Instant::from_unix_seconds(seconds).unwrap_or(Instant::UNIX_EPOCH)
}

/// Trailers of a message; repeated tokens keep every value, one per line, as
/// in the records of `crate::repo::records`.
fn trailers(message: &gix::objs::commit::MessageRef<'_>) -> IndexMap<String, String> {
    message
        .body()
        .map(|body| crate::repo::records::trailers(&body))
        .unwrap_or_default()
}

/// Kind of a signed object.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SignedKind {
    /// Commit.
    Commit,
    /// Annotated tag.
    Tag,
}

/// Signature of the object `data`, whose signature header or field is
/// `header`, verified with the signer of `repo`.
///
/// A signature that cannot be verified is reported as unsupported with the
/// format recognised from the header.
pub(crate) fn signature_of(
    repo: &OpenRepo,
    kind: SignedKind,
    data: &[u8],
    header: Option<&BStr>,
) -> CommitSignature {
    let Some(text) = header else {
        return CommitSignature {
            status: SignatureStatus::None,
            format: None,
            key: None,
            signer: None,
        };
    };
    let hash = repo.repo.object_hash();
    let verified = match kind {
        SignedKind::Commit => verify_commit(data, hash, repo.signer.as_ref(), &repo.verify),
        SignedKind::Tag => verify_tag(data, hash, repo.signer.as_ref(), &repo.verify),
    };
    verified.unwrap_or_else(|_| CommitSignature {
        status: SignatureStatus::Unsupported,
        format: Some(signature_format(text)),
        key: None,
        signer: None,
    })
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
/// when detached, `tag: v1` for tags. The names that follow are in reverse
/// order of the full reference names.
fn decorations(repo: &OpenRepo) -> HashMap<ObjectId, Vec<String>> {
    let mut by_commit: HashMap<ObjectId, BTreeMap<(u8, Reverse<String>), String>> = HashMap::new();
    let current = repo.repo.head_name().ok().flatten();
    if let Ok(platform) = repo.repo.references()
        && let Ok(all) = platform.all()
    {
        for mut reference in all.flatten() {
            let full = reference.name().as_bstr().to_str_lossy().into_owned();
            let Ok(target) = reference.peel_to_id() else {
                continue;
            };
            let Some(commit) = repo.peel_to_commit(target.detach()) else {
                continue;
            };
            let short = reference.name().shorten().to_str_lossy().into_owned();
            let (rank, label) = if current
                .as_ref()
                .is_some_and(|name| name.as_bstr() == full.as_bytes())
            {
                (0, format!("HEAD -> {short}"))
            } else if full.starts_with("refs/tags/") {
                (1, format!("tag: {short}"))
            } else {
                (1, short)
            };
            by_commit
                .entry(commit)
                .or_default()
                .insert((rank, Reverse(full)), label);
        }
    }
    if current.is_none()
        && let Ok(head) = repo.repo.head_id()
    {
        by_commit
            .entry(head.detach())
            .or_default()
            .insert((0, Reverse("HEAD".to_owned())), "HEAD".to_owned());
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
        assert_eq!(
            trailers["Signed-off-by"],
            "A <a@example.com>\nB <b@example.com>"
        );
        assert_eq!(record.refs.unwrap(), ["HEAD -> main", "tag: v1"]);
        assert!(record.signature.is_none());
    }

    #[test]
    fn maps_identities_through_mailmap() {
        let mut fixture = Fixture::new();
        fixture.write_file(
            ".mailmap",
            "Proper Name <proper@example.com> <author@example.com>\n",
        );
        let id = fixture.commit("first", &[("a.txt", Some("a\n"))]);
        let repo = fixture.open();
        let records = CommitRecords::new(&repo, &all());
        let record = records.commit(id).unwrap();
        assert_eq!(record.author, "Proper Name");
        assert_eq!(record.author_email.as_deref(), Some("proper@example.com"));
        assert_eq!(records.author(id).unwrap().name, "Proper Name");
    }

    /// A commit signed with an SSH key, as `git commit -S` writes it.
    const SIGNED_COMMIT: &str = "tree 08585692ce06452da6f82ae66b90d98b55536fca
author Tester <tester@example.test> 1767225600 +0000
committer Tester <tester@example.test> 1767225600 +0000
gpgsig -----BEGIN SSH SIGNATURE-----
 U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAg6kpsY+KcUgq+9VB7Ey7F+ZVHdq
 6+vnuSQh7qaRRG0iwAAAADZ2l0AAAAAAAAAAZzaGE1MTIAAABTAAAAC3NzaC1lZDI1NTE5
 AAAAQEApspBvUHXfArdtPu/9u4OCqb0TgwpRINAzTAJ1YG2KrMBjaOgGghYnKkeKoENnye
 F4XiKcKSFJrlSKMn1QzAg=
 -----END SSH SIGNATURE-----

signed commit
";

    const SIGNING_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOpKbGPinFIKvvVQexMuxfmVR3auvr57kkIe6mkURtIs";

    const SIGNING_FINGERPRINT: &str = "SHA256:z/fSv0Z0RZS+Lccbc6ZoOWwt/fbj1VFJGSBKQbo4icE";

    fn signature_demand() -> FieldDemand {
        FieldDemand::Only(BTreeSet::from(["signature".to_owned()]))
    }

    /// Verified signature of `commit` in a repository that trusts the key
    /// of the fixture when `trusted`.
    fn verified(commit: &str, trusted: bool) -> CommitSignature {
        let mut fixture = Fixture::new();
        let id = fixture.commit_object(commit.as_bytes());
        if trusted {
            let signers = fixture.path("allowed_signers");
            std::fs::write(&signers, format!("tester@example.test {SIGNING_KEY}\n")).unwrap();
            let config = std::path::Path::new(&fixture.root()).join(".git/config");
            let mut text = std::fs::read_to_string(&config).unwrap();
            text.push_str(&format!(
                "[gpg \"ssh\"]\n\tallowedSignersFile = {signers}\n"
            ));
            std::fs::write(&config, text).unwrap();
        }
        let repo = fixture.open();
        let records = CommitRecords::new(&repo, &signature_demand());
        records.commit(id).unwrap().signature.unwrap()
    }

    #[test]
    fn reports_a_good_signature_of_a_trusted_key() {
        let signature = verified(SIGNED_COMMIT, true);
        assert_eq!(signature.status, SignatureStatus::Good);
        assert_eq!(signature.format, Some(SignatureFormat::Ssh));
        assert_eq!(signature.key.as_deref(), Some(SIGNING_FINGERPRINT));
    }

    #[test]
    fn reports_an_unknown_key_without_allowed_signers() {
        let signature = verified(SIGNED_COMMIT, false);
        assert_eq!(signature.status, SignatureStatus::UnknownKey);
        assert_eq!(signature.format, Some(SignatureFormat::Ssh));
    }

    #[test]
    fn reports_a_bad_signature_of_an_altered_commit() {
        let altered = SIGNED_COMMIT.replace("signed commit", "altered commit");
        let signature = verified(&altered, true);
        assert_eq!(signature.status, SignatureStatus::Bad);
        assert_eq!(signature.format, Some(SignatureFormat::Ssh));
    }

    #[test]
    fn reports_an_unparsable_signature_as_bad() {
        let start = SIGNED_COMMIT.find(" U1NIU0lH").unwrap();
        let end = SIGNED_COMMIT.find(" -----END").unwrap();
        let malformed = format!("{} abc\n{}", &SIGNED_COMMIT[..start], &SIGNED_COMMIT[end..]);
        let signature = verified(&malformed, true);
        assert_eq!(signature.status, SignatureStatus::Bad);
        assert_eq!(signature.format, Some(SignatureFormat::Ssh));
    }

    #[test]
    fn reports_signature_only_when_asked_for() {
        let mut fixture = Fixture::new();
        let plain = fixture.commit("plain", &[("a", Some("a\n"))]);
        let repo = fixture.open();
        let records = CommitRecords::new(&repo, &signature_demand());
        let record = records.commit(plain).unwrap();
        assert_eq!(record.signature.unwrap().status, SignatureStatus::None);
        assert!(record.refs.is_none());
        assert!(
            CommitRecords::new(&repo, &all())
                .commit(plain)
                .unwrap()
                .signature
                .is_none()
        );
        assert_eq!(
            signature_format(b"-----BEGIN PGP SIGNATURE".as_bstr()),
            SignatureFormat::Openpgp
        );
        assert_eq!(
            signature_format(b"-----BEGIN SIGNED MESSAGE".as_bstr()),
            SignatureFormat::X509
        );
    }

    #[test]
    fn counts_lines_of_changed_files() {
        let mut fixture = Fixture::new();
        let first = fixture.commit("first", &[("a.txt", Some("a\nb\n")), ("bin", Some("x\0y"))]);
        let second = fixture.commit(
            "second",
            &[("a.txt", Some("a\nB\nc\n")), ("bin", Some("x\0z"))],
        );
        let repo = fixture.open();
        let records = CommitRecords::new(&repo, &all());
        let changes = tree_changes(
            &repo,
            Some(repo.tree_of(first).unwrap()),
            repo.tree_of(second).unwrap(),
            Detect::Renames,
            None,
        )
        .unwrap();
        let (files, stats) = records.file_stats(&changes).unwrap();
        assert_eq!((stats.files, stats.added, stats.removed), (2, 2, 1));
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(files[0].status, GitFileState::Modified);
        assert_eq!(files[1].binary, Some(true));
        assert_eq!(files[1].added, None);
    }

    #[test]
    fn decorations_follow_the_order_of_git() {
        let mut fixture = Fixture::new();
        let id = fixture.commit("first", &[("a", Some("a\n"))]);
        fixture.branch("alpha", id);
        fixture.branch("zeta", id);
        fixture.tag("v1", id);
        let repo = fixture.open();
        let records = CommitRecords::new(&repo, &all());
        assert_eq!(
            records.commit(id).unwrap().refs.unwrap(),
            ["HEAD -> main", "tag: v1", "zeta", "alpha"]
        );
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
