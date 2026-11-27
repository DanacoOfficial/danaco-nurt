// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::info
// Responsibility:   The `info` command: `entry` records of paths, including paths that do not.
// Dependencies:     nurt_contracts, nurt_testkit, nurt_text, pdf_extract, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `info` command: `entry` records of paths, including paths that do not
//! exist.
//!
//! A path that does not exist, or whose parent is not a directory, gives an
//! entry with `exists: false` and no error. For an existing entry the record
//! carries the fields every listing shares and, as far as the rest of the
//! pipeline uses them, the account and group of the file, the access control
//! list, extended attributes, alternate streams, the link target, the volume
//! and the file identifier. Regular files, and links that resolve to one,
//! also get the media type, the binary flag, the encoding, line endings and
//! byte order mark, recognised from the leading bytes of the content. The
//! number of lines, the content hash and the `details` groups read the
//! content only when they are requested.
//!
//! An entry whose entity tag equals the one given for its path in `known` is
//! reported with the shared fields only, and an `unchanged` notable entry
//! names it.
//!
//! The groups `git` and `symbols` use the repository and code services of
//! the execution context; without the service the group is left out and a
//! `source_unavailable` notable entry says why.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use std::sync::Arc;

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, Etag, ExtendedMeta, FileId, FileKind, FileMeta, Follow, Fs, FsErrorKind,
};
use nurt_contracts::record::Entry;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::{GitChange, GitPathStatus, GitQueryService};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, FieldDemand, InputKinds, ReadStage, RecordStream, StageDescriptor,
    StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{self, Fields, Record, Value};
use nurt_text::TextCodec;
use nurt_text::hash::{HashAlgorithm, hash_reader};
use serde_json::Value as Json;

use crate::common::entry::{add_special_node, entry_record, missing_entry};
use crate::common::etag::parse_etag;
use crate::common::notable::at_path;
use crate::common::path::{path_field, path_list_arg, resolve};
use crate::common::text::{record_line_ending, text_codec};
use crate::details::accounts::Accounts;
use crate::details::acl::{self, ACCESS_XATTR, DEFAULT_XATTR};
use crate::details::content::{self, PDF_MIME, Sniffed, TextFacts};
use crate::registry;

const COMMAND: &str = "info";

const PATH: &str = "path";
const HASH: &str = "hash";
const DETAILS: &str = "details";
const KNOWN: &str = "known";

/// Value of `hash` that computes no hash.
const NO_HASH: &str = "none";

/// Names of the `details` groups, in catalog order.
const GROUP_NAMES: [&str; 6] = ["text", "exif", "pdf", "media", "git", "symbols"];

/// Fields recognised from the leading bytes of the content.
const SNIFFED_FIELDS: [&str; 6] = [
    "mime",
    "binary",
    "encoding",
    "line_ending",
    "bom",
    "language",
];

/// Header line of the text form of a macOS ACL.
const ACL_TEXT_HEADER: &str = "!#acl";

/// Prefix of an extended attribute value that is not UTF-8 text.
const HEX_PREFIX: &str = "hex:";

/// Separator of the components of a relative path in language form.
const SEPARATOR: &str = "/";

/// Repository state of an entry outside every repository.
const GIT_NONE: &str = "none";

/// Detail of the note for a missing repository service.
const GIT_UNAVAILABLE: &str =
    "the repository service is not available; `details: [git]` is left out";

/// Detail of the note for a missing code service.
const SYMBOLS_UNAVAILABLE: &str =
    "the code service is not available; `details: [symbols]` is left out";

/// Name of a symbol kind that has no text form.
const OTHER_KIND: &str = "other";

/// Stage of `info`.
pub(crate) struct Info;

impl StageImpl for Info {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Info {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        if let Some(texts) = registry::path_texts(args, PATH)? {
            let records = registry::describe(ctx, &texts)?;
            return Ok(StageOutput::records(
                RecordStream::new(records.into_iter().map(Ok)).checked(ctx),
            ));
        }
        let mut request = Request::parse(args, ctx)?;
        let paths = std::mem::take(&mut request.paths);
        let mut describer = Describer {
            ctx,
            fs: ctx.require_fs()?,
            os: ctx.host().info().os,
            codec: text_codec(ctx),
            request,
            accounts: None,
            noted: BTreeSet::new(),
        };
        let records = paths
            .into_iter()
            .map(move |path| describer.describe(&path).map(Record::from));
        Ok(StageOutput::records(
            RecordStream::new(records).checked(ctx),
        ))
    }
}

/// Stages of this module, for the registry.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Info)]
}

/// Groups requested with `details`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Groups {
    text: bool,
    exif: bool,
    pdf: bool,
    media: bool,
    git: bool,
    symbols: bool,
}

impl Groups {
    fn parse(value: Option<&Value>) -> Result<Self> {
        let items: Vec<&Value> = match value {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::List(items)) => items.iter().collect(),
            Some(other) => vec![other],
        };
        let mut groups = Self::default();
        for item in items {
            let name = enum_text(item).ok_or_else(|| type_error(DETAILS, "a list of names"))?;
            match name {
                "text" => groups.text = true,
                "exif" => groups.exif = true,
                "pdf" => groups.pdf = true,
                "media" => groups.media = true,
                "git" => groups.git = true,
                "symbols" => groups.symbols = true,
                _ => {
                    return Err(Error::new(
                        ErrorCode::UnknownValue,
                        format!(
                            "`{name}` is not a group of `details`: use {}",
                            GROUP_NAMES.join(", ")
                        ),
                    )
                    .with_found(name)
                    .with_expected(Json::from(GROUP_NAMES.to_vec())));
                }
            }
        }
        Ok(groups)
    }

    fn reads_content(self) -> bool {
        self.text || self.exif || self.pdf || self.media || self.symbols
    }
}

/// Arguments of one call.
#[derive(Debug, Default)]
struct Request {
    paths: Vec<AbsPath>,
    hash: Option<HashAlgorithm>,
    groups: Groups,
    known: Vec<(AbsPath, Etag)>,
}

impl Request {
    fn parse(args: &BoundArgs, ctx: &ExecCtx) -> Result<Self> {
        let hash = match args.value(HASH) {
            None | Some(Value::Null) => None,
            Some(value) => {
                let name = enum_text(value).ok_or_else(|| type_error(HASH, "an algorithm name"))?;
                if name == NO_HASH {
                    None
                } else {
                    Some(name.parse::<HashAlgorithm>().map_err(Error::from)?)
                }
            }
        };
        let known = match args.value(KNOWN) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Map(fields)) => fields
                .iter()
                .map(|(path, tag)| match tag {
                    Value::Text(tag) => Ok((resolve(ctx, path)?, parse_etag(tag)?)),
                    _ => Err(type_error(KNOWN, "a map from paths to etags")),
                })
                .collect::<Result<Vec<_>>>()?,
            Some(_) => return Err(type_error(KNOWN, "a map from paths to etags")),
        };
        Ok(Self {
            paths: path_list_arg(args, PATH, ctx)?,
            hash,
            groups: Groups::parse(args.value(DETAILS))?,
            known,
        })
    }
}

/// Builds the entries of one call; keeps the account tables and the notes
/// already given.
struct Describer<'a> {
    ctx: &'a ExecCtx,
    fs: &'a dyn Fs,
    os: Os,
    codec: TextCodec,
    request: Request,
    accounts: Option<Accounts>,
    noted: BTreeSet<&'static str>,
}

impl Describer<'_> {
    fn wants(&self, field: &str) -> bool {
        self.ctx.demand().fields.wants(field)
    }

    /// Returns `true` when the rest of the pipeline names the field; lazy
    /// fields are computed by default only through their `details` group.
    fn names(&self, field: &str) -> bool {
        matches!(&self.ctx.demand().fields, FieldDemand::Only(fields) if fields.contains(field))
    }

    fn describe(&mut self, path: &AbsPath) -> Result<Entry> {
        let meta = match self.fs.metadata(path, Follow::NoFollow) {
            Ok(meta) => meta,
            Err(error)
                if matches!(
                    error.kind,
                    FsErrorKind::NotFound | FsErrorKind::NotADirectory
                ) =>
            {
                return Ok(missing_entry(path));
            }
            Err(error) => return Err(error.into()),
        };
        let mut entry = entry_record(path, &meta);
        add_special_node(self.fs, path, &mut entry);
        entry.exists = Some(true);
        if self.is_known(path, &meta) {
            self.ctx.note(at_path(Notable::Unchanged, path));
            return Ok(entry);
        }
        self.add_identity(path, &meta, &mut entry);
        self.add_link(path, &mut entry);
        self.add_location(path, &meta, &mut entry);
        let mut details = Fields::new();
        if let Some(file) = self.content_meta(path, &meta) {
            self.add_content(path, &file, &mut entry, &mut details)?;
        }
        if self.request.groups.git
            && self.wants(DETAILS)
            && let Some(git) = self.git(path)
        {
            details.insert("git".to_owned(), git);
        }
        if !details.is_empty() {
            entry.details = Some(details);
        }
        Ok(entry)
    }

    fn is_known(&self, path: &AbsPath, meta: &FileMeta) -> bool {
        let current = meta.etag();
        self.request
            .known
            .iter()
            .any(|(known, tag)| known == path && tag.matches(&current))
    }

    /// Account, group, access control list, extended attributes and
    /// alternate streams.
    fn add_identity(&mut self, path: &AbsPath, meta: &FileMeta, entry: &mut Entry) {
        let windows = self.os == Os::Windows;
        let owner = self.wants("owner");
        let group = self.wants("group");
        let acl = self.wants("acl");
        let xattrs = self.wants("xattrs");
        let streams = self.wants("streams");
        let extended = (acl || xattrs || streams || (windows && owner))
            .then(|| self.fs.extended_metadata(path, Follow::NoFollow).ok())
            .flatten();
        let (fs, os) = (self.fs, self.os);
        let empty = Accounts::default();
        let accounts = if !windows && (owner || group || acl) {
            &*self.accounts.get_or_insert_with(|| Accounts::load(fs, os))
        } else {
            &empty
        };
        if windows {
            if owner {
                entry.owner = extended
                    .as_ref()
                    .and_then(|extended| extended.account_sid.clone());
            }
        } else {
            if owner {
                entry.owner = meta.uid.map(|uid| accounts.describe_user(uid));
            }
            if group {
                entry.group = meta.gid.map(|gid| accounts.describe_group(gid));
            }
        }
        let Some(extended) = extended else {
            return;
        };
        if acl {
            entry.acl = acl_lines(&extended, accounts);
        }
        if xattrs {
            let values: Vec<(String, String)> = extended
                .xattrs
                .iter()
                .filter(|xattr| xattr.name != ACCESS_XATTR && xattr.name != DEFAULT_XATTR)
                .map(|xattr| {
                    (
                        xattr.name.to_string_lossy().into_owned(),
                        xattr_text(&xattr.value),
                    )
                })
                .collect();
            if !values.is_empty() {
                entry.xattrs = Some(values.into_iter().collect());
            }
        }
        if streams && !extended.streams.is_empty() {
            entry.streams = Some(
                extended
                    .streams
                    .iter()
                    .map(|stream| stream.name.clone())
                    .collect(),
            );
        }
    }

    fn add_link(&self, path: &AbsPath, entry: &mut Entry) {
        if entry.link_type.is_none() {
            return;
        }
        if self.wants("link_target") {
            let target = self.fs.read_link(path).ok();
            entry.link_target = target.as_deref().map(link_text);
            entry.raw_link_target = target.as_deref().and_then(raw_link_bytes);
        }
        if self.wants("real_path") {
            entry.real_path = self.fs.real_path(path).ok().map(|real| path_field(&real));
        }
    }

    fn add_location(&self, path: &AbsPath, meta: &FileMeta, entry: &mut Entry) {
        if self.wants("volume") {
            entry.volume = self
                .fs
                .volume(path)
                .ok()
                .map(|volume| path_field(&volume.mount));
        }
        if self.wants("file_id") {
            entry.file_id = Some(file_id_text(meta.file_id));
        }
    }

    /// Metadata of the regular file whose content describes the entry: the
    /// entry itself or the file a link resolves to.
    fn content_meta(&self, path: &AbsPath, meta: &FileMeta) -> Option<FileMeta> {
        match meta.kind {
            FileKind::File => Some(meta.clone()),
            FileKind::Symlink | FileKind::Junction => self
                .fs
                .metadata(path, Follow::Follow)
                .ok()
                .filter(|target| target.kind == FileKind::File),
            _ => None,
        }
    }

    fn add_content(
        &mut self,
        path: &AbsPath,
        file: &FileMeta,
        entry: &mut Entry,
        details: &mut Fields,
    ) -> Result<()> {
        let groups = self.request.groups;
        let in_details = self.wants(DETAILS) && groups.reads_content();
        let lines = groups.text || self.names("lines");
        let sniffed_fields = SNIFFED_FIELDS.iter().any(|field| self.wants(field));
        if let Some(algorithm) = self.request.hash
            && self.wants(HASH)
        {
            entry.hash = Some(hash_file(self.fs, path, algorithm)?);
        }
        if !(sniffed_fields || in_details || lines) {
            return Ok(());
        }
        let prefix = content::read_prefix(self.fs, path)?;
        let complete = u64::try_from(prefix.len()).unwrap_or(u64::MAX) >= file.size;
        let sniffed = content::sniff(path, &prefix, complete, &self.codec);
        apply_sniffed(&sniffed, entry);
        if self.wants("language")
            && let Some(code) = self.ctx.code()
        {
            entry.language = code
                .language_of(Some(path.as_path()), &prefix)
                .map(|language| language.to_string());
        }
        let mut whole = Whole::Unread;
        if lines && !sniffed.binary {
            let facts = whole
                .bytes(self.ctx, path, file.size)?
                .and_then(|bytes| content::text_facts(&self.codec, bytes));
            if let Some(facts) = facts {
                apply_facts(&facts, entry);
                if groups.text && in_details {
                    details.insert("text".to_owned(), text_value(&facts));
                }
            }
        }
        if !in_details {
            return Ok(());
        }
        if groups.text && sniffed.binary {
            let mut fields = Fields::new();
            fields.insert("binary".to_owned(), Value::Bool(true));
            details.insert("text".to_owned(), Value::Map(fields));
        }
        if groups.exif
            && sniffed.is_image()
            && let Some(fields) = content::exif_fields(self.fs, path)
        {
            details.insert("exif".to_owned(), Value::Map(fields));
        }
        if groups.pdf
            && sniffed.mime == PDF_MIME
            && let Some(fields) = whole
                .bytes(self.ctx, path, file.size)?
                .and_then(content::pdf_fields)
        {
            details.insert("pdf".to_owned(), Value::Map(fields));
        }
        if groups.media
            && sniffed.is_image()
            && let Some(fields) = content::media_fields(self.fs, path)
        {
            details.insert("media".to_owned(), Value::Map(fields));
        }
        if groups.symbols && !sniffed.binary {
            let bytes = whole.bytes(self.ctx, path, file.size)?.map(<[u8]>::to_vec);
            if let Some(bytes) = bytes
                && let Some(symbols) = self.symbols(path, &bytes)
            {
                details.insert("symbols".to_owned(), symbols);
            }
        }
        Ok(())
    }

    /// Number of code symbols by kind, from the code service.
    fn symbols(&mut self, path: &AbsPath, bytes: &[u8]) -> Option<Value> {
        let ctx = self.ctx;
        let Some(code) = ctx.code() else {
            self.unavailable(path, SYMBOLS_UNAVAILABLE);
            return None;
        };
        let language = code.language_of(Some(path.as_path()), bytes)?;
        let symbols = match code.outline(bytes, &language, None) {
            Ok(symbols) => symbols,
            Err(error) => {
                ctx.note(source_unavailable(path, error.message.clone()));
                return None;
            }
        };
        let mut kinds: BTreeMap<String, i64> = BTreeMap::new();
        for symbol in &symbols {
            let kind = serde_json::to_value(symbol.kind)
                .ok()
                .and_then(|kind| kind.as_str().map(str::to_owned))
                .unwrap_or_else(|| OTHER_KIND.to_owned());
            *kinds.entry(kind).or_default() += 1;
        }
        let mut fields = Fields::new();
        fields.insert("language".to_owned(), Value::Text(language.to_string()));
        fields.insert(
            "total".to_owned(),
            Value::Int(i64::try_from(symbols.len()).unwrap_or(i64::MAX)),
        );
        fields.insert(
            "kinds".to_owned(),
            Value::Map(
                kinds
                    .into_iter()
                    .map(|(kind, count)| (kind, Value::Int(count)))
                    .collect(),
            ),
        );
        Some(Value::Map(fields))
    }

    /// Repository state of the entry, from the repository service.
    fn git(&mut self, path: &AbsPath) -> Option<Value> {
        let ctx = self.ctx;
        let Some(service) = ctx.services().git_query() else {
            self.unavailable(path, GIT_UNAVAILABLE);
            return None;
        };
        match git_status(service, path) {
            Ok(value) => Some(value),
            Err(error) => {
                ctx.note(source_unavailable(path, error.message.clone()));
                None
            }
        }
    }

    /// Notes once per call that a service is missing.
    fn unavailable(&mut self, path: &AbsPath, detail: &'static str) {
        if self.noted.insert(detail) {
            self.ctx.note(source_unavailable(path, detail.to_owned()));
        }
    }
}

/// Content read in whole at most once per entry.
enum Whole {
    Unread,
    Read(Option<Vec<u8>>),
}

impl Whole {
    /// The content; `None` when the file is larger than the text limit of
    /// the execution.
    fn bytes(&mut self, ctx: &ExecCtx, path: &AbsPath, size: u64) -> Result<Option<&[u8]>> {
        if let Self::Unread = self {
            let fs = ctx.require_fs()?;
            let limit = ctx.limits().max_file_bytes_text;
            *self = Self::Read(content::read_whole(fs, path, size, limit)?);
        }
        Ok(match self {
            Self::Read(bytes) => bytes.as_deref(),
            Self::Unread => None,
        })
    }
}

fn apply_sniffed(sniffed: &Sniffed, entry: &mut Entry) {
    entry.mime = Some(sniffed.mime.clone());
    entry.binary = Some(sniffed.binary);
    if let Some(encoding) = &sniffed.encoding {
        entry.encoding = Some(encoding.label.clone());
        entry.bom = Some(encoding.bom);
    }
    entry.line_ending = sniffed.line_ending.map(record_line_ending);
}

fn apply_facts(facts: &TextFacts, entry: &mut Entry) {
    entry.encoding = Some(facts.encoding.label.clone());
    entry.bom = Some(facts.encoding.bom);
    entry.line_ending = Some(record_line_ending(facts.line_ending));
    entry.lines = facts.lines;
}

fn text_value(facts: &TextFacts) -> Value {
    let mut fields = Fields::new();
    fields.insert(
        "encoding".to_owned(),
        Value::Text(facts.encoding.label.clone()),
    );
    fields.insert(
        "line_ending".to_owned(),
        Value::Enum(record_line_ending(facts.line_ending).as_str().to_owned()),
    );
    fields.insert("bom".to_owned(), Value::Bool(facts.encoding.bom));
    if let Some(lines) = facts.lines {
        fields.insert(
            "lines".to_owned(),
            Value::Int(i64::try_from(lines).unwrap_or(i64::MAX)),
        );
    }
    Value::Map(fields)
}

/// Access control entries: the POSIX ACL of Linux, decoded from its
/// extended attribute, or the lines of the text form of macOS. The binary
/// security descriptor of Windows has no entries here.
fn acl_lines(extended: &ExtendedMeta, accounts: &Accounts) -> Option<Vec<String>> {
    if let Some(xattr) = extended
        .xattrs
        .iter()
        .find(|xattr| xattr.name == ACCESS_XATTR)
    {
        return acl::decode(&xattr.value).map(|entries| acl::render(&entries, accounts));
    }
    let text = std::str::from_utf8(extended.acl.as_deref()?).ok()?;
    let lines: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with(ACL_TEXT_HEADER))
        .map(str::to_owned)
        .collect();
    (!lines.is_empty()).then_some(lines)
}

/// Value of an extended attribute: the text when it is UTF-8, otherwise
/// [`HEX_PREFIX`] followed by lower-case hexadecimal digits.
fn xattr_text(value: &[u8]) -> String {
    match std::str::from_utf8(value) {
        Ok(text) => text.to_owned(),
        Err(_) => {
            let digits: String = value.iter().map(|byte| format!("{byte:02x}")).collect();
            format!("{HEX_PREFIX}{digits}")
        }
    }
}

/// Native bytes of the target of a link when it is not valid Unicode, in the
/// same normalisation as [`link_text`].
fn raw_link_bytes(target: &Path) -> Option<value::Bytes> {
    if target.is_absolute()
        && let Ok(absolute) = AbsPath::new(target)
    {
        return absolute.raw_bytes();
    }
    target
        .to_str()
        .is_none()
        .then(|| value::Bytes::new(target.as_os_str().as_encoded_bytes().to_vec()))
}

/// Target of a link in language form: an absolute target normalised, a
/// relative one with `/` separators.
fn link_text(target: &Path) -> value::Path {
    if target.is_absolute()
        && let Ok(absolute) = AbsPath::new(target)
    {
        return path_field(&absolute);
    }
    let parts: Vec<String> = target
        .components()
        .map(|component| match component {
            Component::ParentDir => "..".to_owned(),
            Component::CurDir => ".".to_owned(),
            other => other.as_os_str().to_string_lossy().into_owned(),
        })
        .collect();
    value::Path::new(parts.join(SEPARATOR))
}

/// File identifier: the inode number on Unix, the 128-bit identifier in
/// hexadecimal on Windows.
fn file_id_text(id: FileId) -> String {
    if id.high == 0 {
        id.low.to_string()
    } else {
        format!("{:016x}{:016x}", id.high, id.low)
    }
}

fn hash_file(fs: &dyn Fs, path: &AbsPath, algorithm: HashAlgorithm) -> Result<String> {
    let handle = fs.open_read(path)?;
    hash_reader(algorithm, handle)
        .map(|digest| digest.to_string())
        .map_err(|error| content::read_error(path, &error))
}

/// `details.git`: the state of the entry in its repository (`clean`,
/// `modified`, `untracked`, `ignored`, `conflicted`, or `none` outside every
/// repository) and the root of the working tree.
fn git_status(service: &dyn GitQueryService, path: &AbsPath) -> Result<Value> {
    let mut fields = Fields::new();
    let Some(root) = service.repo_root(path.as_path())? else {
        fields.insert("status".to_owned(), Value::Enum(GIT_NONE.to_owned()));
        return Ok(Value::Map(fields));
    };
    let relative: Vec<String> = path
        .as_path()
        .strip_prefix(&root)
        .map(|relative| {
            relative
                .components()
                .filter_map(|component| match component {
                    Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let paths = if relative.is_empty() {
        Vec::new()
    } else {
        vec![relative.join(SEPARATOR)]
    };
    let statuses = service.status(&root, &paths)?;
    fields.insert(
        "status".to_owned(),
        Value::Enum(summarize(&statuses).to_owned()),
    );
    let root_text = AbsPath::new(&root).map_or_else(
        |_| root.to_string_lossy().into_owned(),
        |root| root.to_language_string(),
    );
    fields.insert("root".to_owned(), Value::Path(root_text));
    Ok(Value::Map(fields))
}

/// State of a set of repository paths, the most significant first:
/// `conflicted`, `modified`, `untracked`, `ignored`, `clean`.
fn summarize(statuses: &[GitPathStatus]) -> &'static str {
    let changes = || {
        statuses
            .iter()
            .flat_map(|status| [&status.index, &status.worktree])
    };
    if changes().any(|change| matches!(change, GitChange::Conflicted)) {
        "conflicted"
    } else if changes().any(|change| {
        !matches!(
            change,
            GitChange::Unmodified | GitChange::Untracked | GitChange::Ignored
        )
    }) {
        "modified"
    } else if changes().any(|change| matches!(change, GitChange::Untracked)) {
        "untracked"
    } else if changes().any(|change| matches!(change, GitChange::Ignored)) {
        "ignored"
    } else {
        "clean"
    }
}

fn source_unavailable(path: &AbsPath, detail: String) -> NotableEntry {
    NotableEntry {
        detail: Some(detail),
        ..at_path(Notable::SourceUnavailable, path)
    }
}

fn enum_text(value: &Value) -> Option<&str> {
    match value {
        Value::Enum(text) | Value::Text(text) => Some(text),
        _ => None,
    }
}

fn type_error(parameter: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{COMMAND}` expects {expected} for `{parameter}`"),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use nurt_contracts::fs::{MetaChange, SpecialKind, SpecialNode, Xattr};
    use nurt_contracts::journal::{FileOp, JournalStore, apply_ops};
    use nurt_contracts::record::{EntryType, LineEnding, LinkType};
    use nurt_contracts::services::Services;
    use nurt_contracts::stage::{ArgValue, Demand, StageData, StageFrame};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::host::TestHost;
    use nurt_testkit::vfs::test_path;
    use pdf_extract::{Dictionary, Document, Object, Stream, StringFormat};

    use super::*;
    use crate::common::etag::etag_text;

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn args(paths: &[&str]) -> BoundArgs {
        let list = paths
            .iter()
            .map(|path| Value::Path(abs(path).to_language_string()))
            .collect();
        BoundArgs::new(COMMAND)
            .with(PATH, ArgValue::Value(Value::List(list)), false)
            .with(HASH, ArgValue::Value(Value::Enum(NO_HASH.to_owned())), true)
            .with(DETAILS, ArgValue::Value(Value::List(Vec::new())), true)
    }

    fn with(args: BoundArgs, name: &str, value: Value) -> BoundArgs {
        args.with(name, ArgValue::Value(value), false)
    }

    fn groups(names: &[&str]) -> Value {
        Value::List(
            names
                .iter()
                .map(|name| Value::Enum((*name).to_owned()))
                .collect(),
        )
    }

    fn entries(ctx: &ExecCtx, args: &BoundArgs) -> Result<Vec<Entry>> {
        let output = Info.execute(args, StageInput::none(), ctx)?;
        let StageData::Records(stream) = output.data else {
            panic!("`info` returns records");
        };
        stream
            .map(|record| Entry::try_from(record?).map_err(Error::from))
            .collect()
    }

    fn set_meta(test: &TestContext, path: &AbsPath, change: MetaChange) {
        let mut tx = test.journal().begin(begin_request("attributes"));
        apply_ops(tx.as_mut(), vec![FileOp::set_meta(path.clone(), change)]).unwrap();
        tx.commit().unwrap();
    }

    fn details_of(entry: &Entry, group: &str) -> Fields {
        match entry
            .details
            .as_ref()
            .and_then(|details| details.get(group))
        {
            Some(Value::Map(fields)) => fields.clone(),
            other => panic!("no `{group}` details: {other:?}"),
        }
    }

    #[test]
    fn missing_paths_are_entries_that_do_not_exist() {
        let test = TestContext::new();
        test.fs().write_file(&abs("/data/a.txt"), b"a").unwrap();
        let found = entries(
            &test.ctx(),
            &args(&["/data/gone.txt", "/data/a.txt/inner", "/data/a.txt"]),
        )
        .unwrap();
        assert_eq!(found[0].exists, Some(false));
        assert_eq!(found[0].name, "gone.txt");
        assert_eq!(found[1].exists, Some(false));
        assert_eq!(found[2].exists, Some(true));
    }

    #[test]
    fn files_carry_content_fields_and_identity() {
        // Owner and group are the numeric identifiers of a Unix host.
        let test = TestContext::with_host(TestHost::linux());
        let path = abs("/data/notes.txt");
        test.fs().write_file(&path, b"one\r\ntwo\r\n").unwrap();
        let entry = entries(&test.ctx(), &args(&["/data/notes.txt"]))
            .unwrap()
            .remove(0);
        assert_eq!(entry.kind, EntryType::File);
        assert_eq!(entry.mime.as_deref(), Some("text/plain"));
        assert_eq!(entry.binary, Some(false));
        assert_eq!(entry.encoding.as_deref(), Some("utf-8"));
        assert_eq!(entry.bom, Some(false));
        assert_eq!(entry.line_ending, Some(LineEnding::Crlf));
        assert_eq!(entry.lines, None);
        assert_eq!(entry.owner.as_deref(), Some("1000"));
        assert_eq!(entry.group.as_deref(), Some("1000"));
        assert!(entry.file_id.is_some());
        assert_eq!(
            entry.volume.map(|volume| volume.as_str().to_owned()),
            Some(abs("/").to_language_string())
        );
        assert_eq!(entry.hash, None);
        assert_eq!(entry.details, None);
    }

    #[test]
    fn account_names_come_from_the_account_tables() {
        let test = TestContext::with_host(TestHost::linux());
        if cfg!(windows) {
            return;
        }
        test.fs()
            .write_file(
                &abs("/etc/passwd"),
                b"user:x:1000:1000::/home/user:/bin/sh\n",
            )
            .unwrap();
        test.fs()
            .write_file(&abs("/etc/group"), b"staff:x:1000:\n")
            .unwrap();
        test.fs().write_file(&abs("/data/a.txt"), b"a").unwrap();
        let entry = entries(&test.ctx(), &args(&["/data/a.txt"]))
            .unwrap()
            .remove(0);
        assert_eq!(entry.owner.as_deref(), Some("user (1000)"));
        assert_eq!(entry.group.as_deref(), Some("staff (1000)"));
    }

    #[test]
    fn windows_entries_take_the_account_from_the_security_identifier() {
        let test = TestContext::with_host(TestHost::windows());
        test.fs().write_file(&abs("/data/a.txt"), b"a").unwrap();
        let entry = entries(&test.ctx(), &args(&["/data/a.txt"]))
            .unwrap()
            .remove(0);
        assert_eq!(entry.owner, None);
        assert_eq!(entry.group, None);
    }

    #[test]
    fn dangling_links_exist_without_content_fields() {
        let test = TestContext::new();
        let link = abs("/data/link");
        test.fs().create_dir_all(&abs("/data")).unwrap();
        test.fs().symlink(PathBuf::from("gone.txt"), &link).unwrap();
        let entry = entries(&test.ctx(), &args(&["/data/link"]))
            .unwrap()
            .remove(0);
        assert_eq!(entry.exists, Some(true));
        assert_eq!(entry.kind, EntryType::Link);
        assert_eq!(entry.link_type, Some(LinkType::Symbolic));
        assert_eq!(
            entry.link_target.map(|target| target.as_str().to_owned()),
            Some("gone.txt".to_owned())
        );
        assert_eq!(entry.real_path, None);
        assert_eq!(entry.mime, None);
    }

    #[test]
    fn links_to_files_describe_the_target_content() {
        let test = TestContext::new();
        let target = abs("/data/a.txt");
        test.fs().write_file(&target, b"text\n").unwrap();
        test.fs()
            .symlink(target.as_path(), &abs("/data/link"))
            .unwrap();
        let entry = entries(&test.ctx(), &args(&["/data/link"]))
            .unwrap()
            .remove(0);
        assert_eq!(entry.kind, EntryType::Link);
        assert_eq!(entry.mime.as_deref(), Some("text/plain"));
        assert_eq!(
            entry.link_target.map(|path| path.as_str().to_owned()),
            Some(target.to_language_string())
        );
        assert_eq!(
            entry.real_path.map(|path| path.as_str().to_owned()),
            Some(target.to_language_string())
        );
    }

    #[test]
    fn directories_and_special_entries_have_no_content_fields() {
        let test = TestContext::new();
        let dir = abs("/data/dir");
        test.fs().create_dir_all(&dir).unwrap();
        let entry = entries(
            &test.ctx(),
            &with(args(&["/data/dir"]), HASH, Value::Enum("sha256".to_owned())),
        )
        .unwrap()
        .remove(0);
        assert_eq!(entry.kind, EntryType::Directory);
        assert_eq!(entry.mime, None);
        assert_eq!(entry.hash, None);

        let ctx = test.ctx();
        let describer = Describer {
            ctx: &ctx,
            fs: ctx.require_fs().unwrap(),
            os: Os::Linux,
            codec: TextCodec::default(),
            request: Request::default(),
            accounts: None,
            noted: BTreeSet::new(),
        };
        let mut special = test.fs().metadata(&dir, Follow::NoFollow).unwrap();
        special.kind = FileKind::Special;
        assert_eq!(describer.content_meta(&dir, &special), None);
    }

    #[test]
    fn a_fifo_and_a_device_node_report_their_type_and_numbers() {
        let test = TestContext::new();
        test.fs().create_dir_all(&abs("/dev")).unwrap();
        test.fs()
            .create_special_node(&abs("/dev/p"), SpecialNode::fifo(), 0o600)
            .unwrap();
        test.fs()
            .create_special_node(
                &abs("/dev/sda"),
                SpecialNode::device(SpecialKind::Block, 8, 0),
                0o660,
            )
            .unwrap();
        let found = entries(&test.ctx(), &args(&["/dev/p", "/dev/sda"])).unwrap();
        assert_eq!(found[0].kind, EntryType::Fifo);
        assert_eq!((found[0].major, found[0].minor), (None, None));
        assert_eq!(found[1].kind, EntryType::Device);
        assert_eq!((found[1].major, found[1].minor), (Some(8), Some(0)));
        assert_eq!(found[1].mode.as_deref(), Some("0660"));
    }

    #[test]
    fn hashes_use_the_requested_algorithm() {
        let test = TestContext::new();
        test.fs()
            .write_file(&abs("/data/a.txt"), b"123456789")
            .unwrap();
        for (algorithm, expected) in [
            ("crc32", "crc32:cbf43926"),
            (
                "sha256",
                "sha256:15e2b0d3c33891ebb0f1ef609ec419420c20e320ce94c65fbc8c3312448eb225",
            ),
        ] {
            let call = with(
                args(&["/data/a.txt"]),
                HASH,
                Value::Enum(algorithm.to_owned()),
            );
            let entry = entries(&test.ctx(), &call).unwrap().remove(0);
            assert_eq!(entry.hash.as_deref(), Some(expected));
        }
        let wrong = with(args(&["/data/a.txt"]), HASH, Value::Enum("sha3".to_owned()));
        let error = entries(&test.ctx(), &wrong).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownValue);
    }

    #[test]
    fn text_details_count_lines_and_binary_files_are_marked() {
        let test = TestContext::new();
        test.fs()
            .write_file(&abs("/data/a.txt"), b"one\ntwo\nthree")
            .unwrap();
        test.fs()
            .write_file(&abs("/data/b.bin"), &[0, 159, 146, 150, 0, 0])
            .unwrap();
        let call = with(
            args(&["/data/a.txt", "/data/b.bin"]),
            DETAILS,
            groups(&["text"]),
        );
        let found = entries(&test.ctx(), &call).unwrap();
        assert_eq!(found[0].lines, Some(3));
        let text = details_of(&found[0], "text");
        assert_eq!(text.get("lines"), Some(&Value::Int(3)));
        assert_eq!(text.get("line_ending"), Some(&Value::Enum("lf".to_owned())));
        assert_eq!(found[1].binary, Some(true));
        assert_eq!(found[1].lines, None);
        assert_eq!(
            details_of(&found[1], "text").get("binary"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn unknown_groups_are_rejected() {
        let test = TestContext::new();
        let call = with(args(&["/data/a.txt"]), DETAILS, groups(&["video"]));
        let error = entries(&test.ctx(), &call).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownValue);
        assert_eq!(error.found.as_deref(), Some("video"));
    }

    #[test]
    fn known_entries_are_reported_unchanged_without_content_fields() {
        let test = TestContext::new();
        let path = abs("/data/a.txt");
        test.fs().write_file(&path, b"text").unwrap();
        let tag = etag_text(&test.fs().etag(&path, Follow::NoFollow).unwrap());
        let mut known = Fields::new();
        known.insert(path.to_language_string(), Value::Text(tag));
        let ctx = test.ctx();
        let call = with(args(&["/data/a.txt"]), KNOWN, Value::Map(known));
        let entry = entries(&ctx, &call).unwrap().remove(0);
        assert_eq!(entry.mime, None);
        assert!(entry.etag.is_some());
        let notes = ctx.take_notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, Notable::Unchanged);

        test.fs().write_file(&path, b"changed").unwrap();
        let entry = entries(&ctx, &call).unwrap().remove(0);
        assert_eq!(entry.mime.as_deref(), Some("text/plain"));
        assert!(ctx.take_notes().is_empty());
    }

    #[test]
    fn narrow_demand_skips_content_reads() {
        let test = TestContext::new();
        test.fs()
            .write_file(&abs("/data/a.txt"), b"a\nb\n")
            .unwrap();
        let fields: BTreeSet<String> = ["path", "lines"].map(str::to_owned).into();
        let ctx = test
            .builder()
            .frame(StageFrame {
                demand: Demand {
                    fields: FieldDemand::Only(fields),
                    ..Demand::default()
                },
                ..StageFrame::default()
            })
            .build();
        let entry = entries(&ctx, &args(&["/data/a.txt"])).unwrap().remove(0);
        assert_eq!(entry.lines, Some(2));
        assert_eq!(entry.owner, None);
        assert_eq!(entry.file_id, None);
        assert_eq!(entry.volume, None);
    }

    #[test]
    fn extended_attributes_and_acl_are_listed() {
        let test = TestContext::new();
        let path = abs("/data/a.txt");
        test.fs().write_file(&path, b"a").unwrap();
        let entries_acl = acl::parse(
            &Value::List(
                [("user", "rw-"), ("group", "r--"), ("other", "---")]
                    .iter()
                    .map(|(tag, perms)| {
                        let mut fields = Fields::new();
                        fields.insert("tag".to_owned(), Value::Text((*tag).to_owned()));
                        fields.insert("perms".to_owned(), Value::Text((*perms).to_owned()));
                        Value::Map(fields)
                    })
                    .collect(),
            ),
            &Accounts::default(),
        )
        .unwrap();
        set_meta(
            &test,
            &path,
            MetaChange {
                set_xattrs: vec![
                    Xattr {
                        name: "user.origin".into(),
                        value: b"camera".to_vec(),
                    },
                    Xattr {
                        name: "user.raw".into(),
                        value: vec![0xff, 0x01],
                    },
                    Xattr {
                        name: ACCESS_XATTR.into(),
                        value: acl::encode(&entries_acl),
                    },
                ],
                ..MetaChange::default()
            },
        );
        let entry = entries(&test.ctx(), &args(&["/data/a.txt"]))
            .unwrap()
            .remove(0);
        let xattrs = entry.xattrs.unwrap();
        assert_eq!(
            xattrs.get("user.origin").map(String::as_str),
            Some("camera")
        );
        assert_eq!(xattrs.get("user.raw").map(String::as_str), Some("hex:ff01"));
        assert!(!xattrs.contains_key(ACCESS_XATTR));
        assert_eq!(
            entry.acl.unwrap(),
            ["user::rw-", "group::r--", "other::---"]
        );
    }

    #[test]
    fn macos_acl_text_is_split_into_entries() {
        let extended = |acl: &[u8]| ExtendedMeta {
            basic: TestContext::new()
                .fs()
                .metadata(&abs("/"), Follow::NoFollow)
                .unwrap(),
            xattrs: Vec::new(),
            acl: Some(acl.to_vec()),
            account_sid: None,
            flags: None,
            streams: Vec::new(),
            link_target: None,
        };
        let text = extended(b"!#acl 1\nuser:ABC:alice:501:allow:read\n\n");
        assert_eq!(
            acl_lines(&text, &Accounts::default()),
            Some(vec!["user:ABC:alice:501:allow:read".to_owned()])
        );
        assert_eq!(
            acl_lines(&extended(&[1, 0, 0xff]), &Accounts::default()),
            None
        );
    }

    #[test]
    fn pdf_and_image_details_describe_the_document() {
        let test = TestContext::new();
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let content_id = document.add_object(Stream::new(Dictionary::new(), Vec::new()));
        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set("Parent", Object::Reference(pages_id));
        page.set("Contents", Object::Reference(content_id));
        let page_id = document.add_object(page);
        let mut tree = Dictionary::new();
        tree.set("Type", Object::Name(b"Pages".to_vec()));
        tree.set("Count", Object::Integer(1));
        tree.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
        tree.set(
            "MediaBox",
            Object::Array([0, 0, 595, 842].map(Object::Integer).to_vec()),
        );
        document.objects.insert(pages_id, Object::Dictionary(tree));
        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog_id = document.add_object(catalog);
        document.trailer.set("Root", Object::Reference(catalog_id));
        let mut info = Dictionary::new();
        info.set(
            "Title",
            Object::String(b"Annual report".to_vec(), StringFormat::Literal),
        );
        let info_id = document.add_object(info);
        document.trailer.set("Info", Object::Reference(info_id));
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        test.fs()
            .write_file(&abs("/data/report.pdf"), &bytes)
            .unwrap();

        let call = with(
            args(&["/data/report.pdf"]),
            DETAILS,
            groups(&["pdf", "media", "exif"]),
        );
        let entry = entries(&test.ctx(), &call).unwrap().remove(0);
        assert_eq!(entry.mime.as_deref(), Some(PDF_MIME));
        let pdf = details_of(&entry, "pdf");
        assert_eq!(pdf.get("pages"), Some(&Value::Int(1)));
        assert_eq!(
            pdf.get("title"),
            Some(&Value::Text("Annual report".to_owned()))
        );
        let details = entry.details.unwrap();
        assert!(!details.contains_key("media"));
        assert!(!details.contains_key("exif"));
    }

    struct FakeGit {
        statuses: Vec<GitPathStatus>,
        fail: bool,
    }

    impl GitQueryService for FakeGit {
        fn repo_root(&self, path: &Path) -> Result<Option<PathBuf>> {
            if self.fail {
                return Err(Error::new(ErrorCode::Io, "index is unreadable"));
            }
            let root = abs("/repo");
            Ok(path
                .starts_with(root.as_path())
                .then(|| root.as_path().to_path_buf()))
        }

        fn status(&self, _root: &Path, paths: &[String]) -> Result<Vec<GitPathStatus>> {
            Ok(self
                .statuses
                .iter()
                .filter(|status| paths.is_empty() || paths.contains(&status.path))
                .cloned()
                .collect())
        }

        fn changed_files(&self, _root: &Path, _against: Option<&str>) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        fn tracked_files(&self, _root: &Path) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        fn is_ignored(&self, _root: &Path, _path: &str) -> Result<bool> {
            Ok(false)
        }

        fn file_history(
            &self,
            _root: &Path,
            _since_days: u32,
            _limit: u32,
        ) -> Result<Vec<nurt_contracts::services::FileHistoryStat>> {
            Ok(Vec::new())
        }
    }

    fn status(path: &str, index: GitChange, worktree: GitChange) -> GitPathStatus {
        GitPathStatus {
            path: path.to_owned(),
            index,
            worktree,
            original_path: None,
        }
    }

    fn git_entries(git: FakeGit, paths: &[&str]) -> Vec<Entry> {
        let test = TestContext::new().with_services(Services::new().with_git_query(Arc::new(git)));
        for path in paths {
            test.fs().write_file(&abs(path), b"x").unwrap();
        }
        entries(&test.ctx(), &with(args(paths), DETAILS, groups(&["git"]))).unwrap()
    }

    fn git_state(entry: &Entry) -> Option<Value> {
        details_of(entry, "git").get("status").cloned()
    }

    #[test]
    fn git_details_give_the_repository_state() {
        let git = FakeGit {
            statuses: vec![
                status("src/a.rs", GitChange::Unmodified, GitChange::Modified),
                status("new.rs", GitChange::Untracked, GitChange::Untracked),
                status("both.rs", GitChange::Conflicted, GitChange::Modified),
            ],
            fail: false,
        };
        let found = git_entries(
            git,
            &[
                "/repo/src/a.rs",
                "/repo/new.rs",
                "/repo/both.rs",
                "/repo/same.rs",
                "/elsewhere.txt",
            ],
        );
        let states: Vec<Option<Value>> = found.iter().map(git_state).collect();
        let expected = ["modified", "untracked", "conflicted", "clean", "none"]
            .map(|state| Some(Value::Enum(state.to_owned())));
        assert_eq!(states, expected);
        assert_eq!(
            details_of(&found[0], "git").get("root"),
            Some(&Value::Path(abs("/repo").to_language_string()))
        );
    }

    #[test]
    fn missing_or_failing_services_leave_the_groups_out() {
        let test = TestContext::new().with_services(Services::new());
        test.fs()
            .write_file(&abs("/data/a.rs"), b"fn a() {}")
            .unwrap();
        test.fs()
            .write_file(&abs("/data/b.rs"), b"fn b() {}")
            .unwrap();
        let ctx = test.ctx();
        let call = with(
            args(&["/data/a.rs", "/data/b.rs"]),
            DETAILS,
            groups(&["git", "symbols"]),
        );
        let found = entries(&ctx, &call).unwrap();
        assert!(found.iter().all(|entry| entry.details.is_none()));
        let notes = ctx.take_notes();
        assert_eq!(notes.len(), 2);
        assert!(
            notes
                .iter()
                .all(|note| note.code == Notable::SourceUnavailable)
        );

        let failing = git_entries(
            FakeGit {
                statuses: Vec::new(),
                fail: true,
            },
            &["/repo/a.rs"],
        );
        assert_eq!(failing[0].details, None);
    }

    #[test]
    fn known_and_details_need_values_of_the_right_type() {
        let test = TestContext::new();
        let wrong = with(args(&["/data/a.txt"]), KNOWN, Value::Int(1));
        assert_eq!(
            entries(&test.ctx(), &wrong).unwrap_err().code(),
            ErrorCode::Type
        );
        let mut tags = Fields::new();
        tags.insert("/data/a.txt".to_owned(), Value::Int(1));
        let wrong = with(args(&["/data/a.txt"]), KNOWN, Value::Map(tags));
        assert_eq!(
            entries(&test.ctx(), &wrong).unwrap_err().code(),
            ErrorCode::Type
        );
        let items = Value::List(vec![Value::Int(1)]);
        let wrong = with(args(&["/data/a.txt"]), DETAILS, items);
        assert_eq!(
            entries(&test.ctx(), &wrong).unwrap_err().code(),
            ErrorCode::Type
        );
    }

    #[test]
    fn relative_link_targets_keep_their_components() {
        assert_eq!(link_text(Path::new("./a/../b")).as_str(), "./a/../b");
        assert_eq!(link_text(Path::new("a/b")).as_str(), "a/b");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_target_that_is_not_valid_unicode_keeps_its_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let relative = Path::new(OsStr::from_bytes(b"../bad\xffname"));
        assert_eq!(
            raw_link_bytes(relative).unwrap().as_slice(),
            b"../bad\xffname"
        );
        let absolute = Path::new(OsStr::from_bytes(b"/data/./bad\xffname"));
        assert_eq!(
            raw_link_bytes(absolute).unwrap().as_slice(),
            b"/data/bad\xffname"
        );
        assert_eq!(raw_link_bytes(Path::new("../a.so")), None);
    }

    #[test]
    fn helpers_format_identifiers_and_values() {
        assert_eq!(
            file_id_text(FileId {
                volume: 1,
                low: 42,
                high: 0
            }),
            "42"
        );
        assert_eq!(
            file_id_text(FileId {
                volume: 1,
                low: 1,
                high: 2
            }),
            "00000000000000020000000000000001"
        );
        assert_eq!(xattr_text(b"plain"), "plain");
        assert_eq!(link_text(Path::new("../lib/a.so")).as_str(), "../lib/a.so");
        assert_eq!(summarize(&[]), "clean");
        assert_eq!(
            summarize(&[status("a", GitChange::Ignored, GitChange::Ignored)]),
            "ignored"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
