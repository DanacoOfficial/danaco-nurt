// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion of repository objects into records of the language.
//!
//! Identifiers are full hexadecimal object names; abbreviated identifiers
//! follow `core.abbrev` and are extended until they are unique. Times are
//! instants in UTC; the offset recorded by `git` is not part of a record.

use bstr::{BStr, ByteSlice};
use indexmap::IndexMap;
use nurt_contracts::error::Error;
use nurt_contracts::record::{
    Branch, Commit, GitConflictKind, GitFileState, GitStatus, GitStatusKind, GitTag, GitTagKind,
};
use nurt_contracts::value::{Instant, Path as RecordPath};

use super::{GitRepo, error};

/// Prefix of local branch references.
pub const HEADS: &str = "refs/heads/";
/// Prefix of remote-tracking branch references.
pub const REMOTES: &str = "refs/remotes/";
/// Prefix of tag references.
pub const TAGS: &str = "refs/tags/";

/// Converts a time of the repository library into an instant.
#[must_use]
pub fn instant(time: gix::date::Time) -> Instant {
    Instant::from_unix_seconds(time.seconds).unwrap_or(Instant::UNIX_EPOCH)
}

/// Text of a name or address of a signature, trimmed, with invalid UTF-8
/// replaced.
#[must_use]
pub fn text(value: &BStr) -> String {
    value.trim().to_str_lossy().into_owned()
}

/// Name, e-mail address and time of a signature.
#[must_use]
pub fn signature(signature: &gix::actor::SignatureRef<'_>) -> (String, String, Instant) {
    let time = signature.time().map(instant).unwrap_or(Instant::UNIX_EPOCH);
    (text(signature.name), text(signature.email), time)
}

/// Record `commit` of a commit, without `refs`, `stats`, `files` and
/// `signature`, which the caller adds when requested.
///
/// # Errors
///
/// `E_IO` when the commit object cannot be decoded.
pub fn commit(repo: &GitRepo, commit: &gix::Commit<'_>) -> Result<Commit, Error> {
    let decode = |source| error::from_gix("decode a commit", &source);
    let (author, author_email, time) = signature(&commit.author().map_err(decode)?);
    let (committer, committer_email, commit_time) = signature(&commit.committer().map_err(decode)?);
    let raw = commit.message_raw().map_err(decode)?;
    let message = raw.to_str_lossy().into_owned();
    let parsed = gix::objs::commit::MessageRef::from_bytes(raw);
    let trailers = parsed
        .body()
        .map(|body| trailers(&body))
        .filter(|map| !map.is_empty());
    Ok(Commit {
        id: commit.id.to_string(),
        short_id: repo.short_id(&commit.id),
        author,
        time,
        subject: parsed.summary().to_str_lossy().into_owned(),
        message,
        parents: commit.parent_ids().map(|id| id.to_string()).collect(),
        author_email: Some(author_email),
        committer: Some(committer),
        committer_email: Some(committer_email),
        commit_time: Some(commit_time),
        tree: commit.tree_id().ok().map(|id| id.to_string()),
        trailers,
        ..Commit::default()
    })
}

/// Trailers of a message body; repeated tokens keep every value, one per
/// line, in message order.
#[must_use]
pub fn trailers(body: &gix::objs::commit::message::BodyRef<'_>) -> IndexMap<String, String> {
    let mut map: IndexMap<String, String> = IndexMap::new();
    for trailer in body.trailers() {
        let token = trailer.token.to_str_lossy().into_owned();
        let value = trailer.value.to_str_lossy().into_owned();
        map.entry(token)
            .and_modify(|existing| {
                existing.push('\n');
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    map
}

/// Record `commit` of the commit `id`.
///
/// # Errors
///
/// `E_GIT_REVISION` when `id` is not a commit; `E_IO` when it cannot be
/// read.
pub fn commit_by_id(repo: &GitRepo, id: gix::ObjectId) -> Result<Commit, Error> {
    let object = repo
        .gix()
        .find_commit(id)
        .map_err(|source| error::from_gix("read a commit", &source))?;
    commit(repo, &object)
}

/// Record `branch` of the local or remote-tracking branch `name` (a full
/// reference name). `current` marks the branch checked out in this working
/// tree; upstream, distance and the last commit are filled in.
///
/// # Errors
///
/// `E_GIT_REF` when the reference does not exist; `E_IO` when it or its
/// commit cannot be read.
pub fn branch(repo: &GitRepo, name: &str, current: bool) -> Result<Branch, Error> {
    let gix = repo.gix();
    let mut reference = gix
        .find_reference(name)
        .map_err(|source| error::from_gix("read a branch", &source))?;
    let id = reference
        .peel_to_id()
        .map_err(|source| error::from_gix("read a branch", &source))?
        .detach();
    let remote = name.starts_with(REMOTES);
    let short = short_ref_name(name);
    let mut record = Branch {
        name: short.to_owned(),
        current,
        commit: id.to_string(),
        remote,
        ..Branch::default()
    };
    if let Ok(object) = gix.find_commit(id) {
        let message = object.message_raw_sloppy();
        record.subject = Some(
            gix::objs::commit::MessageRef::from_bytes(message)
                .summary()
                .to_str_lossy()
                .into_owned(),
        );
        record.time = object.time().ok().map(instant);
    }
    if !remote {
        fill_upstream(repo, short, id, &mut record);
    }
    Ok(record)
}

fn fill_upstream(repo: &GitRepo, branch: &str, id: gix::ObjectId, record: &mut Branch) {
    let Some(tracking) = upstream_tracking_ref(repo, branch) else {
        return;
    };
    record.upstream = Some(short_ref_name(&tracking).to_owned());
    match repo.gix().find_reference(tracking.as_str()) {
        Ok(mut reference) => {
            if let Ok(upstream) = reference.peel_to_id()
                && let Ok((ahead, behind)) = ahead_behind(repo, id, upstream.detach())
            {
                record.ahead = Some(ahead);
                record.behind = Some(behind);
            }
        }
        Err(_) => record.upstream_gone = Some(true),
    }
}

/// Full name of the remote-tracking reference of the upstream of the local
/// branch `branch`, from `branch.<name>.remote` and `branch.<name>.merge`
/// mapped through the fetch refspecs of the remote; a local upstream
/// (`remote = .`) is the merge reference itself.
#[must_use]
pub fn upstream_tracking_ref(repo: &GitRepo, branch: &str) -> Option<String> {
    let remote = repo.config_string(&format!("branch.{branch}.remote"))?;
    let merge = repo.config_string(&format!("branch.{branch}.merge"))?;
    if remote == "." {
        return Some(merge);
    }
    let merge_short = merge.strip_prefix(HEADS)?;
    let refspecs = repo
        .config()
        .strings(format!("remote.{remote}.fetch").as_str())
        .unwrap_or_default();
    for spec in refspecs {
        let spec = spec.to_str_lossy();
        let spec = spec.trim_start_matches('+');
        let Some((source, destination)) = spec.split_once(':') else {
            continue;
        };
        if let Some(mapped) = map_refspec(source, destination, &merge) {
            return Some(mapped);
        }
    }
    Some(format!("{REMOTES}{remote}/{merge_short}"))
}

fn map_refspec(source: &str, destination: &str, name: &str) -> Option<String> {
    match (source.split_once('*'), destination.split_once('*')) {
        (Some((src_pre, src_post)), Some((dst_pre, dst_post))) => {
            let middle = name.strip_prefix(src_pre)?.strip_suffix(src_post)?;
            Some(format!("{dst_pre}{middle}{dst_post}"))
        }
        (None, None) if source == name => Some(destination.to_owned()),
        _ => None,
    }
}

/// Numbers of commits reachable from `local` but not from `upstream`
/// (ahead) and from `upstream` but not from `local` (behind).
///
/// # Errors
///
/// `E_IO` when the history cannot be walked.
pub fn ahead_behind(
    repo: &GitRepo,
    local: gix::ObjectId,
    upstream: gix::ObjectId,
) -> Result<(u64, u64), Error> {
    let count = |tip: gix::ObjectId, hidden: gix::ObjectId| -> Result<u64, Error> {
        let walk = repo
            .gix()
            .rev_walk([tip])
            .with_hidden([hidden])
            .all()
            .map_err(|source| error::from_gix("walk the history", &source))?;
        let mut total = 0_u64;
        for item in walk {
            item.map_err(|source| error::from_gix("walk the history", &source))?;
            total += 1;
        }
        Ok(total)
    };
    Ok((count(local, upstream)?, count(upstream, local)?))
}

/// Record `git_tag` of the tag `name` (a full reference name).
///
/// # Errors
///
/// `E_GIT_REF` when the reference does not exist; `E_IO` when it cannot be
/// read.
pub fn tag(repo: &GitRepo, name: &str) -> Result<GitTag, Error> {
    let gix = repo.gix();
    let reference = gix
        .find_reference(name)
        .map_err(|source| error::from_gix("read a tag", &source))?;
    let Some(id) = reference.target().try_id().map(ToOwned::to_owned) else {
        return Err(error::from_gix(
            "read a tag",
            &gix::Error::from_error(std::io::Error::other("symbolic tag reference")),
        ));
    };
    let short = short_ref_name(name).to_owned();
    let object = gix
        .find_object(id)
        .map_err(|source| error::from_gix("read a tag", &source))?;
    if object.kind != gix::object::Kind::Tag {
        return Ok(GitTag {
            name: short,
            target: id.to_string(),
            kind: GitTagKind::Lightweight,
            ..GitTag::default()
        });
    }
    let tag = object.into_tag();
    let decoded = tag
        .decode()
        .map_err(|source| error::from_gix("decode a tag", &source))?;
    let (tagger, time) = match decoded.tagger() {
        Ok(Some(signature)) => {
            let (name, email, time) = self::signature(&signature);
            (Some(format!("{name} <{email}>")), Some(time))
        }
        _ => (None, None),
    };
    Ok(GitTag {
        name: short,
        target: decoded.target().to_string(),
        kind: GitTagKind::Annotated,
        tagger,
        time,
        message: Some(decoded.message.to_str_lossy().into_owned()),
        ..GitTag::default()
    })
}

/// Name of a reference without `refs/heads/`, `refs/remotes/` or
/// `refs/tags/`.
#[must_use]
pub fn short_ref_name(name: &str) -> &str {
    [HEADS, REMOTES, TAGS]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .unwrap_or(name)
}

/// Record `git_status` with the required fields; optional fields are left
/// empty for the caller.
#[must_use]
pub fn status(
    repo: &GitRepo,
    path: &str,
    index: GitFileState,
    worktree: GitFileState,
    kind: GitStatusKind,
) -> GitStatus {
    GitStatus {
        path: path.to_owned(),
        index,
        worktree,
        kind,
        repo: RecordPath::new(repo.root().to_string_lossy().replace('\\', "/")),
        ..GitStatus::default()
    }
}

/// Kind of a conflict from the stages present in the index for one path:
/// bit 0 for stage 1 (base), bit 1 for stage 2 (ours), bit 2 for stage 3
/// (theirs), as `git status` names the seven combinations.
#[must_use]
pub fn conflict_kind(stages: u8) -> Option<GitConflictKind> {
    const BASE: u8 = 1;
    const OURS: u8 = 2;
    const THEIRS: u8 = 4;
    match stages {
        BASE => Some(GitConflictKind::BothDeleted),
        OURS => Some(GitConflictKind::AddedByUs),
        THEIRS => Some(GitConflictKind::AddedByThem),
        s if s == BASE | OURS => Some(GitConflictKind::DeletedByThem),
        s if s == BASE | THEIRS => Some(GitConflictKind::DeletedByUs),
        s if s == OURS | THEIRS => Some(GitConflictKind::BothAdded),
        s if s == BASE | OURS | THEIRS => Some(GitConflictKind::BothModified),
        _ => None,
    }
}

/// Octal mode of an index or tree entry, as `git` prints it (`100644`).
#[must_use]
pub fn mode_text(mode: gix::index::entry::Mode) -> String {
    format!("{:06o}", mode.bits())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{RepoBuilder, test_ctx};

    #[test]
    fn commit_record_has_identity_message_and_trailers() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "one\n")
            .commit("Add a\n\nBody line.\n\nSigned-off-by: Ada <ada@example.com>\n")
            .build();
        let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
        let head = repo.head_id().unwrap().unwrap();
        let record = commit_by_id(&repo, head).unwrap();
        assert_eq!(record.id, head.to_string());
        assert_eq!(record.short_id.len(), 7);
        assert_eq!(record.subject, "Add a");
        assert_eq!(record.author, RepoBuilder::AUTHOR_NAME);
        assert_eq!(
            record.author_email.as_deref(),
            Some(RepoBuilder::AUTHOR_EMAIL)
        );
        assert_eq!(
            record.time,
            Instant::from_unix_seconds(RepoBuilder::EPOCH).unwrap()
        );
        assert!(record.parents.is_empty());
        let trailers = record.trailers.unwrap();
        assert_eq!(trailers["Signed-off-by"], "Ada <ada@example.com>");
    }

    #[test]
    fn branch_record_reports_upstream_distance() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .commit("one")
            .reference("refs/remotes/origin/main", "HEAD")
            .file("a.txt", "2\n")
            .commit("two")
            .config("branch.main.remote", "origin")
            .config("branch.main.merge", "refs/heads/main")
            .config("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
            .build();
        let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
        let record = branch(&repo, "refs/heads/main", true).unwrap();
        assert_eq!(record.name, "main");
        assert!(record.current);
        assert_eq!(record.upstream.as_deref(), Some("origin/main"));
        assert_eq!((record.ahead, record.behind), (Some(1), Some(0)));
        assert_eq!(record.subject.as_deref(), Some("two"));
    }

    #[test]
    fn missing_upstream_is_reported_as_gone() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .commit("one")
            .config("branch.main.remote", "origin")
            .config("branch.main.merge", "refs/heads/main")
            .build();
        let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
        let record = branch(&repo, "refs/heads/main", true).unwrap();
        assert_eq!(record.upstream_gone, Some(true));
    }

    #[test]
    fn lightweight_and_annotated_tags_are_distinguished() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .commit("one")
            .reference("refs/tags/light", "HEAD")
            .annotated_tag("v1", "HEAD", "release 1\n")
            .build();
        let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
        let light = tag(&repo, "refs/tags/light").unwrap();
        assert_eq!(light.kind, GitTagKind::Lightweight);
        let annotated = tag(&repo, "refs/tags/v1").unwrap();
        assert_eq!(annotated.kind, GitTagKind::Annotated);
        assert_eq!(annotated.message.as_deref(), Some("release 1\n"));
        assert_eq!(
            annotated.target,
            repo.head_id().unwrap().unwrap().to_string()
        );
    }

    #[test]
    fn conflict_kinds_follow_git_status_codes() {
        assert_eq!(conflict_kind(0b111), Some(GitConflictKind::BothModified));
        assert_eq!(conflict_kind(0b110), Some(GitConflictKind::BothAdded));
        assert_eq!(conflict_kind(0b001), Some(GitConflictKind::BothDeleted));
        assert_eq!(conflict_kind(0b011), Some(GitConflictKind::DeletedByThem));
        assert_eq!(conflict_kind(0b101), Some(GitConflictKind::DeletedByUs));
        assert_eq!(conflict_kind(0b010), Some(GitConflictKind::AddedByUs));
        assert_eq!(conflict_kind(0b100), Some(GitConflictKind::AddedByThem));
        assert_eq!(conflict_kind(0), None);
    }

    #[test]
    fn refspec_mapping_handles_globs_and_exact_names() {
        assert_eq!(
            map_refspec("refs/heads/*", "refs/remotes/o/*", "refs/heads/x/y"),
            Some("refs/remotes/o/x/y".to_owned())
        );
        assert_eq!(
            map_refspec("refs/heads/main", "refs/remotes/o/trunk", "refs/heads/main"),
            Some("refs/remotes/o/trunk".to_owned())
        );
        assert_eq!(
            map_refspec("refs/tags/*", "refs/tags/*", "refs/heads/main"),
            None
        );
    }

    #[test]
    fn short_names_drop_known_prefixes() {
        assert_eq!(short_ref_name("refs/heads/a/b"), "a/b");
        assert_eq!(short_ref_name("refs/remotes/o/a"), "o/a");
        assert_eq!(short_ref_name("refs/notes/x"), "refs/notes/x");
    }

    #[test]
    fn status_record_uses_the_repository_root() {
        let fixture = RepoBuilder::new().build();
        let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
        let record = status(
            &repo,
            "a.txt",
            GitFileState::Added,
            GitFileState::Unmodified,
            GitStatusKind::Tracked,
        );
        assert_eq!(
            record.repo.as_str(),
            fixture.path().to_string_lossy().replace('\\', "/")
        );
    }

    #[test]
    fn mode_text_is_six_octal_digits() {
        assert_eq!(mode_text(gix::index::entry::Mode::FILE), "100644");
        assert_eq!(mode_text(gix::index::entry::Mode::DIR), "040000");
    }
}
