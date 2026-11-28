// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::details::acl
// Responsibility:   POSIX access control lists in the extended attribute form of Linux.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! POSIX access control lists in the extended attribute form of Linux.
//!
//! Linux keeps the access ACL of an entry in the extended attribute
//! `system.posix_acl_access`: a little-endian header holding version 2,
//! followed by one 8-byte record per entry with a 16-bit tag, 16-bit
//! permissions and a 32-bit identifier, sorted by tag and identifier. Entries
//! are written in the short text form of `getfacl`, for example
//! `user:alice:r--`.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::value::Value;

use super::accounts::Accounts;

/// Extended attribute of the access ACL.
pub(crate) const ACCESS_XATTR: &str = "system.posix_acl_access";

/// Extended attribute of the default ACL of a directory.
pub(crate) const DEFAULT_XATTR: &str = "system.posix_acl_default";

/// Version of the extended attribute form.
const VERSION: u32 = 2;

/// Length of the header in bytes.
const HEADER_LEN: usize = 4;

/// Length of one entry in bytes.
const ENTRY_LEN: usize = 8;

/// Identifier stored for entries without one.
const UNDEFINED_ID: u32 = u32::MAX;

const TAG_USER_OBJ: u16 = 0x01;
const TAG_USER: u16 = 0x02;
const TAG_GROUP_OBJ: u16 = 0x04;
const TAG_GROUP: u16 = 0x08;
const TAG_MASK: u16 = 0x10;
const TAG_OTHER: u16 = 0x20;

const READ: u8 = 4;
const WRITE: u8 = 2;
const EXECUTE: u8 = 1;
const ALL_PERMISSIONS: u8 = READ | WRITE | EXECUTE;

/// Placeholder of a missing permission in the text form.
const NO_PERMISSION: char = '-';

/// Keys of one entry of the `acl` parameter.
const TAG_KEY: &str = "tag";
const ID_KEY: &str = "id";
const PERMS_KEY: &str = "perms";

/// Class of an entry. The derived order is the order of the stored form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tag {
    /// Account of the file.
    UserObj,
    /// Named account.
    User(u32),
    /// Group of the file.
    GroupObj,
    /// Named group.
    Group(u32),
    /// Upper bound of the permissions of the group class.
    Mask,
    /// Everybody else.
    Other,
}

/// One entry of an access control list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AclEntry {
    pub(crate) tag: Tag,
    pub(crate) perms: u8,
}

/// Decodes the extended attribute form; `None` when the bytes are not an
/// ACL of version 2.
pub(crate) fn decode(bytes: &[u8]) -> Option<Vec<AclEntry>> {
    let (header, body) = bytes.split_at_checked(HEADER_LEN)?;
    let version = u32::from_le_bytes(header.try_into().ok()?);
    let (chunks, rest) = body.as_chunks::<ENTRY_LEN>();
    if version != VERSION || !rest.is_empty() {
        return None;
    }
    chunks
        .iter()
        .map(|&[t0, t1, p0, p1, i0, i1, i2, i3]| {
            let id = u32::from_le_bytes([i0, i1, i2, i3]);
            let tag = match u16::from_le_bytes([t0, t1]) {
                TAG_USER_OBJ => Tag::UserObj,
                TAG_USER => Tag::User(id),
                TAG_GROUP_OBJ => Tag::GroupObj,
                TAG_GROUP => Tag::Group(id),
                TAG_MASK => Tag::Mask,
                TAG_OTHER => Tag::Other,
                _ => return None,
            };
            let perms = u16::from_le_bytes([p0, p1]) & u16::from(ALL_PERMISSIONS);
            Some(AclEntry {
                tag,
                perms: u8::try_from(perms).ok()?,
            })
        })
        .collect()
}

/// Encodes entries in the extended attribute form, sorted as the system
/// stores them.
pub(crate) fn encode(entries: &[AclEntry]) -> Vec<u8> {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|entry| entry.tag);
    let mut bytes = Vec::with_capacity(HEADER_LEN + ENTRY_LEN * sorted.len());
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    for entry in sorted {
        let (tag, id) = match entry.tag {
            Tag::UserObj => (TAG_USER_OBJ, UNDEFINED_ID),
            Tag::User(id) => (TAG_USER, id),
            Tag::GroupObj => (TAG_GROUP_OBJ, UNDEFINED_ID),
            Tag::Group(id) => (TAG_GROUP, id),
            Tag::Mask => (TAG_MASK, UNDEFINED_ID),
            Tag::Other => (TAG_OTHER, UNDEFINED_ID),
        };
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&u16::from(entry.perms).to_le_bytes());
        bytes.extend_from_slice(&id.to_le_bytes());
    }
    bytes
}

/// Entries in the short text form of `getfacl`, with names where the
/// tables know them.
pub(crate) fn render(entries: &[AclEntry], accounts: &Accounts) -> Vec<String> {
    entries
        .iter()
        .map(|entry| {
            let (class, qualifier) = match entry.tag {
                Tag::UserObj => ("user", String::new()),
                Tag::User(id) => (
                    "user",
                    accounts
                        .user_name(id)
                        .map_or_else(|| id.to_string(), str::to_owned),
                ),
                Tag::GroupObj => ("group", String::new()),
                Tag::Group(id) => (
                    "group",
                    accounts
                        .group_name(id)
                        .map_or_else(|| id.to_string(), str::to_owned),
                ),
                Tag::Mask => ("mask", String::new()),
                Tag::Other => ("other", String::new()),
            };
            format!("{class}:{qualifier}:{}", perms_text(entry.perms))
        })
        .collect()
}

/// Permissions in the form `rwx`, with `-` for a missing permission.
fn perms_text(perms: u8) -> String {
    [(READ, 'r'), (WRITE, 'w'), (EXECUTE, 'x')]
        .iter()
        .map(|(bit, letter)| {
            if perms & bit == 0 {
                NO_PERMISSION
            } else {
                *letter
            }
        })
        .collect()
}

/// Entries of the `acl` parameter: a list of maps with `tag` (`user`,
/// `group`, `mask` or `other`), `perms` (letters `r`, `w`, `x`, `-`) and,
/// for a named account or group, `id` as a number or a name.
///
/// The list needs one entry each for the account of the file, the group of
/// the file and others; a missing mask is computed from the group class when
/// named entries are present.
pub(crate) fn parse(value: &Value, accounts: &Accounts) -> Result<Vec<AclEntry>> {
    let Value::List(items) = value else {
        return Err(Error::new(
            ErrorCode::Type,
            "`attributes` expects a list of maps for `acl`",
        ));
    };
    let mut entries = items
        .iter()
        .map(|item| parse_entry(item, accounts))
        .collect::<Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.tag);
    if let Some(pair) = entries.windows(2).find(|pair| pair[0].tag == pair[1].tag) {
        return Err(invalid(format!(
            "`acl` has two entries for {}: keep one",
            tag_name(pair[0].tag)
        )));
    }
    for required in [Tag::UserObj, Tag::GroupObj, Tag::Other] {
        if !entries.iter().any(|entry| entry.tag == required) {
            return Err(invalid(format!(
                "`acl` needs an entry for {}",
                tag_name(required)
            )));
        }
    }
    let named = entries
        .iter()
        .any(|entry| matches!(entry.tag, Tag::User(_) | Tag::Group(_)));
    if named && !entries.iter().any(|entry| entry.tag == Tag::Mask) {
        let mask = entries
            .iter()
            .filter(|entry| matches!(entry.tag, Tag::User(_) | Tag::Group(_) | Tag::GroupObj))
            .fold(0, |mask, entry| mask | entry.perms);
        entries.push(AclEntry {
            tag: Tag::Mask,
            perms: mask,
        });
        entries.sort_by_key(|entry| entry.tag);
    }
    Ok(entries)
}

fn parse_entry(item: &Value, accounts: &Accounts) -> Result<AclEntry> {
    let fields = match item {
        Value::Map(fields) => fields,
        Value::Record(record) => &record.fields,
        _ => {
            return Err(Error::new(
                ErrorCode::Type,
                "every entry of `acl` is a map with `tag` and `perms`",
            ));
        }
    };
    if let Some(key) = fields
        .keys()
        .find(|key| ![TAG_KEY, ID_KEY, PERMS_KEY].contains(&key.as_str()))
    {
        return Err(invalid(format!(
            "`acl` entries have no key `{key}`: use `tag`, `id` and `perms`"
        ))
        .with_found(key.clone()));
    }
    let class = text_of(fields.get(TAG_KEY))
        .ok_or_else(|| invalid("every entry of `acl` needs `tag`: user, group, mask or other"))?;
    let perms = text_of(fields.get(PERMS_KEY))
        .ok_or_else(|| invalid("every entry of `acl` needs `perms`, such as \"r-x\""))?;
    let id = fields.get(ID_KEY).filter(|value| **value != Value::Null);
    let tag = match (class, id) {
        ("user", None) => Tag::UserObj,
        ("user", Some(id)) => Tag::User(qualifier(id, |name| accounts.resolve_user(name))?),
        ("group", None) => Tag::GroupObj,
        ("group", Some(id)) => Tag::Group(qualifier(id, |name| accounts.resolve_group(name))?),
        ("mask", None) => Tag::Mask,
        ("other", None) => Tag::Other,
        ("mask" | "other", Some(_)) => {
            return Err(invalid(format!(
                "the `{class}` entry of `acl` takes no `id`"
            )));
        }
        _ => {
            return Err(invalid(format!(
                "`{class}` is not an ACL tag: use user, group, mask or other"
            ))
            .with_found(class));
        }
    };
    Ok(AclEntry {
        tag,
        perms: parse_perms(perms)?,
    })
}

fn qualifier(value: &Value, lookup: impl Fn(&str) -> Option<u32>) -> Result<u32> {
    match value {
        Value::Int(number) => u32::try_from(*number)
            .map_err(|_| invalid(format!("`{number}` is not an account or group id"))),
        Value::Text(name) | Value::Enum(name) => lookup(name).ok_or_else(|| {
            invalid(format!(
                "no account or group is named `{name}`: give its numeric id"
            ))
            .with_found(name.clone())
        }),
        _ => Err(Error::new(
            ErrorCode::Type,
            "`id` of an `acl` entry is a number or a name",
        )),
    }
}

fn parse_perms(text: &str) -> Result<u8> {
    text.chars().try_fold(0, |perms, letter| match letter {
        'r' => Ok(perms | READ),
        'w' => Ok(perms | WRITE),
        'x' => Ok(perms | EXECUTE),
        NO_PERMISSION => Ok(perms),
        _ => Err(invalid(format!(
            "`{text}` are not ACL permissions: use the letters r, w, x and -"
        ))
        .with_found(text)),
    })
}

fn text_of(value: Option<&Value>) -> Option<&str> {
    match value {
        Some(Value::Text(text) | Value::Enum(text)) => Some(text),
        _ => None,
    }
}

fn tag_name(tag: Tag) -> String {
    match tag {
        Tag::UserObj => "the account of the file".to_owned(),
        Tag::User(id) => format!("account {id}"),
        Tag::GroupObj => "the group of the file".to_owned(),
        Tag::Group(id) => format!("group {id}"),
        Tag::Mask => "the mask".to_owned(),
        Tag::Other => "others".to_owned(),
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidValue, message)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::value::Fields;

    use super::*;

    fn entry(tag: &str, id: Option<Value>, perms: &str) -> Value {
        let mut fields = Fields::new();
        fields.insert(TAG_KEY.to_owned(), Value::Text(tag.to_owned()));
        if let Some(id) = id {
            fields.insert(ID_KEY.to_owned(), id);
        }
        fields.insert(PERMS_KEY.to_owned(), Value::Text(perms.to_owned()));
        Value::Map(fields)
    }

    fn minimal() -> Vec<Value> {
        vec![
            entry("user", None, "rw-"),
            entry("group", None, "r"),
            entry("other", None, ""),
        ]
    }

    #[test]
    fn encoded_entries_decode_to_the_same_list() {
        let entries = parse(&Value::List(minimal()), &Accounts::default()).unwrap();
        let bytes = encode(&entries);
        assert_eq!(bytes.len(), HEADER_LEN + 3 * ENTRY_LEN);
        assert_eq!(&bytes[..HEADER_LEN], &VERSION.to_le_bytes());
        assert_eq!(decode(&bytes), Some(entries));
    }

    #[test]
    fn named_entries_get_a_computed_mask() {
        let mut items = minimal();
        items.push(entry("user", Some(Value::Int(1001)), "rwx"));
        items.push(entry("group", Some(Value::Int(50)), "r-x"));
        let entries = parse(&Value::List(items), &Accounts::default()).unwrap();
        let mask = entries.iter().find(|entry| entry.tag == Tag::Mask).unwrap();
        assert_eq!(mask.perms, ALL_PERMISSIONS);
        assert_eq!(
            render(&entries, &Accounts::default()),
            [
                "user::rw-",
                "user:1001:rwx",
                "group::r--",
                "group:50:r-x",
                "mask::rwx",
                "other::---"
            ]
        );
    }

    #[test]
    fn malformed_lists_are_rejected() {
        let accounts = Accounts::default();
        let cases = [
            vec![entry("user", None, "r"), entry("other", None, "r")],
            vec![
                entry("user", None, "r"),
                entry("user", None, "w"),
                entry("group", None, "r"),
                entry("other", None, ""),
            ],
            vec![entry("owner", None, "r")],
            vec![entry("user", None, "rq")],
            vec![entry("mask", Some(Value::Int(1)), "r")],
            vec![entry(
                "user",
                Some(Value::Text("nobody-here".to_owned())),
                "r",
            )],
        ];
        for items in cases {
            let error = parse(&Value::List(items), &accounts).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "{}", error.message);
        }
        let error = parse(&Value::Text("user::rw-".to_owned()), &accounts).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
        let mut fields = Fields::new();
        fields.insert("who".to_owned(), Value::Text("user".to_owned()));
        let error = parse(&Value::List(vec![Value::Map(fields)]), &accounts).unwrap_err();
        assert_eq!(error.found.as_deref(), Some("who"));
    }

    #[test]
    fn foreign_bytes_are_not_an_acl() {
        assert_eq!(decode(b""), None);
        assert_eq!(decode(&[1, 0, 0, 0]), None);
        let mut bytes = VERSION.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[0x40, 0, 4, 0, 0, 0, 0, 0]);
        assert_eq!(decode(&bytes), None);
        bytes.truncate(HEADER_LEN + 3);
        assert_eq!(decode(&bytes), None);
    }

    #[test]
    fn permissions_render_in_fixed_order() {
        assert_eq!(perms_text(READ | EXECUTE), "r-x");
        assert_eq!(perms_text(0), "---");
        assert_eq!(parse_perms("x-r").unwrap(), READ | EXECUTE);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
