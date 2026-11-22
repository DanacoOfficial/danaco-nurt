// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.tag` command.
//!
//! [`list`] reads the tags of a repository as `git_tag` records in the order
//! of `git tag --list`. [`plan_create`] and [`plan_delete`] read the
//! repository and plan a change without writing; [`TagPlan::apply`] writes
//! the tag object, when there is one, and changes the reference through a
//! [`MutationTx`].
//!
//! An annotated tag object is byte-identical to the object `git tag -a -m`
//! writes for the same tagger, time and message. An annotated tag is signed
//! by the built-in signer of [`nurt_git_transport::sign`] when
//! [`NewTag::sign`] asks for it, or by default when `tag.gpgSign` is true; a
//! tag without a message stays lightweight and unsigned.
//! [`GitTagCommand`] is the implementation used by the executor.

mod command;

use bstr::{BString, ByteSlice};
use gix_hash::ObjectId;
use gix_object::WriteTo;
use gix_ref::{FullName, Target};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{ExemptWrite, MutationTx};
use nurt_contracts::record::{GitTag, GitTagKind};
use nurt_contracts::report::Change;
use nurt_contracts::value::Instant;
use nurt_git_transport::sign::{Signer, SigningConfig, sign_tag};

pub use command::GitTagCommand;

use crate::branch::{quoted_repo, revision_error};
use crate::refs_tx::{PreparedRefTx, PreviousValue, RefEdit, RefLayout, RefOutcome, RefTx};

const TAGS_PREFIX: &str = "refs/tags/";
const ABBREV_DIGITS: usize = 7;
const SECONDS_PER_DAY: i64 = 86_400;
/// Comment character of `core.commentChar` at its default value.
const COMMENT_CHAR: char = '#';

/// Signer and settings used for a tag when `tag.gpgSign` is true.
#[derive(Clone, Copy)]
pub struct TagSigning<'a> {
    /// Signer of the tag object.
    pub signer: &'a dyn Signer,
    /// Key and format settings.
    pub config: &'a SigningConfig,
}

impl std::fmt::Debug for TagSigning<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TagSigning").finish_non_exhaustive()
    }
}

/// Parameters of a new tag.
#[derive(Debug)]
pub struct NewTag<'a> {
    /// Short name, without `refs/tags/`.
    pub name: &'a str,
    /// Revision the tag points to.
    pub revision: &'a str,
    /// Message; a tag with a message is annotated.
    pub message: Option<&'a str>,
    /// Tagger and time of an annotated tag; committer of the reflog entry.
    pub tagger: gix_actor::Signature,
    /// Whether the annotated tag is signed; `None` follows `tag.gpgSign`.
    pub sign: Option<bool>,
    /// Signer used when the tag is signed.
    pub signing: Option<TagSigning<'a>>,
}

impl NewTag<'_> {
    /// Whether the tag is created as a signed annotated tag.
    #[must_use]
    pub fn signs(&self, repo: &gix::Repository) -> bool {
        self.message.is_some() && self.sign.unwrap_or_else(|| signs_tags(repo))
    }
}

/// Planned change of a tag.
#[derive(Debug)]
pub struct TagPlan {
    refs: PreparedRefTx,
    object: Option<(ObjectId, gix_object::Tag)>,
    record: Option<GitTag>,
}

impl TagPlan {
    /// Outcome of the reference edit as planned.
    #[must_use]
    pub fn outcomes(&self) -> &[RefOutcome] {
        self.refs.outcomes()
    }

    /// Record of the created tag.
    #[must_use]
    pub fn record(&self) -> Option<&GitTag> {
        self.record.as_ref()
    }

    /// Planned changes of files, one per target.
    #[must_use]
    pub fn changes(&self) -> Vec<Change> {
        crate::branch::command::target_changes(&self.refs.targets())
    }

    /// Writes the tag object and changes the reference.
    ///
    /// The tag object is written to the object store before the reference
    /// targets are declared; the write is recorded as exempt from the
    /// journal.
    ///
    /// # Errors
    ///
    /// `E_IO` when the object cannot be written, and the errors of
    /// [`PreparedRefTx::apply`].
    pub fn apply(self, repo: &gix::Repository, tx: &mut dyn MutationTx) -> Result<Vec<RefOutcome>> {
        if let Some((planned, tag)) = &self.object {
            let written = repo.write_object(tag).map_err(|error| {
                Error::new(
                    ErrorCode::Io,
                    format!("cannot write the tag object: {error}"),
                )
            })?;
            if written.detach() != *planned {
                return Err(Error::new(
                    ErrorCode::Internal,
                    "the written tag object differs from the planned one",
                ));
            }
            let objects = crate::refs_tx::RefLayout::from_repo(repo)?;
            tx.record_exempt(ExemptWrite::ObjectStore {
                repo: objects.common_dir().clone(),
            });
        }
        self.refs.apply(tx)
    }
}

/// Tags of the repository in the order of `git tag --list`.
///
/// # Errors
///
/// `E_IO` when the references or objects cannot be read.
pub fn list(repo: &gix::Repository) -> Result<Vec<GitTag>> {
    let platform = repo.references().map_err(read_error)?;
    let mut tags = Vec::new();
    for reference in platform.tags().map_err(read_error)? {
        let reference = reference.map_err(read_error)?;
        let full = reference.name().as_bstr().to_owned();
        let Some(id) = reference.target().try_id().map(ToOwned::to_owned) else {
            continue;
        };
        let name = full
            .strip_prefix(TAGS_PREFIX.as_bytes())
            .unwrap_or(&full)
            .to_str_lossy()
            .into_owned();
        tags.push(describe(repo, name, id)?);
    }
    tags.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    Ok(tags)
}

/// Record of the tag `name` whose reference holds `id`.
fn describe(repo: &gix::Repository, name: String, id: ObjectId) -> Result<GitTag> {
    let object = repo.find_object(id).map_err(read_error)?;
    if object.kind != gix_object::Kind::Tag {
        return Ok(lightweight(name, id));
    }
    let tag =
        gix_object::TagRef::from_bytes(&object.data, repo.object_hash()).map_err(read_error)?;
    let tagger = tag.tagger().map_err(read_error)?;
    let time = match &tagger {
        Some(signature) => {
            let time = signature.time().map_err(read_error)?;
            Instant::from_unix_seconds(time.seconds)
        }
        None => None,
    };
    let target = tag.target();
    Ok(GitTag {
        name,
        target: target.to_hex().to_string(),
        kind: GitTagKind::Annotated,
        tagger: tagger.map(|s| format!("{} <{}>", s.name, s.email)),
        time,
        message: Some(tag.message.to_str_lossy().into_owned()),
        signature: None,
    })
}

fn lightweight(name: String, id: ObjectId) -> GitTag {
    GitTag {
        name,
        target: id.to_hex().to_string(),
        kind: GitTagKind::Lightweight,
        tagger: None,
        time: None,
        message: None,
        signature: None,
    }
}

/// Plans a new tag.
///
/// # Errors
///
/// * `E_GIT_REF` with problem `invalid_name` for a name Git rejects and
///   `exists` when the tag exists;
/// * `E_GIT_REVISION` when the revision cannot be resolved;
/// * `E_MISSING_PARAM` with problem `missing_message` when `sign` is true
///   and there is no message;
/// * `E_GIT_SIGN` with problem `signer_unavailable` when the tag is signed
///   and no signer is given, and the errors of the signer;
/// * the errors of [`RefTx::prepare`].
pub fn plan_create(repo: &gix::Repository, new: NewTag<'_>) -> Result<TagPlan> {
    let full = tag_name(repo, new.name)?;
    if new.sign == Some(true) && new.message.is_none() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            "a signed tag is an annotated tag and needs a message; pass message",
        )
        .with_problem("missing_message"));
    }
    let id = repo
        .rev_parse_single(new.revision)
        .map_err(|error| revision_error(repo, new.revision, &error))?
        .detach();
    let mut record = lightweight(new.name.to_owned(), id);
    let mut target = id;
    let mut object = None;
    if let Some(message) = new.message {
        let tag = annotated(repo, &new, id, message)?;
        let bytes = tag_bytes(&tag)?;
        let tag_id = gix_object::compute_hash(repo.object_hash(), gix_object::Kind::Tag, &bytes)
            .map_err(|error| {
                Error::new(
                    ErrorCode::Internal,
                    format!("cannot hash the tag object: {error}"),
                )
            })?;
        record = GitTag {
            kind: GitTagKind::Annotated,
            tagger: Some(format!("{} <{}>", new.tagger.name, new.tagger.email)),
            time: Instant::from_unix_seconds(new.tagger.time.seconds),
            message: Some(tag.message.to_str_lossy().into_owned()),
            ..record
        };
        target = tag_id;
        object = Some((tag_id, tag));
    }
    let message = reflog_message(repo, id)?;
    let mut refs = RefTx::new(RefLayout::from_repo(repo)?, new.tagger.clone());
    refs.push(RefEdit::set(
        full,
        Target::Object(target),
        PreviousValue::MustNotExist,
        message,
    ));
    let refs = refs
        .prepare()
        .map_err(|error| exists_fix(repo, new.name, error))?;
    Ok(TagPlan {
        refs,
        object,
        record: Some(record),
    })
}

/// Plans the deletion of a tag.
///
/// # Errors
///
/// `E_GIT_REF` with problem `missing` when the tag does not exist, and the
/// errors of [`RefTx::prepare`].
pub fn plan_delete(
    repo: &gix::Repository,
    name: &str,
    committer: gix_actor::Signature,
) -> Result<TagPlan> {
    let full = tag_name(repo, name)?;
    let current = repo
        .try_find_reference(full.as_ref())
        .map_err(read_error)?
        .and_then(|reference| reference.target().try_id().map(ToOwned::to_owned));
    let Some(current) = current else {
        return Err(Error::new(
            ErrorCode::GitRef,
            format!("tag {name} does not exist; list the tags to check the name"),
        )
        .with_problem("missing")
        .with_found(name.to_owned())
        .with_next_step(format!("git.tag {}", quoted_repo(repo))));
    };
    let mut refs = RefTx::new(RefLayout::from_repo(repo)?, committer);
    refs.push(RefEdit::delete(
        full,
        PreviousValue::MustExistAndMatch(Target::Object(current)),
        "",
    ));
    Ok(TagPlan {
        refs: refs.prepare()?,
        object: None,
        record: None,
    })
}

/// Full reference name of the tag `name`.
fn tag_name(repo: &gix::Repository, name: &str) -> Result<FullName> {
    let invalid = || {
        Error::new(
            ErrorCode::GitRef,
            format!("{name} is not a valid tag name; use a name accepted by git check-ref-format"),
        )
        .with_problem("invalid_name")
        .with_found(name.to_owned())
        .with_next_step(format!("git.tag {}", quoted_repo(repo)))
    };
    if name.is_empty() || name.starts_with('-') {
        return Err(invalid());
    }
    FullName::try_from(format!("{TAGS_PREFIX}{name}")).map_err(|_| invalid())
}

/// Tag object for an annotated tag, signed when [`NewTag::signs`].
fn annotated(
    repo: &gix::Repository,
    new: &NewTag<'_>,
    target: ObjectId,
    message: &str,
) -> Result<gix_object::Tag> {
    let target_kind = repo.find_object(target).map_err(read_error)?.kind;
    let mut tag = gix_object::Tag {
        target,
        target_kind,
        name: new.name.into(),
        tagger: Some(new.tagger.clone()),
        message: clean_message(message).into(),
        signature: None,
    };
    if new.signs(repo) {
        let signing = new.signing.ok_or_else(|| {
            Error::new(
                ErrorCode::GitSign,
                "the tag is to be signed but no signing key is configured; configure user.signingKey or pass sign: false",
            )
            .with_problem("signer_unavailable")
        })?;
        sign_tag(&mut tag, signing.signer, signing.config)?;
    }
    Ok(tag)
}

/// Whether `tag.gpgSign` asks for signed annotated tags.
pub(crate) fn signs_tags(repo: &gix::Repository) -> bool {
    repo.config_snapshot()
        .boolean("tag.gpgSign")
        .unwrap_or(false)
}

fn tag_bytes(tag: &gix_object::Tag) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    tag.write_to(&mut bytes).map_err(|error| {
        Error::new(
            ErrorCode::Internal,
            format!("cannot encode the tag object: {error}"),
        )
    })?;
    Ok(bytes)
}

/// Message cleaned as `git tag -m` cleans it: lines starting with the comment
/// character `#` removed, trailing whitespace removed from every line, runs
/// of blank lines collapsed, leading and trailing blank lines removed, and a
/// final line feed added to a non-empty message.
pub(crate) fn clean_message(message: &str) -> String {
    let mut out = String::new();
    let mut blank_pending = false;
    for line in message.lines() {
        if line.starts_with(COMMENT_CHAR) {
            continue;
        }
        let line = line.trim_end();
        if line.is_empty() {
            blank_pending = !out.is_empty();
            continue;
        }
        if blank_pending {
            out.push('\n');
            blank_pending = false;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Reflog message of `git tag`, used when a reflog of the tag is written.
fn reflog_message(repo: &gix::Repository, id: ObjectId) -> Result<BString> {
    let object = repo.find_object(id).map_err(read_error)?;
    let short = id.to_hex_with_len(ABBREV_DIGITS).to_string();
    if object.kind != gix_object::Kind::Commit {
        return Ok(format!("tag: tagging {short} ({})", object.kind).into());
    }
    let commit = object.into_commit();
    let subject = commit
        .message()
        .map_err(read_error)?
        .summary()
        .to_str_lossy()
        .into_owned();
    let time = commit.time().map_err(read_error)?;
    Ok(format!("tag: tagging {short} ({subject}, {})", short_date(&time)).into())
}

/// `YYYY-MM-DD` of a time in its own offset.
fn short_date(time: &gix_date::Time) -> String {
    let local = time.seconds + i64::from(time.offset);
    let days = local.div_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Proleptic Gregorian date of a day number counted from 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn exists_fix(repo: &gix::Repository, name: &str, error: Error) -> Error {
    if error.problem.as_deref() == Some("exists") {
        return error.with_fix(format!(
            "git.tag {} delete: {}",
            quoted_repo(repo),
            serde_json::Value::String(name.to_owned())
        ));
    }
    error
}

fn read_error(error: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot read the repository: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::branch::test_repo::TestRepo;

    #[test]
    fn message_is_cleaned_like_git_tag() {
        assert_eq!(
            clean_message("  line one  \n\n \n\ntwo  "),
            "  line one\n\ntwo\n"
        );
        assert_eq!(clean_message("\n\n"), "");
        assert_eq!(clean_message("a"), "a\n");
        assert_eq!(
            clean_message("  first  \n# comment\n\n\nsecond\n\n"),
            "  first\n\nsecond\n"
        );
    }

    #[test]
    fn short_date_uses_the_offset_of_the_time() {
        let time = gix_date::Time {
            seconds: 1_700_000_000,
            offset: 0,
        };
        assert_eq!(short_date(&time), "2023-11-14");
        let east = gix_date::Time {
            seconds: 1_700_000_000,
            offset: 7200,
        };
        assert_eq!(short_date(&east), "2023-11-15");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn lightweight_tag_points_to_the_commit() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        let plan = plan_create(
            &repo.open(),
            NewTag {
                name: "v1",
                revision: "HEAD",
                message: None,
                tagger: repo.signature(),
                sign: None,
                signing: None,
            },
        )
        .unwrap();
        assert_eq!(plan.record().unwrap().kind, GitTagKind::Lightweight);
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        assert_eq!(repo.read("refs/tags/v1"), format!("{head}\n"));
        assert!(!repo.path("logs/refs/tags/v1").exists());
        let tags = list(&repo.open()).unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].target, head.to_string());
    }

    #[test]
    fn annotated_tag_object_matches_git_format() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        let plan = plan_create(
            &repo.open(),
            NewTag {
                name: "v2",
                revision: "HEAD",
                message: Some("release\n\n\nnotes  \n"),
                tagger: repo.signature(),
                sign: None,
                signing: None,
            },
        )
        .unwrap();
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        let id = ObjectId::from_hex(repo.read("refs/tags/v2").trim().as_bytes()).unwrap();
        let opened = repo.open();
        let object = opened.find_object(id).unwrap();
        let expected = format!(
            "object {head}\ntype commit\ntag v2\ntagger {}\n\nrelease\n\nnotes\n",
            repo.ident()
        );
        assert_eq!(object.data.as_bstr(), expected.as_str());
        let tags = list(&repo.open()).unwrap();
        assert_eq!(tags[0].kind, GitTagKind::Annotated);
        assert_eq!(tags[0].target, head.to_string());
        assert_eq!(tags[0].message.as_deref(), Some("release\n\nnotes\n"));
        assert!(tags[0].time.is_some());
    }

    #[test]
    fn existing_tag_and_invalid_names_are_rejected() {
        let repo = TestRepo::new();
        repo.commit("first");
        let create = |name: &str| {
            plan_create(
                &repo.open(),
                NewTag {
                    name,
                    revision: "HEAD",
                    message: None,
                    tagger: repo.signature(),
                    sign: None,
                    signing: None,
                },
            )
        };
        let plan = create("v1").unwrap();
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        let error = create("v1").unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("exists"));
        assert!(error.fix[0].contains("delete: \"v1\""));
        for bad in ["", "-x", "a..b", "a b"] {
            assert_eq!(
                create(bad).unwrap_err().problem.as_deref(),
                Some("invalid_name"),
                "{bad}"
            );
        }
        let error = plan_create(
            &repo.open(),
            NewTag {
                name: "v9",
                revision: "no-such-rev",
                message: None,
                tagger: repo.signature(),
                sign: None,
                signing: None,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::GitRevision);
    }

    #[test]
    fn signing_requires_a_signer_and_reports_its_failure() {
        use nurt_git_transport::sign::{CommitSignature, Signature, VerifyConfig};

        struct Refusing;
        impl Signer for Refusing {
            fn sign(&self, payload: &[u8], _config: &SigningConfig) -> Result<Signature> {
                assert!(payload.starts_with(b"object "));
                Err(Error::new(ErrorCode::GitSign, "no key").with_problem("no_key"))
            }

            fn verify(
                &self,
                _payload: &[u8],
                _signature: &[u8],
                _config: &VerifyConfig,
            ) -> Result<CommitSignature> {
                Err(Error::new(ErrorCode::GitSign, "no key"))
            }
        }
        let repo = TestRepo::new();
        repo.commit("first");
        repo.append_config("[tag]\n\tgpgSign = true\n");
        let config = SigningConfig::new(nurt_git_transport::sign::SignatureFormat::Openpgp, 0);
        fn new<'a>(repo: &TestRepo, signing: Option<TagSigning<'a>>) -> NewTag<'a> {
            NewTag {
                name: "s1",
                revision: "HEAD",
                message: Some("msg"),
                tagger: repo.signature(),
                sign: None,
                signing,
            }
        }
        let new = |signing| new(&repo, signing);
        let error = plan_create(&repo.open(), new(None)).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("signer_unavailable"));
        let signing = TagSigning {
            signer: &Refusing,
            config: &config,
        };
        let error = plan_create(&repo.open(), new(Some(signing))).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("no_key"));
        let lightweight = NewTag {
            message: None,
            ..new(None)
        };
        assert!(plan_create(&repo.open(), lightweight).is_ok());
        let unsigned = NewTag {
            sign: Some(false),
            ..new(None)
        };
        assert!(plan_create(&repo.open(), unsigned).is_ok());
        let without_message = NewTag {
            message: None,
            sign: Some(true),
            ..new(None)
        };
        let error = plan_create(&repo.open(), without_message).unwrap_err();
        assert_eq!(error.code, ErrorCode::MissingParam);
        assert_eq!(error.problem.as_deref(), Some("missing_message"));
    }

    #[test]
    fn explicit_sign_overrides_the_configuration() {
        let repo = TestRepo::new();
        repo.commit("first");
        let new = |sign| NewTag {
            name: "s",
            revision: "HEAD",
            message: Some("m"),
            tagger: repo.signature(),
            sign,
            signing: None,
        };
        let opened = repo.open();
        assert!(!new(None).signs(&opened));
        assert!(new(Some(true)).signs(&opened));
        let error = plan_create(&opened, new(Some(true))).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("signer_unavailable"));
        repo.append_config("[tag]\n\tgpgSign = true\n");
        let opened = repo.open();
        assert!(new(None).signs(&opened));
        assert!(!new(Some(false)).signs(&opened));
    }

    #[test]
    fn deleting_a_packed_tag_rewrites_packed_refs_and_undo_restores_it() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        repo.write(
            "packed-refs",
            &format!("# pack-refs with: peeled fully-peeled sorted \n{head} refs/tags/old\n"),
        );
        let plan = plan_delete(&repo.open(), "old", repo.signature()).unwrap();
        assert_eq!(plan.outcomes().len(), 1);
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        assert_eq!(
            repo.read("packed-refs"),
            "# pack-refs with: peeled fully-peeled sorted \n"
        );
        let error = plan_delete(&repo.open(), "none", repo.signature()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("missing"));
    }

    #[test]
    fn reflog_message_names_the_commit_like_git() {
        let repo = TestRepo::new();
        let head = repo.commit("subject line\n\nbody");
        repo.append_config("[core]\n\tlogAllRefUpdates = always\n");
        let plan = plan_create(
            &repo.open(),
            NewTag {
                name: "logged",
                revision: "HEAD",
                message: None,
                tagger: repo.signature(),
                sign: None,
                signing: None,
            },
        )
        .unwrap();
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        let log = repo.read("logs/refs/tags/logged");
        let short = &head.to_string()[..ABBREV_DIGITS];
        assert!(
            log.ends_with(&format!(
                "\ttag: tagging {short} (subject line, 2026-01-01)\n"
            )),
            "{log}"
        );
    }
}
