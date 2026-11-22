// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parser of `/proc/<pid>/mountinfo` (`proc_pid_mountinfo(5)`).
//!
//! Each line holds the mount identifier, the parent identifier,
//! `major:minor`, the root, the mount point, the mount options, optional
//! fields ended by `-`, the file system type, the source and the super block
//! options. Space, tab, newline and backslash in names are written as octal
//! escapes (`\040`, `\011`, `\012`, `\134`).

/// Digits of an octal escape.
const ESCAPE_DIGITS: usize = 3;

/// One mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mount {
    pub(crate) device_number: String,
    pub(crate) mount_point: String,
    pub(crate) options: Vec<String>,
    pub(crate) fs_type: String,
    pub(crate) source: String,
    pub(crate) super_options: Vec<String>,
}

/// Decodes the octal escapes of a field.
pub(crate) fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let escape = bytes
            .get(index + 1..index + 1 + ESCAPE_DIGITS)
            .filter(|digits| byte == b'\\' && digits.iter().all(|d| (b'0'..=b'7').contains(d)))
            .and_then(|digits| u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok());
        match escape {
            Some(value) => {
                decoded.push(value);
                index += 1 + ESCAPE_DIGITS;
            }
            None => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn options(field: &str) -> Vec<String> {
    field.split(',').map(unescape).collect()
}

fn parse_line(line: &str) -> Option<Mount> {
    let fields: Vec<&str> = line.split(' ').collect();
    let separator = fields.iter().position(|field| *field == "-")?;
    let tail = fields.get(separator + 1..)?;
    Some(Mount {
        device_number: (*fields.get(2)?).to_owned(),
        mount_point: unescape(fields.get(4)?),
        options: options(fields.get(5)?),
        fs_type: unescape(tail.first()?),
        source: unescape(tail.get(1)?),
        super_options: tail.get(2).map(|field| options(field)).unwrap_or_default(),
    })
}

/// Parses the mounts of a `mountinfo` file; malformed lines are skipped.
pub(crate) fn parse(text: &str) -> Vec<Mount> {
    text.lines().filter_map(parse_line).collect()
}
