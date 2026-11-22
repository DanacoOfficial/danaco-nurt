// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Values, units and dynamic records of the language.
//!
//! A [`Value`] is any value a program can hold: a scalar of one of the
//! language types (`text`, `int`, `number`, `bool`, `path`, `glob`, `regex`,
//! `size`, `duration`, `instant`, `key`, `bytes`, `range`, an enumeration
//! value), a list, a map or a [`Record`]. Records are what pipelines exchange
//! between stages; the typed forms of every record kind live in
//! [`crate::record`].
//!
//! # JSON form `nurt-value/1`
//!
//! A value is an object with the type in the member `"t"` and the payload in
//! the member `"v"`; `null` has no payload:
//!
//! ```json
//! {"t": "size", "v": 1048576}
//! {"t": "instant", "v": "2026-11-21T10:00:00Z"}
//! {"t": "list", "v": [{"t": "int", "v": 1}, {"t": "null"}]}
//! ```
//!
//! The tagged form is lossless: a size and an integer with the same number
//! stay distinct, and the order of map keys and record fields is preserved.
//!
//! | Type | Payload |
//! |---|---|
//! | `size` | bytes, unsigned integer |
//! | `duration` | milliseconds, signed integer |
//! | `instant` | RFC 3339 text in UTC with the suffix `Z` and 0, 3, 6 or 9 fraction digits |
//! | `regex` | `{"pattern": "…", "flags": "imsx"}` (flags in this order, omitted when empty) |
//! | `range` | `{"start": 1, "end": 5}`; both bounds inclusive and optional |
//! | `bytes` | standard base64 with padding |
//! | `record` | `{"kind": "entry", "fields": {…}}` |
//!
//! # Examples
//!
//! ```
//! use nurt_contracts::value::{Record, RecordKind, Size, Value};
//!
//! let record = Record::new(RecordKind::Entry)
//!     .with("path", Value::Path("/home/user/notes.txt".to_owned()))
//!     .with("size", Value::Size(Size::from_bytes(1200)));
//! let json = serde_json::to_value(Value::Record(record.clone())).unwrap();
//! assert_eq!(json["t"], "record");
//! assert_eq!(json["v"]["fields"]["size"]["v"], 1200);
//! let back: Value = serde_json::from_value(json).unwrap();
//! assert_eq!(back, Value::Record(record));
//! ```

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

/// Ordered map from field names or map keys to values.
pub type Fields = IndexMap<String, Value>;

/// Error returned when a text or a value does not form a valid value of a
/// language type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValueError {
    /// Name of the type that was expected, for example `instant`.
    pub type_name: String,
    /// The rejected input.
    pub input: String,
    /// Why the input was rejected.
    pub reason: String,
}

impl ValueError {
    /// Creates an error for the type `type_name` and the rejected `input`.
    #[must_use]
    pub fn new(type_name: &str, input: &str, reason: &str) -> Self {
        Self {
            type_name: type_name.to_owned(),
            input: input.to_owned(),
            reason: reason.to_owned(),
        }
    }
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid {} `{}`: {}",
            self.type_name, self.input, self.reason
        )
    }
}

impl std::error::Error for ValueError {}

/// Value of the language.
///
/// The JSON form carries the type in `"t"` and the payload in `"v"` (see the
/// module documentation).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "t",
    content = "v",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[non_exhaustive]
pub enum Value {
    /// Absence of a value, written `null`.
    #[default]
    Null,
    /// Truth value, `true` or `false`.
    Bool(bool),
    /// Signed 64-bit integer.
    Int(i64),
    /// Integer outside the 64-bit range, as decimal digits with an optional
    /// leading `-`; produced when reading documents.
    BigInt(String),
    /// Finite 64-bit floating point number.
    Number(f64),
    /// Number kept as its exact source text because the binary form would
    /// change it (for example `1.10` or `1e400` read from a document).
    Decimal(String),
    /// Unicode text.
    Text(String),
    /// Size in bytes.
    Size(Size),
    /// Signed duration in milliseconds.
    Duration(Duration),
    /// Point in time in UTC with nanosecond precision.
    Instant(Instant),
    /// Date or time of day read from a document, kept in its source form.
    Date(Date),
    /// Path in canonical form: `/` as separator, absolute, upper-case drive
    /// letter, or a provider path such as `env:/PATH`.
    Path(String),
    /// File name pattern.
    Glob(String),
    /// Regular expression.
    Regex(Regex),
    /// Key path inside a structured document, for example `scripts.test`.
    Key(String),
    /// Binary data.
    Bytes(Bytes),
    /// Inclusive range of integers.
    Range(Range),
    /// Value of an enumeration, for example `directory`.
    Enum(String),
    /// Ordered list of values.
    List(Vec<Value>),
    /// Map with ordered keys.
    Map(Fields),
    /// Record of a known kind with ordered fields.
    Record(Record),
}

impl Value {
    /// Name of the language type of the value, as used in error messages
    /// and in `describe`.
    ///
    /// ```
    /// use nurt_contracts::value::Value;
    ///
    /// assert_eq!(Value::Int(3).type_name(), "int");
    /// assert_eq!(Value::Path("/tmp".to_owned()).type_name(), "path");
    /// ```
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Int(_) | Self::BigInt(_) => "int",
            Self::Number(_) | Self::Decimal(_) => "number",
            Self::Text(_) => "text",
            Self::Size(_) => "size",
            Self::Duration(_) => "duration",
            Self::Instant(_) => "instant",
            Self::Date(_) => "date",
            Self::Path(_) => "path",
            Self::Glob(_) => "glob",
            Self::Regex(_) => "regex",
            Self::Key(_) => "key",
            Self::Bytes(_) => "bytes",
            Self::Range(_) => "range",
            Self::Enum(_) => "enum",
            Self::List(_) => "list",
            Self::Map(_) => "map",
            Self::Record(_) => "record",
        }
    }

    /// Whether the value is `null`.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The truth value, when the value is a `bool`.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// The integer, when the value is an `int` within the 64-bit range.
    #[must_use]
    pub const fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    /// The number, when the value is a `number` or an `int`.
    #[must_use]
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::Int(value) => Some(*value as f64),
            _ => None,
        }
    }

    /// The text payload of a `text`, `path`, `glob`, `key` or enumeration
    /// value.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) | Self::Path(text) | Self::Glob(text) | Self::Key(text) => Some(text),
            Self::Enum(text) => Some(text),
            _ => None,
        }
    }

    /// The items, when the value is a list.
    #[must_use]
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }

    /// The entries, when the value is a map.
    #[must_use]
    pub const fn as_map(&self) -> Option<&Fields> {
        match self {
            Self::Map(map) => Some(map),
            _ => None,
        }
    }

    /// The record, when the value is a record.
    #[must_use]
    pub const fn as_record(&self) -> Option<&Record> {
        match self {
            Self::Record(record) => Some(record),
            _ => None,
        }
    }

    /// The entry of a map or the field of a record with the given name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Value> {
        match self {
            Self::Map(map) => map.get(name),
            Self::Record(record) => record.get(name),
            _ => None,
        }
    }

    /// Converts the value to plain JSON without type tags, as used for
    /// records in reports: sizes as bytes, durations as milliseconds,
    /// instants as RFC 3339 text, regular expressions as `/pattern/flags`,
    /// ranges as `1..5`, bytes as base64, records as objects of their fields.
    /// Numbers that are not finite become `null`.
    ///
    /// ```
    /// use nurt_contracts::value::{Size, Value};
    ///
    /// assert_eq!(Value::Size(Size::from_bytes(512)).to_plain_json(), serde_json::json!(512));
    /// ```
    #[must_use]
    pub fn to_plain_json(&self) -> serde_json::Value {
        use serde_json::Value as Json;
        match self {
            Self::Null => Json::Null,
            Self::Bool(value) => Json::Bool(*value),
            Self::Int(value) => Json::from(*value),
            Self::Number(value) => {
                serde_json::Number::from_f64(*value).map_or(Json::Null, Json::Number)
            }
            Self::BigInt(text) | Self::Decimal(text) | Self::Text(text) => {
                Json::String(text.clone())
            }
            Self::Path(text) | Self::Glob(text) | Self::Key(text) | Self::Enum(text) => {
                Json::String(text.clone())
            }
            Self::Size(size) => Json::from(size.bytes()),
            Self::Duration(duration) => Json::from(duration.as_millis()),
            Self::Instant(instant) => Json::String(instant.to_string()),
            Self::Date(date) => Json::String(date.text.clone()),
            Self::Regex(regex) => Json::String(regex.to_string()),
            Self::Bytes(bytes) => Json::String(bytes.to_base64()),
            Self::Range(range) => Json::String(range.to_string()),
            Self::List(items) => Json::Array(items.iter().map(Self::to_plain_json).collect()),
            Self::Map(map) => Json::Object(plain_object(map)),
            Self::Record(record) => Json::Object(plain_object(&record.fields)),
        }
    }

    /// Converts plain JSON to a value: objects become maps, arrays lists,
    /// strings text; integers outside the signed 64-bit range become
    /// [`Value::BigInt`].
    #[must_use]
    pub fn from_plain_json(json: serde_json::Value) -> Self {
        use serde_json::Value as Json;
        match json {
            Json::Null => Self::Null,
            Json::Bool(value) => Self::Bool(value),
            Json::Number(number) => plain_number(&number),
            Json::String(text) => Self::Text(text),
            Json::Array(items) => {
                Self::List(items.into_iter().map(Self::from_plain_json).collect())
            }
            Json::Object(map) => Self::Map(
                map.into_iter()
                    .map(|(key, item)| (key, Self::from_plain_json(item)))
                    .collect(),
            ),
        }
    }
}

fn plain_object(fields: &Fields) -> serde_json::Map<String, serde_json::Value> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), value.to_plain_json()))
        .collect()
}

fn plain_number(number: &serde_json::Number) -> Value {
    if let Some(value) = number.as_i64() {
        Value::Int(value)
    } else if let Some(value) = number.as_u64() {
        Value::BigInt(value.to_string())
    } else {
        number.as_f64().map_or(Value::Null, Value::Number)
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i32> for Value {
    fn from(value: i32) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        i64::try_from(value).map_or_else(|_| Self::BigInt(value.to_string()), Self::Int)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<Size> for Value {
    fn from(value: Size) -> Self {
        Self::Size(value)
    }
}

impl From<Duration> for Value {
    fn from(value: Duration) -> Self {
        Self::Duration(value)
    }
}

impl From<Instant> for Value {
    fn from(value: Instant) -> Self {
        Self::Instant(value)
    }
}

impl From<Path> for Value {
    fn from(value: Path) -> Self {
        Self::Path(value.0)
    }
}

impl From<Regex> for Value {
    fn from(value: Regex) -> Self {
        Self::Regex(value)
    }
}

impl From<Bytes> for Value {
    fn from(value: Bytes) -> Self {
        Self::Bytes(value)
    }
}

impl From<Range> for Value {
    fn from(value: Range) -> Self {
        Self::Range(value)
    }
}

impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Self::List(value)
    }
}

impl From<Fields> for Value {
    fn from(value: Fields) -> Self {
        Self::Map(value)
    }
}

impl From<Record> for Value {
    fn from(value: Record) -> Self {
        Self::Record(value)
    }
}

/// Record: a kind and fields in a fixed order.
///
/// This is the dynamic form used by `filter`, `select`, `sort` and other
/// stages that work on fields by name; the typed form of each kind is in
/// [`crate::record`] and converts to and from this form.
///
/// ```
/// use nurt_contracts::value::{Record, RecordKind, Value};
///
/// let mut record = Record::new(RecordKind::Map).with("count", Value::Int(3));
/// record.set("label", Value::from("done"));
/// assert_eq!(record.field_names().collect::<Vec<_>>(), ["count", "label"]);
/// assert_eq!(record.get("count"), Some(&Value::Int(3)));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// Kind of the record.
    pub kind: RecordKind,
    /// Fields in order; a field that has no value is absent.
    #[serde(default)]
    pub fields: Fields,
}

impl Record {
    /// Creates a record of the given kind without fields.
    #[must_use]
    pub fn new(kind: RecordKind) -> Self {
        Self {
            kind,
            fields: Fields::new(),
        }
    }

    /// Returns the record with the field set to the value; an existing
    /// field keeps its position.
    #[must_use]
    pub fn with(mut self, name: &str, value: Value) -> Self {
        self.set(name, value);
        self
    }

    /// Sets the field and returns the previous value; a new field is added
    /// at the end, an existing field keeps its position.
    pub fn set(&mut self, name: &str, value: Value) -> Option<Value> {
        self.fields.insert(name.to_owned(), value)
    }

    /// Removes the field and returns its value; the other fields keep their
    /// order.
    pub fn remove(&mut self, name: &str) -> Option<Value> {
        self.fields.shift_remove(name)
    }

    /// The value of the field.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields.get(name)
    }

    /// Names of the fields in order.
    pub fn field_names(&self) -> impl Iterator<Item = &str> {
        self.fields.keys().map(String::as_str)
    }

    /// Number of fields.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Whether the record has no fields.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// The fields as a plain JSON object (see [`Value::to_plain_json`]).
    #[must_use]
    pub fn to_plain_json(&self) -> serde_json::Value {
        serde_json::Value::Object(plain_object(&self.fields))
    }
}

macro_rules! record_kinds {
    ($( $(#[$meta:meta])* $variant:ident = $name:literal, )+) => {
        /// Kind of a record.
        ///
        /// Every kind has a stable name used in programs, in the catalog
        /// (`input` and `output` of commands) and in JSON.
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
            JsonSchema,
        )]
        #[non_exhaustive]
        pub enum RecordKind {
            $( $(#[$meta])* #[serde(rename = $name)] $variant, )+
        }

        impl RecordKind {
            /// Every record kind in a stable order.
            pub const ALL: &'static [RecordKind] = &[$(RecordKind::$variant),+];

            /// Stable name of the kind.
            ///
            /// ```
            /// use nurt_contracts::value::RecordKind;
            ///
            /// assert_eq!(RecordKind::TestResult.as_str(), "test_result");
            /// assert_eq!("test_result".parse::<RecordKind>(), Ok(RecordKind::TestResult));
            /// ```
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(RecordKind::$variant => $name,)+
                }
            }
        }

        impl FromStr for RecordKind {
            type Err = ValueError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                match text {
                    $($name => Ok(RecordKind::$variant),)+
                    _ => Err(ValueError::new("record kind", text, "unknown record kind")),
                }
            }
        }
    };
}

record_kinds! {
    /// File system entry (`list`, `find`, `info`).
    Entry = "entry",
    /// Text content of a file (`read`).
    Content = "content",
    /// One line of a file (`lines`).
    Line = "line",
    /// Match of a search (`search`).
    Match = "match",
    /// Value at a key path in a structured document (`read key:`, `query`).
    Item = "item",
    /// Row of a table, CSV file or SQLite query (`sql`, `split`).
    Row = "row",
    /// Node of a syntax tree (`search mode: syntax`).
    Node = "node",
    /// Symbol of source code (`code.symbols`, `code.outline`).
    Symbol = "symbol",
    /// Reference to a symbol (`code.references`).
    Reference = "reference",
    /// Compiler or language server diagnostic (`code.diagnostics`).
    Diagnostic = "diagnostic",
    /// Cell of a notebook (`notebook.cells`).
    Cell = "cell",
    /// Hunk of a difference (`diff`, `git.diff`).
    Hunk = "hunk",
    /// Change of one file within an effect.
    Change = "change",
    /// Result of a finished program run (`run`).
    Exec = "exec",
    /// Background job (`jobs`).
    Job = "job",
    /// Operating system process (`processes`).
    Process = "process",
    /// Network port (`ports`).
    Port = "port",
    /// Mounted disk (`disks`).
    Disk = "disk",
    /// Description of the device (`host`).
    Host = "host",
    /// Environment variable (`environment`).
    EnvironmentVariable = "environment_variable",
    /// Operating system service (`services`).
    Service = "service",
    /// Installed or available package (`package.list`).
    Package = "package",
    /// User or group account (`accounts`).
    Account = "account",
    /// System log event (`logs`).
    LogEvent = "log_event",
    /// Repository commit (`git.log`).
    Commit = "commit",
    /// Status of a path in a repository (`git.status`).
    GitStatus = "git_status",
    /// Repository branch (`git.branch`).
    Branch = "branch",
    /// Scheduled task (`schedule.list`).
    Schedule = "schedule",
    /// Entry of the operation journal (`history`).
    Operation = "operation",
    /// Response that an HTTP request received (`http.request`).
    Response = "response",
    /// Volume as seen by the journal (`journal.space`).
    Volume = "volume",
    /// Description of an installed program (`program.search`).
    ProgramCard = "program_card",
    /// Member of an archive (`archive.list`).
    ArchiveEntry = "archive_entry",
    /// Test run (`test.run`).
    TestRun = "test_run",
    /// Result of one test (`test.show`).
    TestResult = "test_result",
    /// Finding of an analysis or review (`analyze.check`, `git.review`).
    Finding = "finding",
    /// Effect of a changing or running stage; the fields are those of the
    /// effect report.
    Effect = "effect",
    /// Value in the Windows registry (`read "registry:/…"`).
    RegistryValue = "registry_value",
    /// State and limits of the operation journal (`journal.status`,
    /// `journal.limits`).
    JournalState = "journal_state",
    /// Result of an expression (`evaluate`).
    Value = "value",
    /// Group of records (`group`).
    Group = "group",
    /// Statistics of a field (`summarize`).
    Summary = "summary",
    /// Field of the records in a pipeline (`describe`).
    FieldInfo = "field_info",
    /// Management object of Windows (`windows.cim`).
    CimObject = "cim_object",
    /// Line annotated with its last commit (`git.blame`).
    BlameLine = "blame_line",
    /// Description of a symbol at a position (`code.hover`).
    Hover = "hover",
    /// Code action offered by a language server (`code.actions`).
    CodeAction = "code_action",
    /// Supported language and its server (`code.languages`).
    Language = "language",
    /// Planned edit of a rename (`code.rename` preview).
    RenameEdit = "rename_edit",
    /// Entry of a repository profile (`code.profile`).
    ProfileEntry = "profile_entry",
    /// Result of a reachability test (`network.ping`).
    PingResult = "ping_result",
    /// DNS resource record (`network.resolve`).
    DnsRecord = "dns_record",
    /// Network interface (`network.interfaces`).
    NetworkInterface = "network_interface",
    /// Location of a program (`program.find`).
    ProgramLocation = "program_location",
    /// Category of programs (`program.catalog`).
    ProgramCategory = "program_category",
    /// One run of a scheduled task (`schedule.history`).
    ScheduleRun = "schedule_run",
    /// Verdict of an analysis (`analyze.check`).
    Analysis = "analysis",
    /// Analysis tool found in a project (`analyze.tools`).
    AnalyzeTool = "analyze_tool",
    /// Repository tag (`git.tag`).
    GitTag = "git_tag",
    /// Working tree of a repository (`git.worktree`).
    GitWorktree = "git_worktree",
    /// Stash entry (`git.stash`).
    GitStash = "git_stash",
    /// Checkpoint of a working tree (`git.checkpoint`).
    GitCheckpoint = "git_checkpoint",
    /// Update of a reference by a transfer (`git.fetch`, `git.push`).
    GitRefUpdate = "git_ref_update",
    /// Conflict left by a merge (`git.merge`, `git.rebase`).
    GitConflict = "git_conflict",
    /// Effect of an operation on a repository (part `git` of an effect).
    GitEffect = "git_effect",
    /// Record of computed or source-defined fields without a fixed kind
    /// (`select` with computed fields, `parse`, `windows.cim`).
    Map = "map",
}

impl fmt::Display for RecordKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Size in bytes.
///
/// The text form (`12.4kb`) belongs to the language implementation; the
/// JSON form is the number of bytes.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(transparent)]
pub struct Size(u64);

impl Size {
    /// Zero bytes.
    pub const ZERO: Self = Self(0);

    /// Size of the given number of bytes.
    #[must_use]
    pub const fn from_bytes(bytes: u64) -> Self {
        Self(bytes)
    }

    /// Number of bytes.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.0
    }

    /// Sum, or `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(bytes) => Some(Self(bytes)),
            None => None,
        }
    }

    /// Difference, or `None` when `other` is larger.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(bytes) => Some(Self(bytes)),
            None => None,
        }
    }
}

impl From<u64> for Size {
    fn from(bytes: u64) -> Self {
        Self(bytes)
    }
}

impl From<Size> for u64 {
    fn from(size: Size) -> Self {
        size.0
    }
}

/// Signed duration in milliseconds.
///
/// The text form (`1h30m`) belongs to the language implementation; the JSON
/// form is the number of milliseconds.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(transparent)]
pub struct Duration(i64);

impl Duration {
    /// Zero milliseconds.
    pub const ZERO: Self = Self(0);

    /// Duration of the given number of milliseconds.
    #[must_use]
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// Duration of the given number of seconds, or `None` on overflow.
    #[must_use]
    pub const fn from_secs(seconds: i64) -> Option<Self> {
        match seconds.checked_mul(1000) {
            Some(millis) => Some(Self(millis)),
            None => None,
        }
    }

    /// Number of milliseconds.
    #[must_use]
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Whether the duration is below zero.
    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Sum, or `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(millis) => Some(Self(millis)),
            None => None,
        }
    }

    /// Difference, or `None` on overflow.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(millis) => Some(Self(millis)),
            None => None,
        }
    }

    /// Converts from a standard duration, truncating to whole milliseconds;
    /// `None` when it does not fit.
    #[must_use]
    pub fn from_std(duration: std::time::Duration) -> Option<Self> {
        i64::try_from(duration.as_millis()).ok().map(Self)
    }

    /// Converts to a standard duration; `None` when negative.
    #[must_use]
    pub fn to_std(self) -> Option<std::time::Duration> {
        u64::try_from(self.0)
            .ok()
            .map(std::time::Duration::from_millis)
    }
}

const NANOS_PER_SECOND: i64 = 1_000_000_000;
const NANOS_PER_MILLI: i64 = 1_000_000;
const NANOS_PER_MICRO: i64 = 1_000;
const FRACTION_DIGITS: u32 = 9;
const SECONDS_PER_DAY: i64 = 86_400;
const SECONDS_PER_HOUR: i64 = 3_600;
const SECONDS_PER_MINUTE: i64 = 60;
const MINUTES_PER_HOUR: i64 = 60;
const HOURS_PER_DAY: i64 = 24;
const MONTHS_PER_YEAR: i64 = 12;

/// Days of a 400-year cycle of the Gregorian calendar.
const DAYS_PER_ERA: i64 = 146_097;
/// Days from 0000-03-01, the start of the shifted calendar used by the
/// conversion, to 1970-01-01.
const DAYS_TO_UNIX_EPOCH: i64 = 719_468;

/// Pattern of the JSON form of an instant.
const INSTANT_PATTERN: &str = "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\\.[0-9]{3}|\\.[0-9]{6}|\\.[0-9]{9})?Z$";

/// Point in time in UTC with nanosecond precision, from 1677 to 2262.
///
/// The JSON and text form is RFC 3339 in UTC with the suffix `Z` and as many
/// fraction digits as needed in groups of three (`2026-11-21T10:00:00Z`,
/// `2026-11-21T10:00:00.250Z`). Parsing also accepts a lower-case `t` or `z`,
/// any number of fraction digits up to nine and a numeric offset
/// (`+01:00`), which is converted to UTC.
///
/// ```
/// use nurt_contracts::value::Instant;
///
/// let instant: Instant = "2026-11-21T11:00:00.5+01:00".parse().unwrap();
/// assert_eq!(instant.to_string(), "2026-11-21T10:00:00.500Z");
/// assert_eq!(Instant::UNIX_EPOCH.to_string(), "1970-01-01T00:00:00Z");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Instant(i64);

impl Instant {
    /// 1970-01-01T00:00:00Z.
    pub const UNIX_EPOCH: Self = Self(0);

    /// Instant at the given number of nanoseconds since the Unix epoch.
    #[must_use]
    pub const fn from_unix_nanos(nanos: i64) -> Self {
        Self(nanos)
    }

    /// Instant at the given number of seconds since the Unix epoch, or
    /// `None` outside the supported range.
    #[must_use]
    pub const fn from_unix_seconds(seconds: i64) -> Option<Self> {
        match seconds.checked_mul(NANOS_PER_SECOND) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// Nanoseconds since the Unix epoch.
    #[must_use]
    pub const fn as_unix_nanos(self) -> i64 {
        self.0
    }

    /// The instant moved forward by the duration, or `None` outside the
    /// supported range.
    #[must_use]
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        duration
            .as_millis()
            .checked_mul(NANOS_PER_MILLI)
            .and_then(|nanos| self.0.checked_add(nanos))
            .map(Self)
    }

    /// The instant moved back by the duration, or `None` outside the
    /// supported range.
    #[must_use]
    pub fn checked_sub(self, duration: Duration) -> Option<Self> {
        duration
            .as_millis()
            .checked_mul(NANOS_PER_MILLI)
            .and_then(|nanos| self.0.checked_sub(nanos))
            .map(Self)
    }

    /// Time elapsed from `earlier` to this instant, truncated to whole
    /// milliseconds; negative when `earlier` is later.
    #[must_use]
    pub fn duration_since(self, earlier: Self) -> Option<Duration> {
        self.0
            .checked_sub(earlier.0)
            .map(|nanos| Duration::from_millis(nanos / NANOS_PER_MILLI))
    }

    /// Converts from system time; `None` outside the supported range.
    #[must_use]
    pub fn from_system_time(time: std::time::SystemTime) -> Option<Self> {
        match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(after) => i64::try_from(after.as_nanos()).ok().map(Self),
            Err(before) => i64::try_from(before.duration().as_nanos())
                .ok()
                .map(|nanos| Self(-nanos)),
        }
    }

    /// Converts to system time; `None` when the system cannot represent it.
    #[must_use]
    pub fn to_system_time(self) -> Option<std::time::SystemTime> {
        let magnitude = std::time::Duration::from_nanos(self.0.unsigned_abs());
        if self.0 >= 0 {
            std::time::UNIX_EPOCH.checked_add(magnitude)
        } else {
            std::time::UNIX_EPOCH.checked_sub(magnitude)
        }
    }
}

impl fmt::Display for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seconds = self.0.div_euclid(NANOS_PER_SECOND);
        let nanos = self.0.rem_euclid(NANOS_PER_SECOND);
        let (year, month, day) = civil_from_days(seconds.div_euclid(SECONDS_PER_DAY));
        let second_of_day = seconds.rem_euclid(SECONDS_PER_DAY);
        write!(
            f,
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
            second_of_day / SECONDS_PER_HOUR,
            second_of_day / SECONDS_PER_MINUTE % MINUTES_PER_HOUR,
            second_of_day % SECONDS_PER_MINUTE
        )?;
        if nanos != 0 && nanos % NANOS_PER_MILLI == 0 {
            write!(f, ".{:03}", nanos / NANOS_PER_MILLI)?;
        } else if nanos != 0 && nanos % NANOS_PER_MICRO == 0 {
            write!(f, ".{:06}", nanos / NANOS_PER_MICRO)?;
        } else if nanos != 0 {
            write!(f, ".{nanos:09}")?;
        }
        f.write_str("Z")
    }
}

impl FromStr for Instant {
    type Err = ValueError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_rfc3339(text).map(Self).ok_or_else(|| {
            ValueError::new(
                "instant",
                text,
                "expected RFC 3339 date and time, for example 2026-11-21T10:00:00Z",
            )
        })
    }
}

impl Serialize for Instant {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Instant {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(de::Error::custom)
    }
}

impl JsonSchema for Instant {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Instant")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Point in time in UTC, RFC 3339 with the suffix `Z`.",
            "type": "string",
            "pattern": INSTANT_PATTERN
        })
    }
}

const DAYS_PER_YEAR: i64 = 365;
const DAYS_PER_FOUR_YEARS: i64 = 1_461;
const DAYS_PER_CENTURY: i64 = 36_524;
const YEARS_PER_ERA: i64 = 400;
const YEAR_DIGITS: usize = 4;
const FIELD_DIGITS: usize = 2;

/// Converts a day count since 1970-01-01 to a civil date of the proleptic
/// Gregorian calendar.
///
/// The computation is the algorithm `civil_from_days` of H. Hinnant,
/// "chrono-Compatible Low-Level Date Algorithms": years start on 1 March, so
/// that the leap day ends a year, and the first day of the month `m`
/// counted from March is day `(153 * m + 2) / 5` of that year.
const fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + DAYS_TO_UNIX_EPOCH;
    let era = shifted.div_euclid(DAYS_PER_ERA);
    let day_of_era = shifted.rem_euclid(DAYS_PER_ERA);
    let year_of_era = (day_of_era - day_of_era / (DAYS_PER_FOUR_YEARS - 1)
        + day_of_era / DAYS_PER_CENTURY
        - day_of_era / (DAYS_PER_ERA - 1))
        / DAYS_PER_YEAR;
    let day_of_year =
        day_of_era - (DAYS_PER_YEAR * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * YEARS_PER_ERA + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// Converts a civil date to a day count since 1970-01-01; the inverse of
/// [`civil_from_days`] by the same algorithm.
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(YEARS_PER_ERA);
    let year_of_era = year.rem_euclid(YEARS_PER_ERA);
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era =
        year_of_era * DAYS_PER_YEAR + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * DAYS_PER_ERA + day_of_era - DAYS_TO_UNIX_EPOCH
}

/// Whether the year is a leap year of the Gregorian calendar.
const fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % YEARS_PER_ERA == 0
}

const fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Reader of the fixed-width parts of an RFC 3339 text.
struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn number(&mut self, width: usize) -> Option<i64> {
        let end = self.position.checked_add(width)?;
        let digits = self.bytes.get(self.position..end)?;
        let value = digits.iter().try_fold(0_i64, |value, &byte| {
            byte.is_ascii_digit()
                .then(|| value * 10 + i64::from(byte - b'0'))
        })?;
        self.position = end;
        Some(value)
    }

    fn literal(&mut self, accepted: &[u8]) -> Option<()> {
        let byte = *self.bytes.get(self.position)?;
        accepted.contains(&byte).then(|| self.position += 1)
    }

    fn fraction(&mut self) -> Option<i64> {
        if self.bytes.get(self.position) != Some(&b'.') {
            return Some(0);
        }
        self.position += 1;
        let start = self.position;
        let mut nanos = 0_i64;
        while let Some(&byte) = self
            .bytes
            .get(self.position)
            .filter(|byte| byte.is_ascii_digit())
        {
            nanos = nanos * 10 + i64::from(byte - b'0');
            self.position += 1;
        }
        let width = self.position - start;
        let width = u32::try_from(width)
            .ok()
            .filter(|width| (1..=FRACTION_DIGITS).contains(width))?;
        Some(nanos * 10_i64.pow(FRACTION_DIGITS - width))
    }

    fn offset_seconds(&mut self) -> Option<i64> {
        let sign = match self.bytes.get(self.position)? {
            b'Z' | b'z' => {
                self.position += 1;
                return Some(0);
            }
            b'+' => 1,
            b'-' => -1,
            _ => return None,
        };
        self.position += 1;
        let hours = self.number(FIELD_DIGITS)?;
        self.literal(b":")?;
        let minutes = self.number(FIELD_DIGITS)?;
        (hours < HOURS_PER_DAY && minutes < MINUTES_PER_HOUR)
            .then_some(sign * (hours * SECONDS_PER_HOUR + minutes * SECONDS_PER_MINUTE))
    }

    fn date(&mut self) -> Option<i64> {
        let year = self.number(YEAR_DIGITS)?;
        self.literal(b"-")?;
        let month = self.number(FIELD_DIGITS)?;
        self.literal(b"-")?;
        let day = self.number(FIELD_DIGITS)?;
        let valid =
            (1..=MONTHS_PER_YEAR).contains(&month) && day >= 1 && day <= days_in_month(year, month);
        valid.then(|| days_from_civil(year, month, day))
    }

    fn time_of_day(&mut self) -> Option<i64> {
        let hour = self.number(FIELD_DIGITS)?;
        self.literal(b":")?;
        let minute = self.number(FIELD_DIGITS)?;
        self.literal(b":")?;
        let second = self.number(FIELD_DIGITS)?;
        let valid =
            hour < HOURS_PER_DAY && minute < MINUTES_PER_HOUR && second < SECONDS_PER_MINUTE;
        valid.then_some(hour * SECONDS_PER_HOUR + minute * SECONDS_PER_MINUTE + second)
    }
}

/// Parses RFC 3339 text to nanoseconds since the Unix epoch.
fn parse_rfc3339(text: &str) -> Option<i64> {
    let mut cursor = Cursor {
        bytes: text.as_bytes(),
        position: 0,
    };
    let days = cursor.date()?;
    cursor.literal(b"Tt")?;
    let second_of_day = cursor.time_of_day()?;
    let nanos = cursor.fraction()?;
    let offset = cursor.offset_seconds()?;
    if cursor.position != cursor.bytes.len() {
        return None;
    }
    days.checked_mul(SECONDS_PER_DAY)?
        .checked_add(second_of_day)?
        .checked_sub(offset)?
        .checked_mul(NANOS_PER_SECOND)?
        .checked_add(nanos)
}

/// Form of a date or time read from a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DateKind {
    /// Date and time with an offset, for example `2026-11-21T10:00:00+01:00`.
    #[default]
    OffsetDateTime,
    /// Date and time without an offset, for example `2026-11-21T10:00:00`.
    LocalDateTime,
    /// Date only, for example `2026-11-21`.
    LocalDate,
    /// Time of day only, for example `10:00:00`.
    LocalTime,
}

/// Date or time of day as written in a structured document (TOML, YAML,
/// property lists), kept in its source form so that writing it back does
/// not change it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Date {
    /// Form of the date.
    pub kind: DateKind,
    /// Text exactly as in the document.
    pub text: String,
}

/// Inclusive range of integers: `5`, `1..5`, `40..`, `..9`, or every
/// integer.
///
/// Both bounds are inclusive; a missing bound is open. The start is never
/// greater than the end.
///
/// ```
/// use nurt_contracts::value::Range;
///
/// let range = Range::new(Some(1), Some(5)).unwrap();
/// assert!(range.contains(5));
/// assert_eq!(range.to_string(), "1..5");
/// assert_eq!(Range::from_start(40).to_string(), "40..");
/// assert!(Range::new(Some(5), Some(1)).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "RangeBounds", deny_unknown_fields)]
pub struct Range {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end: Option<i64>,
}

/// Unchecked bounds of a range, as read from JSON.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "Range")]
struct RangeBounds {
    #[serde(default)]
    start: Option<i64>,
    #[serde(default)]
    end: Option<i64>,
}

impl TryFrom<RangeBounds> for Range {
    type Error = ValueError;

    fn try_from(bounds: RangeBounds) -> Result<Self, Self::Error> {
        Self::new(bounds.start, bounds.end)
    }
}

impl Range {
    /// Range with the given bounds.
    ///
    /// # Errors
    ///
    /// Returns a [`ValueError`] when `start` is greater than `end`.
    pub fn new(start: Option<i64>, end: Option<i64>) -> Result<Self, ValueError> {
        match (start, end) {
            (Some(low), Some(high)) if low > high => Err(ValueError::new(
                "range",
                &format!("{low}..{high}"),
                "the start is greater than the end",
            )),
            _ => Ok(Self { start, end }),
        }
    }

    /// Range of a single integer.
    #[must_use]
    pub const fn single(value: i64) -> Self {
        Self {
            start: Some(value),
            end: Some(value),
        }
    }

    /// Range from `start` without an upper bound.
    #[must_use]
    pub const fn from_start(start: i64) -> Self {
        Self {
            start: Some(start),
            end: None,
        }
    }

    /// Range without a lower bound up to `end`.
    #[must_use]
    pub const fn up_to(end: i64) -> Self {
        Self {
            start: None,
            end: Some(end),
        }
    }

    /// Range of every integer.
    #[must_use]
    pub const fn full() -> Self {
        Self {
            start: None,
            end: None,
        }
    }

    /// Lower bound, inclusive.
    #[must_use]
    pub const fn start(self) -> Option<i64> {
        self.start
    }

    /// Upper bound, inclusive.
    #[must_use]
    pub const fn end(self) -> Option<i64> {
        self.end
    }

    /// Whether the integer lies in the range.
    #[must_use]
    pub fn contains(self, value: i64) -> bool {
        self.start.is_none_or(|start| value >= start) && self.end.is_none_or(|end| value <= end)
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.start, self.end) {
            (Some(start), Some(end)) if start == end => write!(f, "{start}"),
            (Some(start), Some(end)) => write!(f, "{start}..{end}"),
            (Some(start), None) => write!(f, "{start}.."),
            (None, Some(end)) => write!(f, "..{end}"),
            (None, None) => f.write_str(".."),
        }
    }
}

/// Flags of a regular expression.
///
/// The JSON form is a text of the flag letters in the order `imsx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RegexFlags {
    /// `i`: letters match regardless of case.
    pub case_insensitive: bool,
    /// `m`: `^` and `$` match at line boundaries.
    pub multi_line: bool,
    /// `s`: `.` also matches a line break.
    pub dot_matches_new_line: bool,
    /// `x`: white space and `#` comments in the pattern are ignored.
    pub extended: bool,
}

impl RegexFlags {
    /// Whether no flag is set.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        !(self.case_insensitive || self.multi_line || self.dot_matches_new_line || self.extended)
    }
}

impl fmt::Display for RegexFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let flags = [
            (self.case_insensitive, 'i'),
            (self.multi_line, 'm'),
            (self.dot_matches_new_line, 's'),
            (self.extended, 'x'),
        ];
        for (set, letter) in flags {
            if set {
                fmt::Write::write_char(f, letter)?;
            }
        }
        Ok(())
    }
}

impl FromStr for RegexFlags {
    type Err = ValueError;

    /// Parses flag letters in any order; a letter may appear only once.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut flags = Self::default();
        for letter in text.chars() {
            let flag = match letter {
                'i' => &mut flags.case_insensitive,
                'm' => &mut flags.multi_line,
                's' => &mut flags.dot_matches_new_line,
                'x' => &mut flags.extended,
                _ => {
                    return Err(ValueError::new(
                        "regex flags",
                        text,
                        "known flags are i, m, s, x",
                    ));
                }
            };
            if *flag {
                return Err(ValueError::new("regex flags", text, "a flag is repeated"));
            }
            *flag = true;
        }
        Ok(flags)
    }
}

impl Serialize for RegexFlags {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for RegexFlags {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(de::Error::custom)
    }
}

impl JsonSchema for RegexFlags {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("RegexFlags")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Flag letters in the order `imsx`.",
            "type": "string",
            "pattern": "^i?m?s?x?$"
        })
    }
}

/// Regular expression: pattern and flags.
///
/// The pattern is the source text of the expression without the enclosing
/// `/` and with `\/` already turned into `/`. The text form is
/// `/pattern/flags`.
///
/// ```
/// use nurt_contracts::value::Regex;
///
/// let regex = Regex::new("a/b+", "i").unwrap();
/// assert_eq!(regex.to_string(), "/a\\/b+/i");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Regex {
    /// Source text of the expression.
    pub pattern: String,
    /// Flags; omitted from JSON when empty.
    #[serde(default, skip_serializing_if = "RegexFlags::is_empty")]
    pub flags: RegexFlags,
}

impl Regex {
    /// Regular expression with the pattern and the flag letters.
    ///
    /// # Errors
    ///
    /// Returns a [`ValueError`] when a flag letter is unknown or repeated.
    pub fn new(pattern: &str, flags: &str) -> Result<Self, ValueError> {
        Ok(Self {
            pattern: pattern.to_owned(),
            flags: flags.parse()?,
        })
    }
}

impl fmt::Display for Regex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("/")?;
        let mut escaped = false;
        for character in self.pattern.chars() {
            if character == '/' && !escaped {
                f.write_str("\\")?;
            }
            escaped = character == '\\' && !escaped;
            fmt::Write::write_char(f, character)?;
        }
        write!(f, "/{}", self.flags)
    }
}

/// Binary data.
///
/// The JSON form is standard base64 with padding (RFC 4648).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Bytes(Vec<u8>);

impl Bytes {
    /// Wraps the bytes.
    #[must_use]
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Unwraps the bytes.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }

    /// Standard base64 text with padding.
    ///
    /// ```
    /// use nurt_contracts::value::Bytes;
    ///
    /// let bytes = Bytes::new(b"nurt".to_vec());
    /// assert_eq!(bytes.to_base64(), "bnVydA==");
    /// assert_eq!(Bytes::from_base64("bnVydA=="), Ok(bytes));
    /// ```
    #[must_use]
    pub fn to_base64(&self) -> String {
        let groups = self.0.len().div_ceil(BASE64_GROUP_BYTES);
        let mut text = String::with_capacity(groups * BASE64_GROUP_DIGITS);
        for chunk in self.0.chunks(BASE64_GROUP_BYTES) {
            let group = (0..BASE64_GROUP_BYTES).fold(0_u32, |group, index| {
                (group << BITS_PER_BYTE) | u32::from(chunk.get(index).copied().unwrap_or(0))
            });
            // A chunk of n bytes is written as n + 1 digits followed by padding.
            let significant = chunk.len() + 1;
            for index in 0..BASE64_GROUP_DIGITS {
                let shift = (BASE64_GROUP_DIGITS - 1 - index) as u32 * BASE64_DIGIT_BITS;
                text.push(if index < significant {
                    base64_digit(group >> shift)
                } else {
                    char::from(BASE64_PADDING)
                });
            }
        }
        text
    }

    /// Decodes standard base64 text with padding.
    ///
    /// # Errors
    ///
    /// Returns a [`ValueError`] when the text is not standard base64 with
    /// padding.
    pub fn from_base64(text: &str) -> Result<Self, ValueError> {
        decode_base64(text.as_bytes())
            .map(Self)
            .ok_or_else(|| ValueError::new("bytes", text, "expected standard base64 with padding"))
    }
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BASE64_PADDING: u8 = b'=';
const BASE64_GROUP_BYTES: usize = 3;
const BASE64_GROUP_DIGITS: usize = 4;
const BASE64_DIGIT_BITS: u32 = 6;
const BASE64_DIGIT_MASK: u32 = (1 << BASE64_DIGIT_BITS) - 1;
const BITS_PER_BYTE: u32 = 8;

fn base64_digit(group: u32) -> char {
    BASE64_ALPHABET
        .get((group & BASE64_DIGIT_MASK) as usize)
        .map_or(char::from(BASE64_PADDING), |&byte| char::from(byte))
}

fn base64_value(byte: u8) -> Option<u32> {
    BASE64_ALPHABET
        .iter()
        .position(|&digit| digit == byte)
        .and_then(|value| u32::try_from(value).ok())
}

fn decode_base64(text: &[u8]) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(BASE64_GROUP_DIGITS) {
        return None;
    }
    let groups = text.len() / BASE64_GROUP_DIGITS;
    let mut bytes = Vec::with_capacity(groups * BASE64_GROUP_BYTES);
    for (index, chunk) in text.chunks(BASE64_GROUP_DIGITS).enumerate() {
        let padding = chunk
            .iter()
            .rev()
            .take_while(|&&byte| byte == BASE64_PADDING)
            .count();
        // Only the last group may end with one or two padding digits.
        if padding >= BASE64_GROUP_BYTES || (padding > 0 && index + 1 != groups) {
            return None;
        }
        let mut group = 0_u32;
        for &byte in chunk.get(..BASE64_GROUP_DIGITS - padding)? {
            group = (group << BASE64_DIGIT_BITS) | base64_value(byte)?;
        }
        group <<= BASE64_DIGIT_BITS * padding as u32;
        let decoded = group.to_be_bytes();
        bytes.extend_from_slice(decoded.get(1..=BASE64_GROUP_BYTES - padding)?);
    }
    Some(bytes)
}

impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl From<&[u8]> for Bytes {
    fn from(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_base64())
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_base64(&text).map_err(de::Error::custom)
    }
}

impl JsonSchema for Bytes {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Bytes")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Binary data, standard base64 with padding.",
            "type": "string",
            "contentEncoding": "base64",
            "pattern": "^([A-Za-z0-9+/]{4})*([A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$"
        })
    }
}

/// Path in canonical form, used for the path fields of typed records.
///
/// Canonical form: `/` as separator, absolute, upper-case drive letter
/// (`C:/Users/user`), UNC paths as `//server/share`, provider paths with a
/// prefix (`registry:/HKCU/Software`, `env:/PATH`,
/// `archive:/home/user/a.zip!/dir/file`). Normalisation is done by the
/// language implementation; this type only carries the text.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Path(String);

impl Path {
    /// Wraps the canonical text of a path.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// The text of the path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Unwraps the text of the path.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<String> for Path {
    fn from(path: String) -> Self {
        Self(path)
    }
}

impl From<&str> for Path {
    fn from(path: &str) -> Self {
        Self(path.to_owned())
    }
}

impl AsRef<str> for Path {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
