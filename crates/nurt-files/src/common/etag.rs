// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Entity tags: text form, observed state of targets and `E_STALE`.
//!
//! Records carry the entity tag of a file as text, and programs pass it back
//! in `if_match`. The text form is
//! `<size>-<modified>-<volume>-<id low>-<id high>` in lower-case hexadecimal,
//! with the modification time in nanoseconds as its 64-bit two's complement,
//! followed by `-sha256:<hex>` when the tag holds a content hash.
//!
//! An effect stage observes every target in the plan step ([`observe`]),
//! checks `if_match` against the observation ([`check_if_match`]) and
//! declares the target with [`Observed::expect`]; the journal repeats the
//! check atomically with the change and fails with `E_STALE` when the target
//! changed in between.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, ContentHash, Etag, FileId, FileMeta, Follow, Fs, FsErrorKind};
use nurt_contracts::journal::Expect;
use serde_json::Value as Json;

const SEPARATOR: &str = "-";
const HEX_RADIX: u32 = 16;
const NUMBER_FIELDS: usize = 5;

/// Text form of an entity tag.
///
/// # Examples
///
/// ```
/// use nurt_contracts::fs::{Etag, FileId};
/// use nurt_files::common::etag::{etag_text, parse_etag};
///
/// let tag = Etag {
///     size: 255,
///     modified_ns: 16,
///     file_id: FileId { volume: 1, low: 2, high: 0 },
///     hash: None,
/// };
/// assert_eq!(etag_text(&tag), "ff-10-1-2-0");
/// assert_eq!(parse_etag("ff-10-1-2-0")?, tag);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[must_use]
pub fn etag_text(etag: &Etag) -> String {
    let modified = u64::from_ne_bytes(etag.modified_ns.to_ne_bytes());
    let mut parts: Vec<String> = [
        etag.size,
        modified,
        etag.file_id.volume,
        etag.file_id.low,
        etag.file_id.high,
    ]
    .iter()
    .map(|number| format!("{number:x}"))
    .collect();
    if let Some(hash) = &etag.hash {
        parts.push(hash.to_string());
    }
    parts.join(SEPARATOR)
}

/// Parses the text form of an entity tag.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` when the text is not an entity tag.
pub fn parse_etag(text: &str) -> Result<Etag> {
    let mut parts = text.split(SEPARATOR);
    let mut numbers = [0_u64; NUMBER_FIELDS];
    for slot in &mut numbers {
        *slot = parts
            .next()
            .and_then(parse_hex)
            .ok_or_else(|| syntax_error(text))?;
    }
    let hash = parts
        .next()
        .map(|part| ContentHash::parse(part).map_err(|_| syntax_error(text)))
        .transpose()?;
    if parts.next().is_some() {
        return Err(syntax_error(text));
    }
    let [size, modified, volume, low, high] = numbers;
    Ok(Etag {
        size,
        modified_ns: i64::from_ne_bytes(modified.to_ne_bytes()),
        file_id: FileId { volume, low, high },
        hash,
    })
}

fn parse_hex(part: &str) -> Option<u64> {
    let digits = !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_hexdigit());
    digits
        .then(|| u64::from_str_radix(part, HEX_RADIX).ok())
        .flatten()
}

fn syntax_error(text: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{text}` is not an etag: pass the `etag` field of a record read earlier"),
    )
    .with_problem("etag_syntax")
    .with_found(text)
}

/// State of a target observed in the plan step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// The target does not exist.
    Absent,
    /// The target exists; metadata of the entry itself, a link in the last
    /// component not followed.
    Present(FileMeta, Option<ContentHash>),
}

impl Observed {
    /// Returns `true` when the target exists.
    #[must_use]
    pub fn exists(&self) -> bool {
        matches!(self, Self::Present(..))
    }

    /// Metadata of an existing target.
    #[must_use]
    pub fn meta(&self) -> Option<&FileMeta> {
        match self {
            Self::Absent => None,
            Self::Present(meta, _) => Some(meta),
        }
    }

    /// Entity tag of an existing target.
    #[must_use]
    pub fn etag(&self) -> Option<Etag> {
        match self {
            Self::Absent => None,
            Self::Present(meta, hash) => Some(Etag {
                hash: *hash,
                ..meta.etag()
            }),
        }
    }

    /// State the journal checks immediately before the change:
    /// [`Expect::Absent`] for a missing target, otherwise
    /// [`Expect::Etag`] with the observed tag.
    #[must_use]
    pub fn expect(&self) -> Expect {
        match self {
            Self::Absent => Expect::Absent,
            Self::Present(..) => self.etag().map_or(Expect::Absent, Expect::Etag),
        }
    }
}

/// Observes a target without following a link in its last component, as the
/// journal does when it checks [`Expect::Etag`].
///
/// # Errors
///
/// Fails with the error of the file system for any failure other than a
/// missing entry, for example `E_PERMISSION` or `E_IO`.
pub fn observe(fs: &dyn Fs, path: &AbsPath) -> Result<Observed> {
    match fs.metadata(path, Follow::NoFollow) {
        Ok(meta) => Ok(Observed::Present(meta, None)),
        Err(error) if error.kind == FsErrorKind::NotFound => Ok(Observed::Absent),
        Err(error) => Err(error.into()),
    }
}

/// Checks the tag expected by `if_match` against the observed state.
///
/// # Errors
///
/// Fails with `E_STALE` ([`stale_error`]) when a tag is expected and the
/// target is missing or has another tag.
pub fn check_if_match(path: &AbsPath, observed: &Observed, expected: Option<&Etag>) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = observed.etag();
    if actual
        .as_ref()
        .is_some_and(|actual| actual.matches(expected))
    {
        Ok(())
    } else {
        Err(stale_error(path, expected, actual.as_ref()))
    }
}

/// `E_STALE` for a target that does not have the expected tag; `actual` is
/// `None` when the target no longer exists.
#[must_use]
pub fn stale_error(path: &AbsPath, expected: &Etag, actual: Option<&Etag>) -> Error {
    let message = match actual {
        Some(_) => {
            format!("`{path}` changed since it was read: read it again and use the new etag")
        }
        None => format!("`{path}` no longer exists: read it again before changing it"),
    };
    Error::new(ErrorCode::Stale, message)
        .with_expected(Json::from(etag_text(expected)))
        .with_actual(actual.map_or(Json::Null, |tag| Json::from(etag_text(tag))))
        .with_detail("path", Json::from(path.to_language_string()))
}

#[cfg(test)]
mod tests {
    use nurt_testkit::ctx::TestContext;

    use super::*;
    use crate::common::fixture::{abs, with_file};

    fn tag(modified_ns: i64, hash: Option<ContentHash>) -> Etag {
        Etag {
            size: 1024,
            modified_ns,
            file_id: FileId {
                volume: 0xfd01,
                low: 42,
                high: 7,
            },
            hash,
        }
    }

    #[test]
    fn text_form_round_trips_negative_times_and_hashes() {
        let hash = ContentHash([0xab; 32]);
        for etag in [
            tag(-1, None),
            tag(i64::MIN, Some(hash)),
            tag(i64::MAX, None),
        ] {
            assert_eq!(parse_etag(&etag_text(&etag)).unwrap(), etag);
        }
        let text = etag_text(&tag(0, Some(hash)));
        assert!(text.ends_with(&format!("-{hash}")));
    }

    #[test]
    fn malformed_text_is_rejected() {
        for text in [
            "",
            "1-2-3-4",
            "1-2-3-4-x",
            "1-2-3--5",
            "+1-2-3-4-5",
            "1-2-3-4-5-sha256:00",
            "1-2-3-4-5-sha256:0000000000000000000000000000000000000000000000000000000000000000-6",
            "10000000000000000-0-0-0-0",
        ] {
            let error = parse_etag(text).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "{text}");
            assert_eq!(error.problem.as_deref(), Some("etag_syntax"));
        }
    }

    #[test]
    fn observation_reports_present_and_absent_targets() {
        let (test, path) = with_file("/data/a.txt", b"abc");
        let observed = observe(test.fs().as_ref(), &path).unwrap();
        assert!(observed.exists());
        assert_eq!(observed.meta().map(|meta| meta.size), Some(3));
        let tag = observed.etag().unwrap();
        assert_eq!(observed.expect(), Expect::Etag(tag));

        let missing = observe(test.fs().as_ref(), &abs("/data/b.txt")).unwrap();
        assert_eq!(missing, Observed::Absent);
        assert!(!missing.exists());
        assert_eq!(missing.etag(), None);
        assert_eq!(missing.expect(), Expect::Absent);
    }

    #[test]
    fn observation_propagates_other_failures() {
        let (test, path) = with_file("/data/a.txt", b"abc");
        let inner = path.join_name("inner").unwrap();
        let error = observe(test.fs().as_ref(), &inner).unwrap_err();
        assert_ne!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn if_match_passes_for_the_observed_tag() {
        let (test, path) = with_file("/data/a.txt", b"abc");
        let observed = observe(test.fs().as_ref(), &path).unwrap();
        let current = observed.etag().unwrap();
        check_if_match(&path, &observed, Some(&current)).unwrap();
        check_if_match(&path, &observed, None).unwrap();
        check_if_match(&path, &Observed::Absent, None).unwrap();
    }

    #[test]
    fn if_match_fails_with_stale_for_changed_or_missing_targets() {
        let test = TestContext::new();
        let path = abs("/data/a.txt");
        test.fs().write_file(&path, b"abc").unwrap();
        let before = observe(test.fs().as_ref(), &path).unwrap().etag().unwrap();
        test.fs().write_file(&path, b"abcdef").unwrap();
        let after = observe(test.fs().as_ref(), &path).unwrap();

        let changed = check_if_match(&path, &after, Some(&before)).unwrap_err();
        assert_eq!(changed.code(), ErrorCode::Stale);
        assert_eq!(changed.expected, Some(Json::from(etag_text(&before))));
        assert_eq!(
            changed.actual,
            Some(Json::from(etag_text(&after.etag().unwrap())))
        );
        assert_eq!(
            changed.details.get("path"),
            Some(&Json::from(path.to_language_string()))
        );

        let missing = check_if_match(&path, &Observed::Absent, Some(&before)).unwrap_err();
        assert_eq!(missing.code(), ErrorCode::Stale);
        assert_eq!(missing.actual, Some(Json::Null));
        assert!(missing.message.contains("no longer exists"));
    }
}
