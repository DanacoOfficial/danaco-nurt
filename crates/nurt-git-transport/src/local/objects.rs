// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Selection of the objects a fetch needs and the pack that carries them.

use std::collections::{HashSet, VecDeque};
use std::io::{self, Write};
use std::sync::atomic::AtomicBool;

use gix::ObjectId;
use gix::objs::Kind;
use gix_pack::data::output;

/// Largest payload of one side-band packet: the packet-line limit minus the
/// band byte.
const SIDEBAND_CHUNK: usize = 65515;

/// What the client asked for in one `fetch` command.
#[derive(Debug, Default)]
pub(crate) struct FetchRequest {
    pub(crate) wants: Vec<ObjectId>,
    pub(crate) haves: Vec<ObjectId>,
    pub(crate) done: bool,
    pub(crate) include_tag: bool,
    pub(crate) deepen: Option<u32>,
    pub(crate) client_shallow: Vec<ObjectId>,
}

/// Objects to send and the shallow boundary to report.
#[derive(Debug, Default)]
pub(crate) struct Selection {
    pub(crate) objects: Vec<ObjectId>,
    pub(crate) shallow: Vec<ObjectId>,
    pub(crate) unshallow: Vec<ObjectId>,
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn kind_of(repo: &gix::Repository, id: ObjectId) -> io::Result<Kind> {
    repo.find_header(id)
        .map(|header| header.kind())
        .map_err(|_| invalid(format!("upload-pack: not our ref {id}")))
}

fn parents(repo: &gix::Repository, id: ObjectId) -> io::Result<Vec<ObjectId>> {
    let commit = repo
        .find_commit(id)
        .map_err(|e| invalid(format!("cannot read commit {id}: {e}")))?;
    Ok(commit.parent_ids().map(|p| p.detach()).collect())
}

fn tree_of(repo: &gix::Repository, commit: ObjectId) -> io::Result<ObjectId> {
    repo.find_commit(commit)
        .and_then(|c| c.tree_id().map(|t| t.detach()))
        .map_err(|e| invalid(format!("cannot read commit {commit}: {e}")))
}

/// Adds `tree` and everything below it that is neither in `exclude` nor
/// already in `seen`; submodule commits are not followed.
fn add_tree(
    repo: &gix::Repository,
    tree: ObjectId,
    exclude: &HashSet<ObjectId>,
    seen: &mut HashSet<ObjectId>,
    out: &mut Vec<ObjectId>,
) -> io::Result<()> {
    let mut pending = vec![tree];
    while let Some(tree) = pending.pop() {
        if exclude.contains(&tree) || !seen.insert(tree) {
            continue;
        }
        out.push(tree);
        let object = repo
            .find_tree(tree)
            .map_err(|e| invalid(format!("cannot read tree {tree}: {e}")))?;
        let decoded = object
            .decode()
            .map_err(|e| invalid(format!("cannot decode tree {tree}: {e}")))?;
        for entry in &decoded.entries {
            let id = entry.oid.to_owned();
            if entry.mode.is_commit() {
                continue;
            }
            if entry.mode.is_tree() {
                pending.push(id);
            } else if !exclude.contains(&id) && seen.insert(id) {
                out.push(id);
            }
        }
    }
    Ok(())
}

/// Every tree and blob reachable from the trees of `commits`.
fn reachable_from_trees(
    repo: &gix::Repository,
    commits: &HashSet<ObjectId>,
) -> io::Result<HashSet<ObjectId>> {
    let mut seen = HashSet::new();
    let mut ignored = Vec::new();
    let none = HashSet::new();
    for commit in commits {
        add_tree(
            repo,
            tree_of(repo, *commit)?,
            &none,
            &mut seen,
            &mut ignored,
        )?;
    }
    Ok(seen)
}

/// Splits the wants into commits to walk from and other objects to send
/// as they are, peeling annotated tags.
fn resolve_wants(
    repo: &gix::Repository,
    wants: &[ObjectId],
    out: &mut Vec<ObjectId>,
    seen: &mut HashSet<ObjectId>,
) -> io::Result<Vec<ObjectId>> {
    let mut commits = Vec::new();
    for want in wants {
        let mut id = *want;
        loop {
            match kind_of(repo, id)? {
                Kind::Commit => {
                    commits.push(id);
                    break;
                }
                Kind::Tag => {
                    if seen.insert(id) {
                        out.push(id);
                    }
                    let tag = repo
                        .find_tag(id)
                        .map_err(|e| invalid(format!("cannot read tag {id}: {e}")))?;
                    id = tag
                        .target_id()
                        .map_err(|e| invalid(format!("cannot decode tag {id}: {e}")))?
                        .detach();
                }
                Kind::Tree => {
                    add_tree(repo, id, &HashSet::new(), seen, out)?;
                    break;
                }
                Kind::Blob => {
                    if seen.insert(id) {
                        out.push(id);
                    }
                    break;
                }
            }
        }
    }
    Ok(commits)
}

/// Commits within `depth` of the wants, with the commits at the boundary.
fn walk_shallow(
    repo: &gix::Repository,
    wants: &[ObjectId],
    depth: u32,
    client_shallow: &HashSet<ObjectId>,
) -> io::Result<(Vec<ObjectId>, Vec<ObjectId>, Vec<ObjectId>)> {
    let mut queue: VecDeque<(ObjectId, u32)> = wants.iter().map(|id| (*id, 1)).collect();
    let mut visited = HashSet::new();
    let (mut commits, mut boundary, mut unshallow) = (Vec::new(), Vec::new(), Vec::new());
    while let Some((id, level)) = queue.pop_front() {
        if !visited.insert(id) {
            continue;
        }
        commits.push(id);
        let parents = parents(repo, id)?;
        if level >= depth {
            if !parents.is_empty() {
                boundary.push(id);
            }
            continue;
        }
        if client_shallow.contains(&id) && !parents.is_empty() {
            unshallow.push(id);
        }
        queue.extend(parents.into_iter().map(|p| (p, level + 1)));
    }
    Ok((commits, boundary, unshallow))
}

/// Commits reachable from the wants and not from the commits the client has.
fn walk_incremental(
    repo: &gix::Repository,
    wants: &[ObjectId],
    common: &[ObjectId],
) -> io::Result<Vec<ObjectId>> {
    if wants.is_empty() {
        return Ok(Vec::new());
    }
    let walk = repo
        .rev_walk(wants.iter().copied())
        .with_hidden(common.iter().copied())
        .all()
        .map_err(|e| invalid(format!("cannot walk history: {e}")))?;
    walk.map(|info| {
        info.map(|info| info.id)
            .map_err(|e| invalid(format!("cannot walk history: {e}")))
    })
    .collect()
}

/// Annotated tags whose peeled target is among `sent`, for `include-tag`.
fn included_tags(
    repo: &gix::Repository,
    sent: &HashSet<ObjectId>,
    seen: &mut HashSet<ObjectId>,
    out: &mut Vec<ObjectId>,
) -> io::Result<()> {
    let platform = repo
        .references()
        .map_err(|e| invalid(format!("cannot list references: {e}")))?;
    let tags = platform
        .tags()
        .map_err(|e| invalid(format!("cannot list tags: {e}")))?;
    for reference in tags.flatten() {
        let Some(id) = reference.target().try_id().map(ToOwned::to_owned) else {
            continue;
        };
        if kind_of(repo, id).ok() != Some(Kind::Tag) || seen.contains(&id) {
            continue;
        }
        let mut chain = vec![id];
        let mut target = id;
        while let Ok(tag) = repo.find_tag(target) {
            let Ok(next) = tag.target_id() else { break };
            target = next.detach();
            if kind_of(repo, target).ok() == Some(Kind::Tag) {
                chain.push(target);
            } else {
                break;
            }
        }
        if sent.contains(&target) {
            for tag in chain {
                if seen.insert(tag) {
                    out.push(tag);
                }
            }
        }
    }
    Ok(())
}

/// Determines the objects for `request`, given the commits both sides have.
pub(crate) fn select(
    repo: &gix::Repository,
    request: &FetchRequest,
    common: &[ObjectId],
) -> io::Result<Selection> {
    let mut objects = Vec::new();
    let mut seen = HashSet::new();
    let want_commits = resolve_wants(repo, &request.wants, &mut objects, &mut seen)?;
    let client_shallow: HashSet<ObjectId> = request.client_shallow.iter().copied().collect();
    let mut selection = Selection::default();
    let (commits, exclude) = match request.deepen {
        Some(depth) => {
            let (commits, boundary, unshallow) =
                walk_shallow(repo, &want_commits, depth.max(1), &client_shallow)?;
            selection.shallow = boundary;
            selection.unshallow = unshallow;
            (commits, HashSet::new())
        }
        None => {
            let commits = walk_incremental(repo, &want_commits, common)?;
            let sent: HashSet<ObjectId> = commits.iter().copied().collect();
            let mut edge = HashSet::new();
            for commit in &commits {
                if client_shallow.contains(commit) {
                    continue;
                }
                edge.extend(
                    parents(repo, *commit)?
                        .into_iter()
                        .filter(|p| !sent.contains(p)),
                );
            }
            (commits, reachable_from_trees(repo, &edge)?)
        }
    };
    for commit in &commits {
        if seen.insert(*commit) {
            objects.push(*commit);
        }
        add_tree(
            repo,
            tree_of(repo, *commit)?,
            &exclude,
            &mut seen,
            &mut objects,
        )?;
    }
    if request.include_tag {
        let sent: HashSet<ObjectId> = objects.iter().copied().collect();
        included_tags(repo, &sent, &mut seen, &mut objects)?;
    }
    selection.objects = objects;
    Ok(selection)
}

/// Packet-line writer of side-band channel 1.
struct Sideband<W> {
    out: W,
}

impl<W: Write> Write for Sideband<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for chunk in buf.chunks(SIDEBAND_CHUNK) {
            gix_packetline::blocking_io::encode::band_to_write(
                gix_packetline::Channel::Data,
                chunk,
                &mut self.out,
            )?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// Writes a pack with `objects` to `out`, framed in side-band channel 1.
pub(crate) fn write_pack(
    repo: &gix::Repository,
    objects: Vec<ObjectId>,
    out: impl Write,
) -> io::Result<()> {
    let to_io = |e: gix::error::Error| io::Error::other(e.to_string());
    let db = repo.objects.clone().into_arc()?.into_inner();
    let interrupt = AtomicBool::new(false);
    let mut ids = objects.into_iter().map(Ok);
    let (counts, _) = output::count::objects_unthreaded(
        &db,
        &mut ids,
        &gix::progress::Discard,
        &interrupt,
        output::count::objects::ObjectExpansion::AsIs,
    )
    .map_err(to_io)?;
    let entries = u32::try_from(counts.len())
        .map_err(|_| invalid("too many objects for one pack".to_owned()))?;
    let options = output::entry::iter_from_counts::Options {
        thread_limit: Some(1),
        allow_thin_pack: false,
        ..Default::default()
    };
    let chunks =
        output::entry::iter_from_counts(counts, db, Box::new(gix::progress::Discard), options)
            .map_err(to_io)?;
    let mut sideband = Sideband { out };
    let writer = output::bytes::FromEntriesIter::new(
        chunks.map(|chunk| chunk.map(|(_, entries)| entries)),
        &mut sideband,
        entries,
        gix_pack::data::Version::V2,
        repo.object_hash(),
    );
    for written in writer {
        written.map_err(to_io)?;
    }
    sideband.flush()
}
