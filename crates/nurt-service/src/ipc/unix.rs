// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::ipc::unix
// Responsibility:   Unix socket transport of the control channel.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Unix socket transport of the control channel.

use std::fs::{self, DirBuilder};
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Accepted, Listener, TimedRead};

const OWNER_ONLY_SOCKET: u32 = 0o600;
const OWNER_ONLY_DIR: u32 = 0o700;
/// Directory next to the socket in which the socket is bound; its name is
/// short, so the path of the bound socket is shorter than the final one.
const STAGING_DIR: &str = ".bind";
const STAGING_SOCKET: &str = "s";
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_SOCKET_PATH: usize = 103;

/// Listening socket `service.sock`.
pub(super) struct SocketListener {
    listener: UnixListener,
    path: PathBuf,
}

impl SocketListener {
    /// Binds the socket, replacing a socket file left by a previous service.
    pub(super) fn bind(path: &Path) -> io::Result<Self> {
        if path.as_os_str().len() > MAX_SOCKET_PATH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "socket path {} is longer than {MAX_SOCKET_PATH} bytes; set `state.dir` to a shorter directory",
                    path.display()
                ),
            ));
        }
        remove_stale(path)?;
        let staging_dir = path.with_file_name(STAGING_DIR);
        // A directory left by a service that was killed; it holds no file
        // of value, and a link in its place is removed, not followed.
        let _ = fs::remove_dir_all(&staging_dir);
        DirBuilder::new()
            .mode(OWNER_ONLY_DIR)
            .create(&staging_dir)?;
        let bound = Self::bind_in(&staging_dir, path);
        let _ = fs::remove_dir_all(&staging_dir);
        bound
    }

    /// Binds the socket in the private directory and moves it to `path`
    /// once it has its permissions, so the socket is never reachable with
    /// the permissions of the umask.
    fn bind_in(staging_dir: &Path, path: &Path) -> io::Result<Self> {
        let staged = staging_dir.join(STAGING_SOCKET);
        let listener = UnixListener::bind(&staged)?;
        fs::set_permissions(&staged, fs::Permissions::from_mode(OWNER_ONLY_SOCKET))?;
        fs::rename(&staged, path)?;
        Ok(Self {
            listener,
            path: path.to_owned(),
        })
    }
}

fn remove_stale(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} exists and is not a socket; remove it", path.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

impl TimedRead for UnixStream {
    fn set_idle(&mut self, limit: Option<Duration>) -> io::Result<()> {
        self.set_read_timeout(limit)
    }
}

impl Listener for SocketListener {
    fn accept(&self) -> io::Result<Accepted> {
        let (stream, _) = self.listener.accept()?;
        // macOS refuses the option with EINVAL when the peer has already
        // closed the connection. That connection is not an accept error: a
        // failure here would leave the loop waiting before the next accept,
        // and a client that polls and gives up early would keep the loop from
        // ever reaching a live connection. A write to the closed peer fails
        // on its own.
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let writer = stream.try_clone()?;
        let closer = stream.try_clone()?;
        Ok(Accepted {
            reader: Box::new(stream),
            writer: Box::new(writer),
            closer: Box::new(move || {
                let _ = closer.shutdown(Shutdown::Both);
            }),
        })
    }

    fn wake(&self) {
        let _ = UnixStream::connect(&self.path);
    }

    fn remove(&self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Connects to the socket of a running service; a read waits at most
/// `read_limit`, without a limit when it is `None`.
pub(super) fn connect(
    path: &Path,
    read_limit: Option<Duration>,
) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    let stream = UnixStream::connect(path)?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    stream.set_read_timeout(read_limit)?;
    let writer = stream.try_clone()?;
    Ok((Box::new(stream), Box::new(writer)))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn bound_socket_has_owner_only_permissions_and_leaves_no_staging_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service.sock");
        let listener = SocketListener::bind(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, OWNER_ONLY_SOCKET);
        assert!(!dir.path().join(STAGING_DIR).exists());
        assert!(UnixStream::connect(&path).is_ok());
        listener.remove();
        assert!(!path.exists());
    }

    #[test]
    fn a_connection_closed_before_accept_is_accepted_and_does_not_hide_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service.sock");
        let listener = SocketListener::bind(&path).unwrap();
        for _ in 0..3 {
            drop(UnixStream::connect(&path).unwrap());
        }
        let live = UnixStream::connect(&path).unwrap();
        for _ in 0..4 {
            listener
                .accept()
                .expect("a closed connection is not an error");
        }
        drop(live);
    }

    #[test]
    fn leftover_staging_directory_and_stale_socket_are_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service.sock");
        drop(UnixListener::bind(&path).unwrap());
        let staging = dir.path().join(STAGING_DIR);
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join(STAGING_SOCKET), "left over").unwrap();
        let _listener = SocketListener::bind(&path).unwrap();
        assert!(UnixStream::connect(&path).is_ok());
        assert!(!staging.exists());
    }

    #[test]
    fn a_planted_link_in_place_of_the_staging_directory_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join(STAGING_DIR)).unwrap();
        let path = dir.path().join("service.sock");
        let _listener = SocketListener::bind(&path).unwrap();
        assert!(elsewhere.path().read_dir().unwrap().next().is_none());
        assert!(UnixStream::connect(&path).is_ok());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
