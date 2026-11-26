// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Unix socket transport of the control channel.

use std::fs;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Accepted, Listener, TimedRead};

const OWNER_ONLY_SOCKET: u32 = 0o600;
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
        let listener = UnixListener::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(OWNER_ONLY_SOCKET))?;
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
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
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
