// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Self-relative security descriptors kept in
//! [`ExtendedMeta::acl`](nurt_contracts::fs::ExtendedMeta::acl).
//!
//! The back end stores the descriptor exactly as the system returns it for
//! the account, group, DACL and, when readable, SACL of a file. Whether the
//! SACL was captured is recorded in the descriptor itself:
//!
//! * captured: `SE_SACL_PRESENT` is set; a file without a SACL is stored with
//!   a present null SACL;
//! * not captured (the process lacks the privilege to read audit settings):
//!   `SE_SACL_PRESENT` is clear and restoration reports `sacl` as not
//!   transferred.
//!
//! The layout follows the `SECURITY_DESCRIPTOR_RELATIVE`, `SID` and `ACL`
//! structures of the Windows SDK; all fields are little-endian.

use std::fmt;

/// `SECURITY_DESCRIPTOR_CONTROL` bits.
pub mod control {
    /// A DACL is present.
    pub const SE_DACL_PRESENT: u16 = 0x0004;
    /// A SACL is present.
    pub const SE_SACL_PRESENT: u16 = 0x0010;
    /// The SACL was set by a default mechanism.
    pub const SE_SACL_DEFAULTED: u16 = 0x0020;
    /// The DACL is set up for automatic propagation of inheritable entries.
    pub const SE_DACL_AUTO_INHERITED: u16 = 0x0400;
    /// The SACL is set up for automatic propagation of inheritable entries.
    pub const SE_SACL_AUTO_INHERITED: u16 = 0x0800;
    /// The DACL does not receive inheritable entries from the parent.
    pub const SE_DACL_PROTECTED: u16 = 0x1000;
    /// The SACL does not receive inheritable entries from the parent.
    pub const SE_SACL_PROTECTED: u16 = 0x2000;
    /// The descriptor is in self-relative form.
    pub const SE_SELF_RELATIVE: u16 = 0x8000;
}

/// `SECURITY_INFORMATION` bits selecting parts of a descriptor.
pub mod information {
    /// Account (owner SID).
    pub const OWNER: u32 = 0x0000_0001;
    /// Primary group.
    pub const GROUP: u32 = 0x0000_0002;
    /// Discretionary access control list.
    pub const DACL: u32 = 0x0000_0004;
    /// System access control list (audit settings).
    pub const SACL: u32 = 0x0000_0008;
}

/// Access rights needed to change parts of a descriptor.
pub mod access {
    /// Change the DACL.
    pub const WRITE_DAC: u32 = 0x0004_0000;
    /// Change the account and the group.
    pub const WRITE_OWNER: u32 = 0x0008_0000;
    /// Read or change the SACL; requires the security privilege.
    pub const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;
}

use control::*;

const HEADER_LEN: usize = 20;
const SID_HEADER_LEN: usize = 8;
const SID_SUB_AUTHORITY_LEN: usize = 4;
const SID_REVISION: u8 = 1;
const ACL_HEADER_LEN: usize = 8;
const OFFSET_OWNER: usize = 4;
const OFFSET_GROUP: usize = 8;
const OFFSET_SACL: usize = 12;
const OFFSET_DACL: usize = 16;
const DACL_CONTROL: u16 = SE_DACL_PRESENT | SE_DACL_AUTO_INHERITED | SE_DACL_PROTECTED;
const SACL_CONTROL: u16 =
    SE_SACL_PRESENT | SE_SACL_DEFAULTED | SE_SACL_AUTO_INHERITED | SE_SACL_PROTECTED;
/// Identifier authorities below this value are written in decimal.
const DECIMAL_AUTHORITY_LIMIT: u64 = 1 << 32;

/// Error for bytes that do not form a valid self-relative descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidSecurityDescriptor;

impl fmt::Display for InvalidSecurityDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid self-relative security descriptor")
    }
}

impl std::error::Error for InvalidSecurityDescriptor {}

/// State of an access control list in a descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AclPart<'a> {
    /// The list is not part of the descriptor.
    Absent,
    /// The list is present and null (no entries, no restrictions or audit).
    Null,
    /// Bytes of the list.
    Acl(&'a [u8]),
}

/// Part of a security descriptor that can be restored separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecurityElement {
    /// Account of the file (owner SID).
    Account,
    /// Primary group.
    Group,
    /// Discretionary access control list.
    Dacl,
    /// System access control list.
    Sacl,
}

impl SecurityElement {
    /// All elements in restoration order.
    pub const ALL: [SecurityElement; 4] = [Self::Account, Self::Group, Self::Dacl, Self::Sacl];

    /// Name used in [`MetaTransferReport::not_transferred`](nurt_contracts::fs::MetaTransferReport).
    pub fn name(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Group => "group",
            Self::Dacl => "dacl",
            Self::Sacl => "sacl",
        }
    }

    /// `SECURITY_INFORMATION` bit of the element.
    pub fn information(self) -> u32 {
        match self {
            Self::Account => information::OWNER,
            Self::Group => information::GROUP,
            Self::Dacl => information::DACL,
            Self::Sacl => information::SACL,
        }
    }

    /// Access right needed to change the element.
    pub fn access(self) -> u32 {
        match self {
            Self::Account | Self::Group => access::WRITE_OWNER,
            Self::Dacl => access::WRITE_DAC,
            Self::Sacl => access::ACCESS_SYSTEM_SECURITY,
        }
    }
}

/// Parsed view of a self-relative security descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityDescriptor<'a> {
    control: u16,
    owner: Option<&'a [u8]>,
    group: Option<&'a [u8]>,
    dacl: AclPart<'a>,
    sacl: AclPart<'a>,
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn offset(bytes: &[u8], field: usize) -> Option<usize> {
    usize::try_from(read_u32(bytes, field)?).ok()
}

fn sid_at(bytes: &[u8], at: usize) -> Option<&[u8]> {
    let count = usize::from(*bytes.get(at + 1)?);
    let len = SID_HEADER_LEN + count * SID_SUB_AUTHORITY_LEN;
    let sid = bytes.get(at..at + len)?;
    (sid[0] == SID_REVISION).then_some(sid)
}

fn acl_at(bytes: &[u8], at: usize) -> Option<&[u8]> {
    let len = usize::from(read_u16(bytes, at + 2)?);
    if len < ACL_HEADER_LEN {
        return None;
    }
    bytes.get(at..at + len)
}

fn optional_sid(bytes: &[u8], field: usize) -> Option<Option<&[u8]>> {
    match offset(bytes, field)? {
        0 => Some(None),
        at => sid_at(bytes, at).map(Some),
    }
}

fn acl_part(bytes: &[u8], present: bool, field: usize) -> Option<AclPart<'_>> {
    if !present {
        return Some(AclPart::Absent);
    }
    match offset(bytes, field)? {
        0 => Some(AclPart::Null),
        at => acl_at(bytes, at).map(AclPart::Acl),
    }
}

impl<'a> SecurityDescriptor<'a> {
    /// Parses a self-relative descriptor.
    ///
    /// # Errors
    ///
    /// Fails when the bytes are too short, the self-relative flag is clear or
    /// an offset points outside the bytes or to a malformed SID or ACL.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, InvalidSecurityDescriptor> {
        Self::parse_inner(bytes).ok_or(InvalidSecurityDescriptor)
    }

    fn parse_inner(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < HEADER_LEN {
            return None;
        }
        let control = read_u16(bytes, 2)?;
        if control & SE_SELF_RELATIVE == 0 {
            return None;
        }
        Some(Self {
            control,
            owner: optional_sid(bytes, OFFSET_OWNER)?,
            group: optional_sid(bytes, OFFSET_GROUP)?,
            dacl: acl_part(bytes, control & SE_DACL_PRESENT != 0, OFFSET_DACL)?,
            sacl: acl_part(bytes, control & SE_SACL_PRESENT != 0, OFFSET_SACL)?,
        })
    }

    /// Control bits.
    pub fn control(&self) -> u16 {
        self.control
    }

    /// Account SID bytes.
    pub fn owner(&self) -> Option<&'a [u8]> {
        self.owner
    }

    /// Primary group SID bytes.
    pub fn group(&self) -> Option<&'a [u8]> {
        self.group
    }

    /// Discretionary access control list.
    pub fn dacl(&self) -> AclPart<'a> {
        self.dacl
    }

    /// System access control list.
    pub fn sacl(&self) -> AclPart<'a> {
        self.sacl
    }

    /// Whether the SACL was captured (see the module documentation).
    pub fn sacl_captured(&self) -> bool {
        self.control & SE_SACL_PRESENT != 0
    }

    /// Account in the string form `S-1-…`.
    pub fn account_sid(&self) -> Option<String> {
        self.owner.and_then(sid_to_string)
    }

    /// Whether the descriptor carries the element.
    pub fn has(&self, element: SecurityElement) -> bool {
        match element {
            SecurityElement::Account => self.owner.is_some(),
            SecurityElement::Group => self.group.is_some(),
            SecurityElement::Dacl => self.dacl != AclPart::Absent,
            SecurityElement::Sacl => self.sacl != AclPart::Absent,
        }
    }

    /// Whether the element is equal in both descriptors, including the
    /// inheritance control bits of the lists.
    pub fn same(&self, other: &SecurityDescriptor<'_>, element: SecurityElement) -> bool {
        match element {
            SecurityElement::Account => self.owner == other.owner,
            SecurityElement::Group => self.group == other.group,
            SecurityElement::Dacl => {
                self.dacl == other.dacl
                    && self.control & DACL_CONTROL == other.control & DACL_CONTROL
            }
            SecurityElement::Sacl => {
                self.sacl == other.sacl
                    && self.control & SACL_CONTROL == other.control & SACL_CONTROL
            }
        }
    }
}

/// String form of a binary SID (`S-1-5-21-…`).
///
/// Returns `None` for malformed bytes. Identifier authorities of 2^32 and more
/// are written in hexadecimal, as `ConvertSidToStringSidW` does.
///
/// # Examples
///
/// ```
/// use nurt_fs::windows::security::sid_to_string;
///
/// let local_system = [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
/// assert_eq!(sid_to_string(&local_system).as_deref(), Some("S-1-5-18"));
/// ```
pub fn sid_to_string(sid: &[u8]) -> Option<String> {
    let sid = sid_at(sid, 0)?;
    let authority = sid[2..SID_HEADER_LEN]
        .iter()
        .fold(0_u64, |value, &byte| (value << 8) | u64::from(byte));
    let mut out = if authority < DECIMAL_AUTHORITY_LIMIT {
        format!("S-{}-{authority}", sid[0])
    } else {
        format!("S-{}-0x{authority:012X}", sid[0])
    };
    for chunk in sid[SID_HEADER_LEN..].as_chunks::<SID_SUB_AUTHORITY_LEN>().0 {
        let value = u32::from_le_bytes(*chunk);
        out.push('-');
        out.push_str(&value.to_string());
    }
    Some(out)
}

/// Records in the descriptor whether its SACL was captured (see the module
/// documentation).
///
/// # Errors
///
/// Fails when `descriptor` is not a valid self-relative descriptor.
pub fn mark_sacl_capture(
    descriptor: &[u8],
    captured: bool,
) -> Result<Vec<u8>, InvalidSecurityDescriptor> {
    let parsed = SecurityDescriptor::parse(descriptor)?;
    let mut out = descriptor.to_vec();
    let mut control_bits = parsed.control;
    if captured {
        control_bits |= SE_SACL_PRESENT;
    } else {
        control_bits &= !SACL_CONTROL;
        out[OFFSET_SACL..OFFSET_SACL + 4].copy_from_slice(&0_u32.to_le_bytes());
    }
    out[2..4].copy_from_slice(&control_bits.to_le_bytes());
    Ok(out)
}

/// Elements that differ between two descriptors, by their report names
/// (`account`, `group`, `dacl`, `sacl`). The SACL is compared only when both
/// descriptors captured it.
///
/// Used to verify that a replacement kept the account, group and access
/// control lists of the replaced file.
///
/// # Errors
///
/// Fails when either descriptor is invalid.
pub fn security_differences(
    before: &[u8],
    after: &[u8],
) -> Result<Vec<&'static str>, InvalidSecurityDescriptor> {
    let before = SecurityDescriptor::parse(before)?;
    let after = SecurityDescriptor::parse(after)?;
    Ok(SecurityElement::ALL
        .into_iter()
        .filter(|&element| {
            if element == SecurityElement::Sacl
                && !(before.sacl_captured() && after.sacl_captured())
            {
                return false;
            }
            !before.same(&after, element)
        })
        .map(SecurityElement::name)
        .collect())
}

/// Plan of a descriptor restoration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RestorePlan {
    /// Elements to write, in order.
    pub apply: Vec<SecurityElement>,
    /// Elements that cannot be restored because they were not captured.
    pub not_captured: Vec<&'static str>,
}

/// Elements of `desired` that must be written to reach it from `current`.
///
/// Elements equal in both are skipped, so restoring an unchanged account
/// needs no privilege. When `current` is unknown every element of `desired`
/// is written. A SACL that was not captured is listed in
/// [`RestorePlan::not_captured`].
pub fn restore_plan(
    desired: &SecurityDescriptor<'_>,
    current: Option<&SecurityDescriptor<'_>>,
) -> RestorePlan {
    let mut plan = RestorePlan::default();
    for element in SecurityElement::ALL {
        if !desired.has(element) {
            if element == SecurityElement::Sacl {
                plan.not_captured.push(element.name());
            }
            continue;
        }
        let unchanged =
            current.is_some_and(|current| current.has(element) && desired.same(current, element));
        if !unchanged {
            plan.apply.push(element);
        }
    }
    plan
}

#[cfg(windows)]
mod verify {
    use super::{access, control, information};
    use windows_sys::Win32::Security as sec;
    use windows_sys::Win32::Storage::FileSystem as fs;
    use windows_sys::Win32::System::SystemServices as ss;

    const _: () = assert!(control::SE_DACL_PRESENT == sec::SE_DACL_PRESENT);
    const _: () = assert!(control::SE_SACL_PRESENT == sec::SE_SACL_PRESENT);
    const _: () = assert!(control::SE_SACL_DEFAULTED == sec::SE_SACL_DEFAULTED);
    const _: () = assert!(control::SE_DACL_AUTO_INHERITED == sec::SE_DACL_AUTO_INHERITED);
    const _: () = assert!(control::SE_SACL_AUTO_INHERITED == sec::SE_SACL_AUTO_INHERITED);
    const _: () = assert!(control::SE_DACL_PROTECTED == sec::SE_DACL_PROTECTED);
    const _: () = assert!(control::SE_SACL_PROTECTED == sec::SE_SACL_PROTECTED);
    const _: () = assert!(control::SE_SELF_RELATIVE == sec::SE_SELF_RELATIVE);
    const _: () = assert!(information::OWNER == sec::OWNER_SECURITY_INFORMATION);
    const _: () = assert!(information::GROUP == sec::GROUP_SECURITY_INFORMATION);
    const _: () = assert!(information::DACL == sec::DACL_SECURITY_INFORMATION);
    const _: () = assert!(information::SACL == sec::SACL_SECURITY_INFORMATION);
    const _: () = assert!(access::WRITE_DAC == fs::WRITE_DAC);
    const _: () = assert!(access::WRITE_OWNER == fs::WRITE_OWNER);
    const _: () = assert!(access::ACCESS_SYSTEM_SECURITY == ss::ACCESS_SYSTEM_SECURITY);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `S-1-5-32-544` (Administrators).
    const ADMINISTRATORS: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 0x20, 0x02, 0, 0];
    /// `S-1-5-18` (Local System).
    const LOCAL_SYSTEM: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];

    /// Empty ACL of revision 2 with the given size and no entries.
    fn empty_acl(size: u16) -> Vec<u8> {
        let mut acl = vec![2, 0];
        acl.extend_from_slice(&size.to_le_bytes());
        acl.extend_from_slice(&[0, 0, 0, 0]);
        acl.resize(usize::from(size), 0);
        acl
    }

    /// Builds a self-relative descriptor: header, then owner, group, SACL and
    /// DACL in this order.
    fn descriptor(
        control: u16,
        owner: Option<&[u8]>,
        group: Option<&[u8]>,
        sacl: Option<&[u8]>,
        dacl: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut out = vec![1, 0];
        out.extend_from_slice(&(control | SE_SELF_RELATIVE).to_le_bytes());
        out.resize(HEADER_LEN, 0);
        for (field, part) in [
            (OFFSET_OWNER, owner),
            (OFFSET_GROUP, group),
            (OFFSET_SACL, sacl),
            (OFFSET_DACL, dacl),
        ] {
            if let Some(bytes) = part {
                let at = u32::try_from(out.len()).unwrap();
                out[field..field + 4].copy_from_slice(&at.to_le_bytes());
                out.extend_from_slice(bytes);
            }
        }
        out
    }

    fn typical() -> Vec<u8> {
        descriptor(
            SE_DACL_PRESENT | SE_DACL_AUTO_INHERITED,
            Some(&ADMINISTRATORS),
            Some(&LOCAL_SYSTEM),
            None,
            Some(&empty_acl(8)),
        )
    }

    #[test]
    fn formats_sids() {
        assert_eq!(sid_to_string(&LOCAL_SYSTEM).as_deref(), Some("S-1-5-18"));
        assert_eq!(
            sid_to_string(&ADMINISTRATORS).as_deref(),
            Some("S-1-5-32-544")
        );
        let everyone = [1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        assert_eq!(sid_to_string(&everyone).as_deref(), Some("S-1-1-0"));
        let large = [1, 0, 0x01, 0, 0, 0, 0, 0];
        assert_eq!(sid_to_string(&large).as_deref(), Some("S-1-0x010000000000"));
    }

    #[test]
    fn rejects_malformed_sids() {
        assert_eq!(sid_to_string(&[]), None);
        assert_eq!(sid_to_string(&[1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0]), None);
        assert_eq!(sid_to_string(&[2, 0, 0, 0, 0, 0, 0, 5]), None);
    }

    #[test]
    fn parses_descriptor_parts() {
        let bytes = typical();
        let parsed = SecurityDescriptor::parse(&bytes).unwrap();
        assert_eq!(parsed.owner(), Some(&ADMINISTRATORS[..]));
        assert_eq!(parsed.group(), Some(&LOCAL_SYSTEM[..]));
        assert_eq!(parsed.dacl(), AclPart::Acl(&empty_acl(8)));
        assert_eq!(parsed.sacl(), AclPart::Absent);
        assert!(!parsed.sacl_captured());
        assert_eq!(parsed.account_sid().as_deref(), Some("S-1-5-32-544"));
        assert_ne!(parsed.control() & SE_DACL_AUTO_INHERITED, 0);
    }

    #[test]
    fn distinguishes_null_and_absent_lists() {
        let bytes = descriptor(SE_DACL_PRESENT | SE_SACL_PRESENT, None, None, None, None);
        let parsed = SecurityDescriptor::parse(&bytes).unwrap();
        assert_eq!(parsed.dacl(), AclPart::Null);
        assert_eq!(parsed.sacl(), AclPart::Null);
        assert_eq!(parsed.owner(), None);
        assert_eq!(parsed.account_sid(), None);
        assert!(parsed.has(SecurityElement::Dacl));
        assert!(!parsed.has(SecurityElement::Account));
    }

    #[test]
    fn rejects_invalid_descriptors() {
        assert_eq!(
            SecurityDescriptor::parse(&[1, 0]),
            Err(InvalidSecurityDescriptor)
        );
        let mut absolute = typical();
        absolute[3] &= 0x7F;
        assert!(SecurityDescriptor::parse(&absolute).is_err());
        let mut truncated = typical();
        truncated.truncate(30);
        assert!(SecurityDescriptor::parse(&truncated).is_err());
        let short_acl = descriptor(SE_DACL_PRESENT, None, None, None, Some(&[2, 0, 4, 0]));
        assert!(SecurityDescriptor::parse(&short_acl).is_err());
        assert_eq!(
            InvalidSecurityDescriptor.to_string(),
            "invalid self-relative security descriptor"
        );
    }

    #[test]
    fn marks_captured_sacl_as_present() {
        let marked = mark_sacl_capture(&typical(), true).unwrap();
        let parsed = SecurityDescriptor::parse(&marked).unwrap();
        assert!(parsed.sacl_captured());
        assert_eq!(parsed.sacl(), AclPart::Null);
        assert_eq!(parsed.dacl(), AclPart::Acl(&empty_acl(8)));
    }

    #[test]
    fn marks_uncaptured_sacl_as_absent() {
        let with_sacl = descriptor(
            SE_DACL_PRESENT | SE_SACL_PRESENT | SE_SACL_PROTECTED,
            Some(&ADMINISTRATORS),
            None,
            Some(&empty_acl(8)),
            None,
        );
        let marked = mark_sacl_capture(&with_sacl, false).unwrap();
        let parsed = SecurityDescriptor::parse(&marked).unwrap();
        assert!(!parsed.sacl_captured());
        assert_eq!(parsed.control() & SACL_CONTROL, 0);
        assert_eq!(read_u32(&marked, OFFSET_SACL), Some(0));
        assert!(mark_sacl_capture(&[0; 4], true).is_err());
    }

    #[test]
    fn reports_no_differences_for_equal_descriptors() {
        assert!(
            security_differences(&typical(), &typical())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn reports_changed_account_and_dacl() {
        let changed = descriptor(
            SE_DACL_PRESENT | SE_DACL_AUTO_INHERITED,
            Some(&LOCAL_SYSTEM),
            Some(&LOCAL_SYSTEM),
            None,
            Some(&empty_acl(12)),
        );
        assert_eq!(
            security_differences(&typical(), &changed).unwrap(),
            ["account", "dacl"]
        );
    }

    #[test]
    fn reports_changed_dacl_protection() {
        let protected = descriptor(
            SE_DACL_PRESENT | SE_DACL_AUTO_INHERITED | SE_DACL_PROTECTED,
            Some(&ADMINISTRATORS),
            Some(&LOCAL_SYSTEM),
            None,
            Some(&empty_acl(8)),
        );
        assert_eq!(
            security_differences(&typical(), &protected).unwrap(),
            ["dacl"]
        );
    }

    #[test]
    fn compares_sacl_only_when_both_captured() {
        let captured = mark_sacl_capture(&typical(), true).unwrap();
        let with_audit = descriptor(
            SE_DACL_PRESENT | SE_DACL_AUTO_INHERITED | SE_SACL_PRESENT,
            Some(&ADMINISTRATORS),
            Some(&LOCAL_SYSTEM),
            Some(&empty_acl(8)),
            Some(&empty_acl(8)),
        );
        assert_eq!(
            security_differences(&captured, &with_audit).unwrap(),
            ["sacl"]
        );
        assert!(
            security_differences(&typical(), &with_audit)
                .unwrap()
                .is_empty()
        );
        assert!(security_differences(&typical(), &[0]).is_err());
    }

    #[test]
    fn plans_only_changed_elements() {
        let desired_bytes = mark_sacl_capture(&typical(), true).unwrap();
        let desired = SecurityDescriptor::parse(&desired_bytes).unwrap();
        let current_bytes = descriptor(
            SE_DACL_PRESENT,
            Some(&ADMINISTRATORS),
            Some(&ADMINISTRATORS),
            None,
            Some(&empty_acl(8)),
        );
        let current = SecurityDescriptor::parse(&current_bytes).unwrap();
        let plan = restore_plan(&desired, Some(&current));
        assert_eq!(
            plan.apply,
            [
                SecurityElement::Group,
                SecurityElement::Dacl,
                SecurityElement::Sacl
            ]
        );
        assert!(plan.not_captured.is_empty());
    }

    #[test]
    fn plans_every_element_without_current_state() {
        let bytes = typical();
        let desired = SecurityDescriptor::parse(&bytes).unwrap();
        let plan = restore_plan(&desired, None);
        assert_eq!(
            plan.apply,
            [
                SecurityElement::Account,
                SecurityElement::Group,
                SecurityElement::Dacl
            ]
        );
        assert_eq!(plan.not_captured, ["sacl"]);
    }

    #[test]
    fn describes_elements() {
        let names: Vec<_> = SecurityElement::ALL.iter().map(|e| e.name()).collect();
        assert_eq!(names, ["account", "group", "dacl", "sacl"]);
        let bits: Vec<_> = SecurityElement::ALL
            .iter()
            .map(|e| e.information())
            .collect();
        assert_eq!(bits, [1, 2, 4, 8]);
        assert_eq!(SecurityElement::Account.access(), access::WRITE_OWNER);
        assert_eq!(SecurityElement::Group.access(), access::WRITE_OWNER);
        assert_eq!(SecurityElement::Dacl.access(), access::WRITE_DAC);
        assert_eq!(
            SecurityElement::Sacl.access(),
            access::ACCESS_SYSTEM_SECURITY
        );
    }
}
