// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Server side of `git-upload-pack` with protocol version 2, run in a
//! thread of the client process.
//!
//! The server advertises `ls-refs` (with `symrefs`, `peel`, `ref-prefix` and
//! `unborn`) and `fetch` (with `want`, `have`, `done`, `include-tag`,
//! `ofs-delta`, `thin-pack`, `no-progress`, `deepen` and `shallow`), as
//! described in `gitprotocol-v2(5)`. Requests the server does not support
//! are answered with an `ERR` packet.

use std::io::{self, BufWriter, Read, Write};

use bstr::ByteSlice;
use gix::ObjectId;
use gix_packetline::PacketLineRef;
use gix_packetline::blocking_io::StreamingPeekableIter;
use gix_packetline::blocking_io::encode;

use super::objects::{self, FetchRequest};

/// Buffer of the response writer; responses are flushed per section.
const WRITE_BUFFER: usize = 64 * 1024;

/// One command of the client.
#[derive(Debug, Default)]
struct Command {
    name: String,
    arguments: Vec<String>,
}

/// A reference as advertised by `ls-refs`.
struct Advertised {
    name: String,
    id: Option<ObjectId>,
    symref: Option<String>,
    peeled: Option<ObjectId>,
}

fn protocol_error(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn data_line(out: &mut impl Write, text: &str) -> io::Result<()> {
    encode::data_to_write(text.as_bytes(), out).map(|_| ())
}

/// Serves `repo` on the pair of streams until the client ends the
/// interaction or closes its side.
pub(crate) fn serve(repo: &gix::Repository, input: impl Read, output: impl Write) -> io::Result<()> {
    let mut out = BufWriter::with_capacity(WRITE_BUFFER, output);
    let mut lines = StreamingPeekableIter::new(input, &[PacketLineRef::Flush], false);
    advertise(repo, &mut out)?;
    let mut negotiation = Vec::new();
    loop {
        let Some(command) = read_command(&mut lines)? else {
            return Ok(());
        };
        let result = match command.name.as_str() {
            "ls-refs" => ls_refs(repo, &command.arguments, &mut out),
            "fetch" => fetch(repo, &command.arguments, &mut negotiation, &mut out),
            other => Err(protocol_error(format!("unknown command '{other}'"))),
        };
        if let Err(error) = result {
            encode::error_to_write(error.to_string().as_bytes(), &mut out).ok();
            out.flush().ok();
            return Err(error);
        }
        out.flush()?;
    }
}

fn advertise(repo: &gix::Repository, out: &mut impl Write) -> io::Result<()> {
    data_line(out, "version 2\n")?;
    data_line(out, &format!("agent=git/nurt-{}\n", env!("CARGO_PKG_VERSION")))?;
    data_line(out, "ls-refs=unborn\n")?;
    data_line(out, "fetch=shallow\n")?;
    data_line(out, &format!("object-format={}\n", repo.object_hash()))?;
    encode::flush_to_write(&mut *out)?;
    out.flush()
}

/// Reads one command up to its flush packet; `None` when the client ended
/// the interaction.
fn read_command<R: Read>(lines: &mut StreamingPeekableIter<R>) -> io::Result<Option<Command>> {
    let mut command = Command::default();
    let mut in_arguments = false;
    let mut any = false;
    while let Some(line) = lines.read_line() {
        let line = line?.map_err(|e| protocol_error(e.to_string()))?;
        any = true;
        match line {
            PacketLineRef::Data(data) => {
                let text = data.trim_end_with(|c| c == '\n').to_str_lossy().into_owned();
                if in_arguments {
                    command.arguments.push(text);
                } else if let Some(name) = text.strip_prefix("command=") {
                    command.name = name.to_owned();
                }
            }
            PacketLineRef::Delimiter => in_arguments = true,
            PacketLineRef::Flush | PacketLineRef::ResponseEnd => {}
        }
    }
    lines.reset();
    Ok((any && !command.name.is_empty()).then_some(command))
}

/// Object of `name` after following symbolic references, without peeling
/// tags; `None` for a symbolic reference to a missing reference.
fn resolve_symbolic(repo: &gix::Repository, name: &str) -> Option<ObjectId> {
    let mut current = repo.find_reference(name).ok()?;
    for _ in 0..MAX_SYMREF_DEPTH {
        if let Some(id) = current.target().try_id() {
            return Some(id.to_owned());
        }
        let next = current.target().try_name()?.as_bstr().to_str_lossy().into_owned();
        current = repo.find_reference(next.as_str()).ok()?;
    }
    None
}

fn peeled(repo: &gix::Repository, id: ObjectId) -> Option<ObjectId> {
    let object = repo.find_object(id).ok()?;
    if object.kind != gix::objs::Kind::Tag {
        return None;
    }
    object.peel_tags_to_end().ok().map(|o| o.id)
}

fn advertised_refs(repo: &gix::Repository) -> io::Result<Vec<Advertised>> {
    let mut refs = Vec::new();
    if let Ok(head) = repo.find_reference("HEAD") {
        let symref = head
            .target()
            .try_name()
            .map(|name| name.as_bstr().to_str_lossy().into_owned());
        refs.push(Advertised {
            name: "HEAD".to_owned(),
            id: resolve_symbolic(repo, "HEAD"),
            symref,
            peeled: None,
        });
    }
    let platform = repo
        .references()
        .map_err(|e| protocol_error(format!("cannot list references: {e}")))?;
    let all = platform
        .all()
        .map_err(|e| protocol_error(format!("cannot list references: {e}")))?;
    for reference in all {
        let reference = reference.map_err(|e| protocol_error(format!("cannot read a reference: {e}")))?;
        let name = reference.name().as_bstr().to_str_lossy().into_owned();
        let symref = reference
            .target()
            .try_name()
            .map(|target| target.as_bstr().to_str_lossy().into_owned());
        let id = match reference.target().try_id() {
            Some(id) => Some(id.to_owned()),
            None => resolve_symbolic(repo, &name),
        };
        let Some(id) = id else { continue };
        refs.push(Advertised {
            name,
            peeled: peeled(repo, id),
            id: Some(id),
            symref,
        });
    }
    Ok(refs)
}

fn ls_refs(repo: &gix::Repository, arguments: &[String], out: &mut impl Write) -> io::Result<()> {
    let symrefs = arguments.iter().any(|a| a == "symrefs");
    let peel = arguments.iter().any(|a| a == "peel");
    let unborn = arguments.iter().any(|a| a == "unborn");
    let prefixes: Vec<&str> = arguments.iter().filter_map(|a| a.strip_prefix("ref-prefix ")).collect();
    for reference in advertised_refs(repo)? {
        if !prefixes.is_empty() && !prefixes.iter().any(|p| reference.name.starts_with(p)) {
            continue;
        }
        let mut line = match reference.id {
            Some(id) => format!("{id} {}", reference.name),
            None if unborn && reference.name == "HEAD" => "unborn HEAD".to_owned(),
            None => continue,
        };
        if let (true, Some(target)) = (symrefs || reference.id.is_none(), &reference.symref) {
            line.push_str(&format!(" symref-target:{target}"));
        }
        if let (true, Some(peeled)) = (peel, reference.peeled) {
            line.push_str(&format!(" peeled:{peeled}"));
        }
        line.push('\n');
        data_line(out, &line)?;
    }
    encode::flush_to_write(&mut *out).map(|_| ())
}

fn parse_id(repo: &gix::Repository, text: &str) -> io::Result<ObjectId> {
    let id = ObjectId::from_hex(text.as_bytes()).map_err(|_| protocol_error(format!("invalid object id '{text}'")))?;
    if id.kind() != repo.object_hash() {
        return Err(protocol_error(format!("object id '{text}' uses another hash algorithm")));
    }
    Ok(id)
}

fn parse_fetch(repo: &gix::Repository, arguments: &[String]) -> io::Result<FetchRequest> {
    let mut request = FetchRequest::default();
    for argument in arguments {
        let (key, value) = argument.split_once(' ').unwrap_or((argument.as_str(), ""));
        match key {
            "want" => request.wants.push(parse_id(repo, value)?),
            "have" => request.haves.push(parse_id(repo, value)?),
            "shallow" => request.client_shallow.push(parse_id(repo, value)?),
            "done" => request.done = true,
            "include-tag" => request.include_tag = true,
            "deepen" => {
                let depth = value
                    .parse::<u32>()
                    .map_err(|_| protocol_error(format!("invalid depth '{value}'")))?;
                request.deepen = Some(depth);
            }
            "thin-pack" | "ofs-delta" | "no-progress" => {}
            other => return Err(protocol_error(format!("upload-pack: unsupported fetch argument '{other}'"))),
        }
    }
    if request.wants.is_empty() {
        return Err(protocol_error("upload-pack: fetch without want lines"));
    }
    Ok(request)
}

fn fetch(repo: &gix::Repository, arguments: &[String], common: &mut Vec<ObjectId>, out: &mut impl Write) -> io::Result<()> {
    let request = parse_fetch(repo, arguments)?;
    let mut acknowledged = Vec::new();
    for have in &request.haves {
        if repo.find_header(*have).is_ok() && !common.contains(have) {
            common.push(*have);
            acknowledged.push(*have);
        }
    }
    if !request.done {
        data_line(out, "acknowledgments\n")?;
        if acknowledged.is_empty() {
            data_line(out, "NAK\n")?;
        }
        for id in &acknowledged {
            data_line(out, &format!("ACK {id}\n"))?;
        }
        if common.is_empty() {
            return encode::flush_to_write(&mut *out).map(|_| ());
        }
        data_line(out, "ready\n")?;
        encode::delim_to_write(&mut *out)?;
    }
    let selection = objects::select(repo, &request, common)?;
    if request.deepen.is_some() {
        data_line(out, "shallow-info\n")?;
        for id in &selection.shallow {
            data_line(out, &format!("shallow {id}\n"))?;
        }
        for id in &selection.unshallow {
            data_line(out, &format!("unshallow {id}\n"))?;
        }
        encode::delim_to_write(&mut *out)?;
    }
    data_line(out, "packfile\n")?;
    objects::write_pack(repo, selection.objects, &mut *out)?;
    common.clear();
    encode::flush_to_write(&mut *out).map(|_| ())
}
