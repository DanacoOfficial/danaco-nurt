// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Sending a push request over an established transport.

use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use bstr::ByteSlice;
use gix::ObjectId;
use gix_transport::client::blocking_io::Transport;
use gix_transport::client::{MessageKind, WriteMode};
use nurt_contracts::error::Result;

use super::advertisement::Advertisement;
use super::command::{Negotiated, RefCommand, SendOptions, negotiate, write_commands};
use super::error::{protocol_error, stream_error, transport_error};
use super::objects::objects_to_send;
use super::pack::{PackSummary, write_pack};
use super::report::{PushReport, parse_report};

/// Result of one push request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendOutcome {
    /// Status report; `None` when the remote announces no `report-status`.
    pub report: Option<PushReport>,
    /// Messages of the remote (side band 2), one entry per line.
    pub remote_messages: Vec<String>,
    /// The pack sent; `None` when every command is a deletion.
    pub pack: Option<PackSummary>,
}

/// Sends `commands` and the objects they need to the remote and reads the
/// status report.
///
/// Objects reachable from the advertised references, from `known` and from
/// the `.have` lines of the remote are not sent.
///
/// # Errors
///
/// Fails with `E_GIT_PROTOCOL` when the remote lacks a required capability
/// or breaks the protocol, `E_NETWORK` when the connection fails, `E_IO`
/// with problem `missing_object` when a local object is missing and
/// `E_CANCELLED` when `interrupt` is set.
pub fn send(
    transport: &mut dyn Transport,
    repo: &gix::Repository,
    advertisement: &Advertisement,
    commands: &[RefCommand],
    known: &[ObjectId],
    options: &SendOptions,
    interrupt: &AtomicBool,
) -> Result<SendOutcome> {
    let negotiated = negotiate(advertisement, commands, options)?;
    let mut outcome = SendOutcome::default();
    let tips: Vec<ObjectId> = commands
        .iter()
        .filter(|command| !command.is_delete())
        .map(|command| command.new)
        .collect();
    let pack_objects = if tips.is_empty() {
        None
    } else {
        let mut hidden: Vec<ObjectId> = advertisement.refs().iter().map(|r| r.id).collect();
        hidden.extend_from_slice(advertisement.extra_haves());
        hidden.extend_from_slice(known);
        Some(objects_to_send(repo, &tips, &hidden, interrupt)?)
    };
    let request = transport
        .request(WriteMode::Binary, MessageKind::Flush, false)
        .map_err(|error| transport_error(&error))?;
    let (mut writer, mut reader) = request.into_parts();
    write_commands(writer.as_mut(), commands, &negotiated, &options.push_options)
        .map_err(|error| stream_error(&error))?;
    if let Some(ids) = pack_objects {
        outcome.pack = Some(write_pack(repo, ids, writer.as_mut(), interrupt)?);
    }
    writer.flush().map_err(|error| stream_error(&error))?;
    drop(writer);
    if !negotiated.report_status {
        return Ok(outcome);
    }
    let (report, messages) = read_response(reader.as_mut(), &negotiated)?;
    outcome.report = Some(report);
    outcome.remote_messages = messages;
    Ok(outcome)
}

type Reader<'a> = dyn gix_transport::client::blocking_io::ExtendedBufRead<'a> + Unpin + 'a;

fn read_response(reader: &mut Reader<'_>, negotiated: &Negotiated) -> Result<(PushReport, Vec<String>)> {
    if !negotiated.side_band {
        let mut lines = Vec::new();
        while let Some(line) = reader.readline() {
            let line = line
                .map_err(|error| stream_error(&error))?
                .map_err(|error| protocol_error("malformed_report", error.to_string()))?;
            if let Some(data) = line.as_slice() {
                lines.push(super::pkt::trim_newline(data).to_vec());
            }
        }
        let report = super::report::parse_lines(lines.iter().map(Vec::as_slice))?;
        return Ok((report, Vec::new()));
    }
    let messages = Rc::new(RefCell::new(RemoteMessages::default()));
    let sink = Rc::clone(&messages);
    reader.set_progress_handler(Some(Box::new(move |is_error: bool, text: &[u8]| {
        sink.borrow_mut().push(is_error, text);
        std::ops::ControlFlow::Continue(())
    })));
    let mut data = Vec::new();
    let read = reader.read_to_end(&mut data);
    reader.set_progress_handler(None);
    let messages = messages.take();
    if let Some(fatal) = messages.fatal {
        return Err(protocol_error(
            "remote_fatal",
            format!("the remote failed: {fatal}"),
        ));
    }
    read.map_err(|error| stream_error(&error))?;
    let report = parse_report(&data)?;
    Ok((report, messages.lines))
}

#[derive(Debug, Default)]
struct RemoteMessages {
    lines: Vec<String>,
    partial: String,
    fatal: Option<String>,
}

impl RemoteMessages {
    fn push(&mut self, is_error: bool, text: &[u8]) {
        let text = text.to_str_lossy();
        if is_error {
            self.fatal = Some(text.trim().to_owned());
            return;
        }
        self.partial.push_str(&text);
        while let Some(end) = self.partial.find(['\n', '\r']) {
            let line = self.partial[..end].trim_end().to_owned();
            self.partial.drain(..=end);
            if !line.is_empty() {
                self.lines.push(line);
            }
        }
    }

    fn take(&mut self) -> Self {
        let rest = self.partial.trim().to_owned();
        if !rest.is_empty() {
            self.lines.push(rest);
        }
        std::mem::take(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_remote_messages_into_lines() {
        let mut messages = RemoteMessages::default();
        messages.push(false, b"Resolving deltas: 50%\rResolving deltas: 100%\n");
        messages.push(false, b"error: hook declined to update refs/heads/b2   \nno new");
        messages.push(false, b"line");
        let taken = messages.take();
        assert_eq!(
            taken.lines,
            [
                "Resolving deltas: 50%",
                "Resolving deltas: 100%",
                "error: hook declined to update refs/heads/b2",
                "no newline"
            ]
        );
        let mut fatal = RemoteMessages::default();
        fatal.push(true, b"disk full\n");
        assert_eq!(fatal.take().fatal.as_deref(), Some("disk full"));
    }
}
