// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Classification of Win32 error codes.
//!
//! The codes are defined here with their documented numeric values so that the
//! classification compiles and is tested on every host system. Windows builds
//! assert at compile time that every value equals the one in `windows-sys`.

use std::io;
use std::path::Path;

use nurt_contracts::fs::{FsError, FsErrorKind};

/// Win32 error codes used by the Windows back end.
pub mod codes {
    /// Incorrect function; returned by file systems for unsupported requests.
    pub const ERROR_INVALID_FUNCTION: u32 = 1;
    /// The file does not exist.
    pub const ERROR_FILE_NOT_FOUND: u32 = 2;
    /// A directory component of the path does not exist.
    pub const ERROR_PATH_NOT_FOUND: u32 = 3;
    /// Access denied, also returned for a file in the delete-pending state.
    pub const ERROR_ACCESS_DENIED: u32 = 5;
    /// The drive does not exist.
    pub const ERROR_INVALID_DRIVE: u32 = 15;
    /// Source and destination are on different volumes.
    pub const ERROR_NOT_SAME_DEVICE: u32 = 17;
    /// No more entries in a directory enumeration.
    pub const ERROR_NO_MORE_FILES: u32 = 18;
    /// The medium is write-protected.
    pub const ERROR_WRITE_PROTECT: u32 = 19;
    /// Another open handle denies the requested access.
    pub const ERROR_SHARING_VIOLATION: u32 = 32;
    /// A byte-range lock held by another handle denies the access.
    pub const ERROR_LOCK_VIOLATION: u32 = 33;
    /// End of file reached.
    pub const ERROR_HANDLE_EOF: u32 = 38;
    /// The disk is full.
    pub const ERROR_HANDLE_DISK_FULL: u32 = 39;
    /// The request is not supported.
    pub const ERROR_NOT_SUPPORTED: u32 = 50;
    /// The network path was not found.
    pub const ERROR_BAD_NETPATH: u32 = 53;
    /// The network share name was not found.
    pub const ERROR_BAD_NET_NAME: u32 = 67;
    /// The file exists.
    pub const ERROR_FILE_EXISTS: u32 = 80;
    /// A parameter is incorrect; returned by older systems for unknown
    /// information classes and flags.
    pub const ERROR_INVALID_PARAMETER: u32 = 87;
    /// There is not enough space on the disk.
    pub const ERROR_DISK_FULL: u32 = 112;
    /// The function is not implemented on this system.
    pub const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;
    /// The data area passed to a system call is too small.
    pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    /// The file name, directory name or volume label syntax is incorrect.
    pub const ERROR_INVALID_NAME: u32 = 123;
    /// The directory is not empty.
    pub const ERROR_DIR_NOT_EMPTY: u32 = 145;
    /// The entry exists.
    pub const ERROR_ALREADY_EXISTS: u32 = 183;
    /// The file name or extension is too long.
    pub const ERROR_FILENAME_EXCED_RANGE: u32 = 206;
    /// More data is available.
    pub const ERROR_MORE_DATA: u32 = 234;
    /// The directory name is invalid; returned when a directory is expected.
    pub const ERROR_DIRECTORY: u32 = 267;
    /// The file is in the delete-pending state.
    pub const ERROR_DELETE_PENDING: u32 = 303;
    /// The operation was aborted.
    pub const ERROR_OPERATION_ABORTED: u32 = 995;
    /// `ReplaceFileW`: the replaced file could not be deleted; both files keep
    /// their names.
    pub const ERROR_UNABLE_TO_REMOVE_REPLACED: u32 = 1175;
    /// `ReplaceFileW`: the replacement file could not be renamed; with a backup
    /// name both files keep their names.
    pub const ERROR_UNABLE_TO_MOVE_REPLACEMENT: u32 = 1176;
    /// `ReplaceFileW`: the replacement file could not be moved; the replaced
    /// file exists under the backup name.
    pub const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: u32 = 1177;
    /// The disk quota of the account is exceeded.
    pub const ERROR_DISK_QUOTA_EXCEEDED: u32 = 1295;
    /// The security identifier may not be assigned as the account of the
    /// object.
    pub const ERROR_INVALID_OWNER: u32 = 1307;
    /// A required privilege is not held by the client.
    pub const ERROR_PRIVILEGE_NOT_HELD: u32 = 1314;
    /// The name of the file cannot be resolved by the system (link loop).
    pub const ERROR_CANT_RESOLVE_FILENAME: u32 = 1921;
    /// The file or directory is not a reparse point.
    pub const ERROR_NOT_A_REPARSE_POINT: u32 = 4390;
}

#[allow(
    clippy::wildcard_imports,
    reason = "the classification table uses every code of the module by its Win32 name"
)]
use codes::*;

/// Classifies a Win32 error code.
///
/// `ERROR_ACCESS_DENIED` is also the code of an open request for a file in the
/// delete-pending state (deleted while another handle keeps it open); such a
/// file is reported as [`FsErrorKind::PermissionDenied`] until the last handle
/// closes. Sharing and lock violations are classified as
/// [`FsErrorKind::InUse`]; the back end returns them only after the retry
/// period. Codes without a specific kind map to [`FsErrorKind::Io`].
#[must_use]
pub fn error_kind(code: u32) -> FsErrorKind {
    match code {
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_INVALID_DRIVE | ERROR_BAD_NETPATH
        | ERROR_BAD_NET_NAME => FsErrorKind::NotFound,
        ERROR_ACCESS_DENIED | ERROR_DELETE_PENDING | ERROR_INVALID_OWNER => {
            FsErrorKind::PermissionDenied
        }
        ERROR_PRIVILEGE_NOT_HELD => FsErrorKind::Privilege,
        ERROR_ALREADY_EXISTS | ERROR_FILE_EXISTS => FsErrorKind::Exists,
        ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION => FsErrorKind::InUse,
        ERROR_DISK_FULL | ERROR_HANDLE_DISK_FULL => FsErrorKind::NoSpace,
        ERROR_DISK_QUOTA_EXCEEDED => FsErrorKind::QuotaExceeded,
        ERROR_NOT_SAME_DEVICE => FsErrorKind::CrossDevice,
        ERROR_NOT_SUPPORTED | ERROR_INVALID_FUNCTION | ERROR_CALL_NOT_IMPLEMENTED => {
            FsErrorKind::Unsupported
        }
        ERROR_WRITE_PROTECT => FsErrorKind::ReadOnly,
        ERROR_DIR_NOT_EMPTY => FsErrorKind::NotEmpty,
        ERROR_DIRECTORY => FsErrorKind::NotADirectory,
        ERROR_FILENAME_EXCED_RANGE => FsErrorKind::NameTooLong,
        ERROR_INVALID_NAME | ERROR_NOT_A_REPARSE_POINT => FsErrorKind::InvalidPath,
        ERROR_CANT_RESOLVE_FILENAME => FsErrorKind::Loop,
        ERROR_OPERATION_ABORTED => FsErrorKind::Interrupted,
        _ => FsErrorKind::Io,
    }
}

/// Whether the code reports a conflict with another open handle that may
/// disappear when the other program closes the file.
#[must_use]
pub fn is_sharing_conflict(code: u32) -> bool {
    matches!(code, ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
}

/// Whether the code reports that the system or the file system does not know
/// an information class, flag or control code; callers then use an older
/// mechanism.
#[must_use]
pub fn is_unsupported_request(code: u32) -> bool {
    matches!(
        code,
        ERROR_INVALID_PARAMETER
            | ERROR_NOT_SUPPORTED
            | ERROR_INVALID_FUNCTION
            | ERROR_CALL_NOT_IMPLEMENTED
    )
}

/// Builds a classified error with the raw code kept in `os_code`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::fs::FsErrorKind;
/// use nurt_fs::windows::error::{codes, os_error};
///
/// let error = os_error(codes::ERROR_SHARING_VIOLATION, "rename", None);
/// assert_eq!(error.kind, FsErrorKind::InUse);
/// assert_eq!(error.os_code, Some(32));
/// ```
#[must_use]
pub fn os_error(code: u32, op: &str, path: Option<&Path>) -> FsError {
    let mut error = FsError::new(error_kind(code), op, path);
    error.os_code = Some(code.cast_signed());
    error
}

/// Converts an error of the standard library raised by a Windows call.
///
/// The raw code decides when present; otherwise the [`io::ErrorKind`] is
/// mapped to the nearest [`FsErrorKind`].
#[must_use]
pub fn from_io(error: &io::Error, op: &str, path: Option<&Path>) -> FsError {
    if let Some(code) = error.raw_os_error() {
        return os_error(code.cast_unsigned(), op, path);
    }
    let kind = match error.kind() {
        io::ErrorKind::NotFound => FsErrorKind::NotFound,
        io::ErrorKind::PermissionDenied => FsErrorKind::PermissionDenied,
        io::ErrorKind::AlreadyExists => FsErrorKind::Exists,
        io::ErrorKind::Unsupported => FsErrorKind::Unsupported,
        io::ErrorKind::StorageFull => FsErrorKind::NoSpace,
        io::ErrorKind::QuotaExceeded => FsErrorKind::QuotaExceeded,
        io::ErrorKind::CrossesDevices => FsErrorKind::CrossDevice,
        io::ErrorKind::ReadOnlyFilesystem => FsErrorKind::ReadOnly,
        io::ErrorKind::InvalidFilename => FsErrorKind::InvalidPath,
        io::ErrorKind::NotADirectory => FsErrorKind::NotADirectory,
        io::ErrorKind::IsADirectory => FsErrorKind::IsADirectory,
        io::ErrorKind::DirectoryNotEmpty => FsErrorKind::NotEmpty,
        io::ErrorKind::Interrupted => FsErrorKind::Interrupted,
        _ => FsErrorKind::Io,
    };
    FsError::new(kind, op, path)
}

#[cfg(windows)]
mod verify {
    use super::codes;
    use windows_sys::Win32::Foundation as w;

    macro_rules! same {
        ($($name:ident),* $(,)?) => {
            $(const _: () = assert!(codes::$name == w::$name);)*
        };
    }

    same!(
        ERROR_INVALID_FUNCTION,
        ERROR_FILE_NOT_FOUND,
        ERROR_PATH_NOT_FOUND,
        ERROR_ACCESS_DENIED,
        ERROR_INVALID_DRIVE,
        ERROR_NOT_SAME_DEVICE,
        ERROR_NO_MORE_FILES,
        ERROR_WRITE_PROTECT,
        ERROR_SHARING_VIOLATION,
        ERROR_LOCK_VIOLATION,
        ERROR_HANDLE_EOF,
        ERROR_HANDLE_DISK_FULL,
        ERROR_NOT_SUPPORTED,
        ERROR_BAD_NETPATH,
        ERROR_BAD_NET_NAME,
        ERROR_FILE_EXISTS,
        ERROR_INVALID_PARAMETER,
        ERROR_DISK_FULL,
        ERROR_CALL_NOT_IMPLEMENTED,
        ERROR_INSUFFICIENT_BUFFER,
        ERROR_INVALID_NAME,
        ERROR_DIR_NOT_EMPTY,
        ERROR_ALREADY_EXISTS,
        ERROR_FILENAME_EXCED_RANGE,
        ERROR_MORE_DATA,
        ERROR_DIRECTORY,
        ERROR_DELETE_PENDING,
        ERROR_OPERATION_ABORTED,
        ERROR_UNABLE_TO_REMOVE_REPLACED,
        ERROR_UNABLE_TO_MOVE_REPLACEMENT,
        ERROR_UNABLE_TO_MOVE_REPLACEMENT_2,
        ERROR_DISK_QUOTA_EXCEEDED,
        ERROR_INVALID_OWNER,
        ERROR_PRIVILEGE_NOT_HELD,
        ERROR_CANT_RESOLVE_FILENAME,
        ERROR_NOT_A_REPARSE_POINT,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_documented_code_to_its_kind() {
        let table = [
            (ERROR_FILE_NOT_FOUND, FsErrorKind::NotFound),
            (ERROR_PATH_NOT_FOUND, FsErrorKind::NotFound),
            (ERROR_INVALID_DRIVE, FsErrorKind::NotFound),
            (ERROR_BAD_NETPATH, FsErrorKind::NotFound),
            (ERROR_BAD_NET_NAME, FsErrorKind::NotFound),
            (ERROR_ACCESS_DENIED, FsErrorKind::PermissionDenied),
            (ERROR_DELETE_PENDING, FsErrorKind::PermissionDenied),
            (ERROR_INVALID_OWNER, FsErrorKind::PermissionDenied),
            (ERROR_PRIVILEGE_NOT_HELD, FsErrorKind::Privilege),
            (ERROR_ALREADY_EXISTS, FsErrorKind::Exists),
            (ERROR_FILE_EXISTS, FsErrorKind::Exists),
            (ERROR_SHARING_VIOLATION, FsErrorKind::InUse),
            (ERROR_LOCK_VIOLATION, FsErrorKind::InUse),
            (ERROR_DISK_FULL, FsErrorKind::NoSpace),
            (ERROR_HANDLE_DISK_FULL, FsErrorKind::NoSpace),
            (ERROR_DISK_QUOTA_EXCEEDED, FsErrorKind::QuotaExceeded),
            (ERROR_NOT_SAME_DEVICE, FsErrorKind::CrossDevice),
            (ERROR_NOT_SUPPORTED, FsErrorKind::Unsupported),
            (ERROR_INVALID_FUNCTION, FsErrorKind::Unsupported),
            (ERROR_CALL_NOT_IMPLEMENTED, FsErrorKind::Unsupported),
            (ERROR_WRITE_PROTECT, FsErrorKind::ReadOnly),
            (ERROR_DIR_NOT_EMPTY, FsErrorKind::NotEmpty),
            (ERROR_DIRECTORY, FsErrorKind::NotADirectory),
            (ERROR_FILENAME_EXCED_RANGE, FsErrorKind::NameTooLong),
            (ERROR_INVALID_NAME, FsErrorKind::InvalidPath),
            (ERROR_NOT_A_REPARSE_POINT, FsErrorKind::InvalidPath),
            (ERROR_CANT_RESOLVE_FILENAME, FsErrorKind::Loop),
            (ERROR_OPERATION_ABORTED, FsErrorKind::Interrupted),
            (ERROR_INVALID_PARAMETER, FsErrorKind::Io),
            (ERROR_UNABLE_TO_MOVE_REPLACEMENT_2, FsErrorKind::Io),
            (ERROR_MORE_DATA, FsErrorKind::Io),
        ];
        for (code, kind) in table {
            assert_eq!(error_kind(code), kind, "code {code}");
        }
    }

    #[test]
    fn keeps_raw_code_and_context() {
        let error = os_error(
            ERROR_DISK_QUOTA_EXCEEDED,
            "create_temp_in",
            Some(Path::new("dir")),
        );
        assert_eq!(error.kind, FsErrorKind::QuotaExceeded);
        assert_eq!(error.op, "create_temp_in");
        assert_eq!(error.path.as_deref(), Some(Path::new("dir")));
        assert_eq!(error.os_code, Some(1295));
        assert!(error.holders.is_empty());
    }

    #[test]
    fn recognises_sharing_conflicts_and_unsupported_requests() {
        assert!(is_sharing_conflict(ERROR_SHARING_VIOLATION));
        assert!(is_sharing_conflict(ERROR_LOCK_VIOLATION));
        assert!(!is_sharing_conflict(ERROR_ACCESS_DENIED));
        assert!(is_unsupported_request(ERROR_INVALID_PARAMETER));
        assert!(is_unsupported_request(ERROR_NOT_SUPPORTED));
        assert!(is_unsupported_request(ERROR_INVALID_FUNCTION));
        assert!(is_unsupported_request(ERROR_CALL_NOT_IMPLEMENTED));
        assert!(!is_unsupported_request(ERROR_ACCESS_DENIED));
    }

    #[test]
    fn converts_io_errors_by_raw_code_first() {
        let raw = io::Error::from_raw_os_error(5);
        let error = from_io(&raw, "open_read", None);
        assert_eq!(error.kind, FsErrorKind::PermissionDenied);
        assert_eq!(error.os_code, Some(5));
    }

    #[test]
    fn converts_io_errors_without_raw_code_by_kind() {
        let table = [
            (io::ErrorKind::NotFound, FsErrorKind::NotFound),
            (
                io::ErrorKind::PermissionDenied,
                FsErrorKind::PermissionDenied,
            ),
            (io::ErrorKind::AlreadyExists, FsErrorKind::Exists),
            (io::ErrorKind::Unsupported, FsErrorKind::Unsupported),
            (io::ErrorKind::StorageFull, FsErrorKind::NoSpace),
            (io::ErrorKind::QuotaExceeded, FsErrorKind::QuotaExceeded),
            (io::ErrorKind::CrossesDevices, FsErrorKind::CrossDevice),
            (io::ErrorKind::ReadOnlyFilesystem, FsErrorKind::ReadOnly),
            (io::ErrorKind::InvalidFilename, FsErrorKind::InvalidPath),
            (io::ErrorKind::NotADirectory, FsErrorKind::NotADirectory),
            (io::ErrorKind::IsADirectory, FsErrorKind::IsADirectory),
            (io::ErrorKind::DirectoryNotEmpty, FsErrorKind::NotEmpty),
            (io::ErrorKind::Interrupted, FsErrorKind::Interrupted),
            (io::ErrorKind::UnexpectedEof, FsErrorKind::Io),
        ];
        for (io_kind, kind) in table {
            let error = from_io(&io::Error::from(io_kind), "copy_file", None);
            assert_eq!(error.kind, kind, "{io_kind:?}");
            assert_eq!(error.os_code, None);
        }
    }
}
