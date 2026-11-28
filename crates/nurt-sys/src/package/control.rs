// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::package::control
// Responsibility:   Reader of RFC 822 style control files (`Packages`, dpkg `status`).
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reader of RFC 822 style control files (`Packages`, dpkg `status`).

use std::io::{self, BufRead};

/// Fields kept from each stanza.
const KEPT: &[&str] = &[
    "Package",
    "Version",
    "Architecture",
    "Description",
    "Installed-Size",
    "Status",
    "Phased-Update-Percentage",
];

/// One stanza: the kept fields in file order.
#[derive(Debug, Default)]
pub(crate) struct Stanza {
    fields: Vec<(&'static str, String)>,
}

impl Stanza {
    /// Value of the field `name`.
    #[must_use]
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// Calls `visit` for each stanza of `reader`.
///
/// Continuation lines are dropped, so a multi-line `Description` keeps its
/// synopsis only.
///
/// # Errors
///
/// Returns the error of the reader, including invalid UTF-8 content.
pub(crate) fn for_each_stanza<R: BufRead>(
    mut reader: R,
    mut visit: impl FnMut(&Stanza),
) -> io::Result<()> {
    let mut stanza = Stanza::default();
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        let text = line.trim_end_matches(['\n', '\r']);
        if read == 0 || text.trim().is_empty() {
            if !stanza.is_empty() {
                visit(&stanza);
                stanza = Stanza::default();
            }
            if read == 0 {
                return Ok(());
            }
            continue;
        }
        if text.starts_with([' ', '\t']) {
            continue;
        }
        if let Some((key, value)) = text.split_once(':')
            && let Some(kept) = KEPT.iter().find(|kept| kept.eq_ignore_ascii_case(key))
        {
            stanza.fields.push((kept, value.trim().to_owned()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_stanzas_and_drops_continuation_lines() {
        let text = "Package: a\nVersion: 1\nDescription: first\n second line\nOther: x\n\n\nPackage: b\nVersion: 2";
        let mut seen = Vec::new();
        for_each_stanza(text.as_bytes(), |stanza| {
            seen.push((
                stanza.get("package").map(str::to_owned),
                stanza.get("Description").map(str::to_owned),
            ));
        })
        .expect("stanzas");
        assert_eq!(
            seen,
            [
                (Some("a".to_owned()), Some("first".to_owned())),
                (Some("b".to_owned()), None)
            ]
        );
    }

    #[test]
    fn reports_invalid_utf8_as_error() {
        let bytes: &[u8] = b"Package: \xff\xfe\n";
        assert!(for_each_stanza(bytes, |_| {}).is_err());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
