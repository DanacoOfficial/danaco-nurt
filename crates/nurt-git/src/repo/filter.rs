// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Content filters between the working tree and the object store.
//!
//! [`Filters`] converts file content as `git` does when a file is added
//! (towards the object store, `clean`) and when it is checked out (towards
//! the working tree, `smudge`). Line endings (`core.autocrlf`, `core.eol`
//! and the `text`, `eol`, `crlf` and `binary` attributes), `ident` and
//! `working-tree-encoding` are converted in process.
//!
//! A driver named by the `filter` attribute and configured as
//! `filter.<name>.clean`, `filter.<name>.smudge` or `filter.<name>.process`
//! is a program of the user. It runs through the program service of the
//! execution context under the rules of the `run` command, never directly.
//! Without that service, or when a driver that is not `required` fails, the
//! content passes unfiltered and the effect `filter_not_applied` is reported
//! once per driver with the number of files concerned.
//!
//! The conversions run in the order of `git`: towards the object store the
//! driver runs first and the built-in conversions follow; towards the
//! working tree the built-in conversions run first and the driver last.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};

use bstr::{BStr, ByteSlice};
use gix_object::Find as _;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::{
    ProcessHost, ProgramInput, ProgramSpec, ServiceChild, ServiceSpawnSpec,
};
use serde_json::Value;

use super::{GitRepo, error};

/// Shell that runs filter commands, as `git` runs them.
const SHELL: &str = "sh";
/// Option of [`SHELL`] that runs a command text.
const SHELL_COMMAND: &str = "-c";
/// Output kept from a single-file filter: the whole file content.
const FILTER_OUTPUT_LIMIT: u64 = u64::MAX;
/// Largest payload of one packet line of the long-running protocol.
const PKT_MAX_DATA: usize = 65_516;
/// Bytes of the hexadecimal length prefix of a packet line.
const PKT_HEADER_LEN: usize = 4;
/// Radix of the length prefix of a packet line.
const PKT_LENGTH_RADIX: u32 = 16;
/// Flush packet that ends a list or a content stream.
const FLUSH: &[u8] = b"0000";
/// Largest size of a Git LFS pointer file.
pub const LFS_POINTER_MAX: usize = 1024;
/// First line of a Git LFS pointer file.
const LFS_VERSION_LINE: &[u8] = b"version https://git-lfs.github.com/spec/v1\n";
/// Number of hexadecimal digits of an SHA-256 object name.
const SHA256_HEX_LEN: usize = 64;

/// Direction of a conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Towards the object store, as `git add` converts a file.
    ToGit,
    /// Towards the working tree, as `git checkout` converts a blob.
    ToWorktree,
}

impl Direction {
    /// Name of the driver command and of the request of the long-running
    /// protocol: `clean` or `smudge`.
    #[must_use]
    pub const fn command(self) -> &'static str {
        match self {
            Self::ToGit => "clean",
            Self::ToWorktree => "smudge",
        }
    }
}

/// Filter driver configured as `filter.<name>.*`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterDriver {
    /// Name used by the `filter` attribute.
    pub name: String,
    /// Command run towards the object store.
    pub clean: Option<String>,
    /// Command run towards the working tree.
    pub smudge: Option<String>,
    /// Long-running process that serves both directions.
    pub process: Option<String>,
    /// A failure of the driver fails the operation instead of passing the
    /// content unfiltered.
    pub required: bool,
}

impl FilterDriver {
    fn from_gix(driver: &gix_filter::Driver) -> Self {
        let text =
            |value: &Option<bstr::BString>| value.as_ref().map(|v| v.to_str_lossy().into_owned());
        Self {
            name: driver.name.to_str_lossy().into_owned(),
            clean: text(&driver.clean),
            smudge: text(&driver.smudge),
            process: text(&driver.process),
            required: driver.required,
        }
    }

    /// Single-file command of the driver for `direction`.
    #[must_use]
    pub fn command(&self, direction: Direction) -> Option<&str> {
        match direction {
            Direction::ToGit => self.clean.as_deref(),
            Direction::ToWorktree => self.smudge.as_deref(),
        }
    }
}

/// Content of a Git LFS pointer file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsPointer {
    /// SHA-256 object name of the content, in hexadecimal.
    pub oid: String,
    /// Size of the content in bytes.
    pub size: u64,
}

/// Reads `bytes` as a Git LFS pointer file; `None` when it is not one.
///
/// # Examples
///
/// ```
/// use nurt_git::repo::filter::lfs_pointer;
///
/// let oid = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";
/// let text = format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize 12345\n");
/// let pointer = lfs_pointer(text.as_bytes()).expect("a pointer");
/// assert_eq!(pointer.size, 12345);
/// assert!(lfs_pointer(b"plain text\n").is_none());
/// ```
#[must_use]
pub fn lfs_pointer(bytes: &[u8]) -> Option<LfsPointer> {
    if bytes.len() > LFS_POINTER_MAX || !bytes.starts_with(LFS_VERSION_LINE) {
        return None;
    }
    let text = bytes.to_str().ok()?;
    let mut oid = None;
    let mut size = None;
    for line in text.lines().skip(1) {
        if let Some(hex) = line.strip_prefix("oid sha256:") {
            let valid = hex.len() == SHA256_HEX_LEN
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
            if valid {
                oid = Some(hex.to_owned());
            }
        } else if let Some(value) = line.strip_prefix("size ") {
            size = value.parse().ok();
        }
    }
    Some(LfsPointer {
        oid: oid?,
        size: size?,
    })
}

/// Quotes `text` for a POSIX shell: single quotes, with each `'` written as
/// `'\''`.
///
/// # Examples
///
/// ```
/// use nurt_git::repo::filter::shell_quote;
///
/// assert_eq!(shell_quote("it's"), r"'it'\''s'");
/// ```
#[must_use]
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Expands a driver command as `git` does: `%f` becomes the quoted path of
/// the file and `%%` a single `%`; other sequences stay as written.
#[must_use]
pub fn expand_command(command: &str, path: &BStr) -> String {
    let mut out = String::with_capacity(command.len());
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            Some('f') => {
                chars.next();
                out.push_str(&shell_quote(&path.to_str_lossy()));
            }
            Some('%') => {
                chars.next();
                out.push('%');
            }
            _ => out.push('%'),
        }
    }
    out
}

/// Why a driver did not produce content.
enum Failure {
    /// The program could not be started.
    Start(Error),
    /// The program ran and failed.
    Failed(String),
}

impl Failure {
    fn reason(&self) -> String {
        match self {
            Self::Start(_) => "the filter program could not be started".to_owned(),
            Self::Failed(reason) => reason.clone(),
        }
    }
}

#[derive(Debug)]
struct Skipped {
    count: u64,
    first: String,
    reason: String,
}

/// Content filters of one repository, for any number of files.
///
/// Dropping the value reports the drivers that were not applied as
/// `filter_not_applied` and stops the long-running filter processes.
pub struct Filters<'repo> {
    repo: &'repo GitRepo,
    pipeline: gix_filter::Pipeline,
    stack: gix_worktree::Stack,
    index: gix::index::File,
    drivers: Vec<FilterDriver>,
    processes: BTreeMap<String, LongRunning>,
    skipped: BTreeMap<String, Skipped>,
}

impl std::fmt::Debug for Filters<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Filters")
            .field("drivers", &self.drivers)
            .field("processes", &self.processes.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl GitRepo {
    /// Content filters of this repository, with the attributes of the
    /// working tree (or of the `HEAD` tree in a bare repository).
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the attributes, the index or the filter
    /// configuration cannot be read.
    pub fn filters(&self) -> Result<Filters<'_>, Error> {
        let failed = |source: gix::Error| error::from_gix("prepare the content filters", &source);
        let tree = if self.is_bare() {
            Some(self.gix().head_tree_id_or_empty().map_err(failed)?.detach())
        } else {
            None
        };
        let (pipeline, _) = self.gix().filter_pipeline(tree).map_err(failed)?;
        let (mut pipeline, stack) = pipeline.into_parts();
        let drivers = std::mem::take(&mut pipeline.options_mut().drivers)
            .iter()
            .map(FilterDriver::from_gix)
            .collect();
        Ok(Filters {
            repo: self,
            pipeline,
            stack,
            index: self.index_file()?,
            drivers,
            processes: BTreeMap::new(),
            skipped: BTreeMap::new(),
        })
    }
}

impl Filters<'_> {
    /// Drivers configured in the repository, whether or not an attribute
    /// names them.
    #[must_use]
    pub fn drivers(&self) -> &[FilterDriver] {
        &self.drivers
    }

    /// Converts the working tree content of `path` (relative to the root,
    /// with `/` separators) into the content stored in the repository.
    ///
    /// # Errors
    ///
    /// Fails with `E_EXTERNAL_FAILED` when a `required` driver fails, and
    /// with `E_IO` when a built-in conversion rejects the content.
    pub fn to_git(&mut self, path: &BStr, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let driven = self.run_driver(path, bytes, Direction::ToGit)?;
        self.builtin_to_git(path, driven.as_deref().unwrap_or(bytes))
    }

    /// Converts the stored content of `path` into its working tree content.
    ///
    /// # Errors
    ///
    /// As [`Filters::to_git`].
    pub fn to_worktree(&mut self, path: &BStr, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let converted = self.builtin_to_worktree(path, bytes)?;
        Ok(self
            .run_driver(path, &converted, Direction::ToWorktree)?
            .unwrap_or(converted))
    }

    /// Driver named by the `filter` attribute of `path`, when the driver is
    /// configured.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the attribute files cannot be read.
    pub fn driver_of(&mut self, path: &BStr) -> Result<Option<&FilterDriver>, Error> {
        let name = self.attributes_of(path)?.filter;
        Ok(name.and_then(|name| self.drivers.iter().find(|driver| driver.name == name)))
    }

    fn attributes_of(&mut self, path: &BStr) -> Result<PathAttributes, Error> {
        let repo = self.repo;
        let platform = self
            .stack
            .at_entry(path, None, &repo.gix().objects)
            .map_err(|failure| {
                error::io(&repo.root().join(path.to_str_lossy().as_ref()), &failure)
            })?;
        Ok(PathAttributes::read(&platform))
    }

    fn builtin_to_git(&mut self, path: &BStr, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let repo = self.repo;
        let objects = &repo.gix().objects;
        let relative =
            gix::path::from_bstr(path).map_err(|failure| conversion_failed(path, &failure))?;
        let platform = self
            .stack
            .at_path(relative.as_ref(), None, objects)
            .map_err(|failure| error::io(&repo.root().join(relative.as_ref()), &failure))?;
        let index = &self.index;
        let outcome = self
            .pipeline
            .convert_to_git(
                bytes,
                relative.as_ref(),
                &mut |_, attributes| {
                    platform.matching_attributes(attributes);
                },
                &mut |buf| {
                    let Some(entry) = index.entry_by_path(path) else {
                        return Ok(None);
                    };
                    match objects.try_find(&entry.id, buf) {
                        Ok(Some(data)) if data.kind == gix::object::Kind::Blob => Ok(Some(())),
                        _ => Ok(None),
                    }
                },
            )
            .map_err(|failure| conversion_failed(path, &failure))?;
        match outcome {
            gix_filter::pipeline::convert::ToGitOutcome::Unchanged(_) => Ok(bytes.to_vec()),
            gix_filter::pipeline::convert::ToGitOutcome::Buffer(converted) => {
                Ok(converted.to_vec())
            }
            gix_filter::pipeline::convert::ToGitOutcome::Process(mut read) => {
                let mut out = Vec::new();
                read.read_to_end(&mut out)
                    .map_err(|failure| conversion_failed(path, &failure))?;
                Ok(out)
            }
        }
    }

    fn builtin_to_worktree(&mut self, path: &BStr, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let converted = self.library_to_worktree(path, bytes)?;
        let attributes = self.attributes_of(path)?;
        let hex_len = self.repo.gix().object_hash().len_in_hex();
        Ok(attributes.finish_worktree_content(converted, hex_len))
    }

    fn library_to_worktree(&mut self, path: &BStr, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let repo = self.repo;
        let platform = self
            .stack
            .at_entry(path, None, &repo.gix().objects)
            .map_err(|failure| {
                error::io(&repo.root().join(path.to_str_lossy().as_ref()), &failure)
            })?;
        let outcome = self
            .pipeline
            .convert_to_worktree(
                bytes,
                path,
                &mut |_, attributes| {
                    platform.matching_attributes(attributes);
                },
                gix_filter::pipeline::convert::to_worktree::Options::default(),
            )
            .map_err(|failure| conversion_failed(path, &failure))?;
        match outcome {
            gix_filter::pipeline::convert::ToWorktreeOutcome::Unchanged(same) => Ok(same.to_vec()),
            gix_filter::pipeline::convert::ToWorktreeOutcome::Buffer(converted) => {
                Ok(converted.to_vec())
            }
            gix_filter::pipeline::convert::ToWorktreeOutcome::Process(_) => Err(Error::new(
                ErrorCode::Internal,
                "the built-in conversion produced the output of a driver",
            )),
        }
    }

    fn run_driver(
        &mut self,
        path: &BStr,
        bytes: &[u8],
        direction: Direction,
    ) -> Result<Option<Vec<u8>>, Error> {
        let Some(driver) = self.driver_of(path)?.cloned() else {
            return Ok(None);
        };
        if driver.process.is_none() && driver.command(direction).is_none() {
            return Ok(None);
        }
        let repo = self.repo;
        let Some(host) = repo.ctx().services().process_host() else {
            self.skip(
                &driver.name,
                path,
                "no program service is available to run it".to_owned(),
            );
            return Ok(None);
        };
        let result = match driver.process.as_deref() {
            Some(command) => self.run_long_running(host, &driver, command, path, bytes, direction),
            None => run_single(host, repo, &driver, path, bytes, direction),
        };
        match result {
            Ok(output) => Ok(output),
            Err(Failure::Start(failure)) if driver.required => Err(failure),
            Err(Failure::Failed(reason)) if driver.required => {
                Err(driver_failed(&driver.name, direction, path, &reason))
            }
            Err(failure) => {
                self.skip(&driver.name, path, failure.reason());
                Ok(None)
            }
        }
    }

    fn run_long_running(
        &mut self,
        host: &dyn ProcessHost,
        driver: &FilterDriver,
        command: &str,
        path: &BStr,
        bytes: &[u8],
        direction: Direction,
    ) -> Result<Option<Vec<u8>>, Failure> {
        if !self.processes.contains_key(&driver.name) {
            let started = LongRunning::start(host, self.repo, command)?;
            self.processes.insert(driver.name.clone(), started);
        }
        let Some(process) = self.processes.get_mut(&driver.name) else {
            return Ok(None);
        };
        if !process.supports(direction) {
            return Ok(None);
        }
        match process.request(direction, path, bytes) {
            Ok(output) => Ok(Some(output)),
            Err(Answer::Status(reason)) => Err(Failure::Failed(reason)),
            Err(Answer::Broken(reason)) => {
                self.processes.remove(&driver.name);
                Err(Failure::Failed(reason))
            }
        }
    }

    fn skip(&mut self, driver: &str, path: &BStr, reason: String) {
        self.skipped
            .entry(driver.to_owned())
            .or_insert_with(|| Skipped {
                count: 0,
                first: path.to_str_lossy().into_owned(),
                reason,
            })
            .count += 1;
    }
}

impl Drop for Filters<'_> {
    fn drop(&mut self) {
        for (name, skipped) in std::mem::take(&mut self.skipped) {
            let mut note = NotableEntry::new(Notable::FilterNotApplied);
            note.path = Some(skipped.first);
            note.count = Some(skipped.count);
            note.detail = Some(format!(
                "filter driver `{name}` was not applied: {}; the content is stored unfiltered",
                skipped.reason
            ));
            self.repo.ctx().note(note);
        }
    }
}

/// Attributes of one path consulted besides those of the library pipeline.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PathAttributes {
    filter: Option<String>,
    ident: bool,
}

impl PathAttributes {
    /// Reads `filter` and `ident` for the path of `platform`.
    pub(crate) fn read(platform: &gix_worktree::stack::Platform<'_>) -> Self {
        let mut outcome = gix_attributes::search::Outcome::default();
        outcome.initialize_with_selection(&Default::default(), ["filter", "ident"]);
        platform.matching_attributes(&mut outcome);
        let mut found = Self::default();
        for matched in outcome.iter_selected() {
            let value = match matched.assignment.state {
                gix_attributes::StateRef::Value(value) => {
                    Some(value.as_bstr().to_str_lossy().into_owned())
                }
                _ => None,
            };
            match matched.assignment.name.as_str() {
                "filter" => found.filter = value,
                _ => {
                    found.ident = matches!(matched.assignment.state, gix_attributes::StateRef::Set)
                }
            }
        }
        found
    }

    /// Completes the conversion of the library pipeline that it leaves out:
    /// the space before the closing `$` of an expanded `$Id: <hex> $`.
    ///
    /// `converted` is the output of the pipeline for the path; `hex_len` is
    /// the length of an object identifier in hexadecimal digits.
    pub(crate) fn finish_worktree_content(&self, converted: Vec<u8>, hex_len: usize) -> Vec<u8> {
        if self.ident {
            complete_ident(&converted, hex_len)
        } else {
            converted
        }
    }
}

/// Writes every expanded identifier `$Id: <hex>$` as `git` writes it,
/// `$Id: <hex> $`.
fn complete_ident(bytes: &[u8], hex_len: usize) -> Vec<u8> {
    const OPEN: &[u8] = b"$Id: ";
    let mut out = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some(at) = rest.find(OPEN) {
        let after = at + OPEN.len();
        out.extend_from_slice(&rest[..after]);
        rest = &rest[after..];
        let expanded = rest.len() > hex_len
            && rest[..hex_len].iter().all(u8::is_ascii_hexdigit)
            && rest[hex_len] == b'$';
        if expanded {
            out.extend_from_slice(&rest[..hex_len]);
            out.push(b' ');
            rest = &rest[hex_len..];
        }
    }
    out.extend_from_slice(rest);
    out
}

fn conversion_failed(path: &BStr, failure: &dyn std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::Io,
        format!(
            "content conversion of {path} failed: {failure}; check the eol and working-tree-encoding attributes of the file"
        ),
    )
    .with_problem("filter_failed")
    .with_detail("path", Value::from(path.to_str_lossy().into_owned()))
}

fn driver_failed(driver: &str, direction: Direction, path: &BStr, reason: &str) -> Error {
    Error::new(
        ErrorCode::ExternalFailed,
        format!(
            "{} filter `{driver}` failed for {path}: {reason}; fix the filter program or unset filter.{driver}.required",
            direction.command()
        ),
    )
    .with_problem("filter_failed")
    .with_detail("path", Value::from(path.to_str_lossy().into_owned()))
    .with_detail("driver", Value::from(driver))
}

fn run_single(
    host: &dyn ProcessHost,
    repo: &GitRepo,
    driver: &FilterDriver,
    path: &BStr,
    bytes: &[u8],
    direction: Direction,
) -> Result<Option<Vec<u8>>, Failure> {
    let Some(command) = driver.command(direction) else {
        return Ok(None);
    };
    let mut spec = ProgramSpec::new(
        SHELL,
        vec![SHELL_COMMAND.to_owned(), expand_command(command, path)],
        repo.root(),
    );
    spec.stdin = ProgramInput::Bytes(bytes.to_vec());
    spec.output_limit = Some(FILTER_OUTPUT_LIMIT);
    let outcome = host
        .run_program(&spec, repo.ctx().cancel())
        .map_err(Failure::Start)?;
    if outcome.cancelled {
        return Err(Failure::Failed("the operation was cancelled".to_owned()));
    }
    if outcome.timed_out {
        return Err(Failure::Failed(
            "the filter program reached the time limit".to_owned(),
        ));
    }
    match outcome.exit_code {
        Some(0) => Ok(Some(outcome.stdout)),
        Some(code) => Err(Failure::Failed(format!(
            "the filter program exited with code {code}"
        ))),
        None => Err(Failure::Failed(
            "the filter program ended by a signal".to_owned(),
        )),
    }
}

/// Failed request of the long-running protocol.
enum Answer {
    /// The process answered with a status other than `success`.
    Status(String),
    /// The exchange failed; the process is not used again.
    Broken(String),
}

/// Filter process speaking the long-running protocol, version 2.
struct LongRunning {
    /// Process handle; dropping it stops the process tree.
    _child: Box<dyn ServiceChild>,
    input: Box<dyn Write + Send>,
    output: Box<dyn Read + Send>,
    capabilities: Vec<String>,
}

impl LongRunning {
    fn start(host: &dyn ProcessHost, repo: &GitRepo, command: &str) -> Result<Self, Failure> {
        let spec = ServiceSpawnSpec {
            program: SHELL.to_owned(),
            args: vec![SHELL_COMMAND.to_owned(), command.to_owned()],
            cwd: repo.root().to_path_buf(),
            env: Vec::new(),
            inherit_env: true,
            temp_dir: None,
        };
        let mut child = host.spawn_service(&spec).map_err(Failure::Start)?;
        let (Some(input), Some(output)) = (child.take_stdin(), child.take_stdout()) else {
            return Err(Failure::Failed(
                "the filter process has no standard streams".to_owned(),
            ));
        };
        let mut process = Self {
            _child: child,
            input,
            output,
            capabilities: Vec::new(),
        };
        process.handshake().map_err(|failure| {
            Failure::Failed(format!(
                "the filter process failed the handshake: {failure}"
            ))
        })?;
        Ok(process)
    }

    fn handshake(&mut self) -> io::Result<()> {
        write_text(self.input.as_mut(), "git-filter-client")?;
        write_text(self.input.as_mut(), "version=2")?;
        self.input.write_all(FLUSH)?;
        self.input.flush()?;
        let welcome = read_list(self.output.as_mut())?;
        if !welcome.iter().any(|line| line == "git-filter-server")
            || !welcome.iter().any(|line| line == "version=2")
        {
            return Err(invalid(
                "the process does not speak version 2 of the protocol",
            ));
        }
        for direction in [Direction::ToGit, Direction::ToWorktree] {
            write_text(
                self.input.as_mut(),
                &format!("capability={}", direction.command()),
            )?;
        }
        self.input.write_all(FLUSH)?;
        self.input.flush()?;
        self.capabilities = read_list(self.output.as_mut())?
            .into_iter()
            .filter_map(|line| line.strip_prefix("capability=").map(str::to_owned))
            .collect();
        Ok(())
    }

    fn supports(&self, direction: Direction) -> bool {
        self.capabilities
            .iter()
            .any(|capability| capability == direction.command())
    }

    fn request(
        &mut self,
        direction: Direction,
        path: &BStr,
        bytes: &[u8],
    ) -> Result<Vec<u8>, Answer> {
        let broken =
            |failure: io::Error| Answer::Broken(format!("the filter process failed: {failure}"));
        self.send(direction, path, bytes).map_err(broken)?;
        let header = read_list(self.output.as_mut()).map_err(broken)?;
        match status_of(&header) {
            Some("success") => {}
            Some("abort") => {
                self.capabilities
                    .retain(|capability| capability != direction.command());
                return Err(Answer::Status(
                    "the filter process aborted the command".to_owned(),
                ));
            }
            status => {
                return Err(Answer::Status(format!(
                    "the filter process answered with status {}",
                    status.unwrap_or("none")
                )));
            }
        }
        let content = read_content(self.output.as_mut()).map_err(broken)?;
        let trailer = read_list(self.output.as_mut()).map_err(broken)?;
        match status_of(&trailer) {
            None | Some("success") => Ok(content),
            Some(status) => Err(Answer::Status(format!(
                "the filter process answered with status {status}"
            ))),
        }
    }

    fn send(&mut self, direction: Direction, path: &BStr, bytes: &[u8]) -> io::Result<()> {
        let input = self.input.as_mut();
        write_text(input, &format!("command={}", direction.command()))?;
        write_text(input, &format!("pathname={}", path.to_str_lossy()))?;
        input.write_all(FLUSH)?;
        for chunk in bytes.chunks(PKT_MAX_DATA) {
            write_packet(input, chunk)?;
        }
        input.write_all(FLUSH)?;
        input.flush()
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

fn status_of(list: &[String]) -> Option<&str> {
    list.iter()
        .rev()
        .find_map(|line| line.strip_prefix("status="))
}

fn write_packet(out: &mut dyn Write, data: &[u8]) -> io::Result<()> {
    out.write_all(format!("{:04x}", data.len() + PKT_HEADER_LEN).as_bytes())?;
    out.write_all(data)
}

fn write_text(out: &mut dyn Write, line: &str) -> io::Result<()> {
    write_packet(out, format!("{line}\n").as_bytes())
}

/// Reads one packet line; `None` for a flush packet.
fn read_packet(input: &mut dyn Read) -> io::Result<Option<Vec<u8>>> {
    let mut header = [0_u8; PKT_HEADER_LEN];
    input.read_exact(&mut header)?;
    let length = std::str::from_utf8(&header)
        .ok()
        .and_then(|text| usize::from_str_radix(text, PKT_LENGTH_RADIX).ok())
        .ok_or_else(|| invalid("invalid packet length"))?;
    if length == 0 {
        return Ok(None);
    }
    if length < PKT_HEADER_LEN {
        return Err(invalid("unexpected special packet"));
    }
    let mut data = vec![0_u8; length - PKT_HEADER_LEN];
    input.read_exact(&mut data)?;
    Ok(Some(data))
}

fn read_list(input: &mut dyn Read) -> io::Result<Vec<String>> {
    let mut lines = Vec::new();
    while let Some(data) = read_packet(input)? {
        lines.push(
            String::from_utf8_lossy(&data)
                .trim_end_matches('\n')
                .to_owned(),
        );
    }
    Ok(lines)
}

fn read_content(input: &mut dyn Read) -> io::Result<Vec<u8>> {
    let mut content = Vec::new();
    while let Some(data) = read_packet(input)? {
        content.extend_from_slice(&data);
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use nurt_contracts::env::WallTime;
    use nurt_contracts::services::{ChildExit, ProgramOutcome, Services};
    use nurt_contracts::stage::ExecCtx;

    use super::*;
    use crate::testutil::{FixedClock, RepoBuilder, TestHost};

    /// Program service that upper-cases the input of single-file filters
    /// and serves long-running filters from a prepared answer.
    #[derive(Default)]
    struct FakeHost {
        exit_code: i32,
        runs: Mutex<Vec<ProgramSpec>>,
        answer: Vec<u8>,
        sent: Arc<Mutex<Vec<u8>>>,
    }

    impl ProcessHost for FakeHost {
        fn resolve_program(
            &self,
            program: &str,
            _cwd: &Path,
        ) -> nurt_contracts::error::Result<PathBuf> {
            Ok(PathBuf::from(program))
        }

        fn run_program(
            &self,
            spec: &ProgramSpec,
            _cancel: &nurt_contracts::stage::CancelToken,
        ) -> nurt_contracts::error::Result<ProgramOutcome> {
            self.runs.lock().unwrap().push(spec.clone());
            let ProgramInput::Bytes(input) = &spec.stdin else {
                panic!("filters pass the content on standard input");
            };
            Ok(ProgramOutcome {
                exit_code: Some(self.exit_code),
                stdout: input.to_ascii_uppercase(),
                ..ProgramOutcome::default()
            })
        }

        fn spawn_service(
            &self,
            _spec: &ServiceSpawnSpec,
        ) -> nurt_contracts::error::Result<Box<dyn ServiceChild>> {
            Ok(Box::new(FakeChild {
                answer: Some(self.answer.clone()),
                sent: Arc::clone(&self.sent),
            }))
        }
    }

    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FakeChild {
        answer: Option<Vec<u8>>,
        sent: Arc<Mutex<Vec<u8>>>,
    }

    impl ServiceChild for FakeChild {
        fn pid(&self) -> u32 {
            1
        }

        fn started(&self) -> WallTime {
            WallTime::from_unix_nanos(0)
        }

        fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
            Some(Box::new(SharedWriter(Arc::clone(&self.sent))))
        }

        fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
            self.answer
                .take()
                .map(|bytes| Box::new(io::Cursor::new(bytes)) as Box<dyn Read + Send>)
        }

        fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>> {
            None
        }

        fn try_wait(&mut self) -> nurt_contracts::error::Result<Option<ChildExit>> {
            Ok(None)
        }

        fn wait_timeout(
            &mut self,
            _timeout: Duration,
        ) -> nurt_contracts::error::Result<Option<ChildExit>> {
            Ok(None)
        }

        fn kill_tree(&mut self) -> nurt_contracts::error::Result<()> {
            Ok(())
        }
    }

    fn ctx(host: Option<Arc<FakeHost>>) -> ExecCtx {
        let mut services = Services::new();
        if let Some(host) = host {
            services = services.with_process_host(host);
        }
        ExecCtx::builder(
            Arc::new(FixedClock::at_unix_seconds(RepoBuilder::EPOCH)),
            Arc::new(TestHost::new("/nonexistent-home")),
        )
        .services(Arc::new(services))
        .build()
    }

    fn packets(lines: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for line in lines {
            if line.is_empty() {
                out.extend_from_slice(FLUSH);
            } else {
                write_packet(&mut out, line.as_bytes()).unwrap();
            }
        }
        out
    }

    fn upper_driver() -> RepoBuilder {
        RepoBuilder::new()
            .file(".gitattributes", "*.txt filter=upper\n")
            .config("filter.upper.clean", "upper %f")
            .config("filter.upper.smudge", "upper")
    }

    #[test]
    fn autocrlf_converts_line_endings_in_both_directions() {
        let fixture = RepoBuilder::new().config("core.autocrlf", "true").build();
        let repo = GitRepo::open(fixture.path(), &ctx(None)).unwrap();
        let mut filters = repo.filters().unwrap();
        assert_eq!(
            filters.to_worktree("a.txt".into(), b"a\nb\n").unwrap(),
            b"a\r\nb\r\n"
        );
        assert_eq!(
            filters.to_git("a.txt".into(), b"a\r\nb\r\n").unwrap(),
            b"a\nb\n"
        );
    }

    #[test]
    fn eol_attribute_applies_without_configuration() {
        let fixture = RepoBuilder::new()
            .file(".gitattributes", "*.bat text eol=crlf\n*.bin binary\n")
            .build();
        let repo = GitRepo::open(fixture.path(), &ctx(None)).unwrap();
        let mut filters = repo.filters().unwrap();
        assert_eq!(
            filters.to_worktree("run.bat".into(), b"x\n").unwrap(),
            b"x\r\n"
        );
        assert_eq!(
            filters.to_worktree("data.bin".into(), b"x\n").unwrap(),
            b"x\n"
        );
    }

    #[test]
    fn ident_expands_and_collapses_the_object_name() {
        let fixture = RepoBuilder::new()
            .file(".gitattributes", "*.c ident\n")
            .build();
        let repo = GitRepo::open(fixture.path(), &ctx(None)).unwrap();
        let mut filters = repo.filters().unwrap();
        let expanded = filters.to_worktree("x.c".into(), b"$Id$\n").unwrap();
        assert!(expanded.starts_with(b"$Id: "));
        assert!(expanded.ends_with(b" $\n"));
        assert_eq!(filters.to_git("x.c".into(), &expanded).unwrap(), b"$Id$\n");
    }

    #[test]
    fn working_tree_encoding_converts_to_utf16() {
        let fixture = RepoBuilder::new()
            .file(".gitattributes", "*.u16 working-tree-encoding=UTF-16LE\n")
            .build();
        let repo = GitRepo::open(fixture.path(), &ctx(None)).unwrap();
        let mut filters = repo.filters().unwrap();
        let encoded = filters.to_worktree("a.u16".into(), b"hi\n").unwrap();
        assert_eq!(encoded, [b'h', 0, b'i', 0, b'\n', 0]);
        assert_eq!(filters.to_git("a.u16".into(), &encoded).unwrap(), b"hi\n");
    }

    #[test]
    fn working_tree_encoding_converts_to_utf16_big_endian() {
        let fixture = RepoBuilder::new()
            .file(".gitattributes", "*.u16 working-tree-encoding=UTF-16BE\n")
            .build();
        let repo = GitRepo::open(fixture.path(), &ctx(None)).unwrap();
        let mut filters = repo.filters().unwrap();
        let encoded = filters.to_worktree("a.u16".into(), b"hi\n").unwrap();
        assert_eq!(encoded, [0, b'h', 0, b'i', 0, b'\n']);
        assert_eq!(filters.to_git("a.u16".into(), &encoded).unwrap(), b"hi\n");
    }

    #[test]
    fn driver_without_program_service_is_reported_once_with_a_count() {
        let fixture = upper_driver().build();
        let context = ctx(None);
        let repo = GitRepo::open(fixture.path(), &context).unwrap();
        {
            let mut filters = repo.filters().unwrap();
            assert_eq!(filters.to_git("a.txt".into(), b"abc\n").unwrap(), b"abc\n");
            assert_eq!(
                filters.to_worktree("b.txt".into(), b"abc\n").unwrap(),
                b"abc\n"
            );
            assert_eq!(filters.to_git("c.md".into(), b"abc\n").unwrap(), b"abc\n");
        }
        let notes = context.take_notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, Notable::FilterNotApplied);
        assert_eq!(notes[0].count, Some(2));
        assert_eq!(notes[0].path.as_deref(), Some("a.txt"));
    }

    #[test]
    fn single_file_driver_runs_through_the_program_service() {
        let fixture = upper_driver().build();
        let host = Arc::new(FakeHost::default());
        let repo = GitRepo::open(fixture.path(), &ctx(Some(Arc::clone(&host)))).unwrap();
        let mut filters = repo.filters().unwrap();
        assert_eq!(filters.to_git("a.txt".into(), b"abc\n").unwrap(), b"ABC\n");
        let runs = host.runs.lock().unwrap();
        assert_eq!(runs[0].program, "sh");
        assert_eq!(
            runs[0].args,
            vec!["-c".to_owned(), "upper 'a.txt'".to_owned()]
        );
        assert_eq!(runs[0].cwd, fixture.path());
    }

    #[test]
    fn failing_optional_driver_passes_content_unfiltered() {
        let fixture = upper_driver().build();
        let host = Arc::new(FakeHost {
            exit_code: 1,
            ..FakeHost::default()
        });
        let context = ctx(Some(host));
        let repo = GitRepo::open(fixture.path(), &context).unwrap();
        {
            let mut filters = repo.filters().unwrap();
            assert_eq!(filters.to_git("a.txt".into(), b"abc\n").unwrap(), b"abc\n");
        }
        let notes = context.take_notes();
        assert!(
            notes[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("exited with code 1")
        );
    }

    #[test]
    fn failing_required_driver_fails_the_conversion() {
        let fixture = upper_driver()
            .config("filter.upper.required", "true")
            .build();
        let host = Arc::new(FakeHost {
            exit_code: 2,
            ..FakeHost::default()
        });
        let repo = GitRepo::open(fixture.path(), &ctx(Some(host))).unwrap();
        let mut filters = repo.filters().unwrap();
        let failure = filters.to_git("a.txt".into(), b"abc\n").unwrap_err();
        assert_eq!(failure.code, ErrorCode::ExternalFailed);
        assert_eq!(failure.problem.as_deref(), Some("filter_failed"));
    }

    #[test]
    fn long_running_driver_speaks_protocol_version_two() {
        let fixture = RepoBuilder::new()
            .file(".gitattributes", "*.bin filter=lfs\n")
            .config("filter.lfs.process", "git-lfs filter-process")
            .build();
        let answer = packets(&[
            "git-filter-server\n",
            "version=2\n",
            "",
            "capability=clean\n",
            "capability=smudge\n",
            "",
            "status=success\n",
            "",
            "CONTENT",
            "",
            "",
        ]);
        let host = Arc::new(FakeHost {
            answer,
            ..FakeHost::default()
        });
        let repo = GitRepo::open(fixture.path(), &ctx(Some(Arc::clone(&host)))).unwrap();
        let mut filters = repo.filters().unwrap();
        assert_eq!(
            filters.to_worktree("a.bin".into(), b"pointer").unwrap(),
            b"CONTENT"
        );
        let sent = host.sent.lock().unwrap().clone();
        let expected_request =
            packets(&["command=smudge\n", "pathname=a.bin\n", "", "pointer", ""]);
        assert!(sent.starts_with(&packets(&["git-filter-client\n", "version=2\n", ""])));
        assert!(sent.ends_with(&expected_request));
    }

    #[test]
    fn packet_lines_round_trip() {
        let bytes = packets(&["a\n", "", "bc"]);
        let mut input = bytes.as_slice();
        assert_eq!(read_list(&mut input).unwrap(), vec!["a".to_owned()]);
        assert_eq!(
            read_content(&mut io::Cursor::new(packets(&["bc", ""]))).unwrap(),
            b"bc"
        );
        assert!(read_packet(&mut io::Cursor::new(b"zzzz".to_vec())).is_err());
    }

    #[test]
    fn command_expansion_quotes_the_path() {
        assert_eq!(
            expand_command("f %f 100%% %x", "a b.txt".into()),
            "f 'a b.txt' 100% %x"
        );
    }

    #[test]
    fn lfs_pointers_require_oid_and_size() {
        let oid = "a".repeat(SHA256_HEX_LEN);
        let valid =
            format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize 7\n");
        assert_eq!(
            lfs_pointer(valid.as_bytes()),
            Some(LfsPointer {
                oid: oid.clone(),
                size: 7
            })
        );
        let no_size = format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\n");
        assert_eq!(lfs_pointer(no_size.as_bytes()), None);
        let mut large = valid.into_bytes();
        large.resize(LFS_POINTER_MAX + 1, b'x');
        assert_eq!(lfs_pointer(&large), None);
    }
}
