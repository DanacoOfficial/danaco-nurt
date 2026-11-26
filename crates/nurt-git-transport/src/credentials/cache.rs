// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Client of the `git credential-cache` daemon.
//!
//! The daemon keeps credentials in memory and listens on a Unix socket. A
//! request is `action=<action>`, `timeout=<seconds>` and the attributes of
//! the credential protocol, one per line; the client closes its side and
//! reads the answer. The daemon is not started when it does not run, so a
//! missing socket means the cache holds nothing.

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gix_credentials::protocol::Context;

/// Lifetime of stored credentials sent with every request; the default of
/// `git credential-cache --timeout`.
const CACHE_TIMEOUT_SECONDS: u32 = 900;

/// Time allowed for each read and write on the socket.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Largest answer accepted from the daemon; a credential is a few lines.
const MAX_ANSWER_BYTES: usize = 64 * 1024;

/// Time allowed for the whole exchange, so that a daemon that answers a byte
/// at a time cannot hold the operation.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);

/// Directory used by `git` when it exists, before the XDG cache directory.
const LEGACY_DIRECTORY: &str = ".git-credential-cache";

/// Socket of the daemon: `--socket`, `~/.git-credential-cache/socket` when
/// that directory exists, otherwise `$XDG_CACHE_HOME/git/credential/socket`.
pub(crate) fn socket_path(
    option: Option<&str>,
    home: Option<&Path>,
    xdg_cache: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(path) = option {
        return Some(super::expand_home(path, home));
    }
    let home = home?;
    let legacy = home.join(LEGACY_DIRECTORY);
    if legacy.is_dir() {
        return Some(legacy.join("socket"));
    }
    let cache = xdg_cache.map_or_else(|| home.join(".cache"), Path::to_path_buf);
    Some(cache.join("git").join("credential").join("socket"))
}

/// Sends `action` with `context` and returns the answer of the daemon.
pub(crate) fn request(socket: &Path, action: &str, context: &Context) -> io::Result<Vec<u8>> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut message = format!("action={action}\ntimeout={CACHE_TIMEOUT_SECONDS}\n").into_bytes();
    context.write_to(&mut message)?;
    stream.write_all(&message)?;
    stream.shutdown(Shutdown::Write)?;
    read_answer(&mut stream, MAX_ANSWER_BYTES, TOTAL_TIMEOUT)
}

/// Reads `source` to its end; fails with `InvalidData` when more than `limit`
/// bytes arrive and with `TimedOut` when the end does not come within
/// `total`.
fn read_answer(source: &mut impl Read, limit: usize, total: Duration) -> io::Result<Vec<u8>> {
    let started = Instant::now();
    let mut answer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = source.read(&mut chunk)?;
        if read == 0 {
            return Ok(answer);
        }
        answer.extend_from_slice(&chunk[..read]);
        if answer.len() > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the answer of the credential cache is too large",
            ));
        }
        if started.elapsed() > total {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the credential cache did not finish its answer in time",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn prefers_option_then_legacy_directory_then_xdg_cache() {
        let home = tempfile::tempdir().expect("temporary directory");
        assert_eq!(
            socket_path(Some("~/s"), Some(home.path()), None),
            Some(home.path().join("s"))
        );
        assert_eq!(
            socket_path(None, Some(home.path()), Some(Path::new("/xdg"))),
            Some(PathBuf::from("/xdg/git/credential/socket"))
        );
        std::fs::create_dir(home.path().join(LEGACY_DIRECTORY)).expect("directory created");
        assert_eq!(
            socket_path(None, Some(home.path()), None),
            Some(home.path().join(LEGACY_DIRECTORY).join("socket"))
        );
        assert_eq!(socket_path(None, None, None), None);
    }

    #[test]
    fn exchanges_a_request_with_the_daemon() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let socket = dir.path().join("socket");
        let listener = UnixListener::bind(&socket).expect("socket bound");
        let daemon = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("connection");
            let mut request = String::new();
            stream.read_to_string(&mut request).expect("request");
            stream
                .write_all(b"username=bob\npassword=pw\n")
                .expect("answer");
            request
        });
        let context = Context {
            protocol: Some("https".to_owned()),
            host: Some("example.com".to_owned()),
            ..Context::default()
        };
        let answer = request(&socket, "get", &context).expect("answer");
        let request = daemon.join().expect("daemon thread");
        assert!(request.starts_with("action=get\ntimeout=900\n"));
        assert!(request.contains("host=example.com\n"));
        assert_eq!(answer, b"username=bob\npassword=pw\n");
    }

    struct Dripping;

    impl Read for Dripping {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            std::thread::sleep(Duration::from_millis(20));
            buffer[0] = b'x';
            Ok(1)
        }
    }

    #[test]
    fn oversized_answer_is_refused() {
        let mut source = io::Cursor::new(vec![b'x'; 100]);
        let error = read_answer(&mut source, 50, TOTAL_TIMEOUT).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let mut source = io::Cursor::new(vec![b'x'; 50]);
        assert_eq!(
            read_answer(&mut source, 50, TOTAL_TIMEOUT).unwrap().len(),
            50
        );
    }

    #[test]
    fn answer_that_never_ends_times_out() {
        let error = read_answer(&mut Dripping, usize::MAX, Duration::from_millis(100)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn missing_daemon_is_an_error() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert!(request(&dir.path().join("none"), "get", &Context::default()).is_err());
    }
}
