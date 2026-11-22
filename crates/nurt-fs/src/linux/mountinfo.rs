// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parser of the mount table of a process (`/proc/<pid>/mountinfo`).
//!
//! The line format is documented in `proc_pid_mountinfo(5)`:
//!
//! ```text
//! 36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue
//! ```
//!
//! The kernel escapes space, tab, newline and backslash in paths and the
//! source as three-digit octal sequences (`\040`), and additionally commas in
//! option values written by `seq_show_option` (`\054`). Paths are kept as raw
//! bytes because Linux file names need not be valid UTF-8.

use std::error::Error;
use std::fmt;

/// Number of fields before the optional fields of a line.
const FIXED_FIELDS: usize = 6;
/// Number of fields after the `-` separator.
const TAIL_FIELDS: usize = 3;
/// File system type of the kernel overlay file system.
const OVERLAY_TYPE: &str = "overlay";

/// One mount of the mount table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    /// Unique mount identifier; equals `stx_mnt_id` reported by `statx`.
    pub mount_id: u64,
    /// Identifier of the parent mount.
    pub parent_id: u64,
    /// Major number of the device of the file system.
    pub major: u32,
    /// Minor number of the device of the file system.
    pub minor: u32,
    /// Directory of the file system that is the root of this mount (raw
    /// bytes, unescaped); differs from `/` for bind mounts and btrfs
    /// subvolumes.
    pub root: Vec<u8>,
    /// Mount point relative to the root directory of the process (raw bytes,
    /// unescaped).
    pub mount_point: Vec<u8>,
    /// Per-mount options (`rw`, `nosuid`, `relatime`).
    pub mount_options: Vec<String>,
    /// File system type, with the subtype for FUSE (`fuse.sshfs`).
    pub fs_type: String,
    /// Mount source (`/dev/sda1`, `server:/export`, `overlay`; raw bytes,
    /// unescaped).
    pub source: Vec<u8>,
    /// Per-superblock options (raw bytes, unescaped), for example
    /// `subvolid=256` or `upperdir=/var/lib/upper`.
    pub super_options: Vec<Vec<u8>>,
}

/// Layers of an overlay mount, taken from its superblock options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayLayers {
    /// Upper (writable) layer; `None` for a read-only overlay.
    pub upper: Option<Vec<u8>>,
    /// Lower layers from the top-most to the bottom-most, including
    /// data-only layers.
    pub lower: Vec<Vec<u8>>,
    /// Work directory of the upper layer.
    pub work: Option<Vec<u8>>,
}

impl MountEntry {
    /// Whether the mount or its superblock is read-only.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.mount_options.iter().any(|option| option == "ro") || self.has_super_option("ro")
    }

    /// Whether the superblock options contain `name`, either as a flag or as
    /// `name=value`.
    #[must_use]
    pub fn has_super_option(&self, name: &str) -> bool {
        self.super_option(name).is_some()
    }

    /// Value of the superblock option `name`; an empty slice for a flag
    /// without value, `None` when the option is absent.
    #[must_use]
    pub fn super_option(&self, name: &str) -> Option<&[u8]> {
        let name = name.as_bytes();
        self.super_options.iter().find_map(|option| {
            let rest = option.strip_prefix(name)?;
            match rest.split_first() {
                None => Some(&rest[..0]),
                Some((b'=', value)) => Some(value),
                Some(_) => None,
            }
        })
    }

    /// Device number of the file system in the encoding of `st_dev`.
    #[must_use]
    pub fn device(&self) -> u64 {
        device_number(self.major, self.minor)
    }

    /// Layers of an overlay mount; `None` for other file system types.
    ///
    /// Lower layers are separated by `:` (`::` before data-only layers); a
    /// colon escaped as `\:` belongs to the path. Layers given one by one
    /// (`lowerdir+`, `datadir+`) are appended in order.
    #[must_use]
    pub fn overlay_layers(&self) -> Option<OverlayLayers> {
        if self.fs_type != OVERLAY_TYPE {
            return None;
        }
        let mut layers = OverlayLayers::default();
        for option in &self.super_options {
            if let Some(value) = option.strip_prefix(b"upperdir=") {
                layers.upper = Some(value.to_vec());
            } else if let Some(value) = option.strip_prefix(b"workdir=") {
                layers.work = Some(value.to_vec());
            } else if let Some(value) = option.strip_prefix(b"lowerdir=") {
                layers.lower.extend(split_layers(value));
            } else if let Some(value) = option
                .strip_prefix(b"lowerdir+=")
                .or_else(|| option.strip_prefix(b"datadir+="))
            {
                layers.lower.push(value.to_vec());
            }
        }
        Some(layers)
    }
}

/// Device number in the encoding used by `st_dev` on Linux (`makedev` of
/// glibc and of the kernel `new_encode_dev`).
///
/// # Examples
///
/// ```
/// use nurt_fs::linux::device_number;
///
/// assert_eq!(device_number(8, 1), 0x801);
/// ```
#[must_use]
pub fn device_number(major: u32, minor: u32) -> u64 {
    let major = u64::from(major);
    let minor = u64::from(minor);
    ((major & 0xffff_f000) << 32)
        | ((major & 0x0000_0fff) << 8)
        | ((minor & 0xffff_ff00) << 12)
        | (minor & 0x0000_00ff)
}

/// Mount table of a process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MountTable {
    entries: Vec<MountEntry>,
}

impl MountTable {
    /// Parses the content of `/proc/<pid>/mountinfo`.
    ///
    /// Empty lines are ignored.
    ///
    /// # Errors
    ///
    /// Fails with [`MountinfoError`] naming the first line that does not
    /// follow the format.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_fs::linux::MountTable;
    ///
    /// let text = b"25 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw\n\
    ///              30 25 0:40 / /dev/shm rw,nosuid - tmpfs shm rw,size=65536k\n";
    /// let table = MountTable::parse(text)?;
    /// let entry = table.containing(b"/dev/shm/data", None).map(|entry| entry.fs_type.as_str());
    /// assert_eq!(entry, Some("tmpfs"));
    /// # Ok::<(), nurt_fs::linux::MountinfoError>(())
    /// ```
    pub fn parse(text: &[u8]) -> Result<Self, MountinfoError> {
        let mut entries = Vec::new();
        for (index, line) in text.split(|&byte| byte == b'\n').enumerate() {
            if line.is_empty() {
                continue;
            }
            let entry = parse_line(line).ok_or(MountinfoError { line: index + 1 })?;
            entries.push(entry);
        }
        Ok(Self { entries })
    }

    /// Mounts in the order of the table (later mounts shadow earlier mounts
    /// on the same mount point).
    #[must_use]
    pub fn entries(&self) -> &[MountEntry] {
        &self.entries
    }

    /// Mount with the given identifier.
    #[must_use]
    pub fn by_mount_id(&self, mount_id: u64) -> Option<&MountEntry> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.mount_id == mount_id)
    }

    /// Mount whose mount point is the longest component-wise prefix of
    /// `path` (an absolute path with symbolic links resolved).
    ///
    /// When `device` is given, mounts of that device (major and minor
    /// number) are preferred, which distinguishes a mount from mounts that
    /// shadow parts of it. Among equal candidates the later mount wins.
    #[must_use]
    pub fn containing(&self, path: &[u8], device: Option<(u32, u32)>) -> Option<&MountEntry> {
        let candidates = || {
            self.entries
                .iter()
                .filter(|entry| is_within(path, &entry.mount_point))
        };
        if let Some((major, minor)) = device {
            let same_device =
                candidates().filter(|entry| entry.major == major && entry.minor == minor);
            if let Some(entry) = longest_mount_point(same_device) {
                return Some(entry);
            }
        }
        longest_mount_point(candidates())
    }
}

/// Entry with the longest mount point; the later entry wins a tie.
fn longest_mount_point<'a>(entries: impl Iterator<Item = &'a MountEntry>) -> Option<&'a MountEntry> {
    entries.fold(None, |best, entry| match best {
        Some(best) if best.mount_point.len() > entry.mount_point.len() => Some(best),
        _ => Some(entry),
    })
}

/// Error of [`MountTable::parse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountinfoError {
    /// Line number, starting at 1.
    pub line: usize,
}

impl fmt::Display for MountinfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed mount table line {}", self.line)
    }
}

impl Error for MountinfoError {}

fn parse_line(line: &[u8]) -> Option<MountEntry> {
    let fields: Vec<&[u8]> = line.split(|&byte| byte == b' ').collect();
    let separator = fields
        .iter()
        .skip(FIXED_FIELDS)
        .position(|field| *field == b"-")?
        + FIXED_FIELDS;
    let tail = fields.get(separator + 1..)?;
    if tail.len() != TAIL_FIELDS {
        return None;
    }
    let (major, minor) = parse_device(fields[2])?;
    Some(MountEntry {
        mount_id: parse_number(fields[0])?,
        parent_id: parse_number(fields[1])?,
        major,
        minor,
        root: unescape(fields[3]),
        mount_point: unescape(fields[4]),
        mount_options: fields[5]
            .split(|&byte| byte == b',')
            .filter(|option| !option.is_empty())
            .map(|option| String::from_utf8_lossy(option).into_owned())
            .collect(),
        fs_type: String::from_utf8_lossy(tail[0]).into_owned(),
        source: unescape(tail[1]),
        super_options: tail[2]
            .split(|&byte| byte == b',')
            .filter(|option| !option.is_empty())
            .map(unescape)
            .collect(),
    })
}

fn parse_number<T: std::str::FromStr>(field: &[u8]) -> Option<T> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

fn parse_device(field: &[u8]) -> Option<(u32, u32)> {
    let colon = field.iter().position(|&byte| byte == b':')?;
    Some((
        parse_number(&field[..colon])?,
        parse_number(&field[colon + 1..])?,
    ))
}

/// Replaces three-digit octal escapes (`\040`) by the bytes they denote;
/// other backslashes are kept.
fn unescape(field: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(field.len());
    let mut rest = field;
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'\\'
            && let Some(value) = octal_byte(tail)
        {
            out.push(value);
            rest = &tail[3..];
            continue;
        }
        out.push(byte);
        rest = tail;
    }
    out
}

fn octal_byte(digits: &[u8]) -> Option<u8> {
    let digits = digits.get(..3)?;
    let mut value: u16 = 0;
    for &digit in digits {
        if !(b'0'..=b'7').contains(&digit) {
            return None;
        }
        value = value * 8 + u16::from(digit - b'0');
    }
    u8::try_from(value).ok()
}

/// Splits the value of `lowerdir=` at unescaped colons, dropping the empty
/// segment of the `::` data-layer separator and removing the escapes.
fn split_layers(value: &[u8]) -> Vec<Vec<u8>> {
    let mut layers = Vec::new();
    let mut current = Vec::new();
    let mut rest = value;
    while let Some((&byte, tail)) = rest.split_first() {
        match byte {
            b'\\' if tail.first() == Some(&b':') => {
                current.push(b':');
                rest = &tail[1..];
                continue;
            }
            b':' => {
                if !current.is_empty() {
                    layers.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(byte),
        }
        rest = tail;
    }
    if !current.is_empty() {
        layers.push(current);
    }
    layers
}

fn is_within(path: &[u8], mount_point: &[u8]) -> bool {
    if mount_point == b"/" {
        return path.first() == Some(&b'/');
    }
    path.strip_prefix(mount_point)
        .is_some_and(|rest| rest.is_empty() || rest.first() == Some(&b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mount table of a container: overlay root with two lower layers, a
    /// network share, a FUSE mount, a 9p share, memory and disk file systems,
    /// a btrfs subvolume and a mount point containing a space.
    const SAMPLE: &[u8] = b"\
2172 2003 0:186 / / rw,relatime master:1100 - overlay overlay rw,lowerdir=/var/lib/l/1:/var/lib/l/2,upperdir=/var/lib/u/diff,workdir=/var/lib/u/work
2173 2172 0:189 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw
2174 2172 0:190 / /dev rw,nosuid - tmpfs tmpfs rw,size=65536k,mode=755,inode64
2180 2172 0:52 / /mnt/nfs rw,relatime shared:200 - nfs4 server:/export rw,vers=4.2,rsize=1048576
2181 2172 0:53 / /mnt/share rw,relatime - cifs //server/share rw,vers=3.1.1
2182 2172 0:54 / /mnt/remote rw,nosuid,nodev - fuse.sshfs user@host:/home/user rw,user_id=1000,group_id=1000
2183 2172 0:55 / /mnt/host rw,relatime - 9p hostshare rw,trans=virtio
2184 2172 8:1 / /data rw,relatime - ext4 /dev/sda1 rw,errors=remount-ro
2185 2172 8:2 / /srv ro,relatime - xfs /dev/sda2 ro,attr2,inode64,logbufs=8,noquota
2186 2172 0:60 /@home /home rw,relatime - btrfs /dev/nvme0n1p2 rw,ssd,space_cache=v2,subvolid=257,subvol=/@home
2187 2184 8:1 /photos /data/my\\040photos rw,relatime - ext4 /dev/sda1 rw
2188 2172 0:61 / /dev/shm rw,nosuid,nodev - tmpfs shm rw,size=65536k
";

    fn table() -> MountTable {
        MountTable::parse(SAMPLE).expect("sample mount table")
    }

    #[test]
    fn parses_every_line_of_sample_table() {
        let table = table();
        assert_eq!(table.entries().len(), 12);
        let root = &table.entries()[0];
        assert_eq!(root.mount_id, 2172);
        assert_eq!(root.parent_id, 2003);
        assert_eq!((root.major, root.minor), (0, 186));
        assert_eq!(root.root, b"/");
        assert_eq!(root.mount_point, b"/");
        assert_eq!(root.mount_options, ["rw", "relatime"]);
        assert_eq!(root.fs_type, "overlay");
        assert_eq!(root.source, b"overlay");
    }

    #[test]
    fn extracts_overlay_layers() {
        let layers = table().entries()[0]
            .overlay_layers()
            .expect("overlay layers");
        assert_eq!(layers.upper.as_deref(), Some(&b"/var/lib/u/diff"[..]));
        assert_eq!(layers.work.as_deref(), Some(&b"/var/lib/u/work"[..]));
        assert_eq!(layers.lower, [b"/var/lib/l/1".to_vec(), b"/var/lib/l/2".to_vec()]);
    }

    #[test]
    fn returns_no_overlay_layers_for_other_types() {
        assert_eq!(table().entries()[7].overlay_layers(), None);
    }

    #[test]
    fn splits_lower_layers_with_escaped_colon_and_data_layers() {
        let entry = parse_line(
            b"1 0 0:1 / / rw - overlay overlay rw,lowerdir=/a\\:b:/c::/data,lowerdir+=/d,datadir+=/e",
        )
        .expect("line");
        let layers = entry.overlay_layers().expect("overlay layers");
        assert_eq!(layers.upper, None);
        assert_eq!(
            layers.lower,
            [
                b"/a:b".to_vec(),
                b"/c".to_vec(),
                b"/data".to_vec(),
                b"/d".to_vec(),
                b"/e".to_vec()
            ]
        );
    }

    #[test]
    fn unescapes_octal_sequences_in_paths() {
        let table = table();
        let entry = &table.entries()[10];
        assert_eq!(entry.mount_point, b"/data/my photos");
        assert_eq!(entry.root, b"/photos");
        assert_eq!(unescape(b"a\\011b\\012c\\134d\\054e"), b"a\tb\nc\\d,e");
    }

    #[test]
    fn keeps_backslash_without_octal_digits() {
        assert_eq!(unescape(b"a\\x"), b"a\\x");
        assert_eq!(unescape(b"a\\08"), b"a\\08");
        assert_eq!(unescape(b"end\\"), b"end\\");
        assert_eq!(unescape(b"\\777"), b"\\777");
    }

    #[test]
    fn reads_network_and_fuse_sources() {
        let table = table();
        let types: Vec<(&str, &[u8])> = table.entries()[3..7]
            .iter()
            .map(|entry| (entry.fs_type.as_str(), entry.source.as_slice()))
            .collect();
        assert_eq!(
            types,
            [
                ("nfs4", &b"server:/export"[..]),
                ("cifs", &b"//server/share"[..]),
                ("fuse.sshfs", &b"user@host:/home/user"[..]),
                ("9p", &b"hostshare"[..]),
            ]
        );
    }

    #[test]
    fn reads_btrfs_subvolume_options() {
        let table = table();
        let home = &table.entries()[9];
        assert_eq!(home.fs_type, "btrfs");
        assert_eq!(home.root, b"/@home");
        assert_eq!(home.super_option("subvolid"), Some(&b"257"[..]));
        assert_eq!(home.super_option("subvol"), Some(&b"/@home"[..]));
        assert_eq!(home.super_option("ssd"), Some(&b""[..]));
        assert_eq!(home.super_option("space"), None);
        assert!(!home.is_read_only());
    }

    #[test]
    fn detects_read_only_mounts() {
        let table = table();
        assert!(table.entries()[8].is_read_only());
        let super_only = parse_line(b"1 0 8:3 / /ro rw - ext4 /dev/sda3 ro").expect("line");
        assert!(super_only.is_read_only());
    }

    #[test]
    fn finds_mount_containing_path() {
        let table = table();
        let find = |path: &[u8]| {
            table
                .containing(path, None)
                .map(|entry| entry.mount_point.clone())
        };
        assert_eq!(find(b"/"), Some(b"/".to_vec()));
        assert_eq!(find(b"/data"), Some(b"/data".to_vec()));
        assert_eq!(find(b"/data/file"), Some(b"/data".to_vec()));
        assert_eq!(find(b"/datafile"), Some(b"/".to_vec()));
        assert_eq!(find(b"/data/my photos/x"), Some(b"/data/my photos".to_vec()));
        assert_eq!(find(b"/dev/shm/a"), Some(b"/dev/shm".to_vec()));
        assert_eq!(find(b"relative"), None);
    }

    #[test]
    fn prefers_mount_of_matching_device() {
        let table = table();
        let entry = table
            .containing(b"/data/my photos/x", Some((0, 186)))
            .expect("root mount");
        assert_eq!(entry.mount_point, b"/");
        let entry = table
            .containing(b"/data/x", Some((9, 9)))
            .expect("fallback without device match");
        assert_eq!(entry.mount_point, b"/data");
    }

    #[test]
    fn later_mount_shadows_earlier_mount_on_same_point() {
        let text = b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
                     2 1 8:2 / /mnt rw - ext4 /dev/sda2 rw\n\
                     3 1 0:30 / /mnt rw - tmpfs tmpfs rw\n";
        let table = MountTable::parse(text).expect("table");
        let entry = table.containing(b"/mnt/a", None).expect("entry");
        assert_eq!(entry.mount_id, 3);
        assert_eq!(table.by_mount_id(2).map(|entry| entry.minor), Some(2));
        assert_eq!(table.by_mount_id(99), None);
    }

    #[test]
    fn accepts_lines_without_optional_fields_and_empty_lines() {
        let table = MountTable::parse(b"\n1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\n").expect("table");
        assert_eq!(table.entries().len(), 1);
        assert!(MountTable::parse(b"").expect("empty").entries().is_empty());
    }

    #[test]
    fn reports_number_of_malformed_line() {
        let cases: [&[u8]; 6] = [
            b"1 0 8:1 / / rw ext4 /dev/sda1 rw",
            b"1 0 8:1 / / rw - ext4 /dev/sda1",
            b"1 0 8:1 / / rw - ext4 /dev/sda1 rw extra",
            b"x 0 8:1 / / rw - ext4 /dev/sda1 rw",
            b"1 0 81 / / rw - ext4 /dev/sda1 rw",
            b"1 0 8:y / / rw - ext4 /dev/sda1 rw",
        ];
        for case in cases {
            let mut text = b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n".to_vec();
            text.extend_from_slice(case);
            let error = MountTable::parse(&text).expect_err("malformed line");
            assert_eq!(error, MountinfoError { line: 2 });
            assert_eq!(error.to_string(), "malformed mount table line 2");
        }
    }

    #[test]
    fn encodes_device_numbers_like_the_kernel() {
        assert_eq!(device_number(0, 0), 0);
        assert_eq!(device_number(8, 1), 0x0801);
        assert_eq!(device_number(259, 3), 0x0001_0303);
        assert_eq!(device_number(0x1234, 0x5_6789), 0x0000_1000_5672_3489);
        assert_eq!(table().entries()[7].device(), 0x0801);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn device_encoding_matches_system_makedev() {
        for (major, minor) in [(0, 0), (8, 1), (259, 3), (0x1234, 0x5_6789), (u32::MAX, u32::MAX)] {
            assert_eq!(device_number(major, minor), rustix::fs::makedev(major, minor));
        }
    }
}
