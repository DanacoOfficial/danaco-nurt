// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Update commands and capability selection of a push request.

use std::io::{self, Write};

use bstr::{BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::Result;

use super::advertisement::Advertisement;
use super::error::protocol_error;
use super::pkt;

/// One reference update sent to the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefCommand {
    /// Value the remote reference must have; the null identifier when it must
    /// not exist.
    pub old: ObjectId,
    /// New value; the null identifier deletes the reference.
    pub new: ObjectId,
    /// Full name of the remote reference.
    pub name: BString,
}

impl RefCommand {
    /// Whether the command deletes the reference.
    #[must_use]
    pub fn is_delete(&self) -> bool {
        self.new.is_null()
    }

    /// Whether the command creates the reference.
    #[must_use]
    pub fn is_create(&self) -> bool {
        self.old.is_null()
    }
}

/// Request options that select capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOptions {
    /// All references are updated or none (`atomic`).
    pub atomic: bool,
    /// The remote sends no progress messages (`quiet`).
    pub quiet: bool,
    /// Push options passed to the hooks of the remote (`push-options`).
    pub push_options: Vec<String>,
    /// Agent string sent when the remote announces `agent`.
    pub agent: String,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self {
            atomic: false,
            quiet: true,
            push_options: Vec::new(),
            agent: format!("nurt/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Capabilities chosen for one request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Negotiated {
    /// Capability words sent after the first command.
    pub words: Vec<String>,
    /// The remote reports the outcome of each reference.
    pub report_status: bool,
    /// The report carries `option` lines (`report-status-v2`).
    pub report_status_v2: bool,
    /// The response is multiplexed on side bands.
    pub side_band: bool,
}

/// Selects the capabilities for `commands` from those announced by the
/// remote.
///
/// # Errors
///
/// Fails with `E_GIT_PROTOCOL` when an option needs a capability the remote
/// does not announce: `atomic`, `push-options`, `delete-refs` or a matching
/// object format.
///
/// # Examples
///
/// ```
/// use nurt_git_transport::send_pack::{Advertisement, SendOptions, negotiate};
///
/// let caps = vec![("report-status".to_owned(), None), ("side-band-64k".to_owned(), None)];
/// let negotiated = negotiate(&Advertisement::new(Vec::new(), caps), &[], &SendOptions::default())?;
/// assert!(negotiated.report_status && negotiated.side_band);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn negotiate(
    advertisement: &Advertisement,
    commands: &[RefCommand],
    options: &SendOptions,
) -> Result<Negotiated> {
    let mut negotiated = Negotiated::default();
    if advertisement.supports("report-status-v2") {
        negotiated.words.push("report-status-v2".to_owned());
        negotiated.report_status = true;
        negotiated.report_status_v2 = true;
    } else if advertisement.supports("report-status") {
        negotiated.words.push("report-status".to_owned());
        negotiated.report_status = true;
    }
    if advertisement.supports("side-band-64k") {
        negotiated.words.push("side-band-64k".to_owned());
        negotiated.side_band = true;
    }
    if options.quiet && advertisement.supports("quiet") {
        negotiated.words.push("quiet".to_owned());
    }
    require(options.atomic, advertisement, "atomic", &mut negotiated)?;
    require(
        !options.push_options.is_empty(),
        advertisement,
        "push-options",
        &mut negotiated,
    )?;
    require(
        commands.iter().any(RefCommand::is_delete),
        advertisement,
        "delete-refs",
        &mut negotiated,
    )?;
    if advertisement.supports("ofs-delta") {
        negotiated.words.push("ofs-delta".to_owned());
    }
    if let Some(format) = advertisement.capability("object-format") {
        negotiated.words.push(format!("object-format={format}"));
    }
    if advertisement.supports("agent") {
        negotiated.words.push(format!("agent={}", options.agent));
    }
    Ok(negotiated)
}

fn require(
    wanted: bool,
    advertisement: &Advertisement,
    capability: &str,
    negotiated: &mut Negotiated,
) -> Result<()> {
    if !wanted {
        return Ok(());
    }
    if !advertisement.supports(capability) {
        return Err(protocol_error(
            &format!("{}_unsupported", capability.replace('-', "_")),
            format!("the remote does not support `{capability}`; push without the option"),
        ));
    }
    negotiated.words.push(capability.to_owned());
    Ok(())
}

/// Writes the commands, the capability list and the push options as packet
/// lines.
///
/// # Errors
///
/// Fails when writing to `out` fails.
pub fn write_commands(
    out: &mut dyn Write,
    commands: &[RefCommand],
    negotiated: &Negotiated,
    push_options: &[String],
) -> io::Result<()> {
    for (position, command) in commands.iter().enumerate() {
        let mut line = Vec::new();
        line.extend_from_slice(command.old.to_hex().to_string().as_bytes());
        line.push(b' ');
        line.extend_from_slice(command.new.to_hex().to_string().as_bytes());
        line.push(b' ');
        line.extend_from_slice(command.name.as_bytes());
        if position == 0 {
            line.push(0);
            line.extend_from_slice(negotiated.words.join(" ").as_bytes());
        }
        pkt::write_text(out, &line)?;
    }
    pkt::write_flush(out)?;
    if negotiated.words.iter().any(|word| word == "push-options") {
        for option in push_options {
            pkt::write_text(out, option.as_bytes())?;
        }
        pkt::write_flush(out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::error::ErrorCode;

    fn advertisement(words: &[&str]) -> Advertisement {
        Advertisement::new(
            Vec::new(),
            words
                .iter()
                .map(|word| match word.split_once('=') {
                    Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
                    None => ((*word).to_owned(), None),
                })
                .collect(),
        )
    }

    fn command(old: ObjectId, new: ObjectId) -> RefCommand {
        RefCommand {
            old,
            new,
            name: "refs/heads/main".into(),
        }
    }

    fn id(byte: u8) -> ObjectId {
        ObjectId::from_bytes_or_panic(&[byte; 20])
    }

    #[test]
    fn prefers_report_status_v2_and_adds_agent_and_format() {
        let remote = advertisement(&[
            "report-status",
            "report-status-v2",
            "side-band-64k",
            "quiet",
            "ofs-delta",
            "object-format=sha1",
            "agent=git/2.53",
        ]);
        let options = SendOptions {
            agent: "nurt/1".to_owned(),
            ..SendOptions::default()
        };
        let negotiated = negotiate(&remote, &[], &options).unwrap();
        assert_eq!(
            negotiated.words,
            [
                "report-status-v2",
                "side-band-64k",
                "quiet",
                "ofs-delta",
                "object-format=sha1",
                "agent=nurt/1"
            ]
        );
        assert!(negotiated.report_status_v2);
    }

    #[test]
    fn requires_capabilities_for_options() {
        let remote = advertisement(&["report-status"]);
        let atomic = SendOptions {
            atomic: true,
            ..SendOptions::default()
        };
        let error = negotiate(&remote, &[], &atomic).unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitProtocol);
        assert_eq!(error.problem.as_deref(), Some("atomic_unsupported"));
        let delete = [command(id(1), ObjectId::null(gix::hash::Kind::Sha1))];
        let error = negotiate(&remote, &delete, &SendOptions::default()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("delete_refs_unsupported"));
        let options = SendOptions {
            push_options: vec!["ci.skip".to_owned()],
            ..SendOptions::default()
        };
        assert!(negotiate(&remote, &[], &options).is_err());
    }

    #[test]
    fn writes_capabilities_on_the_first_command_only() {
        let remote = advertisement(&["report-status", "push-options"]);
        let options = SendOptions {
            push_options: vec!["ci.skip".to_owned()],
            ..SendOptions::default()
        };
        let commands = [command(id(1), id(2)), command(id(2), id(3))];
        let negotiated = negotiate(&remote, &commands, &options).unwrap();
        let mut out = Vec::new();
        write_commands(&mut out, &commands, &negotiated, &options.push_options).unwrap();
        let packets = pkt::parse_all(&out).unwrap();
        assert_eq!(packets.len(), 5);
        let pkt::Packet::Data(first) = &packets[0] else {
            panic!("data expected")
        };
        assert!(first.contains(&0));
        let pkt::Packet::Data(second) = &packets[1] else {
            panic!("data expected")
        };
        assert!(!second.contains(&0));
        assert_eq!(packets[3], pkt::Packet::Data(b"ci.skip\n".to_vec()));
        assert!(commands[0].is_create() == commands[0].old.is_null());
        assert!(!commands[0].is_delete());
    }
}
