// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Download of a file over HTTP.
//!
//! `http.download` streams the body of a `GET` response into the partial
//! file `<to>.part` next to the target and records the address and the
//! validator of the response (a strong `ETag`, otherwise `Last-Modified`) in
//! `<to>.part.meta`. With `resume: true` a later run asks for the rest with
//! `Range` and `If-Range` (RFC 9110, sections 14.2 and 13.1.5): a server
//! whose resource changed answers with the whole content and the download
//! starts again from the first byte.
//!
//! The digest given in `hash` is computed while the bytes arrive. After a
//! complete download with a matching digest the content is switched in
//! through the journal transaction, so the previous content of the target
//! can be restored, and the partial file is removed. A digest mismatch
//! removes the partial file and leaves the target unchanged.
//!
//! The partial file is written outside the journal: it is not a target named
//! by the user, and it stays after a failure or a cancellation so that the
//! download can continue.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use indexmap::IndexMap;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, Follow};
use nurt_contracts::journal::{Expect, Mutation, MutationTx, TargetAction, TargetPath, TargetSpec};
use nurt_contracts::record::{Change, ChangeAction, Effect, EffectStatus};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds,
    NetworkTarget, RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, Size, Value};
use sha2::{Digest, Sha256, Sha512};

use crate::client::{
    ClientSettings, Exchange, HttpClient, Method, NameResolver, RequestSpec, ResponseHead, Target,
};

/// Name of the command in the catalog.
pub const COMMAND: &str = "http.download";

const PARAM_URL: &str = "url";
const PARAM_TO: &str = "to";
const PARAM_RESUME: &str = "resume";
const PARAM_HASH: &str = "hash";
const PARAM_HEADERS: &str = "headers";
const PARAM_TIMEOUT: &str = "timeout";

/// Time limit of the download when `timeout` is not given.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
/// Suffix of the partial file.
pub const PART_SUFFIX: &str = ".part";
/// Suffix of the file that describes the partial file.
pub const META_SUFFIX: &str = ".part.meta";
/// Bytes read from the response per step.
const CHUNK: usize = 64 * 1024;
/// Status of a response with a part of the content.
const STATUS_PARTIAL: u16 = 206;
/// Status of a response to a range beyond the end of the content.
const STATUS_RANGE_NOT_SATISFIABLE: u16 = 416;
/// Lowest status of a successful response.
const STATUS_SUCCESS_MIN: u16 = 200;
/// Lowest status that is not successful.
const STATUS_SUCCESS_END: u16 = 300;
/// Line prefix of the address in the description of the partial file.
const META_URL: &str = "url ";
/// Line prefix of the validator in the description of the partial file.
const META_VALIDATOR: &str = "validator ";
/// Headers set by the command itself.
const RESERVED_HEADERS: &[&str] = &["range", "if-range"];

/// Digest algorithm of the `hash` parameter.
///
/// ```
/// use nurt_net::http::download::HashAlgorithm;
///
/// assert_eq!(HashAlgorithm::Blake3.as_str(), "blake3");
/// assert_eq!(HashAlgorithm::Sha512.digest_len(), 64);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HashAlgorithm {
    /// SHA-256.
    Sha256,
    /// SHA-512.
    Sha512,
    /// BLAKE3 with a 256-bit output.
    Blake3,
}

impl HashAlgorithm {
    /// Every algorithm, in the order of the documentation.
    pub const ALL: &'static [Self] = &[Self::Sha256, Self::Sha512, Self::Blake3];

    /// Name used as the prefix of a digest, for example `sha256`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
            Self::Blake3 => "blake3",
        }
    }

    /// Length of a digest in bytes.
    #[must_use]
    pub const fn digest_len(self) -> usize {
        match self {
            Self::Sha256 | Self::Blake3 => 32,
            Self::Sha512 => 64,
        }
    }

    fn hasher(self) -> Hasher {
        match self {
            Self::Sha256 => Hasher::Sha256(Sha256::new()),
            Self::Sha512 => Hasher::Sha512(Sha512::new()),
            Self::Blake3 => Hasher::Blake3(Box::new(blake3::Hasher::new())),
        }
    }
}

impl fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Digest of the downloaded content expected by the `hash` parameter.
///
/// The text form is `<algorithm>:<hexadecimal digest>`; the algorithm and the
/// digits are read without regard to case.
///
/// ```
/// use nurt_net::http::download::{ExpectedHash, HashAlgorithm};
///
/// let text = format!("SHA256:{}", "AB".repeat(32));
/// let hash: ExpectedHash = text.parse().unwrap();
/// assert_eq!(hash.algorithm, HashAlgorithm::Sha256);
/// assert_eq!(hash.to_text(), format!("sha256:{}", "ab".repeat(32)));
/// assert!("md5:00".parse::<ExpectedHash>().is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedHash {
    /// Algorithm.
    pub algorithm: HashAlgorithm,
    /// Digest bytes.
    pub digest: Vec<u8>,
}

impl ExpectedHash {
    /// Text form with a lower-case algorithm and digits.
    #[must_use]
    pub fn to_text(&self) -> String {
        digest_text(self.algorithm, &self.digest)
    }
}

impl FromStr for ExpectedHash {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let names: Vec<&str> = HashAlgorithm::ALL
            .iter()
            .map(|kind| kind.as_str())
            .collect();
        let invalid = |reason: &str| {
            Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "hash {reason}; give <algorithm>:<hex digest> with one of {}, for example sha256:<64 hex digits>",
                    names.join(", ")
                ),
            )
            .with_found(text)
            .with_expected(serde_json::json!(names))
        };
        let (name, digits) = text
            .trim()
            .split_once(':')
            .ok_or_else(|| invalid("has no algorithm prefix"))?;
        let algorithm = HashAlgorithm::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str().eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| invalid("names an unknown algorithm"))?;
        let digest = hex::decode(digits.trim().to_ascii_lowercase())
            .map_err(|_| invalid("is not hexadecimal"))?;
        if digest.len() != algorithm.digest_len() {
            return Err(invalid(&format!(
                "must have {} hexadecimal digits for {algorithm}",
                algorithm.digest_len() * 2
            )));
        }
        Ok(Self { algorithm, digest })
    }
}

fn digest_text(algorithm: HashAlgorithm, digest: &[u8]) -> String {
    format!("{algorithm}:{}", hex::encode(digest))
}

/// Digest computed while the content arrives.
enum Hasher {
    Sha256(Sha256),
    Sha512(Sha512),
    Blake3(Box<blake3::Hasher>),
}

impl Hasher {
    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha256(hasher) => hasher.update(bytes),
            Self::Sha512(hasher) => hasher.update(bytes),
            Self::Blake3(hasher) => {
                hasher.update(bytes);
            }
        }
    }

    fn finish(self) -> Vec<u8> {
        match self {
            Self::Sha256(hasher) => hasher.finalize().to_vec(),
            Self::Sha512(hasher) => hasher.finalize().to_vec(),
            Self::Blake3(hasher) => hasher.finalize().as_bytes().to_vec(),
        }
    }
}

/// Arguments of one download.
#[derive(Debug, Clone)]
struct Params {
    target: Target,
    to: AbsPath,
    resume: bool,
    hash: Option<ExpectedHash>,
    headers: Vec<(String, String)>,
    timeout: Duration,
}

impl Params {
    fn from_args(args: &BoundArgs) -> Result<Self> {
        args.require(PARAM_URL)?;
        let url = args
            .text(PARAM_URL)?
            .ok_or_else(|| type_error(PARAM_URL, "an address"))?;
        Ok(Self {
            target: Target::parse(url)?,
            to: target_path(args)?,
            resume: args.bool(PARAM_RESUME)?.unwrap_or(true),
            hash: args.text(PARAM_HASH)?.map(str::parse).transpose()?,
            headers: headers(args)?,
            timeout: timeout(args)?,
        })
    }
}

fn type_error(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{COMMAND}` expects {expected} for `{name}`"),
    )
}

fn target_path(args: &BoundArgs) -> Result<AbsPath> {
    match args.require(PARAM_TO)? {
        Value::Path(path) | Value::Text(path) => Ok(AbsPath::new(path.as_str())?),
        other => Err(type_error(
            PARAM_TO,
            &format!("a path, not {}", other.type_name()),
        )),
    }
}

fn headers(args: &BoundArgs) -> Result<Vec<(String, String)>> {
    let fields = match args.value(PARAM_HEADERS) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Map(fields)) => fields,
        Some(other) => {
            return Err(type_error(
                PARAM_HEADERS,
                &format!("a map of texts, not {}", other.type_name()),
            ));
        }
    };
    fields
        .iter()
        .map(|(name, value)| {
            if RESERVED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                return Err(Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{PARAM_HEADERS}` cannot set `{name}`; the command sets it to resume a download"),
                )
                .with_found(name.as_str()));
            }
            match value {
                Value::Text(text) => Ok((name.clone(), text.clone())),
                other => Err(type_error(
                    PARAM_HEADERS,
                    &format!("text for the header `{name}`, not {}", other.type_name()),
                )),
            }
        })
        .collect()
}

fn timeout(args: &BoundArgs) -> Result<Duration> {
    match args.value(PARAM_TIMEOUT) {
        None | Some(Value::Null) => Ok(DEFAULT_TIMEOUT),
        Some(Value::Duration(duration)) => duration
            .to_std()
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    "timeout must be longer than zero; give a value such as timeout: 300s",
                )
                .with_found(format!("{}ms", duration.as_millis()))
            }),
        Some(other) => Err(type_error(
            PARAM_TIMEOUT,
            &format!("a duration, not {}", other.type_name()),
        )),
    }
}

fn io_error(path: &Path, action: &str, error: &io::Error) -> Error {
    let code = match error.kind() {
        io::ErrorKind::PermissionDenied => ErrorCode::Permission,
        io::ErrorKind::StorageFull => ErrorCode::NoSpace,
        io::ErrorKind::NotFound => ErrorCode::NotFound,
        _ => ErrorCode::Io,
    };
    Error::new(
        code,
        format!("{} cannot be {action}: {error}", path.display()),
    )
    .with_detail("path", serde_json::Value::from(path.display().to_string()))
}

/// Partial file of a target and the file that describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PartFile {
    part: PathBuf,
    meta: PathBuf,
}

/// Point from which an interrupted download continues.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResumePoint {
    offset: u64,
    validator: Option<String>,
}

impl PartFile {
    fn for_target(to: &AbsPath) -> Self {
        let with_suffix = |suffix: &str| {
            let mut name = to.as_path().as_os_str().to_owned();
            name.push(suffix);
            PathBuf::from(name)
        };
        Self {
            part: with_suffix(PART_SUFFIX),
            meta: with_suffix(META_SUFFIX),
        }
    }

    /// The resume point of a partial file left by a download of `url`.
    fn resume_point(&self, url: &str) -> Option<ResumePoint> {
        let text = fs::read_to_string(&self.meta).ok()?;
        let mut stored_url = None;
        let mut validator = None;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix(META_URL) {
                stored_url = Some(value);
            } else if let Some(value) = line.strip_prefix(META_VALIDATOR) {
                validator = Some(value.to_owned());
            }
        }
        let part = fs::symlink_metadata(&self.part).ok()?;
        if !part.is_file() {
            return None;
        }
        let offset = part.len();
        // Without a validator the server cannot tell that the content changed,
        // so the bytes of an earlier download could be joined to another version.
        (stored_url == Some(url) && offset > 0 && validator.is_some())
            .then_some(ResumePoint { offset, validator })
    }

    fn write_meta(&self, url: &str, validator: Option<&str>) -> Result<()> {
        let mut text = format!("{META_URL}{url}\n");
        if let Some(validator) = validator {
            text.push_str(&format!("{META_VALIDATOR}{validator}\n"));
        }
        write_new(&self.meta, text.as_bytes())
            .map_err(|error| io_error(&self.meta, "written", &error))
    }

    fn create(&self) -> Result<File> {
        create_new(&self.part).map_err(|error| io_error(&self.part, "created", &error))
    }

    /// Opens the partial file at `offset`, feeding the bytes before it to
    /// `hasher`.
    fn reopen(&self, offset: u64, hasher: &mut Hasher) -> Result<File> {
        let failure = |error: io::Error| io_error(&self.part, "read", &error);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.part)
            .map_err(failure)?;
        file.set_len(offset).map_err(failure)?;
        let mut buffer = vec![0_u8; CHUNK];
        loop {
            let read = file.read(&mut buffer).map_err(failure)?;
            if read == 0 {
                break;
            }
            hasher.update(buffer.get(..read).unwrap_or_default());
        }
        file.seek(SeekFrom::Start(offset)).map_err(failure)?;
        Ok(file)
    }

    /// Removes the partial file and its description; missing files are not
    /// an error.
    fn discard(&self) -> Result<()> {
        for path in [&self.part, &self.meta] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(path, "removed", &error)),
            }
        }
        Ok(())
    }
}

/// Creates the file at `path` as a new regular file. An existing file or link
/// of that name is removed first, so that a link planted at the predictable
/// name of a partial file is never followed.
fn create_new(path: &Path) -> io::Result<File> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Writes `bytes` to a new regular file at `path` (see [`create_new`]).
fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    create_new(path)?.write_all(bytes)
}

/// Range of a `Content-Range` header: first byte and complete length, each
/// when given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentRange {
    start: Option<u64>,
    total: Option<u64>,
}

/// Parses `bytes <first>-<last>/<length>` and `bytes */<length>` (RFC 9110,
/// section 14.4); `*` as the length leaves it unknown.
fn content_range(head: &ResponseHead) -> Option<ContentRange> {
    let spec = head
        .header("content-range")?
        .trim()
        .strip_prefix("bytes")?
        .trim();
    let (range, total) = spec.split_once('/')?;
    let start = match range.trim() {
        "*" => None,
        range => Some(range.split_once('-')?.0.trim().parse().ok()?),
    };
    Some(ContentRange {
        start,
        total: total.trim().parse().ok(),
    })
}

/// Validator usable in `If-Range`: a strong entity tag, otherwise the
/// modification date (RFC 9110, section 13.1.5).
fn validator(head: &ResponseHead) -> Option<String> {
    head.header("etag")
        .map(str::trim)
        .filter(|tag| !tag.starts_with("W/") && !tag.is_empty())
        .or_else(|| head.header("last-modified").map(str::trim))
        .map(str::to_owned)
}

/// Content in the partial file after a complete transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fetched {
    status: u16,
    final_url: String,
    size: u64,
    resumed_from: u64,
    algorithm: HashAlgorithm,
    digest: Vec<u8>,
}

/// Transfer of one download into its partial file.
struct Transfer<'a> {
    params: &'a Params,
    part: PartFile,
    client: HttpClient,
    ctx: &'a ExecCtx,
}

impl<'a> Transfer<'a> {
    fn new(params: &'a Params, ctx: &'a ExecCtx) -> Self {
        let mut client = HttpClient::new(ClientSettings::from_config(ctx.config()));
        if let Some(resolver) = ctx.extensions().get::<dyn NameResolver>() {
            client = client.with_resolver(resolver);
        }
        Self {
            params,
            part: PartFile::for_target(&params.to),
            client,
            ctx,
        }
    }

    fn algorithm(&self) -> HashAlgorithm {
        self.params
            .hash
            .as_ref()
            .map_or(HashAlgorithm::Sha256, |hash| hash.algorithm)
    }

    fn request(&self, point: Option<&ResumePoint>) -> RequestSpec {
        let mut spec = RequestSpec::new(Method::Get, self.params.target.clone());
        spec.headers.clone_from(&self.params.headers);
        spec.timeout = self.params.timeout;
        spec.compression = false;
        if let Some(point) = point {
            spec.headers
                .push(("Range".to_owned(), format!("bytes={}-", point.offset)));
            if let Some(validator) = &point.validator {
                spec.headers
                    .push(("If-Range".to_owned(), validator.clone()));
            }
        }
        spec
    }

    /// Downloads into the partial file, continuing an earlier download of
    /// the same address when `resume` is set.
    fn run(&self) -> Result<Fetched> {
        let mut point = if self.params.resume {
            self.part.resume_point(&self.params.target.display_url())
        } else {
            None
        };
        loop {
            let exchange = self.client.send(&self.request(point.as_ref()), self.ctx)?;
            let Some(resume) = point.take() else {
                return self.receive_full(exchange);
            };
            let range = content_range(&exchange.head);
            match exchange.head.status {
                STATUS_PARTIAL if range.and_then(|range| range.start) == Some(resume.offset) => {
                    return self.receive(exchange, resume.offset);
                }
                STATUS_RANGE_NOT_SATISFIABLE
                    if range.and_then(|range| range.total) == Some(resume.offset) =>
                {
                    return self.complete(&exchange.head, resume.offset);
                }
                STATUS_PARTIAL | STATUS_RANGE_NOT_SATISFIABLE => {}
                _ => return self.receive_full(exchange),
            }
        }
    }

    fn receive_full(&self, exchange: Exchange) -> Result<Fetched> {
        let status = exchange.head.status;
        if !(STATUS_SUCCESS_MIN..STATUS_SUCCESS_END).contains(&status) || status == STATUS_PARTIAL {
            return Err(status_error(&exchange.head));
        }
        self.part.write_meta(
            &self.params.target.display_url(),
            validator(&exchange.head).as_deref(),
        )?;
        self.receive(exchange, 0)
    }

    /// Writes the body after the first `offset` bytes of the partial file.
    fn receive(&self, mut exchange: Exchange, offset: u64) -> Result<Fetched> {
        let mut hasher = self.algorithm().hasher();
        let mut file = if offset == 0 {
            self.part.create()?
        } else {
            self.part.reopen(offset, &mut hasher)?
        };
        let failure = |error: io::Error| io_error(&self.part.part, "written", &error);
        let mut buffer = vec![0_u8; CHUNK];
        let mut size = offset;
        loop {
            self.ctx.checkpoint()?;
            let read = exchange.body.read_chunk(&mut buffer)?;
            if read == 0 {
                break;
            }
            let chunk = buffer.get(..read).unwrap_or_default();
            file.write_all(chunk).map_err(failure)?;
            hasher.update(chunk);
            size = size.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        }
        file.sync_data().map_err(failure)?;
        if let Some(expected) = exchange.body.content_length()
            && exchange.body.received() < expected
        {
            return Err(Error::new(
                ErrorCode::Network,
                format!(
                    "the connection closed after {} of {expected} bytes; run the command again to resume",
                    exchange.body.received()
                ),
            )
            .with_problem("incomplete"));
        }
        Ok(Fetched {
            status: exchange.head.status,
            final_url: exchange.head.final_url.clone(),
            size,
            resumed_from: offset,
            algorithm: self.algorithm(),
            digest: hasher.finish(),
        })
    }

    /// A partial file that already holds the whole content.
    fn complete(&self, head: &ResponseHead, size: u64) -> Result<Fetched> {
        let mut hasher = self.algorithm().hasher();
        self.part.reopen(size, &mut hasher)?;
        Ok(Fetched {
            status: head.status,
            final_url: head.final_url.clone(),
            size,
            resumed_from: size,
            algorithm: self.algorithm(),
            digest: hasher.finish(),
        })
    }
}

fn status_error(head: &ResponseHead) -> Error {
    Error::new(
        ErrorCode::Network,
        format!(
            "`{}` answered with status {}; the target is unchanged",
            head.final_url, head.status
        ),
    )
    .with_problem("http_status")
    .with_detail("status", serde_json::Value::from(head.status))
    .with_detail("url", serde_json::Value::from(head.final_url.clone()))
}

/// State of the target observed by the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Existing {
    size: Option<u64>,
}

/// Private state passed from the plan to the execution.
#[derive(Debug, Clone)]
struct Planned {
    params: Params,
    existing: Option<Existing>,
}

fn existing_target(ctx: &ExecCtx, to: &AbsPath) -> Result<Option<Existing>> {
    let not_file = || {
        Error::new(
            ErrorCode::InvalidValue,
            format!(
                "`{}` is not a regular file; give the path of a file to write",
                to.to_language_string()
            ),
        )
        .with_found(to.to_language_string())
    };
    if let Some(fs) = ctx.fs() {
        return match fs.metadata(to, Follow::Follow) {
            Ok(meta) if meta.kind == FileKind::File => Ok(Some(Existing {
                size: Some(meta.size),
            })),
            Ok(_) => Err(not_file()),
            Err(_) => Ok(None),
        };
    }
    match fs::metadata(to.as_path()) {
        Ok(meta) if meta.is_file() => Ok(Some(Existing {
            size: Some(meta.len()),
        })),
        Ok(_) => Err(not_file()),
        Err(_) => Ok(None),
    }
}

fn require_directory(to: &AbsPath) -> Result<()> {
    let parent = to.parent().ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidValue,
            "`to` must name a file, not the root directory",
        )
    })?;
    if fs::metadata(parent.as_path()).is_ok_and(|meta| meta.is_dir()) {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::NotFound,
        format!(
            "directory `{}` does not exist; create it before the download",
            parent.to_language_string()
        ),
    )
    .with_found(parent.to_language_string()))
}

fn planned_change(to: &AbsPath, existing: Option<Existing>) -> Change {
    let action = if existing.is_some() {
        ChangeAction::Modify
    } else {
        ChangeAction::Create
    };
    let mut change = Change::new(to.to_language_string(), action);
    change.size_before = existing.and_then(|state| state.size).map(Size::from_bytes);
    change
}

/// Switches the content of the partial file into the target through `tx`.
fn switch_in(
    planned: &Planned,
    part: &PartFile,
    fetched: &Fetched,
    tx: &mut dyn MutationTx,
) -> Result<Change> {
    let to = &planned.params.to;
    let expect = if planned.existing.is_some() {
        Expect::Unchecked
    } else {
        Expect::Absent
    };
    let preserved = tx.preserve(&[TargetSpec {
        path: TargetPath::Fs(to.clone()),
        action: TargetAction::Modify,
        expect,
        size_hint: planned.existing.and_then(|state| state.size),
    }])?;
    let target = preserved
        .into_iter()
        .next()
        .ok_or_else(|| Error::new(ErrorCode::Internal, "the journal declared no target"))?;
    {
        let mut handle = tx.stage(target.index)?;
        let mut source =
            File::open(&part.part).map_err(|error| io_error(&part.part, "read", &error))?;
        io::copy(&mut source, &mut *handle)
            .map_err(|error| io_error(to.as_path(), "written", &error))?;
    }
    let applied = tx.apply(target.index, Mutation::CommitStaged)?;
    let action = if target.before.exists {
        ChangeAction::Modify
    } else {
        ChangeAction::Create
    };
    let mut change = Change::new(to.to_language_string(), action);
    change.size_before = target.before.size.map(Size::from_bytes);
    change.size_after = Some(Size::from_bytes(applied.after.size.unwrap_or(fetched.size)));
    change.hash_after = Some(digest_text(fetched.algorithm, &fetched.digest));
    change.store = Some(target.mode.as_str().to_owned());
    change.irreversible = target.irreversible;
    Ok(change)
}

fn download_object(params: &Params, fetched: &Fetched) -> Value {
    let mut fields = IndexMap::new();
    fields.insert("url".to_owned(), Value::Text(params.target.display_url()));
    fields.insert(
        "final_url".to_owned(),
        Value::Text(fetched.final_url.clone()),
    );
    fields.insert("status".to_owned(), Value::Int(i64::from(fetched.status)));
    fields.insert(
        "bytes".to_owned(),
        Value::Size(Size::from_bytes(fetched.size)),
    );
    fields.insert(
        "resumed_from".to_owned(),
        Value::Size(Size::from_bytes(fetched.resumed_from)),
    );
    fields.insert(
        "hash_algorithm".to_owned(),
        params.hash.as_ref().map_or(Value::Null, |hash| {
            Value::Text(hash.algorithm.as_str().to_owned())
        }),
    );
    fields.insert(
        "hash_verified".to_owned(),
        Value::Bool(params.hash.is_some()),
    );
    Value::Map(fields)
}

/// Implementation of the `http.download` command.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_net::http::download::Download;
///
/// let stage = Download::new();
/// assert_eq!(stage.descriptor().command, "http.download");
/// assert!(stage.as_effect().is_some());
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct Download;

impl Download {
    /// The command.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl StageImpl for Download {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let params = Params::from_args(args)?;
        let mut info = StaticInfo::default();
        info.network.push(NetworkTarget {
            scheme: params.target.scheme().to_owned(),
            host: params.target.host(),
            port: Some(params.target.port()),
            url: Some(params.target.display_url()),
        });
        info.paths.push(params.to.to_language_string());
        Ok(info)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Download {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        ctx.checkpoint()?;
        let params = Params::from_args(args)?;
        require_directory(&params.to)?;
        let existing = existing_target(ctx, &params.to)?;
        let mut plan = EffectPlan::new();
        plan.changes.push(planned_change(&params.to, existing));
        Ok(plan.with_state(Planned { params, existing }))
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let planned = match plan.take_state::<Planned>() {
            Some(planned) => planned,
            None => {
                let params = Params::from_args(args)?;
                let existing = existing_target(ctx, &params.to)?;
                Planned { params, existing }
            }
        };
        let transfer = Transfer::new(&planned.params, ctx);
        let fetched = transfer.run()?;
        if let Some(expected) = &planned.params.hash
            && expected.digest != fetched.digest
        {
            transfer.part.discard()?;
            return Err(Error::new(
                ErrorCode::HashMismatch,
                format!(
                    "the downloaded content does not match `{PARAM_HASH}`; `{}` is unchanged",
                    planned.params.to.to_language_string()
                ),
            )
            .with_expected(serde_json::Value::from(expected.to_text()))
            .with_actual(serde_json::Value::from(digest_text(
                fetched.algorithm,
                &fetched.digest,
            ))));
        }
        let change = switch_in(&planned, &transfer.part, &fetched, tx)?;
        transfer.part.discard()?;
        let effect = Effect {
            status: EffectStatus::Ok,
            kind: COMMAND.to_owned(),
            op: tx.op().map(|op| op.0),
            targets: Some(1),
            changes: Some(vec![change.clone()]),
            after: Some(download_object(&planned.params, &fetched)),
            ..Effect::default()
        };
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(
            Record::from(effect),
        )));
        outcome.changes = vec![change];
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Duration as ValueDuration;

    use super::*;

    fn head(headers: &[(&str, &str)], status: u16) -> ResponseHead {
        ResponseHead {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            url: "http://127.0.0.1/file".to_owned(),
            final_url: "http://127.0.0.1/file".to_owned(),
            redirects: Vec::new(),
            remote_address: None,
            proxy: None,
            attempts: 1,
        }
    }

    fn args(values: &[(&str, Value)]) -> BoundArgs {
        values
            .iter()
            .fold(BoundArgs::new(COMMAND), |args, (name, value)| {
                args.with(*name, ArgValue::Value(value.clone()), false)
            })
    }

    fn base_args(extra: &[(&str, Value)]) -> BoundArgs {
        let target = std::env::temp_dir().join("file.bin");
        let mut values = vec![
            (PARAM_URL, Value::Text("http://127.0.0.1:9/file".to_owned())),
            (PARAM_TO, Value::Path(target.display().to_string())),
        ];
        values.extend(extra.iter().cloned());
        args(&values)
    }

    #[test]
    fn hash_parsing_checks_algorithm_and_length() {
        let sha512 = format!("sha512:{}", "0f".repeat(64));
        assert_eq!(
            sha512.parse::<ExpectedHash>().unwrap().algorithm,
            HashAlgorithm::Sha512
        );
        for text in [
            "deadbeef",
            "md5:d41d8cd98f00b204e9800998ecf8427e",
            "sha256:abc",
            "blake3:zz",
        ] {
            let error = text.parse::<ExpectedHash>().unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "{text}");
            assert_eq!(error.found.as_deref(), Some(text));
        }
    }

    #[test]
    fn hashers_match_reference_digests() {
        let mut sha256 = HashAlgorithm::Sha256.hasher();
        sha256.update(b"abc");
        assert_eq!(
            hex::encode(sha256.finish()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut blake = HashAlgorithm::Blake3.hasher();
        blake.update(b"abc");
        assert_eq!(blake.finish(), blake3::hash(b"abc").as_bytes().to_vec());
        let mut sha512 = HashAlgorithm::Sha512.hasher();
        sha512.update(b"");
        assert_eq!(sha512.finish().len(), HashAlgorithm::Sha512.digest_len());
    }

    #[test]
    fn content_range_reads_start_and_length() {
        let partial = head(&[("content-range", "bytes 100-199/200")], 206);
        assert_eq!(
            content_range(&partial),
            Some(ContentRange {
                start: Some(100),
                total: Some(200)
            })
        );
        let unsatisfied = head(&[("content-range", "bytes */200")], 416);
        assert_eq!(
            content_range(&unsatisfied),
            Some(ContentRange {
                start: None,
                total: Some(200)
            })
        );
        assert_eq!(content_range(&head(&[], 206)), None);
        let unknown = head(&[("content-range", "bytes 0-9/*")], 206);
        assert_eq!(content_range(&unknown).and_then(|range| range.total), None);
    }

    #[test]
    fn validator_prefers_strong_entity_tag() {
        let strong = head(&[("etag", "\"v1\""), ("last-modified", "Mon")], 200);
        assert_eq!(validator(&strong).as_deref(), Some("\"v1\""));
        let weak = head(&[("etag", "W/\"v1\""), ("last-modified", "Mon")], 200);
        assert_eq!(validator(&weak).as_deref(), Some("Mon"));
        assert_eq!(validator(&head(&[("etag", "W/\"v1\"")], 200)), None);
    }

    #[test]
    fn parameters_take_catalog_defaults() {
        let params = Params::from_args(&base_args(&[])).unwrap();
        assert!(params.resume);
        assert_eq!(params.timeout, DEFAULT_TIMEOUT);
        assert!(params.hash.is_none());
        assert!(params.headers.is_empty());
    }

    #[test]
    fn reserved_headers_and_zero_timeout_are_rejected() {
        let mut fields = IndexMap::new();
        fields.insert("Range".to_owned(), Value::Text("bytes=0-".to_owned()));
        let error =
            Params::from_args(&base_args(&[(PARAM_HEADERS, Value::Map(fields))])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);

        let zero = Value::Duration(ValueDuration::from_millis(0));
        let error = Params::from_args(&base_args(&[(PARAM_TIMEOUT, zero)])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);

        let mut fields = IndexMap::new();
        fields.insert("X-Count".to_owned(), Value::Int(1));
        let error =
            Params::from_args(&base_args(&[(PARAM_HEADERS, Value::Map(fields))])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
    }

    #[cfg(unix)]
    #[test]
    fn partial_file_is_created_without_following_a_planted_link() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        fs::write(&victim, b"keep").unwrap();
        let to = AbsPath::new(dir.path().join("file.bin")).unwrap();
        let part = PartFile::for_target(&to);
        std::os::unix::fs::symlink(&victim, &part.part).unwrap();
        std::os::unix::fs::symlink(&victim, &part.meta).unwrap();
        assert_eq!(part.resume_point("http://a/"), None);
        part.write_meta("http://a/", Some("\"v1\"")).unwrap();
        let mut file = part.create().unwrap();
        file.write_all(b"new").unwrap();
        drop(file);
        assert_eq!(fs::read(&victim).unwrap(), b"keep");
        assert_eq!(fs::read(&part.part).unwrap(), b"new");
        assert!(fs::symlink_metadata(&part.meta).unwrap().is_file());
    }

    #[test]
    fn resume_needs_a_validator_of_the_earlier_response() {
        let dir = tempfile::tempdir().unwrap();
        let to = AbsPath::new(dir.path().join("file.bin")).unwrap();
        let part = PartFile::for_target(&to);
        fs::write(&part.part, b"12345").unwrap();
        part.write_meta("http://a/", None).unwrap();
        assert_eq!(part.resume_point("http://a/"), None);
        part.write_meta("http://a/", Some("Mon")).unwrap();
        assert_eq!(
            part.resume_point("http://a/").map(|point| point.offset),
            Some(5)
        );
    }

    #[test]
    fn description_file_names_the_address_without_credentials() {
        let target = Target::parse("http://user:secret@a.test/file").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let to = AbsPath::new(dir.path().join("file.bin")).unwrap();
        let part = PartFile::for_target(&to);
        part.write_meta(&target.display_url(), Some("\"v1\""))
            .unwrap();
        let text = fs::read_to_string(&part.meta).unwrap();
        assert!(!text.contains("secret") && !text.contains("user"), "{text}");
    }

    #[test]
    fn part_file_resumes_only_the_same_address() {
        let dir = tempfile::tempdir().unwrap();
        let to = AbsPath::new(dir.path().join("file.bin")).unwrap();
        let part = PartFile::for_target(&to);
        assert!(part.part.ends_with("file.bin.part"));
        assert_eq!(part.resume_point("http://a/"), None);
        fs::write(&part.part, b"12345").unwrap();
        part.write_meta("http://a/", Some("\"v1\"")).unwrap();
        assert_eq!(
            part.resume_point("http://a/"),
            Some(ResumePoint {
                offset: 5,
                validator: Some("\"v1\"".to_owned())
            })
        );
        assert_eq!(part.resume_point("http://b/"), None);
        part.discard().unwrap();
        assert!(!part.part.exists() && !part.meta.exists());
        part.discard().unwrap();
    }
}
