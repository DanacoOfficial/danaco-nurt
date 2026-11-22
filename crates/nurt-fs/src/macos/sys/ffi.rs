// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Foreign calls without bindings in `rustix`, with their safety conditions.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::path::Path;
use std::ptr::{self, NonNull};

use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::string::CFStringRef;
use core_foundation::url::{
    CFURL, CFURLRef, kCFURLVolumeAvailableCapacityKey, kCFURLVolumeIsEjectableKey,
    kCFURLVolumeIsRemovableKey, kCFURLVolumeTotalCapacityKey,
};

use crate::macos::{
    AttrList, Q_GETQUOTA, QuotaFigures, USRQUOTA, VOLUME_INFO_REPLY_SIZE, VolumeInfoReply,
    acl_from_system_text, creation_time_buffer, parse_volume_info, quota_command,
};

/// `ACL_TYPE_EXTENDED` of `sys/acl.h`: the only ACL type of macOS.
const ACL_TYPE_EXTENDED: c_int = 0x0000_0100;
/// `CLONE_NOFOLLOW` of `sys/clonefile.h`: clone a symbolic link itself.
const CLONE_NOFOLLOW: u32 = 0x0001;
/// Additional entries allocated for mounts added between the two calls of
/// `getfsstat`.
const MOUNT_SLACK: usize = 8;
/// Bytes of a `u32` word of an attribute buffer.
const WORD_BYTES: usize = 4;

type AclT = *mut c_void;

unsafe extern "C" {
    fn acl_get_file(path: *const c_char, acl_type: c_int) -> AclT;
    fn acl_get_link_np(path: *const c_char, acl_type: c_int) -> AclT;
    fn acl_get_fd_np(fd: c_int, acl_type: c_int) -> AclT;
    fn acl_set_fd_np(fd: c_int, acl: AclT, acl_type: c_int) -> c_int;
    fn acl_from_text(text: *const c_char) -> AclT;
    fn acl_to_text(acl: AclT, length: *mut isize) -> *mut c_char;
    fn acl_init(count: c_int) -> AclT;
    fn acl_free(object: *mut c_void) -> c_int;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFURLVolumeAvailableCapacityForImportantUsageKey: CFStringRef;
    fn CFURLCopyResourcePropertyForKey(
        url: CFURLRef,
        key: CFStringRef,
        value: *mut CFTypeRef,
        error: *mut *mut c_void,
    ) -> u8;
}

/// Object allocated by the ACL library, released with `acl_free`.
struct AclObject(NonNull<c_void>);

impl AclObject {
    fn new(raw: *mut c_void) -> io::Result<Self> {
        NonNull::new(raw)
            .map(Self)
            .ok_or_else(io::Error::last_os_error)
    }
}

impl Drop for AclObject {
    fn drop(&mut self) {
        // SAFETY: the pointer was returned by the ACL library, is non-null and
        // is released exactly once, here.
        unsafe {
            acl_free(self.0.as_ptr());
        }
    }
}

fn acl_text(acl: &AclObject) -> io::Result<Option<Vec<u8>>> {
    let mut length: isize = 0;
    // SAFETY: `acl` is a valid `acl_t`; `length` is a valid out pointer. The
    // returned text is owned by the caller and released by `AclObject`.
    let text = AclObject::new(unsafe { acl_to_text(acl.0.as_ptr(), &raw mut length) }.cast())?;
    // SAFETY: `acl_to_text` returns a NUL-terminated string that stays valid
    // until `text` is dropped at the end of this function.
    let bytes = unsafe { CStr::from_ptr(text.0.as_ptr().cast::<c_char>()) }.to_bytes();
    Ok(acl_from_system_text(bytes))
}

fn read_acl_result(raw: AclT) -> io::Result<Option<Vec<u8>>> {
    match AclObject::new(raw) {
        Ok(acl) => acl_text(&acl),
        Err(error) if no_acl(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Whether a failed ACL query means that the entry has no extended ACL or that
/// the volume does not support ACLs.
fn no_acl(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOENT | libc::ENOTSUP | libc::EOPNOTSUPP)
    )
}

/// ACL of the entry at `path` in stored text form; `None` without an ACL.
pub(super) fn read_path_acl(path: &CStr, follow: bool) -> io::Result<Option<Vec<u8>>> {
    let raw = if follow {
        // SAFETY: `path` is NUL-terminated and valid for the call.
        unsafe { acl_get_file(path.as_ptr(), ACL_TYPE_EXTENDED) }
    } else {
        // SAFETY: `path` is NUL-terminated and valid for the call.
        unsafe { acl_get_link_np(path.as_ptr(), ACL_TYPE_EXTENDED) }
    };
    read_acl_result(raw)
}

/// ACL of an open entry in stored text form; `None` without an ACL.
pub(super) fn read_fd_acl(fd: BorrowedFd<'_>) -> io::Result<Option<Vec<u8>>> {
    // SAFETY: `fd` is an open descriptor for the duration of the call.
    read_acl_result(unsafe { acl_get_fd_np(fd.as_raw_fd(), ACL_TYPE_EXTENDED) })
}

/// Replaces the ACL of an open entry; `None` removes every entry.
pub(super) fn write_fd_acl(fd: BorrowedFd<'_>, text: Option<&CStr>) -> io::Result<()> {
    let raw = match text {
        // SAFETY: `text` is NUL-terminated and valid for the call.
        Some(text) => unsafe { acl_from_text(text.as_ptr()) },
        // SAFETY: `acl_init` has no pointer arguments.
        None => unsafe { acl_init(0) },
    };
    let acl = AclObject::new(raw)?;
    // SAFETY: `fd` is open and `acl` is a valid `acl_t` that outlives the call.
    let result = unsafe { acl_set_fd_np(fd.as_raw_fd(), acl.0.as_ptr(), ACL_TYPE_EXTENDED) };
    check(result)
}

/// `fchflags`.
pub(super) fn set_fd_flags(fd: BorrowedFd<'_>, flags: u32) -> io::Result<()> {
    // SAFETY: `fd` is an open descriptor for the duration of the call.
    check(unsafe { libc::fchflags(fd.as_raw_fd(), flags) })
}

/// `lchflags`: changes the flags of a symbolic link itself, not its target.
pub(super) fn set_link_flags(path: &CStr, flags: u32) -> io::Result<()> {
    // SAFETY: `path` is NUL-terminated and valid for the call.
    check(unsafe { libc::lchflags(path.as_ptr(), flags) })
}

/// Sets the creation time of an open entry with `fsetattrlist`.
pub(super) fn set_creation_time(fd: BorrowedFd<'_>, nanos: i64) -> io::Result<()> {
    let mut request = AttrList::creation_time();
    let mut buffer = creation_time_buffer(nanos);
    // SAFETY: `request` has the layout of `struct attrlist` (asserted at
    // compile time) and asks for `ATTR_CMN_CRTIME` only; `buffer` holds one
    // `struct timespec` of exactly `buffer.len()` bytes. Both outlive the call.
    let result = unsafe {
        libc::fsetattrlist(
            fd.as_raw_fd(),
            (&raw mut request).cast(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            0,
        )
    };
    check(result)
}

/// Capabilities and UUID of the volume mounted at `mount`.
pub(super) fn volume_attributes(mount: &CStr) -> io::Result<VolumeInfoReply> {
    let mut request = AttrList::volume_info();
    let mut words = [0_u32; VOLUME_INFO_REPLY_SIZE / WORD_BYTES];
    let size = size_of_val(&words);
    // SAFETY: `mount` is NUL-terminated; `request` has the layout of
    // `struct attrlist`; `words` is a 4-byte aligned buffer of `size` bytes.
    // All three outlive the call.
    let result = unsafe {
        libc::getattrlist(
            mount.as_ptr(),
            (&raw mut request).cast(),
            words.as_mut_ptr().cast(),
            size,
            0,
        )
    };
    check(result)?;
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_ne_bytes()).collect();
    parse_volume_info(&bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "short volume attribute reply"))
}

/// `pathconf(_PC_CASE_SENSITIVE)`; `None` when the volume does not answer.
pub(super) fn case_sensitive(path: &CStr) -> Option<bool> {
    // SAFETY: `path` is NUL-terminated and valid for the call.
    match unsafe { libc::pathconf(path.as_ptr(), libc::_PC_CASE_SENSITIVE) } {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

/// `clonefile(src, dst, CLONE_NOFOLLOW)`.
pub(super) fn clone_file(src: &CStr, dst: &CStr) -> io::Result<()> {
    // SAFETY: both paths are NUL-terminated and valid for the call.
    check(unsafe { libc::clonefile(src.as_ptr(), dst.as_ptr(), CLONE_NOFOLLOW) })
}

/// Copies content and extended attributes between open files with
/// `fcopyfile(COPYFILE_DATA | COPYFILE_XATTR)`.
pub(super) fn copy_data_and_xattrs(src: BorrowedFd<'_>, dst: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: both descriptors are open for the duration of the call; a null
    // state is accepted by `fcopyfile`.
    check(unsafe {
        libc::fcopyfile(
            src.as_raw_fd(),
            dst.as_raw_fd(),
            ptr::null_mut(),
            libc::COPYFILE_DATA | libc::COPYFILE_XATTR,
        )
    })
}

/// Every mounted file system, as `getfsstat(MNT_NOWAIT)` reports it.
pub(super) fn mounted_volumes() -> io::Result<Vec<libc::statfs>> {
    // SAFETY: a null buffer of size zero asks only for the number of mounts.
    let count = unsafe { libc::getfsstat(ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    let count = usize::try_from(count).map_err(|_| io::Error::last_os_error())?;
    // SAFETY: `struct statfs` consists of integers and byte arrays, for which
    // the all-zero pattern is a valid value.
    let mut mounts = vec![unsafe { std::mem::zeroed::<libc::statfs>() }; count + MOUNT_SLACK];
    let bytes = c_int::try_from(size_of_val(mounts.as_slice()))
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    // SAFETY: `mounts` provides `bytes` bytes of writable `struct statfs`
    // entries for the duration of the call.
    let filled = unsafe { libc::getfsstat(mounts.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
    let filled = usize::try_from(filled).map_err(|_| io::Error::last_os_error())?;
    mounts.truncate(filled);
    Ok(mounts)
}

/// Device number held in `statfs.f_fsid.val[0]`; `libc` keeps the field
/// private.
pub(super) fn fsid_device(fsid: libc::fsid_t) -> i32 {
    // SAFETY: `fsid_t` is `struct { int32_t val[2]; }` with C layout; the
    // sizes of both types are equal, which `transmute` checks at compile time.
    let words: [i32; 2] = unsafe { std::mem::transmute(fsid) };
    words[0]
}

/// Byte quota of the account `uid` on the volume mounted at `mount`; `None`
/// when quotas are not enforced or cannot be read.
pub(super) fn user_quota(mount: &CStr, uid: u32) -> Option<QuotaFigures> {
    let id = c_int::try_from(uid).ok()?;
    // SAFETY: `struct dqblk` consists of integers, for which the all-zero
    // pattern is a valid value.
    let mut quota = unsafe { std::mem::zeroed::<libc::dqblk>() };
    // SAFETY: `mount` is NUL-terminated; `quota` is a writable `struct dqblk`
    // that outlives the call, as `Q_GETQUOTA` requires.
    let result = unsafe {
        libc::quotactl(
            mount.as_ptr(),
            quota_command(Q_GETQUOTA, USRQUOTA),
            id,
            (&raw mut quota).cast(),
        )
    };
    (result == 0).then_some(QuotaFigures {
        hard_limit: quota.dqb_bhardlimit,
        used: quota.dqb_curbytes,
    })
}

/// Volume resource values of Core Foundation for a mount point.
pub(super) struct VolumeUrl(CFURL);

/// Resource key of a volume value.
#[derive(Debug, Clone, Copy)]
pub(super) enum VolumeKey {
    TotalCapacity,
    AvailableCapacity,
    ImportantCapacity,
    IsRemovable,
    IsEjectable,
}

impl VolumeKey {
    fn cf_key(self) -> CFStringRef {
        // SAFETY: the keys are immutable string constants exported by Core
        // Foundation; reading them has no side effects.
        unsafe {
            match self {
                Self::TotalCapacity => kCFURLVolumeTotalCapacityKey,
                Self::AvailableCapacity => kCFURLVolumeAvailableCapacityKey,
                Self::ImportantCapacity => kCFURLVolumeAvailableCapacityForImportantUsageKey,
                Self::IsRemovable => kCFURLVolumeIsRemovableKey,
                Self::IsEjectable => kCFURLVolumeIsEjectableKey,
            }
        }
    }
}

impl VolumeUrl {
    pub(super) fn new(mount: &Path) -> Option<Self> {
        CFURL::from_path(mount, true).map(Self)
    }

    fn property(&self, key: VolumeKey) -> Option<CFType> {
        let mut value: CFTypeRef = ptr::null();
        // SAFETY: the URL and the key are valid Core Foundation objects for
        // the call; `value` receives an owned reference or stays null; a null
        // error pointer means that no error object is requested.
        let found = unsafe {
            CFURLCopyResourcePropertyForKey(
                self.0.as_concrete_TypeRef(),
                key.cf_key(),
                &raw mut value,
                ptr::null_mut(),
            )
        };
        if found == 0 || value.is_null() {
            return None;
        }
        // SAFETY: `CFURLCopyResourcePropertyForKey` follows the create rule;
        // the reference is released when the `CFType` is dropped.
        Some(unsafe { CFType::wrap_under_create_rule(value) })
    }

    /// Numeric value of a key; `None` when absent or negative.
    pub(super) fn number(&self, key: VolumeKey) -> Option<u64> {
        let number = self.property(key)?.downcast::<CFNumber>()?;
        u64::try_from(number.to_i64()?).ok()
    }

    /// Boolean value of a key; `None` when absent.
    pub(super) fn flag(&self, key: VolumeKey) -> Option<bool> {
        self.property(key)?.downcast::<CFBoolean>().map(bool::from)
    }
}

fn check(result: c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
