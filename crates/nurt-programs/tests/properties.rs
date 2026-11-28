// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-programs/tests/properties.rs
// Responsibility:   Property tests of the parsers of package databases, manual pages and desktop entries.
// Dependencies:     proptest, support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Property tests of the parsers of package databases, manual pages and
//! desktop entries: arbitrary input never panics, and well-formed input is
//! read back unchanged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code fails loudly on broken fixtures"
)]

mod support;

use nurt_programs::sources::desktop::{DesktopEntry, exec_command};
use nurt_programs::sources::man::{
    parse_name_section, parse_whatis, split_name_line, strip_escapes,
};
use nurt_programs::sources::rpm::header::{
    Header, TAG_BASENAMES, TAG_DESCRIPTION, TAG_DIR_INDEXES, TAG_DIRNAMES, TAG_EPOCH, TAG_NAME,
    TAG_RELEASE, TAG_SUMMARY, TAG_VERSION,
};
use nurt_programs::sources::rpm::{ndb, package_from_header};
use nurt_programs::sources::{Compression, decompress, dpkg, pacman};
use nurt_programs::text::{clip, decode, edit_distance, fold, query_words};
use proptest::prelude::*;
use support::HeaderBuilder;

/// Largest output requested from a decoder in these tests.
const DECODE_LIMIT: u64 = 4096;

/// Largest difference of lengths for which `edit_distance` computes the
/// distance instead of returning its far value.
const COMPARED_LENGTH_DIFFERENCE: usize = 3;

/// Block of combining diacritical marks.
const COMBINING_MARKS: std::ops::RangeInclusive<char> = '\u{300}'..='\u{36f}';

/// Tags read by the package reader, all queried on every parsed header.
const READ_TAGS: &[u32] = &[
    TAG_NAME,
    TAG_VERSION,
    TAG_RELEASE,
    TAG_EPOCH,
    TAG_SUMMARY,
    TAG_DESCRIPTION,
    TAG_DIRNAMES,
    TAG_BASENAMES,
    TAG_DIR_INDEXES,
];

fn query_header(header: &Header) {
    for tag in READ_TAGS {
        let _ = header.strings(*tag);
        let _ = header.string(*tag);
        let _ = header.integers(*tag);
        let _ = header.integer(*tag);
        let _ = header.localized(*tag);
    }
    let _ = header.file_paths();
    let _ = header.full_version();
    let _ = package_from_header(header);
}

/// Header with arbitrary index entries over an arbitrary data store.
fn raw_header(entries: &[(u32, u32, u32, u32)], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(u32::try_from(entries.len()).unwrap().to_be_bytes());
    out.extend(u32::try_from(data.len()).unwrap().to_be_bytes());
    for (tag, kind, offset, count) in entries {
        for value in [tag, kind, offset, count] {
            out.extend(value.to_be_bytes());
        }
    }
    out.extend(data);
    out
}

fn entry() -> impl Strategy<Value = (u32, u32, u32, u32)> {
    (
        prop::sample::select(READ_TAGS.to_vec()),
        0u32..12,
        0u32..96,
        0u32..48,
    )
}

proptest! {
    #[test]
    fn rpm_header_arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        if let Ok(header) = Header::parse(&bytes) {
            query_header(&header);
        }
    }

    #[test]
    fn rpm_header_arbitrary_entries_never_panic(
        entries in prop::collection::vec(entry(), 0..12),
        data in prop::collection::vec(any::<u8>(), 0..128),
    ) {
        if let Ok(header) = Header::parse(&raw_header(&entries, &data)) {
            query_header(&header);
        }
    }

    #[test]
    fn rpm_header_round_trip(
        name in "[a-z][a-z0-9+._-]{0,30}",
        version in "[0-9]{1,3}(\\.[0-9]{1,3}){0,2}",
        release in "[0-9]{1,2}\\.[a-z0-9]{1,6}",
        files in prop::collection::vec("[a-z][a-z0-9-]{0,12}", 1..6),
    ) {
        let names: Vec<&str> = files.iter().map(String::as_str).collect();
        let indexes = vec![0; names.len()];
        let blob = HeaderBuilder::default()
            .strings(TAG_NAME, HeaderBuilder::STRING, &[&name])
            .strings(TAG_VERSION, HeaderBuilder::STRING, &[&version])
            .strings(TAG_RELEASE, HeaderBuilder::STRING, &[&release])
            .strings(TAG_DIRNAMES, HeaderBuilder::STRING_ARRAY, &["/usr/bin/"])
            .strings(TAG_BASENAMES, HeaderBuilder::STRING_ARRAY, &names)
            .int32(TAG_DIR_INDEXES, &indexes)
            .build();
        let header = Header::parse(&blob).unwrap();
        let package = package_from_header(&header).unwrap();
        prop_assert_eq!(package.name, name);
        prop_assert_eq!(package.version, Some(format!("{version}-{release}")));
        let expected: Vec<String> = files.iter().map(|file| format!("/usr/bin/{file}")).collect();
        prop_assert_eq!(package.executables, expected);
    }

    #[test]
    fn ndb_arbitrary_bytes_never_panic(
        body in prop::collection::vec(any::<u8>(), 0..2048),
        with_magic in any::<bool>(),
    ) {
        let mut bytes = Vec::new();
        if with_magic {
            bytes.extend(ndb::PKGDB_MAGIC.to_le_bytes());
            bytes.extend(ndb::PKGDB_VERSION.to_le_bytes());
        }
        bytes.extend(body);
        let _ = ndb::read(&bytes);
    }

    #[test]
    fn dpkg_status_arbitrary_text_never_panics(text in "\\PC{0,400}") {
        for stanza in dpkg::parse_stanzas(&text) {
            let _ = dpkg::package_from_stanza(&stanza);
            let _ = stanza.get("Status").map(dpkg::is_installed);
        }
        let _ = dpkg::list_executables(&text);
    }

    #[test]
    fn dpkg_status_round_trip(
        name in "[a-z0-9][a-z0-9+.-]{1,30}",
        version in "[0-9]{1,3}\\.[0-9]{1,3}-[0-9]{1,2}",
        summary in "[A-Za-z][A-Za-z0-9 ]{0,40}[A-Za-z0-9]",
        lines in prop::collection::vec("[A-Za-z][A-Za-z0-9 ,]{0,40}", 0..4),
    ) {
        let mut text = format!("Package: {name}\nStatus: install ok installed\nVersion: {version}\nDescription: {summary}\n");
        for line in &lines {
            text.push_str(&format!(" {line}\n"));
        }
        let stanzas = dpkg::parse_stanzas(&format!("{text}\n"));
        prop_assert_eq!(stanzas.len(), 1);
        let package = dpkg::package_from_stanza(&stanzas[0]).unwrap();
        prop_assert_eq!(package.name, name);
        prop_assert_eq!(package.version, Some(version));
        prop_assert_eq!(package.summary.unwrap().text, summary);
    }

    #[test]
    fn pacman_arbitrary_text_never_panics(text in "\\PC{0,400}") {
        let _ = pacman::parse_sections(&text);
        let _ = pacman::package_from_desc(&text);
        let _ = pacman::executables_from_files(&text);
        let _ = pacman::db_path_from_conf(&text);
    }

    #[test]
    fn pacman_desc_round_trip(
        name in "[a-z0-9][a-z0-9@._+-]{0,30}",
        version in "[0-9]{1,3}\\.[0-9]{1,3}-[0-9]{1,2}",
        description in "[A-Za-z][A-Za-z0-9 ]{0,40}",
    ) {
        let text = format!("%NAME%\n{name}\n\n%VERSION%\n{version}\n\n%DESC%\n{description}\n\n");
        let package = pacman::package_from_desc(&text).unwrap();
        prop_assert_eq!(package.name, name);
        prop_assert_eq!(package.version, Some(version));
        prop_assert_eq!(package.summary.unwrap().text, description.trim_end());
    }

    #[test]
    fn man_arbitrary_page_never_panics(body in "\\PC{0,400}") {
        let _ = parse_name_section(&format!(".TH X 1\n.SH NAME\n{body}"));
        let _ = parse_name_section(&format!(".Sh NAME\n.Nm {body}\n.Nd {body}\n"));
        let _ = parse_name_section(&body);
        let _ = strip_escapes(&body);
        let _ = split_name_line(&body);
        let _ = parse_whatis(&body);
    }

    #[test]
    fn man_name_round_trip(
        names in prop::collection::vec("[a-z][a-z0-9_-]{0,12}", 1..4),
        description in "[a-z]{1,10}( [a-z]{1,10}){0,6}",
    ) {
        let page = format!(".TH X 1\n.SH NAME\n{} \\- {description}\n.SH SYNOPSIS\n", names.join(", "));
        let parsed = parse_name_section(&page).unwrap();
        prop_assert_eq!(parsed.names, names);
        prop_assert_eq!(parsed.description, description);
    }

    #[test]
    fn desktop_arbitrary_entry_never_panics(text in "\\PC{0,400}") {
        let entry = DesktopEntry::parse(&format!("[Desktop Entry]\n{text}"));
        let _ = entry.localized("Name");
        let _ = entry.list("Categories");
        let _ = entry.is_visible_application();
        let _ = exec_command(&text);
    }

    #[test]
    fn decoders_reject_or_bound_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        for compression in [
            Compression::None,
            Compression::Gzip,
            Compression::Xz,
            Compression::Bzip2,
            Compression::Zstd,
            Compression::Lz4,
        ] {
            if let Ok(out) = decompress(&bytes, compression, DECODE_LIMIT) {
                prop_assert!(out.len() as u64 <= DECODE_LIMIT);
            }
        }
        let _ = decode(&bytes);
    }

    #[test]
    fn fold_removes_combining_marks(text in "\\PC{0,80}") {
        let folded = fold(&text);
        let marks = folded.chars().filter(|c| COMBINING_MARKS.contains(c)).count();
        prop_assert_eq!(marks, 0);
        prop_assert_eq!(fold(&text.to_ascii_uppercase()).to_ascii_lowercase(), fold(&text).to_ascii_lowercase());
        let _ = query_words(&text, true);
    }

    #[test]
    fn edit_distance_is_symmetric_and_bounded(left in "\\PC{0,16}", right in "\\PC{0,16}") {
        let distance = edit_distance(&left, &right);
        prop_assert_eq!(distance, edit_distance(&right, &left));
        let (left_chars, right_chars) = (left.chars().count(), right.chars().count());
        if left_chars.abs_diff(right_chars) <= COMPARED_LENGTH_DIFFERENCE {
            prop_assert!(distance <= left_chars.max(right_chars));
        }
        prop_assert_eq!(edit_distance(&left, &left), 0);
    }

    #[test]
    fn clip_never_exceeds_limit(text in "\\PC{0,120}", limit in 2usize..100) {
        prop_assert!(clip(&text, limit).chars().count() <= limit);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
