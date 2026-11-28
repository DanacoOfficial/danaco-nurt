// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::send_pack::report
// Responsibility:   Parsing of `report-status` and `report-status-v2` responses.
// Dependencies:     bstr, gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parsing of `report-status` and `report-status-v2` responses.

use bstr::{BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::Result;

use super::error::{clean_remote_text, protocol_error};
use super::pkt::{self, Packet};

/// Outcome of one reference as reported by the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefStatus {
    /// The reference was updated (`ok`).
    Ok,
    /// The reference was not updated (`ng`) with the reason of the remote.
    Rejected(String),
}

/// Report of one reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefReport {
    /// Reference named in the command.
    pub name: BString,
    /// Outcome.
    pub status: RefStatus,
    /// Reference actually updated, when a hook of the remote rewrote it.
    pub updated_name: Option<BString>,
    /// Old value reported by `report-status-v2`.
    pub old: Option<ObjectId>,
    /// New value reported by `report-status-v2`.
    pub new: Option<ObjectId>,
    /// The remote reports a forced update.
    pub forced: bool,
}

/// Complete status report of a push.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushReport {
    /// Error text of the unpack step; `None` for `unpack ok`.
    pub unpack_error: Option<String>,
    /// Reports of the references, in the order of the remote.
    pub refs: Vec<RefReport>,
}

impl PushReport {
    /// Report of the reference `name`.
    #[must_use]
    pub fn get(&self, name: &[u8]) -> Option<&RefReport> {
        self.refs.iter().find(|report| report.name == name)
    }
}

/// Classified reason of a rejected update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// The update is not a fast-forward.
    NonFastForward,
    /// The remote has commits that are not present locally.
    FetchFirst,
    /// The remote reference changed since it was last seen.
    StaleInfo,
    /// A hook of the remote declined the update.
    HookDeclined,
    /// The branch is checked out in the remote working tree.
    CheckedOut,
    /// Another reference of an atomic push failed.
    AtomicFailed,
    /// The remote does not allow deleting references.
    DeletionProhibited,
    /// The tag exists on the remote.
    AlreadyExists,
    /// The reference is hidden by the remote.
    HiddenRef,
    /// The remote could not unpack the objects.
    UnpackFailed,
    /// Any other reason.
    Other,
}

impl RejectReason {
    /// Classifies a reason text of the remote or of the local check.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_transport::send_pack::RejectReason;
    ///
    /// assert_eq!(RejectReason::classify("pre-receive hook declined"), RejectReason::HookDeclined);
    /// assert_eq!(RejectReason::classify("non-fast-forward").code(), "non_fast_forward");
    /// ```
    #[must_use]
    pub fn classify(text: &str) -> Self {
        const RULES: &[(&str, RejectReason)] = &[
            ("non-fast-forward", RejectReason::NonFastForward),
            ("fetch first", RejectReason::FetchFirst),
            ("stale info", RejectReason::StaleInfo),
            ("hook declined", RejectReason::HookDeclined),
            ("currently checked out", RejectReason::CheckedOut),
            ("atomic push fail", RejectReason::AtomicFailed),
            ("atomic transaction failed", RejectReason::AtomicFailed),
            ("deletion", RejectReason::DeletionProhibited),
            ("already exists", RejectReason::AlreadyExists),
            ("hidden ref", RejectReason::HiddenRef),
            ("unpacker error", RejectReason::UnpackFailed),
        ];
        RULES
            .iter()
            .find(|(pattern, _)| text.contains(pattern))
            .map_or(Self::Other, |(_, reason)| *reason)
    }

    /// Stable code of the reason, used in `git_ref_update.reason`.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::NonFastForward => "non_fast_forward",
            Self::FetchFirst => "fetch_first",
            Self::StaleInfo => "stale_info",
            Self::HookDeclined => "hook_declined",
            Self::CheckedOut => "checked_out",
            Self::AtomicFailed => "atomic_failed",
            Self::DeletionProhibited => "deletion_prohibited",
            Self::AlreadyExists => "already_exists",
            Self::HiddenRef => "hidden_ref",
            Self::UnpackFailed => "unpack_failed",
            Self::Other => "rejected",
        }
    }
}

/// Parses the status report from its packet lines.
///
/// # Errors
///
/// Fails with `E_GIT_PROTOCOL` when the report is malformed.
pub fn parse_report(bytes: &[u8]) -> Result<PushReport> {
    let packets = pkt::parse_all(bytes).map_err(|error| {
        protocol_error(
            "malformed_report",
            format!("the status report of the remote is malformed: {error}"),
        )
    })?;
    parse_lines(packets.iter().filter_map(|packet| match packet {
        Packet::Data(data) => Some(pkt::trim_newline(data)),
        Packet::Flush | Packet::Delimiter => None,
    }))
}

/// Parses the status report from its lines without line feeds.
///
/// # Errors
///
/// Fails with `E_GIT_PROTOCOL` when the report is malformed.
pub fn parse_lines<'a>(lines: impl IntoIterator<Item = &'a [u8]>) -> Result<PushReport> {
    let mut lines = lines.into_iter();
    let first = lines.next().ok_or_else(|| malformed(b"(empty report)"))?;
    let unpack = first
        .strip_prefix(b"unpack ")
        .ok_or_else(|| malformed(first))?;
    let mut report = PushReport {
        unpack_error: (unpack != b"ok").then(|| clean_remote_text(&unpack.to_str_lossy())),
        refs: Vec::new(),
    };
    for line in lines {
        parse_line(line, &mut report)?;
    }
    Ok(report)
}

fn parse_line(line: &[u8], report: &mut PushReport) -> Result<()> {
    if let Some(name) = line.strip_prefix(b"ok ") {
        report.refs.push(new_report(name, RefStatus::Ok));
        return Ok(());
    }
    if let Some(rest) = line.strip_prefix(b"ng ") {
        let (name, reason) = rest.split_once_str(b" ").unwrap_or((rest, b"failed"));
        report.refs.push(new_report(
            name,
            RefStatus::Rejected(clean_remote_text(&reason.to_str_lossy())),
        ));
        return Ok(());
    }
    if let Some(option) = line.strip_prefix(b"option ") {
        let current = report.refs.last_mut().ok_or_else(|| malformed(line))?;
        return apply_option(current, option).ok_or_else(|| malformed(line));
    }
    Err(malformed(line))
}

fn new_report(name: &[u8], status: RefStatus) -> RefReport {
    RefReport {
        name: name.into(),
        status,
        updated_name: None,
        old: None,
        new: None,
        forced: false,
    }
}

fn apply_option(report: &mut RefReport, option: &[u8]) -> Option<()> {
    if option == b"forced-update" {
        report.forced = true;
        return Some(());
    }
    let (key, value) = option.split_once_str(b" ")?;
    match key {
        b"refname" => report.updated_name = Some(value.into()),
        b"old-oid" => report.old = Some(ObjectId::from_hex(value).ok()?),
        b"new-oid" => report.new = Some(ObjectId::from_hex(value).ok()?),
        _ => {}
    }
    Some(())
}

fn malformed(line: &[u8]) -> nurt_contracts::error::Error {
    protocol_error(
        "malformed_report",
        format!(
            "the remote sent an unexpected status line `{}`",
            clean_remote_text(&line.to_str_lossy())
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::error::ErrorCode;

    fn encode(lines: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for line in lines {
            pkt::write_text(&mut out, line.as_bytes()).unwrap();
        }
        pkt::write_flush(&mut out).unwrap();
        out
    }

    #[test]
    fn parses_v1_report() {
        let report = parse_report(&encode(&[
            "unpack ok",
            "ok refs/heads/main",
            "ng refs/heads/b2 hook declined",
        ]))
        .unwrap();
        assert_eq!(report.unpack_error, None);
        assert_eq!(report.refs.len(), 2);
        assert_eq!(
            report.get(b"refs/heads/main").unwrap().status,
            RefStatus::Ok
        );
        assert_eq!(
            report.get(b"refs/heads/b2").unwrap().status,
            RefStatus::Rejected("hook declined".to_owned())
        );
        assert!(report.get(b"refs/heads/none").is_none());
    }

    #[test]
    fn parses_v2_options() {
        let old = "1111111111111111111111111111111111111111";
        let new = "2222222222222222222222222222222222222222";
        let report = parse_report(&encode(&[
            "unpack ok",
            "ok refs/for/main",
            "option refname refs/changes/1",
            &format!("option old-oid {old}"),
            &format!("option new-oid {new}"),
            "option forced-update",
            "option unknown value",
        ]))
        .unwrap();
        let first = &report.refs[0];
        assert_eq!(
            first.updated_name.as_ref().map(|name| name.as_bstr()),
            Some(b"refs/changes/1".as_bstr())
        );
        assert_eq!(first.old, ObjectId::from_hex(old.as_bytes()).ok());
        assert_eq!(first.new, ObjectId::from_hex(new.as_bytes()).ok());
        assert!(first.forced);
    }

    #[test]
    fn reports_unpack_errors_and_rejects_malformed_reports() {
        let report = parse_report(&encode(&[
            "unpack index-pack failed",
            "ng refs/heads/x unpacker error",
        ]))
        .unwrap();
        assert_eq!(report.unpack_error.as_deref(), Some("index-pack failed"));
        for lines in [
            &["garbage"][..],
            &["unpack ok", "option refname x"][..],
            &["unpack ok", "what refs/heads/x"][..],
            &["unpack ok", "ok refs/heads/x", "option old-oid zz"][..],
        ] {
            assert_eq!(
                parse_report(&encode(lines)).unwrap_err().code(),
                ErrorCode::GitProtocol
            );
        }
        assert!(parse_report(b"").is_err());
        assert!(parse_report(b"00").is_err());
    }

    #[test]
    fn classifies_reasons_of_git() {
        let cases = [
            ("non-fast-forward", "non_fast_forward"),
            ("fetch first", "fetch_first"),
            ("stale info", "stale_info"),
            ("pre-receive hook declined", "hook_declined"),
            ("branch is currently checked out", "checked_out"),
            ("atomic push failure", "atomic_failed"),
            ("atomic push failed", "atomic_failed"),
            ("atomic transaction failed", "atomic_failed"),
            ("deletion prohibited", "deletion_prohibited"),
            ("already exists", "already_exists"),
            ("deny updating a hidden ref", "hidden_ref"),
            ("unpacker error", "unpack_failed"),
            ("something else", "rejected"),
        ];
        for (text, code) in cases {
            assert_eq!(RejectReason::classify(text).code(), code, "{text}");
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
