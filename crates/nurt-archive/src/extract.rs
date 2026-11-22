// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Planning extraction and streaming member content.
//!
//! Extraction runs in two passes over the archive. [`plan`] reads only the
//! headers: it checks every member with the rules of [`crate::safety`],
//! drops members replaced by a later member with the same destination,
//! enforces [`Limits`] on the declared sizes, and returns the complete list
//! of destinations before anything is written. [`stream`] then reads the
//! archive again and hands the content of each planned regular file to the
//! caller, one member at a time, checking that the archive still matches
//! the plan and that no member yields more bytes than it declares.
//!
//! Directories, links and permissions are applied from the plan. Writing
//! directories first, then files, then hard links and symbolic links last
//! keeps every file write out of links created by the archive.

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};

use crate::create::ExactReader;
use crate::list::{self, ArchiveError, Flow, Format, LimitKind, Member, MemberKind};
use crate::safety::{Guard, Rules, SkipReason, Verdict, Violation};

/// Default largest total size of extracted content: 1 TiB.
pub const DEFAULT_MAX_SIZE: u64 = 1 << 40;
/// Default largest ratio of extracted size to archive size.
pub const DEFAULT_MAX_RATIO: u64 = 1_000;
/// Default extracted size below which the ratio is not checked: 1 GiB.
/// Small archives of repetitive data reach high ratios without risk.
pub const DEFAULT_RATIO_FLOOR: u64 = 1 << 30;

/// Limits that stop extraction of a decompression bomb with `E_LIMIT`.
///
/// The limits apply to the sizes the members declare. [`stream`] fails
/// when a member yields more bytes than it declares, so the declared total
/// bounds the bytes written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Largest total size of the extracted regular files in bytes.
    pub max_size: u64,
    /// Largest ratio of the extracted size to the size of the archive file.
    pub max_ratio: u64,
    /// Extracted size in bytes up to which the ratio is not checked.
    pub ratio_floor: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_size: DEFAULT_MAX_SIZE,
            max_ratio: DEFAULT_MAX_RATIO,
            ratio_floor: DEFAULT_RATIO_FLOOR,
        }
    }
}

impl Limits {
    fn check(&self, total: u64, archive_size: u64) -> Result<(), ArchiveError> {
        if total > self.max_size {
            return Err(ArchiveError::Limit {
                limit: LimitKind::Size,
                allowed: self.max_size,
                actual: total,
            });
        }
        let ratio = total / archive_size.max(1);
        if total > self.ratio_floor && ratio > self.max_ratio {
            return Err(ArchiveError::Limit {
                limit: LimitKind::Ratio,
                allowed: self.max_ratio,
                actual: ratio,
            });
        }
        Ok(())
    }
}

/// Member that will be extracted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedEntry {
    /// Position of the member in the archive, counted from 0.
    pub ordinal: u64,
    /// The member as stored.
    pub member: Member,
    /// Destination relative to `into`, with `/` separators and without
    /// `.`, `..` or empty components. Only when [`PlannedEntry::waived`] is
    /// set may it contain `..` or start with `/`.
    pub dest: String,
    /// For a hard link, the destination of the file it links to.
    pub hard_link_to: Option<String>,
    /// Violation ignored because of `allow_outside: true`.
    pub waived: Option<Violation>,
}

/// Member that is not extracted because it would leave the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    /// Position of the member in the archive, counted from 0.
    pub ordinal: u64,
    /// Path of the member inside the archive.
    pub path: String,
    /// Reason.
    pub violation: Violation,
}

/// Result of [`plan`]: what extraction will write, before anything is
/// written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractPlan {
    /// Format of the archive.
    pub format: Format,
    /// Members to extract in archive order, one per destination.
    pub entries: Vec<PlannedEntry>,
    /// Members rejected by the containment checks, in archive order; they
    /// are reported with the notable `archive_path_escape`.
    pub rejected: Vec<Rejected>,
    /// Paths of devices, FIFOs and other special files, which are never
    /// extracted.
    pub special: Vec<String>,
    /// Number of members not selected by `only`, removed entirely by
    /// `strip_components`, naming the destination itself, or replaced by a
    /// later member with the same destination.
    pub skipped: u64,
    /// Total declared size of the regular files in [`ExtractPlan::entries`].
    pub total_size: u64,
}

/// Plans the extraction of an archive.
///
/// `rules.only` receives [`Member::path`]; the `only` glob of
/// `archive.extract` is compiled into it by the caller.
///
/// # Errors
///
/// * [`ArchiveError::Format`] or [`ArchiveError::Unsupported`] when the
///   archive cannot be read;
/// * [`ArchiveError::Encrypted`] when a selected regular file is
///   encrypted;
/// * [`ArchiveError::Limit`] when the declared sizes exceed `limits`;
/// * [`ArchiveError::Io`] when the reader fails.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
/// use nurt_archive::create::{create, CreateOptions, Input};
/// use nurt_archive::extract::{plan, Limits};
/// use nurt_archive::list::Format;
/// use nurt_archive::safety::{Rules, Violation};
///
/// let inputs = vec![Input::directory("pkg"), Input::symlink("pkg/up", "../..")];
/// let archive = create(Vec::new(), &CreateOptions::new(Format::Tar(None)), inputs, |_| {
///     Ok(Box::new(std::io::empty()))
/// })
/// .unwrap();
///
/// let plan = plan(Cursor::new(archive), Rules::default(), &Limits::default()).unwrap();
/// assert_eq!(plan.entries.len(), 1);
/// assert_eq!(plan.rejected[0].violation, Violation::LinkOutside);
/// ```
pub fn plan<R: Read + Seek + Send>(
    mut reader: R,
    rules: Rules<'_>,
    limits: &Limits,
) -> Result<ExtractPlan, ArchiveError> {
    let archive_size = reader.seek(SeekFrom::End(0))?;
    let mut guard = Guard::new(rules);
    let mut slots: Vec<Option<PlannedEntry>> = Vec::new();
    let mut by_dest: HashMap<String, usize> = HashMap::new();
    let mut rejected = Vec::new();
    let mut special = Vec::new();
    let mut skipped = 0_u64;
    let mut total_size = 0_u64;
    let mut ordinal = 0_u64;
    let format = list::visit(reader, &mut |member, _body| {
        let current = ordinal;
        ordinal += 1;
        match guard.check(member) {
            Verdict::Skip(SkipReason::Special) => special.push(member.path.clone()),
            Verdict::Skip(_) => skipped += 1,
            Verdict::Reject(violation) => rejected.push(Rejected {
                ordinal: current,
                path: member.path.clone(),
                violation,
            }),
            Verdict::Extract {
                dest,
                hard_link_to,
                waived,
            } => {
                if member.kind == MemberKind::File {
                    if member.encrypted {
                        return Err(ArchiveError::Encrypted {
                            path: member.path.clone(),
                        });
                    }
                    total_size = total_size.saturating_add(member.size);
                }
                if let Some(previous) = by_dest.insert(dest.clone(), slots.len())
                    && let Some(replaced) = slots[previous].take()
                {
                    skipped += 1;
                    if replaced.member.kind == MemberKind::File {
                        total_size -= replaced.member.size;
                    }
                }
                limits.check(total_size, archive_size)?;
                slots.push(Some(PlannedEntry {
                    ordinal: current,
                    member: member.clone(),
                    dest,
                    hard_link_to,
                    waived,
                }));
            }
        }
        Ok(Flow::Continue)
    })?;
    let mut entries = Vec::with_capacity(slots.len());
    for entry in slots.into_iter().flatten() {
        let dangling = entry.waived.is_none()
            && entry
                .hard_link_to
                .as_deref()
                .is_some_and(|target| !guard.is_file(target));
        if dangling {
            rejected.push(Rejected {
                ordinal: entry.ordinal,
                path: entry.member.path,
                violation: Violation::HardLinkOutside,
            });
        } else {
            entries.push(entry);
        }
    }
    rejected.sort_by_key(|rejected| rejected.ordinal);
    Ok(ExtractPlan {
        format,
        entries,
        rejected,
        special,
        skipped,
        total_size,
    })
}

/// Reads the archive again and passes the content of every planned regular
/// file to `sink`, in archive order.
///
/// The reader given to `sink` yields exactly the declared size of the
/// member; content the sink leaves unread is read and verified after it
/// returns. Reading stops after the last planned file.
///
/// # Errors
///
/// * [`ArchiveError::Changed`] when the archive no longer matches the plan;
/// * [`ArchiveError::Format`] when a member is damaged or yields a
///   different number of bytes than it declares;
/// * [`ArchiveError::Io`] when the reader fails;
/// * any error returned by `sink`, which ends the stream.
///
/// # Examples
///
/// ```
/// use std::io::{Cursor, Read};
/// use nurt_archive::create::{create, CreateOptions, Input};
/// use nurt_archive::extract::{plan, stream, Limits};
/// use nurt_archive::list::Format;
/// use nurt_archive::safety::Rules;
///
/// let archive = create(Vec::new(), &CreateOptions::new(Format::Zip), vec![Input::file("a.txt", 5)], |_| {
///     Ok(Box::new(&b"hello"[..]))
/// })
/// .unwrap();
/// let plan = plan(Cursor::new(&archive), Rules::default(), &Limits::default()).unwrap();
///
/// let mut text = String::new();
/// stream(Cursor::new(&archive), &plan, |entry, content| {
///     assert_eq!(entry.dest, "a.txt");
///     content.read_to_string(&mut text)?;
///     Ok(())
/// })
/// .unwrap();
/// assert_eq!(text, "hello");
/// ```
pub fn stream<R, F>(reader: R, plan: &ExtractPlan, mut sink: F) -> Result<(), ArchiveError>
where
    R: Read + Seek + Send,
    F: FnMut(&PlannedEntry, &mut dyn Read) -> Result<(), ArchiveError>,
{
    let mut files = plan
        .entries
        .iter()
        .filter(|entry| entry.member.kind == MemberKind::File)
        .peekable();
    if files.peek().is_none() {
        return Ok(());
    }
    let mut ordinal = 0_u64;
    let format = list::visit(reader, &mut |member, body| {
        let current = ordinal;
        ordinal += 1;
        let Some(entry) = files.next_if(|entry| entry.ordinal == current) else {
            return Ok(Flow::Continue);
        };
        if member.path != entry.member.path || member.size != entry.member.size {
            return Err(ArchiveError::Changed {
                path: entry.member.path.clone(),
                detail: format!(
                    "member {current} is now {} of {} bytes",
                    member.path, member.size
                ),
            });
        }
        let format = plan.format;
        let mut content = ExactReader::new(body.open()?, member.size);
        let result = sink(entry, &mut content)
            .and_then(|()| Ok(io::copy(&mut content, &mut io::sink()).map(drop)?));
        if let Some(detail) = content.mismatch {
            return Err(ArchiveError::format(
                Some(format),
                format!("member {}: {detail}", member.path),
            ));
        }
        result.map_err(|error| match error {
            ArchiveError::Io(error) if content.read_failed => {
                ArchiveError::from_read(format, error)
            }
            other => other,
        })?;
        Ok(if files.peek().is_some() {
            Flow::Continue
        } else {
            Flow::Stop
        })
    })?;
    if format != plan.format {
        return Err(ArchiveError::Changed {
            path: String::new(),
            detail: format!("the archive is now {format}, planned as {}", plan.format),
        });
    }
    match files.next() {
        Some(entry) => Err(ArchiveError::Changed {
            path: entry.member.path.clone(),
            detail: "the archive ends before this member".to_owned(),
        }),
        None => Ok(()),
    }
}
