// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::ini::format
// Responsibility:   Registered format `ini`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Registered format `ini`.
//!
//! A document opened without a profile gets the profile its content
//! implies: `[Desktop Entry]` selects `desktop`, systemd unit sections
//! `systemd`, quoted subsections or a `[core]` section `gitconfig`, a
//! `[PHP]` section `php`, glob sections or a leading `root` key
//! `editorconfig`; any other content is `generic`.

use std::path::Path;

use nurt_contracts::error::Result;
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;

use super::support::flat::block_range;
use super::support::handle::{Backend, KeyValueHandle, build_tree, reparsed, utf8_text};
use super::support::tree::Node;
use super::{IniDocument, IniProfile, collect_lines};
use crate::model::{
    DocHandle, DocTree, FormatSemantics, NodeId, NodeKind, NodeSpec, Repeated, Span, doc_parse,
};
use crate::registry::{Confidence, DocFormat, DocOptions};

/// File name extensions of the format: generic INI files, systemd units
/// and desktop entries.
const FORMAT_EXTENSIONS: [&str; 19] = [
    "ini",
    "cfg",
    "service",
    "socket",
    "device",
    "mount",
    "automount",
    "swap",
    "target",
    "path",
    "timer",
    "slice",
    "scope",
    "network",
    "netdev",
    "link",
    "nspawn",
    "desktop",
    "directory",
];

/// File name patterns of Git configuration, EditorConfig and PHP files.
const FORMAT_FILE_NAMES: [&str; 5] = [
    ".gitconfig",
    "gitconfig",
    ".git/config",
    ".editorconfig",
    "php*.ini",
];

/// Section names of systemd unit and network files.
const SYSTEMD_SECTIONS: [&str; 18] = [
    "Unit",
    "Service",
    "Install",
    "Socket",
    "Mount",
    "Automount",
    "Swap",
    "Path",
    "Timer",
    "Slice",
    "Scope",
    "Match",
    "Link",
    "Network",
    "NetDev",
    "Address",
    "Route",
    "DHCPv4",
];

/// Lines read when sniffing.
const SNIFF_LINES: usize = 64;

/// Key path rules of INI: a key written several times is selected by
/// index. Section names and key names compare without regard to ASCII case
/// in the profiles that fold them; names of subsections are exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IniSemantics {
    fold_sections: bool,
    fold_keys: bool,
}

impl FormatSemantics for IniSemantics {
    fn repeated(&self) -> Repeated {
        Repeated::Conflict
    }

    fn name_matches(&self, tree: &DocTree, node: NodeId, name: &str) -> bool {
        let Some(own) = tree[node].name.as_deref() else {
            return false;
        };
        if own == name {
            return true;
        }
        let fold = if tree[node].kind == NodeKind::Map {
            let section = tree[node]
                .parent
                .is_some_and(|parent| tree[parent].parent.is_none());
            section && self.fold_sections
        } else {
            self.fold_keys
        };
        fold && own.eq_ignore_ascii_case(name)
    }
}

static EXACT: IniSemantics = IniSemantics {
    fold_sections: false,
    fold_keys: false,
};
static FOLDED_KEYS: IniSemantics = IniSemantics {
    fold_sections: false,
    fold_keys: true,
};
static FOLDED_SECTIONS: IniSemantics = IniSemantics {
    fold_sections: true,
    fold_keys: false,
};
static FOLDED: IniSemantics = IniSemantics {
    fold_sections: true,
    fold_keys: true,
};

/// The format `ini`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::ini::IniFormat;
/// use nurt_doc::model::SetOptions;
/// use nurt_doc::registry::{DocFormat, DocOptions};
///
/// let text = b"[Service]\nExecStart=/bin/a\nExecStart=/bin/b\n".to_vec();
/// let mut handle = IniFormat::new().open(text, &DocOptions::default())?;
/// let key = KeyPath::parse("Service.ExecStart[1]")?;
/// handle.set(&key, &Value::Text("/bin/c".to_owned()), &SetOptions::default())?;
/// assert_eq!(handle.text(), b"[Service]\nExecStart=/bin/a\nExecStart=/bin/c\n");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct IniFormat {
    profile: Option<IniProfile>,
}

impl IniFormat {
    /// Format that selects the profile of each document from its content.
    #[must_use]
    pub const fn new() -> Self {
        Self { profile: None }
    }

    /// Format that reads every document with `profile`.
    #[must_use]
    pub const fn with_profile(profile: IniProfile) -> Self {
        Self {
            profile: Some(profile),
        }
    }
}

impl DocFormat for IniFormat {
    fn id(&self) -> DocFormatId {
        DocFormatId::INI
    }

    fn extensions(&self) -> &[&'static str] {
        &FORMAT_EXTENSIONS
    }

    fn file_names(&self) -> &[&'static str] {
        &FORMAT_FILE_NAMES
    }

    /// Section headers with key lines are probably an INI document; key
    /// lines alone could be one.
    fn sniff(&self, content: &[u8]) -> Confidence {
        let Ok(text) = std::str::from_utf8(content) else {
            return Confidence::None;
        };
        let mut header = false;
        let mut pairs = false;
        for line in text.lines().take(SNIFF_LINES).map(str::trim) {
            if line.is_empty() || line.starts_with([';', '#']) {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                header = true;
            } else if line.contains(['=', ':']) {
                pairs = true;
            } else {
                return Confidence::None;
            }
        }
        match (header, pairs) {
            (true, true) => Confidence::Medium,
            (false, true) => Confidence::Low,
            _ => Confidence::None,
        }
    }

    fn open(&self, content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        let text = utf8_text(content, &DocFormatId::INI)?;
        let profile = self.profile.unwrap_or_else(|| detect_profile(&text));
        open_document(&text, profile)
    }

    /// The profile of a format opened without one follows the file name when
    /// the name implies a dialect, and the content otherwise.
    fn open_at(
        &self,
        path: Option<&Path>,
        content: Vec<u8>,
        _options: &DocOptions,
    ) -> Result<Box<dyn DocHandle>> {
        let text = utf8_text(content, &DocFormatId::INI)?;
        let profile = self
            .profile
            .or_else(|| {
                path.and_then(Path::to_str)
                    .and_then(IniProfile::from_file_name)
            })
            .unwrap_or_else(|| detect_profile(&text));
        open_document(&text, profile)
    }
}

fn open_document(text: &str, profile: IniProfile) -> Result<Box<dyn DocHandle>> {
    Ok(Box::new(KeyValueHandle::new(IniDocument::parse(
        text, profile,
    )?)))
}

/// Profile implied by the sections and the first key of `text`.
fn detect_profile(text: &str) -> IniProfile {
    let headers: Vec<&str> = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix('[')?.split(']').next())
        .collect();
    let any = |test: &dyn Fn(&str) -> bool| headers.iter().any(|header| test(header));
    if any(&|h| h == "Desktop Entry" || h.starts_with("Desktop Action ")) {
        IniProfile::Desktop
    } else if any(&|h| SYSTEMD_SECTIONS.contains(&h)) {
        IniProfile::Systemd
    } else if any(&|h| h.contains(" \"") || h.eq_ignore_ascii_case("core")) {
        IniProfile::Gitconfig
    } else if any(&|h| h == "PHP") {
        IniProfile::Php
    } else if any(&|h| h.contains(['*', '{', '/'])) || starts_with_root_key(text) {
        IniProfile::Editorconfig
    } else {
        IniProfile::Generic
    }
}

/// The first key line of `text` is `root = …`, as in EditorConfig files.
fn starts_with_root_key(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with([';', '#']))
        .and_then(|line| line.split_once('='))
        .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case("root"))
}

impl Backend for IniDocument {
    fn format(&self) -> DocFormatId {
        DocFormatId::INI
    }

    fn bytes(&self) -> Vec<u8> {
        self.text.as_bytes().to_vec()
    }

    fn tree(&self, bytes: &[u8]) -> DocTree {
        build_tree(bytes, &self.tree, &|node, path| self.describe(node, path))
    }

    fn semantics(&self) -> &'static dyn FormatSemantics {
        let rules = self.profile.rules();
        match (rules.fold_section_names, rules.fold_key_names) {
            (true, true) => &FOLDED,
            (true, false) => &FOLDED_SECTIONS,
            (false, true) => &FOLDED_KEYS,
            (false, false) => &EXACT,
        }
    }

    fn set(&mut self, key: &KeyPath, value: &Value, create: bool) -> Result<usize> {
        Self::set(self, key, value, create)
    }

    fn remove(&mut self, key: &KeyPath) -> Result<usize> {
        Self::remove(self, key)
    }

    fn reference(&self, bytes: &[u8]) -> Result<Vec<Value>> {
        let text = std::str::from_utf8(bytes).map_err(|error| {
            doc_parse(
                &DocFormatId::INI,
                bytes,
                Some(error.valid_up_to()),
                "the text is not valid UTF-8",
            )
        })?;
        reparsed(Self::parse(text, self.profile))
    }

    fn text_values(&self) -> bool {
        true
    }
}

impl IniDocument {
    /// Spans, value and comment of a node: an entry spans its lines with
    /// the comment lines directly above, a section or a repeated key the
    /// lines from the first to the last of its headers and entries.
    fn describe(&self, node: &Node, path: &[KeySegment]) -> NodeSpec {
        let parsed = &self.parsed;
        let whole = Span::new(0, self.text.len());
        if path.is_empty() {
            return NodeSpec::new(NodeKind::Map, whole);
        }
        if let Node::Leaf(id) = node {
            let entry = &parsed.entries[*id];
            let block = block_range(
                &parsed.lines,
                &parsed.comment_lines,
                entry.first_line,
                entry.last_line,
                0,
            );
            let mut spec = NodeSpec::new(NodeKind::Scalar, span(&block))
                .with_key_span(span(&entry.name_range))
                .with_value_span(span(&entry.value_range))
                .with_value(self.entry_value(*id));
            if let Some(comment) = &entry.comment {
                spec = spec.with_comment(comment.clone());
            }
            return spec;
        }
        let mut lines = Vec::new();
        collect_lines(parsed, node, &mut lines);
        let (Some(&first), Some(&last)) = (lines.iter().min(), lines.iter().max()) else {
            return NodeSpec::new(NodeKind::Map, whole);
        };
        let block = block_range(&parsed.lines, &parsed.comment_lines, first, last, 0);
        let mut spec = NodeSpec::new(NodeKind::Map, span(&block)).with_value_span(Span::new(
            parsed.lines[first].start,
            parsed.lines[last].content_end,
        ));
        if let Node::Map(map) = node
            && let Some(comment) = map
                .ids
                .first()
                .and_then(|&section| parsed.sections[section].comment.clone())
        {
            spec = spec.with_comment(comment);
        }
        spec
    }
}

fn span(range: &std::ops::Range<usize>) -> Span {
    Span::new(range.start, range.end)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;

    use super::*;
    use crate::model::SetOptions;

    const CREATE: SetOptions = SetOptions { create: true };

    fn open(text: &str) -> Box<dyn DocHandle> {
        IniFormat::new()
            .open(text.as_bytes().to_vec(), &DocOptions::default())
            .unwrap()
    }

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    #[test]
    fn detects_profiles_from_content() {
        assert_eq!(
            detect_profile("[Desktop Entry]\nName=a\n"),
            IniProfile::Desktop
        );
        assert_eq!(
            detect_profile("[Unit]\nDescription=x\n"),
            IniProfile::Systemd
        );
        assert_eq!(
            detect_profile("[remote \"origin\"]\nurl = x\n"),
            IniProfile::Gitconfig
        );
        assert_eq!(detect_profile("[PHP]\nengine = On\n"), IniProfile::Php);
        assert_eq!(
            detect_profile("root = true\n[x]\n"),
            IniProfile::Editorconfig
        );
        assert_eq!(
            detect_profile("[*.md]\nindent_size = 2\n"),
            IniProfile::Editorconfig
        );
        assert_eq!(detect_profile("[db]\nhost = x\n"), IniProfile::Generic);
    }

    #[test]
    fn sets_numbers_as_text_and_creates_keys_in_their_section() {
        let mut handle = open("; db\n[db]\nport = 80\n\n[web]\nhost = a\n");
        handle
            .set(&key("db.port"), &Value::Int(8080), &CREATE)
            .unwrap();
        let report = handle
            .set(&key("db.user"), &Value::Text("me".into()), &CREATE)
            .unwrap();
        assert_eq!(report.flags, ["keys_created"]);
        let value = handle.to_value().unwrap();
        assert_eq!(
            value.get("db").and_then(|db| db.get("port")),
            Some(&Value::Text("8080".into()))
        );
        assert!(String::from_utf8_lossy(handle.text()).contains("port = 8080\n"));
        let db = handle.get(&key("db")).unwrap();
        assert_eq!(handle.tree()[db[0].node].comment.as_deref(), Some("db"));
    }

    #[test]
    fn repeated_keys_are_selected_by_index() {
        let mut handle = open("[s]\nk = 1\nk = 2\n");
        let error = handle.get(&key("s.k")).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict);
        handle
            .set(&key("s.k[1]"), &Value::Text("3".into()), &CREATE)
            .unwrap();
        assert_eq!(handle.text(), b"[s]\nk = 1\nk = 3\n");
    }

    #[test]
    fn folded_profiles_match_names_without_case() {
        let mut handle = open("[Core]\n\tBare = false\n");
        let found = handle.get(&key("core.bare")).unwrap();
        assert_eq!(found.len(), 1);
        handle.remove(&key("core.bare")).unwrap();
        assert_eq!(handle.text(), b"[Core]\n");
    }

    #[test]
    fn section_maps_match_key_names_without_case_in_folded_profiles() {
        let mut document =
            IniDocument::parse("[core]\n\tbare = false\n", IniProfile::Gitconfig).unwrap();
        let fields = [("BARE".to_owned(), Value::Text("true".into()))]
            .into_iter()
            .collect();
        document
            .set(&key("core"), &Value::Map(fields), false)
            .unwrap();
        assert_eq!(document.text(), "[core]\n\tbare = true\n");
    }

    #[test]
    fn sniffs_sections_and_key_lines() {
        let format = IniFormat::new();
        assert_eq!(format.sniff(b"; c\n[a]\nb = 1\n"), Confidence::Medium);
        assert_eq!(format.sniff(b"b: 1\n"), Confidence::Low);
        assert_eq!(format.sniff(b"plain words\n"), Confidence::None);
        assert_eq!(format.sniff(&[0xFF]), Confidence::None);
        assert!(format.open(vec![0xFF], &DocOptions::default()).is_err());
        let fixed = IniFormat::with_profile(IniProfile::Generic);
        assert!(
            fixed
                .open(b"[Unit]\na=1\n".to_vec(), &DocOptions::default())
                .is_ok()
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
