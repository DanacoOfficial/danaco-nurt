// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::clone::layout
// Responsibility:   Files of a new clone that `git clone` writes itself: the configuration.
// Dependencies:     bstr, gix, nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Files of a new clone that `git clone` writes itself: the configuration,
//! `packed-refs`, `HEAD`, the branch and its reflogs.
//!
//! The protocol library receives the objects; it does not write references
//! or the configuration. The content written here follows `git`: the
//! `core` section of the repository first, then `remote "origin"` and
//! `branch "<name>"`; remote-tracking references and tags in `packed-refs`
//! with the header `# pack-refs with: peeled fully-peeled sorted `; the
//! checked out branch and `refs/remotes/origin/HEAD` as loose references;
//! and the reflog message `clone: from <address>`.

use std::collections::HashSet;
use std::path::Path;

use bstr::ByteSlice;
use gix::ObjectId;
use gix::actor::Signature;
use gix::protocol::fetch::refmap::SpecIndex;
use gix::protocol::fetch::{RefMap, Tags};
use gix::protocol::handshake::Ref;
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::REMOTE;
use crate::fetch::{error_io, head, ref_file, update};
use crate::repo::records::HEADS;

/// Name of the first branch of a repository whose remote names none.
const DEFAULT_BRANCH: &str = "master";

/// Header of `packed-refs`.
const PACKED_HEADER: &str = "# pack-refs with: peeled fully-peeled sorted \n";

/// Namespace of the remote-tracking references of `origin`.
const TRACKING: &str = "refs/remotes/origin/";

/// Reflog message of the references created by a clone.
const LOG_MESSAGE: &str = "clone: from ";

/// A reference created by the clone.
#[derive(Debug, Clone)]
pub(super) struct Packed {
    /// Full local name.
    pub(super) name: String,
    /// Full name on the remote.
    pub(super) remote_name: String,
    /// Object the reference points to.
    pub(super) id: ObjectId,
    /// Object after peeling tags.
    pub(super) peeled: ObjectId,
}

/// What the remote `HEAD` points at.
#[derive(Debug, Clone, Default)]
pub(super) struct RemoteHead {
    target: Option<String>,
    id: Option<ObjectId>,
}

impl RemoteHead {
    /// Reads `HEAD` from the advertised references. A `HEAD` without a
    /// symbolic target is matched to a branch with the same commit, the
    /// default branch name first, as `git` does.
    pub(super) fn of(refs: &[Ref]) -> Self {
        for reference in refs {
            match reference {
                Ref::Symbolic {
                    full_ref_name,
                    target,
                    object,
                    ..
                } if full_ref_name == "HEAD" => {
                    return Self {
                        target: Some(target.to_str_lossy().into_owned()),
                        id: Some(*object),
                    };
                }
                Ref::Unborn {
                    full_ref_name,
                    target,
                } if full_ref_name == "HEAD" => {
                    return Self {
                        target: Some(target.to_str_lossy().into_owned()),
                        id: None,
                    };
                }
                Ref::Direct {
                    full_ref_name,
                    object,
                } if full_ref_name == "HEAD" => {
                    return Self {
                        target: guess(refs, *object),
                        id: Some(*object),
                    };
                }
                _ => {}
            }
        }
        Self::default()
    }

    /// Short name of the branch `HEAD` points at.
    pub(super) fn branch(&self) -> Option<String> {
        self.target
            .as_deref()
            .and_then(|name| name.strip_prefix(HEADS))
            .map(str::to_owned)
    }
}

/// Branch of the remote that has the commit `id`.
fn guess(refs: &[Ref], id: ObjectId) -> Option<String> {
    let default = format!("{HEADS}{DEFAULT_BRANCH}");
    let heads: Vec<String> = refs
        .iter()
        .filter_map(update::advertised)
        .filter(|reference| reference.id == id && reference.name.starts_with(HEADS))
        .map(|reference| reference.name)
        .collect();
    if heads.contains(&default) {
        return Some(default);
    }
    heads.into_iter().next()
}

/// Where the local `HEAD` points after the clone.
#[derive(Debug, Clone)]
pub(super) enum HeadTarget {
    /// A branch; unborn when `commit` is `None`.
    Branch {
        /// Short name of the branch.
        name: String,
        /// Commit of the branch.
        commit: Option<ObjectId>,
        /// Branch of the remote `HEAD` and its commit, when it is stored.
        origin_head: Option<(String, ObjectId)>,
    },
    /// A commit that no branch names.
    Detached(ObjectId),
}

impl HeadTarget {
    /// Short name of the branch, `None` for a detached `HEAD`.
    pub(super) fn branch(&self) -> Option<&str> {
        match self {
            Self::Branch { name, .. } => Some(name),
            Self::Detached(_) => None,
        }
    }

    /// Commit `HEAD` resolves to.
    pub(super) fn commit(&self) -> Option<ObjectId> {
        match self {
            Self::Branch { commit, .. } => *commit,
            Self::Detached(commit) => Some(*commit),
        }
    }
}

/// Refspec that selects the references a clone stores.
pub(super) fn fetch_spec(bare: bool, branch: Option<&str>) -> String {
    let (source, destination) = match (bare, branch) {
        (false, Some(name)) => (name.to_owned(), format!("remotes/{REMOTE}/{name}")),
        (false, None) => ("*".to_owned(), format!("remotes/{REMOTE}/*")),
        (true, Some(name)) => (name.to_owned(), format!("heads/{name}")),
        (true, None) => ("*".to_owned(), "heads/*".to_owned()),
    };
    format!("+refs/heads/{source}:refs/{destination}")
}

/// References to create from the mapping of a fetch. A tag is stored when
/// all tags are wanted or when its object arrived with the pack.
pub(super) fn collect(repo: &gix::Repository, ref_map: &RefMap, tags: Tags) -> Vec<Packed> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for mapping in &ref_map.mappings {
        let Some(local) = &mapping.local else {
            continue;
        };
        let Some(remote) = update::remote_ref(&mapping.remote) else {
            continue;
        };
        if matches!(mapping.spec_index, SpecIndex::Implicit(_)) {
            let wanted = match tags {
                Tags::All => true,
                Tags::None => false,
                Tags::Included => repo.has_object(remote.peeled) && repo.has_object(remote.id),
            };
            if !wanted {
                continue;
            }
        }
        let name = local.to_str_lossy().into_owned();
        if seen.insert(name.clone()) {
            out.push(Packed {
                name,
                remote_name: remote.name,
                id: remote.id,
                peeled: remote.peeled,
            });
        }
    }
    out.sort_by(|left, right| left.name.cmp(&right.name));
    out
}

/// `HEAD` of the clone: the chosen branch, the default branch of an empty
/// remote, or the commit of a detached remote `HEAD`.
pub(super) fn head_target(
    refs: &[Packed],
    remote_head: &RemoteHead,
    branch: Option<&str>,
    bare: bool,
) -> HeadTarget {
    let find = |name: &str| refs.iter().find(|reference| reference.name == name);
    let local = |name: &str| {
        let local_name = if bare {
            format!("{HEADS}{name}")
        } else {
            format!("{TRACKING}{name}")
        };
        find(&local_name).map(|reference| reference.id)
    };
    let name = match branch {
        Some(name) => name.to_owned(),
        None => match (&remote_head.id, refs.is_empty()) {
            (Some(id), _) => return HeadTarget::Detached(*id),
            (None, false) => return HeadTarget::Detached(refs[0].peeled),
            (None, true) => DEFAULT_BRANCH.to_owned(),
        },
    };
    let origin_head = remote_head.branch().and_then(|remote_branch| {
        let id = find(&format!("{TRACKING}{remote_branch}"))?.id;
        (!bare).then_some((remote_branch, id))
    });
    HeadTarget::Branch {
        commit: local(&name),
        name,
        origin_head,
    }
}

/// Address stored in `remote.origin.url`: without a password, absolute for
/// a local path.
///
/// # Errors
///
/// `E_INVALID_VALUE` for an empty address or a relative local path.
pub(super) fn stored_url(url: &str) -> Result<String> {
    if url.is_empty() {
        return Err(invalid_url(
            url,
            "the address is empty; pass the address of a repository",
        ));
    }
    if let Some(scheme_end) = url.find("://") {
        let rest = &url[scheme_end + 3..];
        let host_end = rest.find('/').unwrap_or(rest.len());
        let stored = match rest[..host_end].rfind('@') {
            Some(at) => {
                let user = rest[..at].split(':').next().unwrap_or_default();
                let user_info = if user.is_empty() {
                    String::new()
                } else {
                    format!("{user}@")
                };
                format!("{}{user_info}{}", &url[..scheme_end + 3], &rest[at + 1..])
            }
            None => url.to_owned(),
        };
        return Ok(stored);
    }
    if Path::new(url).is_absolute() {
        // Git for Windows stores a local path with `/` separators.
        let url = if cfg!(windows) {
            url.replace('\\', "/")
        } else {
            url.to_owned()
        };
        let trimmed = url.trim_end_matches('/');
        let kept = trimmed.is_empty() || trimmed.ends_with(':');
        return Ok(if kept { url.as_str() } else { trimmed }.to_owned());
    }
    match url.find(':') {
        Some(colon) if colon > 1 && !url[..colon].contains('/') => Ok(url.to_owned()),
        _ => Err(invalid_url(
            url,
            "a local repository needs an absolute path; pass the full path or a file:// address",
        )),
    }
}

fn invalid_url(url: &str, message: &str) -> Error {
    Error::new(ErrorCode::InvalidValue, message)
        .with_problem("invalid_url")
        .with_found(head::anonymize(url))
}

/// Writes `bytes` to `path`, creating missing parent directories.
pub(super) fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|failure| error_io(parent, &failure.to_string()))?;
    }
    std::fs::write(path, bytes).map_err(|failure| error_io(path, &failure.to_string()))
}

/// Writer of the repository files of a clone.
pub(super) struct Layout<'a> {
    git_dir: &'a Path,
    bare: bool,
    url: String,
    core: Option<String>,
    committer: Option<Signature>,
    links: bool,
}

impl<'a> Layout<'a> {
    /// Layout of the repository directory `git_dir`; `url` is the stored
    /// address.
    pub(super) fn new(git_dir: &'a Path, bare: bool, url: &str) -> Self {
        Self {
            git_dir,
            bare,
            url: url.to_owned(),
            core: None,
            committer: None,
            links: true,
        }
    }

    /// Whether the configuration outside the repository enables symbolic
    /// links (`core.symlinks`); without it, `git` for Windows records
    /// `symlinks = false` in a new repository and checks links out as files.
    pub(super) fn set_links(&mut self, enabled: bool) {
        self.links = enabled;
    }

    /// Identity written to the reflogs.
    pub(super) fn set_committer(&mut self, committer: Signature) {
        self.committer = Some(committer);
    }

    /// Writes `config`: the `core` section as created for the repository,
    /// the remote with its address and, when given, its refspec, and the
    /// section of the branch that follows the remote.
    pub(super) fn write_config(&mut self, fetch: Option<&str>, branch: Option<&str>) -> Result<()> {
        let path = self.git_dir.join("config");
        let core = match &self.core {
            Some(core) => core.clone(),
            None => {
                let text = std::fs::read_to_string(&path)
                    .map_err(|failure| error_io(&path, &failure.to_string()))
                    .map(|text| as_git_writes(&text, self.links))?;
                self.core = Some(text.clone());
                text
            }
        };
        let mut text = core;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!(
            "[remote \"{REMOTE}\"]\n\turl = {}\n",
            value(&self.url)
        ));
        if let Some(spec) = fetch {
            text.push_str(&format!("\tfetch = {}\n", value(spec)));
        }
        if let Some(name) = branch {
            text.push_str(&format!(
                "[branch \"{}\"]\n\tremote = {REMOTE}\n\tmerge = {}\n",
                subsection(name),
                value(&format!("{HEADS}{name}"))
            ));
        }
        write_file(&path, text.as_bytes())
    }

    /// Writes `packed-refs`, `HEAD`, the checked out branch,
    /// `refs/remotes/origin/HEAD` and the reflogs.
    ///
    /// # Errors
    ///
    /// `E_GIT_REF` when the remote names a reference or the branch of `HEAD`
    /// in a form that is not a valid reference name, such as one with a `..`
    /// component; no path is written then.
    pub(super) fn write_refs(&self, refs: &[Packed], head: &HeadTarget) -> Result<()> {
        for reference in refs {
            verify_name(&reference.name)?;
        }
        if let HeadTarget::Branch { name, .. } = head {
            verify_name(&format!("{HEADS}{name}"))?;
        }
        if !refs.is_empty() {
            write_file(
                &self.git_dir.join("packed-refs"),
                packed_refs(refs).as_bytes(),
            )?;
        }
        let logged = !self.bare;
        match head {
            HeadTarget::Branch {
                name,
                commit,
                origin_head,
            } => {
                let full = format!("{HEADS}{name}");
                write_file(
                    &self.git_dir.join("HEAD"),
                    format!("ref: {full}\n").as_bytes(),
                )?;
                let Some(commit) = commit.filter(|_| logged) else {
                    return Ok(());
                };
                write_file(
                    &ref_file(self.git_dir, &full),
                    format!("{commit}\n").as_bytes(),
                )?;
                self.log("HEAD", commit)?;
                self.log(&full, commit)?;
                if let Some((remote_branch, id)) = origin_head {
                    let link = format!("{TRACKING}HEAD");
                    let target = format!("ref: {TRACKING}{remote_branch}\n");
                    write_file(&ref_file(self.git_dir, &link), target.as_bytes())?;
                    self.log(&link, *id)?;
                }
            }
            HeadTarget::Detached(commit) => {
                write_file(&self.git_dir.join("HEAD"), format!("{commit}\n").as_bytes())?;
                if logged {
                    self.log("HEAD", *commit)?;
                }
            }
        }
        Ok(())
    }

    fn log(&self, name: &str, new: ObjectId) -> Result<()> {
        let Some(committer) = &self.committer else {
            return Ok(());
        };
        let path = name
            .split('/')
            .fold(self.git_dir.join("logs"), |path, part| path.join(part));
        let mut line = format!("{} {new} ", ObjectId::null(new.kind())).into_bytes();
        committer
            .write_to(&mut line)
            .map_err(|failure| error_io(&path, &failure.to_string()))?;
        line.extend_from_slice(
            format!("\t{LOG_MESSAGE}{}\n", head::anonymize(&self.url)).as_bytes(),
        );
        write_file(&path, &line)
    }
}

/// Fails when `name` is not a valid full reference name, before it becomes
/// part of a path below the repository directory.
fn verify_name(name: &str) -> Result<()> {
    if gix::refs::FullName::try_from(name).is_ok() {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::GitRef,
        format!("the remote names the invalid reference `{name}`; clone another repository"),
    )
    .with_problem("invalid_name")
    .with_found(name.to_owned()))
}

/// `core` settings that `git` leaves out while they have their default
/// value.
const DEFAULT_SETTINGS: [&str; 3] = [
    "ignorecase = false",
    "precomposeunicode = false",
    "logallrefupdates = false",
];

/// The `core` section as `git` writes it into a new repository: `\n` line
/// ends on every system, without the settings of [`DEFAULT_SETTINGS`] and,
/// on Windows, with `symlinks = false` unless `links` is set.
fn as_git_writes(text: &str, links: bool) -> String {
    let mut text = without_defaults(&text.replace("\r\n", "\n"));
    let recorded = text
        .lines()
        .any(|line| line.trim_start().starts_with("symlinks"));
    if cfg!(windows) && !links && !recorded {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("\tsymlinks = false\n");
    }
    text
}

/// The configuration without the settings of [`DEFAULT_SETTINGS`].
fn without_defaults(text: &str) -> String {
    text.split_inclusive('\n')
        .filter(|line| !DEFAULT_SETTINGS.contains(&line.trim()))
        .collect()
}

/// Content of `packed-refs`: sorted references, each annotated tag followed
/// by the line of the object it peels to.
fn packed_refs(refs: &[Packed]) -> String {
    let mut text = String::from(PACKED_HEADER);
    for reference in refs {
        text.push_str(&format!("{} {}\n", reference.id, reference.name));
        if reference.peeled != reference.id {
            text.push_str(&format!("^{}\n", reference.peeled));
        }
    }
    text
}

/// Configuration value, quoted when it holds characters with a meaning in
/// the file format.
fn value(text: &str) -> String {
    let needs_quotes =
        text.starts_with(' ') || text.ends_with(' ') || text.contains(['"', '\\', ';', '#', '\n']);
    if needs_quotes {
        format!("\"{}\"", subsection(text).replace('\n', "\\n"))
    } else {
        text.to_owned()
    }
}

/// `text` with the characters escaped that end a quoted string.
fn subsection(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> ObjectId {
        ObjectId::from_bytes_or_panic(&[byte; 20])
    }

    fn packed(name: &str, byte: u8, peeled: u8) -> Packed {
        Packed {
            name: name.to_owned(),
            remote_name: name.to_owned(),
            id: id(byte),
            peeled: id(peeled),
        }
    }

    #[test]
    fn stored_url_drops_the_password_and_keeps_the_user() {
        assert_eq!(
            stored_url("https://user:secret@example.com/app.git").unwrap(),
            "https://user@example.com/app.git"
        );
        assert_eq!(
            stored_url("https://example.com/app.git").unwrap(),
            "https://example.com/app.git"
        );
        assert_eq!(
            stored_url("git@host.example:org/app.git").unwrap(),
            "git@host.example:org/app.git"
        );
    }

    #[test]
    fn stored_url_of_a_password_without_a_user_has_no_user_info() {
        assert_eq!(
            stored_url("https://:token@example.com/app.git").unwrap(),
            "https://example.com/app.git"
        );
    }

    #[test]
    fn detached_head_without_remote_head_peels_the_first_reference() {
        let head = head_target(
            &[packed("refs/tags/v1", 1, 2)],
            &RemoteHead::default(),
            None,
            false,
        );
        assert_eq!(head.branch(), None);
        assert_eq!(head.commit(), Some(id(2)));
    }

    #[test]
    fn stored_url_requires_absolute_local_paths() {
        assert_eq!(
            stored_url(nurt_testkit::paths::abs("/srv/up/").as_str()).unwrap(),
            nurt_testkit::paths::abs("/srv/up").as_str()
        );
        assert_eq!(
            stored_url(nurt_testkit::paths::abs("/").as_str()).unwrap(),
            nurt_testkit::paths::abs("/").as_str()
        );
        let error = stored_url("relative/up").unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert!(stored_url("").is_err());
    }

    #[test]
    fn fetch_spec_names_the_tracking_namespace() {
        assert_eq!(
            fetch_spec(false, None),
            "+refs/heads/*:refs/remotes/origin/*"
        );
        assert_eq!(
            fetch_spec(false, Some("dev")),
            "+refs/heads/dev:refs/remotes/origin/dev"
        );
        assert_eq!(fetch_spec(true, None), "+refs/heads/*:refs/heads/*");
        assert_eq!(
            fetch_spec(true, Some("dev")),
            "+refs/heads/dev:refs/heads/dev"
        );
    }

    #[test]
    fn packed_refs_lists_peeled_tags_after_their_line() {
        let text = packed_refs(&[
            packed("refs/remotes/origin/main", 1, 1),
            packed("refs/tags/v1", 2, 3),
        ]);
        let expected = format!(
            "{PACKED_HEADER}{} refs/remotes/origin/main\n{} refs/tags/v1\n^{}\n",
            id(1),
            id(2),
            id(3)
        );
        assert_eq!(text, expected);
    }

    #[test]
    fn default_core_settings_are_left_out() {
        let text = "[core]\n\tbare = true\n\tignorecase = false\n\tlogallrefupdates = true\n";
        assert_eq!(
            without_defaults(text),
            "[core]\n\tbare = true\n\tlogallrefupdates = true\n"
        );
    }

    #[test]
    fn values_with_special_characters_are_quoted() {
        assert_eq!(value("/srv/up"), "/srv/up");
        assert_eq!(value("a#b"), "\"a#b\"");
        assert_eq!(value("say \"x\""), "\"say \\\"x\\\"\"");
    }

    #[test]
    fn values_with_line_feeds_or_edge_spaces_are_quoted_and_escaped() {
        assert_eq!(value("a\nb"), "\"a\\nb\"");
        assert_eq!(value(" a"), "\" a\"");
        assert_eq!(value("a "), "\"a \"");
        assert_eq!(subsection("a\\b\"c"), "a\\\\b\\\"c");
    }

    #[test]
    fn stored_url_keeps_scp_like_and_file_addresses() {
        assert_eq!(
            stored_url("file:///srv/up.git").unwrap(),
            "file:///srv/up.git"
        );
        assert_eq!(
            stored_url("ssh://git@example.com:2222/up.git").unwrap(),
            "ssh://git@example.com:2222/up.git"
        );
        assert!(stored_url("a:b").is_err());
    }

    #[test]
    fn remote_head_guesses_the_branch_by_commit() {
        let refs = vec![
            Ref::Direct {
                full_ref_name: "HEAD".into(),
                object: id(5),
            },
            Ref::Direct {
                full_ref_name: "refs/heads/a".into(),
                object: id(5),
            },
            Ref::Direct {
                full_ref_name: "refs/heads/master".into(),
                object: id(5),
            },
        ];
        assert_eq!(RemoteHead::of(&refs).branch().as_deref(), Some("master"));
        let detached = vec![Ref::Direct {
            full_ref_name: "HEAD".into(),
            object: id(5),
        }];
        assert_eq!(RemoteHead::of(&detached).branch(), None);
    }

    #[test]
    fn branch_names_with_comment_characters_are_quoted_in_the_configuration() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config"), "[core]\n\tbare = false\n").unwrap();
        let mut layout = Layout::new(dir.path(), false, "https://example.com/app.git");
        layout
            .write_config(Some("+refs/heads/*:refs/remotes/origin/*"), Some("fix#1;x"))
            .unwrap();
        let config = std::fs::read_to_string(dir.path().join("config")).unwrap();
        let expected = format!(
            "[branch \"fix#1;x\"]\n\tremote = {REMOTE}\n\tmerge = \"refs/heads/fix#1;x\"\n"
        );
        assert!(config.ends_with(&expected), "{config}");
    }

    #[test]
    fn write_refs_refuses_names_that_leave_the_repository_directory() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join("repo.git");
        std::fs::create_dir(&git_dir).unwrap();
        let layout = Layout::new(&git_dir, false, "https://example.com/app.git");
        let head = HeadTarget::Branch {
            name: "../../escape".to_owned(),
            commit: Some(id(1)),
            origin_head: None,
        };
        let refs = [packed("refs/remotes/origin/ok", 1, 1)];
        let error = layout.write_refs(&refs, &head).unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitRef);
        assert_eq!(error.data().problem.as_deref(), Some("invalid_name"));
        assert!(!dir.path().join("escape").exists());
        assert!(!git_dir.join("HEAD").exists());
        assert!(!git_dir.join("packed-refs").exists());
        let bad = [packed("refs/remotes/origin/../../x", 1, 1)];
        let error = layout
            .write_refs(&bad, &HeadTarget::Detached(id(1)))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitRef);
        assert!(!git_dir.join("packed-refs").exists());
    }

    #[test]
    fn write_refs_writes_the_branch_head_and_packed_references() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), false, "https://example.com/app.git");
        let head = HeadTarget::Branch {
            name: "feature/a".to_owned(),
            commit: Some(id(1)),
            origin_head: None,
        };
        let refs = [packed("refs/remotes/origin/feature/a", 1, 1)];
        layout.write_refs(&refs, &head).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("HEAD")).unwrap(),
            "ref: refs/heads/feature/a\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("refs/heads/feature/a")).unwrap(),
            format!("{}\n", id(1))
        );
        let packed_refs = std::fs::read_to_string(dir.path().join("packed-refs")).unwrap();
        assert_eq!(
            packed_refs,
            format!("{PACKED_HEADER}{} refs/remotes/origin/feature/a\n", id(1))
        );
    }

    #[test]
    fn head_target_of_an_empty_remote_is_the_default_branch() {
        let head = head_target(&[], &RemoteHead::default(), None, false);
        assert_eq!(head.branch(), Some(DEFAULT_BRANCH));
        assert_eq!(head.commit(), None);
    }

    #[test]
    fn head_target_of_a_detached_remote_is_the_commit() {
        let remote = RemoteHead {
            target: None,
            id: Some(id(9)),
        };
        let head = head_target(
            &[packed("refs/remotes/origin/x", 1, 1)],
            &remote,
            None,
            false,
        );
        assert_eq!(head.branch(), None);
        assert_eq!(head.commit(), Some(id(9)));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
