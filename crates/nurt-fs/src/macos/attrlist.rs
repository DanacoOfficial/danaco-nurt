// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Buffers of `getattrlist` and `setattrlist`.
//!
//! The layouts follow `sys/attr.h` for 64-bit targets: `struct attrlist` is a
//! 24-byte header, `struct timespec` occupies 16 bytes, attribute values are
//! packed in the order of their bits and a `getattrlist` reply starts with a
//! `u32` holding its total length.

use crate::macos::volume::VolumeCapabilities;

/// `ATTR_BIT_MAP_COUNT`.
pub const ATTR_BIT_MAP_COUNT: u16 = 5;
/// `ATTR_CMN_CRTIME`: creation time.
pub const ATTR_CMN_CRTIME: u32 = 0x0000_0200;
/// `ATTR_VOL_CAPABILITIES`: `vol_capabilities_attr_t`.
pub const ATTR_VOL_CAPABILITIES: u32 = 0x0002_0000;
/// `ATTR_VOL_UUID`: `uuid_t` of the volume.
pub const ATTR_VOL_UUID: u32 = 0x0004_0000;
/// `ATTR_VOL_INFO`: required in every request for volume attributes.
pub const ATTR_VOL_INFO: u32 = 0x8000_0000;

/// Size of `struct timespec` on 64-bit targets.
pub const TIMESPEC_SIZE: usize = 16;
/// Size of `vol_capabilities_attr_t`: four capability words and four valid
/// masks.
const VOL_CAPABILITIES_SIZE: usize = 32;
/// Size of `uuid_t`.
const UUID_SIZE: usize = 16;
/// Size of the length word that starts every `getattrlist` reply.
const LENGTH_SIZE: usize = 4;
/// Index of the format word in `vol_capabilities_set_t`.
const VOL_CAPABILITIES_FORMAT: usize = 0;
/// Index of the interfaces word in `vol_capabilities_set_t`.
const VOL_CAPABILITIES_INTERFACES: usize = 1;
/// Words in `vol_capabilities_set_t`.
const VOL_CAPABILITIES_WORDS: usize = 4;
/// Size of a word of `vol_capabilities_set_t`.
const WORD_SIZE: usize = 4;
/// Size of a reply to [`AttrList::volume_info`].
pub const VOLUME_INFO_REPLY_SIZE: usize = LENGTH_SIZE + VOL_CAPABILITIES_SIZE + UUID_SIZE;

const NANOS_PER_SECOND: i64 = 1_000_000_000;

/// `struct attrlist` of `sys/attr.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttrList {
    /// Number of attribute groups; always [`ATTR_BIT_MAP_COUNT`].
    pub bitmapcount: u16,
    /// Reserved; zero.
    pub reserved: u16,
    /// Common attributes (`ATTR_CMN_*`).
    pub commonattr: u32,
    /// Volume attributes (`ATTR_VOL_*`).
    pub volattr: u32,
    /// Directory attributes (`ATTR_DIR_*`).
    pub dirattr: u32,
    /// File attributes (`ATTR_FILE_*`).
    pub fileattr: u32,
    /// Fork attributes (`ATTR_FORK_*`).
    pub forkattr: u32,
}

impl AttrList {
    /// Request that addresses the creation time of a file system object.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_fs::macos::{ATTR_CMN_CRTIME, AttrList};
    ///
    /// assert_eq!(AttrList::creation_time().commonattr, ATTR_CMN_CRTIME);
    /// ```
    #[must_use]
    pub fn creation_time() -> Self {
        Self {
            commonattr: ATTR_CMN_CRTIME,
            ..Self::empty()
        }
    }

    /// Request for the capabilities and the UUID of a volume; the path of a
    /// `getattrlist` call with this request must be the mount point.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_fs::macos::{ATTR_VOL_INFO, AttrList};
    ///
    /// assert_ne!(AttrList::volume_info().volattr & ATTR_VOL_INFO, 0);
    /// ```
    #[must_use]
    pub fn volume_info() -> Self {
        Self {
            volattr: ATTR_VOL_INFO | ATTR_VOL_CAPABILITIES | ATTR_VOL_UUID,
            ..Self::empty()
        }
    }

    fn empty() -> Self {
        Self {
            bitmapcount: ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: 0,
            volattr: 0,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        }
    }
}

/// Splits nanoseconds since the Unix epoch into seconds and a nanosecond part
/// in `0..1_000_000_000`, as `struct timespec` requires for times before the
/// epoch.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::split_nanos;
///
/// assert_eq!(split_nanos(1_500_000_000), (1, 500_000_000));
/// assert_eq!(split_nanos(-1), (-1, 999_999_999));
/// ```
#[must_use]
pub fn split_nanos(nanos: i64) -> (i64, i64) {
    (
        nanos.div_euclid(NANOS_PER_SECOND),
        nanos.rem_euclid(NANOS_PER_SECOND),
    )
}

/// Nanoseconds since the Unix epoch from a `struct timespec`; saturates at the
/// bounds of `i64`.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::timespec_nanos;
///
/// assert_eq!(timespec_nanos(-1, 999_999_999), -1);
/// ```
#[must_use]
pub fn timespec_nanos(seconds: i64, nanos: i64) -> i64 {
    seconds
        .saturating_mul(NANOS_PER_SECOND)
        .saturating_add(nanos)
}

/// Attribute buffer of `setattrlist` for [`AttrList::creation_time`]: one
/// `struct timespec` in native byte order, without a length word.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::{TIMESPEC_SIZE, creation_time_buffer};
///
/// assert_eq!(creation_time_buffer(0).len(), TIMESPEC_SIZE);
/// ```
#[must_use]
pub fn creation_time_buffer(nanos: i64) -> [u8; TIMESPEC_SIZE] {
    let (seconds, fraction) = split_nanos(nanos);
    let mut buffer = [0_u8; TIMESPEC_SIZE];
    let (head, tail) = buffer.split_at_mut(TIMESPEC_SIZE / 2);
    head.copy_from_slice(&seconds.to_ne_bytes());
    tail.copy_from_slice(&fraction.to_ne_bytes());
    buffer
}

/// Capabilities and UUID of a volume decoded from a `getattrlist` reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeInfoReply {
    /// Reported capabilities.
    pub capabilities: VolumeCapabilities,
    /// Volume UUID; `None` when the volume reports the null UUID.
    pub uuid: Option<[u8; UUID_SIZE]>,
}

/// Decodes a reply to [`AttrList::volume_info`] in native byte order.
///
/// Returns `None` when the reply is shorter than its length word or than
/// [`VOLUME_INFO_REPLY_SIZE`].
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::{VOLUME_INFO_REPLY_SIZE, parse_volume_info};
///
/// let mut reply = [0_u8; VOLUME_INFO_REPLY_SIZE];
/// reply[..4].copy_from_slice(&(VOLUME_INFO_REPLY_SIZE as u32).to_ne_bytes());
/// let decoded = parse_volume_info(&reply).unwrap();
/// assert_eq!(decoded.uuid, None);
/// assert!(parse_volume_info(&reply[..8]).is_none());
/// ```
#[must_use]
pub fn parse_volume_info(reply: &[u8]) -> Option<VolumeInfoReply> {
    let length = usize::try_from(u32::from_ne_bytes(read_array(reply, 0)?)).ok()?;
    if length < VOLUME_INFO_REPLY_SIZE || reply.len() < VOLUME_INFO_REPLY_SIZE {
        return None;
    }
    let word = |index: usize| -> Option<u32> {
        read_array(reply, LENGTH_SIZE + index * WORD_SIZE).map(u32::from_ne_bytes)
    };
    let valid = |index: usize| word(VOL_CAPABILITIES_WORDS + index);
    let capabilities = VolumeCapabilities {
        format: word(VOL_CAPABILITIES_FORMAT)?,
        format_valid: valid(VOL_CAPABILITIES_FORMAT)?,
        interfaces: word(VOL_CAPABILITIES_INTERFACES)?,
        interfaces_valid: valid(VOL_CAPABILITIES_INTERFACES)?,
    };
    let uuid: [u8; UUID_SIZE] = read_array(reply, LENGTH_SIZE + VOL_CAPABILITIES_SIZE)?;
    Some(VolumeInfoReply {
        capabilities,
        uuid: uuid.iter().any(|&byte| byte != 0).then_some(uuid),
    })
}

/// Text form of a volume UUID as reported by `kCFURLVolumeUUIDStringKey`:
/// upper-case hexadecimal digits in groups of 8, 4, 4, 4 and 12.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::uuid_text;
///
/// let uuid = [0xAB; 16];
/// assert_eq!(uuid_text(&uuid), "ABABABAB-ABAB-ABAB-ABAB-ABABABABABAB");
/// ```
#[must_use]
pub fn uuid_text(uuid: &[u8; UUID_SIZE]) -> String {
    const GROUP_ENDS: [usize; 4] = [4, 6, 8, 10];
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut text = String::with_capacity(UUID_SIZE * 2 + GROUP_ENDS.len());
    for (index, byte) in uuid.iter().enumerate() {
        if GROUP_ENDS.contains(&index) {
            text.push('-');
        }
        for nibble in [byte >> 4, byte & 0x0f] {
            text.push(char::from(DIGITS[usize::from(nibble)]));
        }
    }
    text
}

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    bytes
        .get(offset..offset.checked_add(N)?)
        .and_then(|slice| slice.try_into().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Size of `struct attrlist` on every Darwin target.
    const ATTRLIST_SIZE: usize = 24;

    #[test]
    fn lays_out_attrlist_header_like_sys_attr_h() {
        assert_eq!(std::mem::size_of::<AttrList>(), ATTRLIST_SIZE);
        assert_eq!(std::mem::align_of::<AttrList>(), 4);
        let request = AttrList::creation_time();
        assert_eq!(request.bitmapcount, 5);
        assert_eq!(request.reserved, 0);
        assert_eq!(request.commonattr, 0x200);
        assert_eq!(
            (
                request.volattr,
                request.dirattr,
                request.fileattr,
                request.forkattr
            ),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn requests_volume_info_with_capabilities_and_uuid() {
        let request = AttrList::volume_info();
        assert_eq!(request.volattr, 0x8006_0000);
        assert_eq!(request.commonattr, 0);
    }

    #[test]
    fn encodes_creation_time_as_native_timespec() {
        let nanos = 1_700_000_000_123_456_789;
        let buffer = creation_time_buffer(nanos);
        assert_eq!(buffer.len(), 16);
        let seconds = i64::from_ne_bytes(buffer[..8].try_into().unwrap());
        let fraction = i64::from_ne_bytes(buffer[8..].try_into().unwrap());
        assert_eq!((seconds, fraction), (1_700_000_000, 123_456_789));
    }

    #[test]
    fn encodes_creation_time_before_epoch_with_positive_fraction() {
        let buffer = creation_time_buffer(-1_500_000_000);
        let seconds = i64::from_ne_bytes(buffer[..8].try_into().unwrap());
        let fraction = i64::from_ne_bytes(buffer[8..].try_into().unwrap());
        assert_eq!((seconds, fraction), (-2, 500_000_000));
    }

    #[test]
    fn round_trips_nanoseconds_through_timespec() {
        for nanos in [0, 1, -1, 999_999_999, -1_000_000_000, 1_234_567_890_123] {
            let (seconds, fraction) = split_nanos(nanos);
            assert!((0..NANOS_PER_SECOND).contains(&fraction));
            assert_eq!(timespec_nanos(seconds, fraction), nanos);
        }
        assert_eq!(timespec_nanos(i64::MAX, 0), i64::MAX);
    }

    fn reply(words: [u32; 8], uuid: [u8; 16]) -> Vec<u8> {
        let mut bytes = u32::try_from(VOLUME_INFO_REPLY_SIZE)
            .unwrap()
            .to_ne_bytes()
            .to_vec();
        for word in words {
            bytes.extend_from_slice(&word.to_ne_bytes());
        }
        bytes.extend_from_slice(&uuid);
        bytes
    }

    #[test]
    fn decodes_capabilities_and_uuid_of_volume_info_reply() {
        let uuid: [u8; 16] = std::array::from_fn(|index| u8::try_from(index + 1).unwrap());
        let bytes = reply([0x100, 0x4_0000, 0, 0, 0x100, 0xF_0000, 0, 0], uuid);
        assert_eq!(bytes.len(), 52);
        let decoded = parse_volume_info(&bytes).unwrap();
        assert_eq!(decoded.capabilities.format, 0x100);
        assert_eq!(decoded.capabilities.format_valid, 0x100);
        assert_eq!(decoded.capabilities.interfaces, 0x4_0000);
        assert_eq!(decoded.capabilities.interfaces_valid, 0xF_0000);
        assert_eq!(decoded.uuid, Some(uuid));
        assert_eq!(uuid_text(&uuid), "01020304-0506-0708-090A-0B0C0D0E0F10");
    }

    #[test]
    fn rejects_truncated_volume_info_reply() {
        let bytes = reply([0; 8], [1; 16]);
        assert!(parse_volume_info(&bytes[..51]).is_none());
        assert!(parse_volume_info(&[]).is_none());
        let mut short_length = bytes.clone();
        short_length[..4].copy_from_slice(&20_u32.to_ne_bytes());
        assert!(parse_volume_info(&short_length).is_none());
    }

    #[test]
    fn reports_null_uuid_as_absent() {
        let decoded = parse_volume_info(&reply([0; 8], [0; 16])).unwrap();
        assert_eq!(decoded.uuid, None);
    }
}
