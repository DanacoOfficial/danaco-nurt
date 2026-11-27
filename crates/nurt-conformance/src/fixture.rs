// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-conformance::fixture
// Responsibility:   Fixtures that the runner provides to cases.
// Dependencies:     axum, base64, serde_json, tokio
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Fixtures that the runner provides to cases.
//!
//! A case names the fixtures it needs in `requires` as `fixture:<name>`:
//! `fixture:http` starts a local HTTP server on the loopback interface for the
//! case, `fixture:second-volume` gives the case a directory on another file
//! system than its working directory. The module also determines the mount
//! point of the working directory for the placeholder `${work_mount}`.

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use base64::Engine as _;
use serde_json::{Map, Value, json};
use tokio::sync::oneshot;

use crate::Error;
use crate::snapshot::{EntryType, Snapshot};

/// Fixture that a case requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fixture {
    /// Local HTTP server on the loopback interface.
    Http,
    /// Directory on another file system than the working directory.
    SecondVolume,
}

impl Fixture {
    /// Every fixture.
    pub const ALL: [Fixture; 2] = [Self::Http, Self::SecondVolume];

    /// Name of the fixture in `requires`, after `fixture:`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::SecondVolume => "second-volume",
        }
    }

    /// Returns the fixture with the given name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|fixture| fixture.name() == name)
    }
}

/// Largest body of the route `/bytes/{count}`.
pub const MAX_GENERATED_BYTES: u64 = 16 * 1024 * 1024;

/// Body of the last answer of the route `/redirect/{count}`.
pub const REDIRECT_TARGET_BODY: &str = "redirected\n";

/// Content types of served files by extension; other files are
/// `application/octet-stream`.
const CONTENT_TYPES: [(&str, &str); 5] = [
    ("txt", "text/plain; charset=utf-8"),
    ("json", "application/json"),
    ("html", "text/html; charset=utf-8"),
    ("csv", "text/csv; charset=utf-8"),
    ("xml", "application/xml"),
];

/// Content type of files without a known extension.
const OCTET_STREAM: &str = "application/octet-stream";

/// Content type of texts produced by the server itself.
const TEXT_PLAIN: &str = "text/plain; charset=utf-8";

/// Smallest status code that `/status/{code}` returns; informational
/// codes cannot be final answers.
const MIN_STATUS: u16 = 200;

/// Largest status code that `/status/{code}` returns.
const MAX_STATUS: u16 = 599;

/// Name of the thread that runs the server.
const THREAD_NAME: &str = "nurt-conformance-http";

/// Files served below `/files/`, by path.
type Files = BTreeMap<String, Vec<u8>>;

/// Local HTTP server of one case.
///
/// The server listens on `127.0.0.1` on a port chosen by the system and
/// answers the routes described in the documentation of the suite. It stops
/// when the value is dropped.
#[derive(Debug)]
pub struct HttpFixture {
    base: String,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl HttpFixture {
    /// Starts the server; the files of `files` are served below `/files/`.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshot holds a file whose content cannot
    /// be decoded, or when the server cannot listen on the loopback
    /// interface.
    pub fn start(files: Option<&Snapshot>) -> Result<Self, Error> {
        let files = Arc::new(served_files(files)?);
        let unavailable = |error: std::io::Error| {
            Error::Invalid(format!("the HTTP fixture cannot start: {error}"))
        };
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(unavailable)?;
        listener.set_nonblocking(true).map_err(unavailable)?;
        let address = listener.local_addr().map_err(unavailable)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .map_err(unavailable)?;
        let (stop, stopped) = oneshot::channel::<()>();
        let app = router(files);
        let thread = std::thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || {
                runtime.block_on(async move {
                    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                        return;
                    };
                    tokio::select! {
                        _ = axum::serve(listener, app).into_future() => {}
                        _ = stopped => {}
                    }
                });
            })
            .map_err(unavailable)?;
        Ok(Self {
            base: format!("http://{address}"),
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    /// Address of the server, `http://127.0.0.1:<port>`, without a
    /// trailing `/`.
    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        // The server thread may already have ended; both results are
        // irrelevant once the fixture is no longer used.
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn served_files(snapshot: Option<&Snapshot>) -> Result<Files, Error> {
    let mut files = Files::new();
    for entry in snapshot.iter().flat_map(|s| &s.entries) {
        if entry.kind == EntryType::File {
            let content = entry
                .content()
                .map_err(|error| Error::Invalid(format!("http_files: {error}")))?;
            files.insert(entry.path.clone(), content);
        }
    }
    Ok(files)
}

fn router(files: Arc<Files>) -> Router {
    Router::new()
        .route("/echo", any(echo))
        .route("/echo/{*rest}", any(echo))
        .route("/status/{code}", any(status))
        .route("/bytes/{count}", get(bytes))
        .route("/redirect/{count}", get(redirect))
        .route("/files/{*path}", get(file))
        .with_state(files)
}

fn text(status: StatusCode, body: impl Into<String>) -> Response {
    (status, [(header::CONTENT_TYPE, TEXT_PLAIN)], body.into()).into_response()
}

/// Describes the request as JSON: method, path, query, headers and body.
async fn echo(method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, value) in &headers {
        values
            .entry(name.as_str().to_owned())
            .or_default()
            .push(String::from_utf8_lossy(value.as_bytes()).into_owned());
    }
    let headers: Map<String, Value> = values
        .into_iter()
        .map(|(name, values)| (name, Value::String(values.join(", "))))
        .collect();
    let mut document = json!({
        "method": method.as_str(),
        "path": uri.path(),
        "query": uri.query(),
        "headers": headers,
    });
    match std::str::from_utf8(&body) {
        Ok(content) => document["body_utf8"] = json!(content),
        Err(_) => {
            document["body_base64"] =
                json!(base64::engine::general_purpose::STANDARD.encode(&body));
        }
    }
    let body = serde_json::to_string(&document).unwrap_or_default();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

async fn status(UrlPath(code): UrlPath<u16>) -> Response {
    match StatusCode::from_u16(code) {
        Ok(status) if (MIN_STATUS..=MAX_STATUS).contains(&code) => status.into_response(),
        _ => text(
            StatusCode::BAD_REQUEST,
            format!("status {code} is not a final status code\n"),
        ),
    }
}

async fn bytes(UrlPath(count): UrlPath<u64>) -> Response {
    let Some(length) = usize::try_from(count)
        .ok()
        .filter(|_| count <= MAX_GENERATED_BYTES)
    else {
        return text(
            StatusCode::BAD_REQUEST,
            format!("at most {MAX_GENERATED_BYTES} bytes are generated\n"),
        );
    };
    let body: Vec<u8> = (0..=u8::MAX).cycle().take(length).collect();
    (StatusCode::OK, [(header::CONTENT_TYPE, OCTET_STREAM)], body).into_response()
}

async fn redirect(UrlPath(count): UrlPath<u32>) -> Response {
    match count.checked_sub(1) {
        None => text(StatusCode::OK, REDIRECT_TARGET_BODY),
        Some(next) => (
            StatusCode::FOUND,
            [(header::LOCATION, format!("/redirect/{next}"))],
        )
            .into_response(),
    }
}

async fn file(State(files): State<Arc<Files>>, UrlPath(path): UrlPath<String>) -> Response {
    let Some(content) = files.get(&path) else {
        return text(StatusCode::NOT_FOUND, format!("no file {path}\n"));
    };
    let content_type = Path::new(&path)
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(|extension| {
            CONTENT_TYPES
                .iter()
                .find(|(known, _)| known.eq_ignore_ascii_case(extension))
        })
        .map_or(OCTET_STREAM, |(_, content_type)| content_type);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        content.clone(),
    )
        .into_response()
}

/// Returns the mount point of the file system that holds `path`, with `/`
/// as separator.
///
/// On Linux the mount table of the process decides; on other Unix systems
/// the highest ancestor directory on the same device; on Windows the root of
/// the drive or share, for example `C:/`.
///
/// # Errors
///
/// Returns an error when `path` does not exist or its mount point cannot be
/// determined.
pub fn mount_point(path: &Path) -> Result<String, Error> {
    let canonical = path.canonicalize().map_err(|e| Error::io(path, e))?;
    platform::mount_point(&canonical)
        .map(|mount| mount.replace('\\', "/"))
        .ok_or_else(|| {
            Error::Invalid(format!(
                "the mount point of {} cannot be determined",
                path.display()
            ))
        })
}

/// Checks that `dir` is a directory on another volume than the temporary
/// directory of the system, where the runner creates the working
/// directories, and returns its canonical path.
///
/// # Errors
///
/// Returns an error when `dir` is not a directory or is on the volume of
/// the temporary directory.
pub fn check_second_volume(dir: &Path) -> Result<PathBuf, Error> {
    if !dir.is_dir() {
        return Err(Error::Invalid(format!(
            "the second volume {} is not a directory",
            dir.display()
        )));
    }
    let temp = std::env::temp_dir();
    let (volume, temp_volume) = (mount_point(dir)?, mount_point(&temp)?);
    if volume == temp_volume {
        return Err(Error::Invalid(format!(
            "the second volume {} is on the volume {volume} of the temporary directory {}; give a directory on another file system",
            dir.display(),
            temp.display()
        )));
    }
    dir.canonicalize().map_err(|e| Error::io(dir, e))
}

#[cfg(unix)]
mod platform {
    use std::os::unix::fs::MetadataExt as _;
    use std::path::{Path, PathBuf};

    /// Mount table of the process on Linux; absent on other systems.
    const MOUNT_INFO: &str = "/proc/self/mountinfo";

    /// Index of the mount point among the fields of a line of the table.
    const MOUNT_POINT_FIELD: usize = 4;

    /// Number of octal digits of an escaped character of the table.
    const ESCAPE_DIGITS: usize = 3;

    pub(super) fn mount_point(path: &Path) -> Option<String> {
        std::fs::read_to_string(MOUNT_INFO)
            .ok()
            .and_then(|table| longest_mount(&table, path))
            .or_else(|| device_root(path))
            .map(|mount| mount.to_string_lossy().into_owned())
    }

    /// Returns the mount point of the table that is the longest ancestor of
    /// `path`.
    pub(super) fn longest_mount(table: &str, path: &Path) -> Option<PathBuf> {
        table
            .lines()
            .filter_map(|line| line.split(' ').nth(MOUNT_POINT_FIELD))
            .map(|field| PathBuf::from(unescape(field)))
            .filter(|mount| path.starts_with(mount))
            .max_by_key(|mount| mount.components().count())
    }

    /// Decodes the octal escapes `\NNN` of a field of the mount table.
    pub(super) fn unescape(field: &str) -> String {
        let mut out = Vec::with_capacity(field.len());
        let bytes = field.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let escape = bytes
                .get(index + 1..=index + ESCAPE_DIGITS)
                .and_then(|digits| {
                    let digits = std::str::from_utf8(digits).ok()?;
                    u8::from_str_radix(digits, 8).ok()
                });
            match (bytes[index], escape) {
                (b'\\', Some(byte)) => {
                    out.push(byte);
                    index += 1 + ESCAPE_DIGITS;
                }
                (byte, _) => {
                    out.push(byte);
                    index += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Returns the highest ancestor of `path` on the device of `path`.
    fn device_root(path: &Path) -> Option<PathBuf> {
        let device = std::fs::metadata(path).ok()?.dev();
        let mut root = path;
        while let Some(parent) = root.parent() {
            if std::fs::metadata(parent).ok()?.dev() != device {
                break;
            }
            root = parent;
        }
        Some(root.to_path_buf())
    }
}

#[cfg(windows)]
mod platform {
    use std::path::{Component, Path, Prefix};

    pub(super) fn mount_point(path: &Path) -> Option<String> {
        let Component::Prefix(prefix) = path.components().next()? else {
            return None;
        };
        match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                Some(format!("{}:/", char::from(letter)))
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => Some(format!(
                "//{}/{}/",
                server.to_string_lossy(),
                share.to_string_lossy()
            )),
            _ => None,
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use std::path::Path;

    pub(super) fn mount_point(_path: &Path) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;

    /// Sends one request with `Connection: close` and returns the status
    /// line, the headers in lower case and the body.
    fn request(base: &str, method: &str, target: &str, body: &[u8]) -> (String, String, Vec<u8>) {
        let address = base.trim_start_matches("http://");
        let mut stream = TcpStream::connect(address).unwrap();
        let head = format!(
            "{method} {target} HTTP/1.1\r\nHost: {address}\r\nX-Case: one\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).unwrap();
        let split = answer
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("end of the head");
        let head = String::from_utf8(answer[..split].to_vec()).unwrap();
        let (status, headers) = head.split_once("\r\n").unwrap_or((&head, ""));
        (
            status.to_owned(),
            headers.to_ascii_lowercase(),
            answer[split + 4..].to_vec(),
        )
    }

    #[test]
    fn fixture_names_round_trip() {
        for fixture in Fixture::ALL {
            assert_eq!(Fixture::from_name(fixture.name()), Some(fixture));
        }
        assert_eq!(Fixture::from_name("dns"), None);
    }

    fn started() -> HttpFixture {
        let files = Snapshot::from_json(&json!({"entries": [
            {"path": "a.txt", "type": "file", "content_utf8": "alpha\n"},
            {"path": "dir/b.bin", "type": "file", "content_base64": "AAE="},
            {"path": "empty", "type": "dir"}
        ]}))
        .unwrap();
        HttpFixture::start(Some(&files)).unwrap()
    }

    #[test]
    fn the_http_fixture_serves_files() {
        let fixture = started();
        let base = fixture.base().to_owned();
        assert!(base.starts_with("http://127.0.0.1:"), "{base}");

        let (status, headers, body) = request(&base, "GET", "/files/a.txt", b"");
        assert!(status.ends_with("200 OK"), "{status}");
        assert!(headers.contains("content-type: text/plain; charset=utf-8"));
        assert_eq!(body, b"alpha\n");
        let (_, headers, body) = request(&base, "GET", "/files/dir/b.bin", b"");
        assert!(headers.contains("content-type: application/octet-stream"));
        assert_eq!(body, [0, 1]);
        let (status, _, body) = request(&base, "HEAD", "/files/a.txt", b"");
        assert!(status.ends_with("200 OK"), "{status}");
        assert!(body.is_empty());
        let (status, _, _) = request(&base, "GET", "/files/none.txt", b"");
        assert!(status.contains(" 404 "), "{status}");
    }

    #[test]
    fn the_http_fixture_echoes_requests() {
        let fixture = started();
        let base = fixture.base().to_owned();
        let (status, headers, body) = request(&base, "PATCH", "/echo/x?a=1", b"payload");
        assert!(status.ends_with("200 OK"), "{status}");
        assert!(headers.contains("content-type: application/json"));
        let echo: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(echo["method"], "PATCH");
        assert_eq!(echo["path"], "/echo/x");
        assert_eq!(echo["query"], "a=1");
        assert_eq!(echo["headers"]["x-case"], "one");
        assert_eq!(echo["body_utf8"], "payload");
        let (_, _, body) = request(&base, "POST", "/echo", &[0xff, 0x00]);
        let echo: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(echo["body_base64"], "/wA=");
        assert_eq!(echo["query"], Value::Null);
    }

    #[test]
    fn the_http_fixture_serves_status_bytes_and_redirects() {
        let fixture = started();
        let base = fixture.base().to_owned();
        let (status, _, _) = request(&base, "DELETE", "/status/418", b"");
        assert!(status.contains(" 418 "), "{status}");
        let (status, _, _) = request(&base, "GET", "/status/101", b"");
        assert!(status.contains(" 400 "), "{status}");
        let (status, _, _) = request(&base, "GET", "/status/599", b"");
        assert!(status.contains(" 599 "), "{status}");
        let (status, _, _) = request(&base, "GET", "/status/600", b"");
        assert!(status.contains(" 400 "), "{status}");
        let (_, _, body) = request(&base, "GET", "/bytes/300", b"");
        assert_eq!(body.len(), 300);
        assert_eq!((body[0], body[255], body[256]), (0, 255, 0));
        let (status, _, _) = request(&base, "GET", "/bytes/999999999999", b"");
        assert!(status.contains(" 400 "), "{status}");
        let (status, headers, _) = request(&base, "GET", "/redirect/2", b"");
        assert!(status.contains(" 302 "), "{status}");
        assert!(headers.contains("location: /redirect/1"), "{headers}");
        let (_, _, body) = request(&base, "GET", "/redirect/0", b"");
        assert_eq!(body, REDIRECT_TARGET_BODY.as_bytes());

        drop(fixture);
        assert!(TcpStream::connect(base.trim_start_matches("http://")).is_err());
    }

    #[test]
    fn invalid_served_files_are_errors() {
        let mut files = Snapshot::from_json(
            &json!({"entries": [{"path": "a", "type": "file", "content_utf8": "a"}]}),
        )
        .unwrap();
        files.entries[0].content_utf8 = None;
        files.entries[0].content_base64 = Some("!".to_owned());
        assert!(HttpFixture::start(Some(&files)).is_err());
        assert!(HttpFixture::start(None).is_ok());
    }

    #[test]
    fn mount_points_contain_their_paths() {
        let dir = tempfile::tempdir().unwrap();
        let mount = mount_point(dir.path()).unwrap();
        assert!(!mount.contains('\\'));
        let canonical = dir
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let canonical = canonical.trim_start_matches("//?/");
        assert!(
            canonical.starts_with(mount.trim_end_matches('/')),
            "{mount} {canonical}"
        );
        assert!(mount_point(&dir.path().join("none")).is_err());
    }

    #[test]
    fn the_temporary_directory_is_not_a_second_volume() {
        let dir = tempfile::tempdir().unwrap();
        let error = check_second_volume(dir.path()).unwrap_err().to_string();
        assert!(error.contains("another file system"), "{error}");
        let file = dir.path().join("f");
        std::fs::write(&file, "").unwrap();
        assert!(
            check_second_volume(&file)
                .unwrap_err()
                .to_string()
                .contains("not a directory")
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_longest_mount_of_the_table_is_chosen() {
        let table = "\
22 1 0:21 / / rw - ext4 /dev/root rw
23 22 0:22 / /tmp rw - tmpfs tmpfs rw
24 23 0:23 / /tmp/my\\040disk rw - tmpfs tmpfs rw
25 22 0:24 / /tmpfiles rw - tmpfs tmpfs rw
";
        let pick = |path: &str| platform::longest_mount(table, Path::new(path));
        assert_eq!(pick("/tmp/my disk/a"), Some(PathBuf::from("/tmp/my disk")));
        assert_eq!(pick("/tmp/x"), Some(PathBuf::from("/tmp")));
        assert_eq!(pick("/tmpfiles/x"), Some(PathBuf::from("/tmpfiles")));
        assert_eq!(pick("/srv"), Some(PathBuf::from("/")));
        assert_eq!(platform::unescape("a\\134b\\0"), "a\\b\\0");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
