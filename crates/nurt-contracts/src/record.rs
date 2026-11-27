// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Typed records that flow through pipelines.
//!
//! Every record kind of [`RecordKind`] except [`RecordKind::Map`] has a
//! structure here with its fields in a fixed order. A field of type
//! `Option<T>` is filled only when it makes sense or was computed; every
//! other field is always present.
//!
//! A typed record converts to the dynamic [`Record`] with
//! `Record::from(entry)` and back with `Entry::try_from(record)`; the
//! conversion keeps the field order and the language types of the fields
//! (a size stays a size, a path stays a path). [`TypedRecord`] holds a typed
//! record of any kind.
//!
//! # JSON form
//!
//! The JSON form of a typed record is a plain object: sizes in bytes,
//! durations in milliseconds, instants as RFC 3339 text in UTC, paths as
//! text, enumeration values as text, fields of type `any` in the tagged form
//! of [`Value`]. Absent fields are left out. A [`TypedRecord`] is written as
//! `{"kind": "entry", "record": {…}}`.
//!
//! ```
//! use nurt_contracts::record::{Entry, EntryType};
//! use nurt_contracts::value::{Instant, Path, Record, RecordKind, Size, Value};
//!
//! let entry = Entry {
//!     path: Path::from("/home/user/notes.txt"),
//!     name: "notes.txt".to_owned(),
//!     kind: EntryType::File,
//!     size: Size::from_bytes(1200),
//!     modified: "2026-11-21T09:12:00Z".parse::<Instant>().unwrap(),
//!     ..Entry::default()
//! };
//! let json = serde_json::to_value(&entry).unwrap();
//! assert_eq!(json["type"], "file");
//! assert_eq!(json["size"], 1200);
//!
//! let record = Record::from(entry.clone());
//! assert_eq!(record.kind, RecordKind::Entry);
//! assert_eq!(record.get("size"), Some(&Value::Size(Size::from_bytes(1200))));
//! assert_eq!(Entry::try_from(record).unwrap(), entry);
//! ```

use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorCode};
use crate::report::{Irreversible, IrreversibleEntry};
use crate::value::{
    Bytes, Duration, Fields, Instant, Path, Range, Record, RecordKind, Size, Value, ValueError,
};

/// Error of a conversion between a dynamic [`Record`] and a typed record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "problem", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecordError {
    /// The record has another kind than the typed record.
    KindMismatch {
        /// Kind of the typed record.
        expected: RecordKind,
        /// Kind of the dynamic record.
        found: RecordKind,
    },
    /// The kind has no typed structure (`map`).
    Untyped {
        /// Kind of the dynamic record.
        kind: RecordKind,
    },
    /// A required field is missing.
    MissingField {
        /// Field path, for example `files.0.path`.
        field: String,
    },
    /// The record has a field that the kind does not define.
    UnknownField {
        /// Field path.
        field: String,
    },
    /// A field has a value of another type.
    FieldType {
        /// Field path.
        field: String,
        /// Expected type.
        expected: String,
        /// Type found.
        found: String,
    },
}

impl RecordError {
    /// Prefixes the field path with the name of the enclosing field.
    fn within(self, name: &str) -> Self {
        let join = |field: String| {
            if field.is_empty() {
                name.to_owned()
            } else {
                format!("{name}.{field}")
            }
        };
        match self {
            Self::MissingField { field } => Self::MissingField { field: join(field) },
            Self::UnknownField { field } => Self::UnknownField { field: join(field) },
            Self::FieldType {
                field,
                expected,
                found,
            } => Self::FieldType {
                field: join(field),
                expected,
                found,
            },
            other @ (Self::KindMismatch { .. } | Self::Untyped { .. }) => other,
        }
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KindMismatch { expected, found } => {
                write!(f, "expected a `{expected}` record, found `{found}`")
            }
            Self::Untyped { kind } => write!(f, "records of kind `{kind}` have no typed form"),
            Self::MissingField { field } => write!(f, "missing field `{field}`"),
            Self::UnknownField { field } => write!(f, "unknown field `{field}`"),
            Self::FieldType {
                field,
                expected,
                found,
            } => write!(f, "field `{field}`: expected {expected}, found {found}"),
        }
    }
}

impl std::error::Error for RecordError {}

impl From<RecordError> for Error {
    fn from(error: RecordError) -> Self {
        let code = match error {
            RecordError::UnknownField { .. } => ErrorCode::UnknownField,
            _ => ErrorCode::Type,
        };
        let detail = serde_json::to_value(&error).unwrap_or_default();
        Self::new(code, error.to_string()).with_detail("record", detail)
    }
}

/// Names and language types of the fields of a typed record, in the order of
/// its structure.
pub type FieldTypes = &'static [(&'static str, &'static str)];

/// Conversion of a field of a typed record to and from a [`Value`].
///
/// Implemented for the field types of typed records: text, numbers,
/// truth values, units, paths, enumerations, nested objects and records,
/// and `Option`, `Vec` and `IndexMap` of those.
pub trait FieldValue: Sized {
    /// Name of the language type of the values of the field, as listed in
    /// the type names of values (`size`, `text`, `list`, …); `any` when the
    /// field holds a value of any type.
    const TYPE: &'static str = "any";

    /// The field as a value; `None` when the field is absent.
    fn to_field(self) -> Option<Value>;

    /// Reads the field from a value of a record, which is `None` when the
    /// field is absent.
    ///
    /// # Errors
    ///
    /// Returns [`RecordError::MissingField`] when a required field is absent
    /// and [`RecordError::FieldType`] when the value has another type; the
    /// field path is empty and is filled in by the enclosing record.
    fn from_field(value: Option<Value>) -> Result<Self, RecordError>;

    /// Whether the field is absent; absent fields are left out of JSON.
    fn is_absent(&self) -> bool {
        false
    }
}

fn missing() -> RecordError {
    RecordError::MissingField {
        field: String::new(),
    }
}

fn mismatch(expected: &str, found: &Value) -> RecordError {
    RecordError::FieldType {
        field: String::new(),
        expected: expected.to_owned(),
        found: found.type_name().to_owned(),
    }
}

fn present(value: Option<Value>) -> Result<Value, RecordError> {
    value.ok_or_else(missing)
}

fn put<T: FieldValue>(fields: &mut Fields, name: &str, value: T) {
    if let Some(value) = value.to_field() {
        fields.insert(name.to_owned(), value);
    }
}

fn take<T: FieldValue>(fields: &mut Fields, name: &str) -> Result<T, RecordError> {
    T::from_field(fields.shift_remove(name)).map_err(|error| error.within(name))
}

fn reject_unknown(fields: &Fields) -> Result<(), RecordError> {
    match fields.keys().next() {
        Some(name) => Err(RecordError::UnknownField {
            field: name.clone(),
        }),
        None => Ok(()),
    }
}

impl FieldValue for Value {
    fn to_field(self) -> Option<Value> {
        Some(self)
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        present(value)
    }
}

impl FieldValue for String {
    const TYPE: &'static str = "text";

    fn to_field(self) -> Option<Value> {
        Some(Value::Text(self))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::Text(text) | Value::Enum(text) | Value::Key(text) => Ok(text),
            Value::Path(text) | Value::Glob(text) => Ok(text),
            other => Err(mismatch("text", &other)),
        }
    }
}

impl FieldValue for bool {
    const TYPE: &'static str = "bool";

    fn to_field(self) -> Option<Value> {
        Some(Value::Bool(self))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::Bool(value) => Ok(value),
            other => Err(mismatch("bool", &other)),
        }
    }
}

impl FieldValue for i64 {
    const TYPE: &'static str = "int";

    fn to_field(self) -> Option<Value> {
        Some(Value::Int(self))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::Int(value) => Ok(value),
            other => Err(mismatch("int", &other)),
        }
    }
}

impl FieldValue for u64 {
    const TYPE: &'static str = "int";

    fn to_field(self) -> Option<Value> {
        Some(Value::from(self))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::Int(value) => {
                u64::try_from(value).map_err(|_| mismatch("count", &Value::Int(value)))
            }
            Value::BigInt(text) => text
                .parse()
                .map_err(|_| mismatch("count", &Value::BigInt(text.clone()))),
            other => Err(mismatch("count", &other)),
        }
    }
}

impl FieldValue for f64 {
    const TYPE: &'static str = "number";

    fn to_field(self) -> Option<Value> {
        Some(Value::Number(self))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::Number(value) => Ok(value),
            Value::Int(value) => Ok(value as f64),
            other => Err(mismatch("number", &other)),
        }
    }
}

impl FieldValue for Size {
    const TYPE: &'static str = "size";

    fn to_field(self) -> Option<Value> {
        Some(Value::Size(self))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::Size(size) => Ok(size),
            Value::Int(bytes) => u64::try_from(bytes)
                .map(Size::from_bytes)
                .map_err(|_| mismatch("size", &Value::Int(bytes))),
            other => Err(mismatch("size", &other)),
        }
    }
}

impl FieldValue for Duration {
    const TYPE: &'static str = "duration";

    fn to_field(self) -> Option<Value> {
        Some(Value::Duration(self))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::Duration(duration) => Ok(duration),
            Value::Int(millis) => Ok(Duration::from_millis(millis)),
            other => Err(mismatch("duration", &other)),
        }
    }
}

impl FieldValue for Instant {
    const TYPE: &'static str = "instant";

    fn to_field(self) -> Option<Value> {
        Some(Value::Instant(self))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::Instant(instant) => Ok(instant),
            Value::Text(text) => text
                .parse()
                .map_err(|_| mismatch("instant", &Value::Text(text))),
            other => Err(mismatch("instant", &other)),
        }
    }
}

impl FieldValue for Path {
    const TYPE: &'static str = "path";

    fn to_field(self) -> Option<Value> {
        Some(Value::Path(self.into_string()))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::Path(text) | Value::Text(text) => Ok(Path::from(text)),
            other => Err(mismatch("path", &other)),
        }
    }
}

impl FieldValue for Bytes {
    const TYPE: &'static str = "bytes";

    fn to_field(self) -> Option<Value> {
        Some(Value::Bytes(self))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::Bytes(bytes) => Ok(bytes),
            other => Err(mismatch("bytes", &other)),
        }
    }
}

impl FieldValue for Range {
    const TYPE: &'static str = "range";

    fn to_field(self) -> Option<Value> {
        Some(Value::Range(self))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::Range(range) => Ok(range),
            Value::Int(line) => Ok(Range::single(line)),
            other => Err(mismatch("range", &other)),
        }
    }
}

/// Implements [`FieldValue`] for a type of the effect report through its
/// plain JSON form, so that records and reports share one definition.
macro_rules! report_field {
    ($ty:ty, $name:literal) => {
        impl FieldValue for $ty {
            fn to_field(self) -> Option<Value> {
                serde_json::to_value(&self).ok().map(Value::from_plain_json)
            }

            fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
                let value = present(value)?;
                serde_json::from_value(value.to_plain_json()).map_err(|_| mismatch($name, &value))
            }
        }
    };
}

report_field!(Irreversible, "irreversible reason");
report_field!(IrreversibleEntry, "irreversible entry");

impl<T: FieldValue> FieldValue for Option<T> {
    const TYPE: &'static str = T::TYPE;

    fn to_field(self) -> Option<Value> {
        self.and_then(T::to_field)
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match value {
            None | Some(Value::Null) => Ok(None),
            Some(value) => T::from_field(Some(value)).map(Some),
        }
    }

    fn is_absent(&self) -> bool {
        self.is_none()
    }
}

impl<T: FieldValue> FieldValue for Vec<T> {
    const TYPE: &'static str = "list";

    fn to_field(self) -> Option<Value> {
        let items = self
            .into_iter()
            .map(|item| item.to_field().unwrap_or_default())
            .collect();
        Some(Value::List(items))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        match present(value)? {
            Value::List(items) => items
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    T::from_field(Some(item)).map_err(|error| error.within(&index.to_string()))
                })
                .collect(),
            other => Err(mismatch("list", &other)),
        }
    }
}

impl<T: FieldValue> FieldValue for IndexMap<String, T> {
    const TYPE: &'static str = "map";

    fn to_field(self) -> Option<Value> {
        let entries = self
            .into_iter()
            .map(|(key, item)| (key, item.to_field().unwrap_or_default()))
            .collect();
        Some(Value::Map(entries))
    }

    fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
        let entries = match present(value)? {
            Value::Map(entries) => entries,
            Value::Record(record) => record.fields,
            other => return Err(mismatch("map", &other)),
        };
        entries
            .into_iter()
            .map(|(key, item)| {
                let item = T::from_field(Some(item)).map_err(|error| error.within(&key))?;
                Ok((key, item))
            })
            .collect()
    }
}

/// Defines an enumeration of field values with stable names.
macro_rules! text_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $text:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema,
        )]
        #[non_exhaustive]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $text)] $variant, )+
        }

        impl $name {
            /// Every value in a stable order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// Stable name of the value in programs and in JSON.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ValueError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                match text {
                    $($text => Ok($name::$variant),)+
                    _ => Err(ValueError::new(stringify!($name), text, "unknown value")),
                }
            }
        }

        impl FieldValue for $name {
            const TYPE: &'static str = "enum";

            fn to_field(self) -> Option<Value> {
                Some(Value::Enum(self.as_str().to_owned()))
            }

            fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
                match present(value)? {
                    Value::Enum(text) | Value::Text(text) => {
                        text.parse().map_err(|_| mismatch(stringify!($name), &Value::Enum(text)))
                    }
                    other => Err(mismatch(stringify!($name), &other)),
                }
            }
        }
    };
}

/// Name of a field in JSON and in the dynamic record.
macro_rules! field_name {
    ($field:ident) => {
        stringify!($field)
    };
    ($field:ident $json:literal) => {
        $json
    };
}

/// Defines a structure of named fields with conversion to and from fields.
macro_rules! define_struct {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$fmeta:meta])* $field:ident $(as $json:literal)? : $ty:ty ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
        pub struct $name {
            $(
                $(#[$fmeta])*
                $(#[serde(rename = $json)])?
                #[serde(skip_serializing_if = "FieldValue::is_absent")]
                pub $field: $ty,
            )+
        }

        impl $name {
            /// Names of the fields in JSON and in the dynamic record, each
            /// with the name of its language type (`any` for a field of any
            /// type).
            pub const FIELD_TYPES: FieldTypes = &[
                $( (field_name!($field $($json)?), <$ty as FieldValue>::TYPE), )+
            ];

            fn into_fields(self) -> Fields {
                let mut fields = Fields::new();
                $( put(&mut fields, field_name!($field $($json)?), self.$field); )+
                fields
            }

            fn from_fields(mut fields: Fields) -> Result<Self, RecordError> {
                let value = Self {
                    $( $field: take(&mut fields, field_name!($field $($json)?))?, )+
                };
                reject_unknown(&fields)?;
                Ok(value)
            }
        }
    };
}

/// Defines a nested object of a record; its value is a map.
macro_rules! object_struct {
    ($(#[$meta:meta])* $name:ident $body:tt) => {
        define_struct! { $(#[$meta])* $name $body }

        impl FieldValue for $name {
            const TYPE: &'static str = "map";

            fn to_field(self) -> Option<Value> {
                Some(Value::Map(self.into_fields()))
            }

            fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
                match present(value)? {
                    Value::Map(fields) => Self::from_fields(fields),
                    other => Err(mismatch("map", &other)),
                }
            }
        }
    };
}

/// Defines the typed structure of a record kind.
macro_rules! record_struct {
    ($(#[$meta:meta])* $name:ident ($kind:ident) $body:tt) => {
        define_struct! { $(#[$meta])* $name $body }

        impl $name {
            /// Kind of this record.
            pub const KIND: RecordKind = RecordKind::$kind;
        }

        impl From<$name> for Record {
            fn from(record: $name) -> Self {
                Record {
                    kind: RecordKind::$kind,
                    fields: record.into_fields(),
                }
            }
        }

        impl TryFrom<Record> for $name {
            type Error = RecordError;

            fn try_from(record: Record) -> Result<Self, Self::Error> {
                if record.kind != RecordKind::$kind {
                    return Err(RecordError::KindMismatch {
                        expected: RecordKind::$kind,
                        found: record.kind,
                    });
                }
                Self::from_fields(record.fields)
            }
        }

        impl FieldValue for $name {
            fn to_field(self) -> Option<Value> {
                Some(Value::Record(Record::from(self)))
            }

            fn from_field(value: Option<Value>) -> Result<Self, RecordError> {
                match present(value)? {
                    Value::Record(record) => Self::try_from(record),
                    other => Err(mismatch(RecordKind::$kind.as_str(), &other)),
                }
            }
        }
    };
}

text_enum! {
    /// Type of a file system entry or of an archive member.
    EntryType {
        /// Regular file.
        #[default]
        File = "file",
        /// Directory; keys of the Windows registry are shown as directories.
        Directory = "directory",
        /// Symbolic link, junction or other link.
        Link = "link",
        /// Named pipe (FIFO).
        Fifo = "fifo",
        /// Character or block device node; `major` and `minor` hold its numbers.
        Device = "device",
        /// Socket or another special object.
        Other = "other",
    }
}

text_enum! {
    /// Kind of a link.
    LinkType {
        /// Symbolic link.
        #[default]
        Symbolic = "symbolic",
        /// Windows junction of a directory.
        Junction = "junction",
        /// Hard link.
        Hard = "hard",
    }
}

text_enum! {
    /// Line endings of a text.
    LineEnding {
        /// Line feed, `\n`.
        #[default]
        Lf = "lf",
        /// Carriage return and line feed, `\r\n`.
        Crlf = "crlf",
        /// Carriage return, `\r`.
        Cr = "cr",
        /// Several kinds of line endings.
        Mixed = "mixed",
        /// No line ending in the text.
        None = "none",
    }
}

text_enum! {
    /// Engine that evaluated a regular expression.
    RegexEngine {
        /// Linear-time engine.
        #[default]
        Linear = "linear",
        /// Backtracking engine with a step limit, used for look-around and
        /// back-references.
        Backtracking = "backtracking",
    }
}

text_enum! {
    /// Type of a value in a structured document.
    ValueType {
        /// Text.
        #[default]
        Text = "text",
        /// Integer.
        Int = "int",
        /// Number with a fraction or an exponent.
        Number = "number",
        /// Truth value.
        Bool = "bool",
        /// Null.
        Null = "null",
        /// Sequence.
        List = "list",
        /// Mapping.
        Map = "map",
        /// Date or time.
        Date = "date",
        /// Binary data.
        Bytes = "bytes",
    }
}

text_enum! {
    /// Kind of a symbol of source code.
    SymbolKind {
        /// Function.
        #[default]
        Function = "function",
        /// Method.
        Method = "method",
        /// Class.
        Class = "class",
        /// Structure.
        Struct = "struct",
        /// Interface or trait.
        Interface = "interface",
        /// Enumeration.
        Enum = "enum",
        /// Variable.
        Variable = "variable",
        /// Constant.
        Constant = "constant",
        /// Field or property.
        Field = "field",
        /// Module, package or namespace.
        Module = "module",
        /// Type alias or type definition.
        Type = "type",
        /// Macro.
        Macro = "macro",
        /// Any other symbol.
        Other = "other",
    }
}

text_enum! {
    /// Source of code intelligence.
    SymbolSource {
        /// Language server.
        #[default]
        Lsp = "lsp",
        /// Built-in syntax analysis.
        Syntax = "syntax",
    }
}

text_enum! {
    /// Role of a reference to a symbol.
    ReferenceRole {
        /// The definition.
        #[default]
        Definition = "definition",
        /// A use.
        Reference = "reference",
        /// An import.
        Import = "import",
        /// A use in a test.
        Test = "test",
        /// The role is not known.
        Unknown = "unknown",
    }
}

text_enum! {
    /// Severity of a diagnostic.
    DiagnosticSeverity {
        /// Error.
        #[default]
        Error = "error",
        /// Warning.
        Warning = "warning",
        /// Information.
        Info = "info",
        /// Hint.
        Hint = "hint",
    }
}

text_enum! {
    /// Type of a notebook cell.
    CellType {
        /// Code.
        #[default]
        Code = "code",
        /// Markdown text.
        Markdown = "markdown",
        /// Raw content.
        Raw = "raw",
    }
}

text_enum! {
    /// Status of a file in a difference.
    HunkStatus {
        /// Content changed.
        #[default]
        Modified = "modified",
        /// File added.
        Added = "added",
        /// File deleted.
        Deleted = "deleted",
        /// File renamed.
        Renamed = "renamed",
        /// Binary file changed.
        Binary = "binary",
        /// File copied from another file.
        Copied = "copied",
    }
}

text_enum! {
    /// Subject of a finding of a review.
    FindingTarget {
        /// Content of a file.
        #[default]
        File = "file",
        /// Message of a commit.
        Message = "message",
    }
}

text_enum! {
    /// Unit in which a difference compares content.
    DiffUnit {
        /// Lines.
        #[default]
        Lines = "lines",
        /// Words within lines.
        Words = "words",
    }
}

text_enum! {
    /// Action of a change of one file.
    ChangeAction {
        /// Created.
        #[default]
        Create = "create",
        /// Content modified.
        Modify = "modify",
        /// Deleted.
        Delete = "delete",
        /// Renamed or moved.
        Rename = "rename",
        /// Attributes changed.
        Attributes = "attributes",
        /// Link created.
        Link = "link",
        /// Truncated or extended.
        Truncate = "truncate",
        /// State of a target outside the file system changed, for example a
        /// service; `before` and `after` hold the states.
        State = "state",
    }
}

text_enum! {
    /// Outcome of a repository operation.
    GitEffectState {
        /// Completed.
        #[default]
        Completed = "completed",
        /// Stopped with conflicts to resolve.
        Conflicts = "conflicts",
        /// Stopped before completion, for example by a hook.
        Stopped = "stopped",
        /// The branch was moved forward without a merge.
        FastForward = "fast_forward",
        /// The repository was already up to date.
        UpToDate = "up_to_date",
        /// There was nothing to do.
        NothingToDo = "nothing_to_do",
    }
}

text_enum! {
    /// Operation left in progress in a repository.
    GitInProgress {
        /// No operation is in progress.
        #[default]
        None = "none",
        /// A merge is in progress.
        Merge = "merge",
        /// A rebase is in progress.
        Rebase = "rebase",
        /// A cherry-pick is in progress.
        CherryPick = "cherry_pick",
        /// A revert is in progress.
        Revert = "revert",
        /// A bisection is in progress.
        Bisect = "bisect",
    }
}

text_enum! {
    /// How a program was started.
    Launcher {
        /// Directly.
        #[default]
        Direct = "direct",
        /// Through the Windows command interpreter, for batch files.
        Cmd = "cmd",
    }
}

text_enum! {
    /// Signal that ended a process, with names common to all systems.
    Signal {
        /// Termination request.
        #[default]
        Terminate = "terminate",
        /// Forced termination.
        Kill = "kill",
        /// Interrupt.
        Interrupt = "interrupt",
        /// Hang-up.
        Hangup = "hangup",
        /// Quit.
        Quit = "quit",
        /// Abort.
        Abort = "abort",
        /// Segmentation fault.
        Segv = "segv",
        /// Bus error.
        Bus = "bus",
        /// Arithmetic error.
        Fpe = "fpe",
        /// Illegal instruction.
        Ill = "ill",
        /// Broken pipe.
        Pipe = "pipe",
        /// Any other signal.
        Other = "other",
    }
}

text_enum! {
    /// Who ended a process.
    KilledBy {
        /// The time limit.
        #[default]
        Timeout = "timeout",
        /// The idle limit.
        Idle = "idle",
        /// A resource limit.
        Limit = "limit",
        /// The command `stop`.
        Stop = "stop",
        /// Cancellation of the call.
        Cancel = "cancel",
    }
}

text_enum! {
    /// Resource limit that ended a process.
    LimitHit {
        /// Processor time.
        #[default]
        Cpu = "cpu",
        /// Memory.
        Memory = "memory",
    }
}

text_enum! {
    /// How file changes of a run are detected.
    Tracking {
        /// Not detected.
        #[default]
        None = "none",
        /// By comparing scans before and after.
        Scan = "scan",
        /// By file system events.
        Events = "events",
    }
}

text_enum! {
    /// Previous content kept before a run.
    Preimage {
        /// Nothing kept.
        #[default]
        None = "none",
        /// Clones of the files.
        Clone = "clone",
        /// Copies of the files.
        Copy = "copy",
        /// Part of the files, within the budget.
        Partial = "partial",
    }
}

text_enum! {
    /// State of a background job.
    JobState {
        /// Running.
        #[default]
        Running = "running",
        /// Suspended.
        Suspended = "suspended",
        /// Exited with code 0.
        Exited = "exited",
        /// Exited with another code.
        Failed = "failed",
        /// Stopped by `stop`.
        Stopped = "stopped",
        /// Ended by the time limit.
        TimedOut = "timed_out",
        /// Ended by the idle limit.
        Stalled = "stalled",
        /// Ended by a resource limit.
        Limit = "limit",
        /// The supervisor disappeared; the state of the process is not known.
        Lost = "lost",
    }
}

text_enum! {
    /// State of a process.
    ProcessState {
        /// Running.
        #[default]
        Running = "running",
        /// Sleeping.
        Sleeping = "sleeping",
        /// Stopped.
        Stopped = "stopped",
        /// Ended but not yet collected.
        Zombie = "zombie",
        /// Idle.
        Idle = "idle",
        /// Any other state.
        Other = "other",
    }
}

text_enum! {
    /// Transport protocol of a port.
    Protocol {
        /// TCP.
        #[default]
        Tcp = "tcp",
        /// UDP.
        Udp = "udp",
    }
}

text_enum! {
    /// State of a TCP connection; `none` for UDP.
    PortState {
        /// Listening.
        #[default]
        Listen = "listen",
        /// Established.
        Established = "established",
        /// Waiting after close.
        TimeWait = "time_wait",
        /// Waiting for the local close.
        CloseWait = "close_wait",
        /// Connection request sent.
        SynSent = "syn_sent",
        /// Connection request received.
        SynReceived = "syn_received",
        /// Closing after the local close.
        FinWait = "fin_wait",
        /// Both sides closing.
        Closing = "closing",
        /// Waiting for the last acknowledgement.
        LastAck = "last_ack",
        /// Closed.
        Closed = "closed",
        /// No connection state.
        None = "none",
    }
}

text_enum! {
    /// Address family.
    AddressFamily {
        /// IPv4.
        #[default]
        Ipv4 = "ipv4",
        /// IPv6.
        Ipv6 = "ipv6",
    }
}

text_enum! {
    /// Type of a disk.
    DiskType {
        /// Fixed disk.
        #[default]
        Fixed = "fixed",
        /// Removable disk.
        Removable = "removable",
        /// Network file system.
        Network = "network",
        /// File system in memory.
        Ram = "ram",
        /// Any other type.
        Other = "other",
    }
}

text_enum! {
    /// Operating system.
    OsKind {
        /// Linux.
        #[default]
        Linux = "linux",
        /// Apple macOS.
        Macos = "macos",
        /// Windows.
        Windows = "windows",
    }
}

text_enum! {
    /// State of the background service of the instance.
    ServiceAvailability {
        /// Running.
        #[default]
        Running = "running",
        /// Installed and stopped.
        Stopped = "stopped",
        /// Not installed.
        NotInstalled = "not_installed",
    }
}

text_enum! {
    /// Scope of an environment variable.
    EnvironmentScope {
        /// The process environment.
        #[default]
        Process = "process",
        /// The persistent environment of the user (Windows).
        User = "user",
        /// The persistent environment of the machine (Windows).
        Machine = "machine",
    }
}

text_enum! {
    /// State of a service.
    ServiceState {
        /// Running.
        #[default]
        Running = "running",
        /// Stopped.
        Stopped = "stopped",
        /// Starting.
        Starting = "starting",
        /// Stopping.
        Stopping = "stopping",
        /// Failed.
        Failed = "failed",
        /// Paused.
        Paused = "paused",
        /// The state is not known.
        Unknown = "unknown",
    }
}

text_enum! {
    /// Start type of a service.
    StartType {
        /// Started at boot.
        #[default]
        Automatic = "automatic",
        /// Started on request.
        Manual = "manual",
        /// Never started.
        Disabled = "disabled",
        /// Started by activation, such as a socket or a timer.
        OnDemand = "on_demand",
        /// Started only as a dependency.
        Static = "static",
        /// The start type is not known.
        Unknown = "unknown",
    }
}

text_enum! {
    /// Service manager.
    ServiceManager {
        /// The systemd manager.
        #[default]
        Systemd = "systemd",
        /// Windows service control manager.
        Windows = "windows",
        /// The launchd manager.
        Launchd = "launchd",
    }
}

text_enum! {
    /// Scope of a service.
    ServiceScope {
        /// The whole system.
        #[default]
        System = "system",
        /// One user.
        User = "user",
    }
}

text_enum! {
    /// Package manager or package database.
    PackageManager {
        /// APT.
        #[default]
        Apt = "apt",
        /// DNF.
        Dnf = "dnf",
        /// The pacman manager.
        Pacman = "pacman",
        /// The zypper manager.
        Zypper = "zypper",
        /// Homebrew.
        Brew = "brew",
        /// The winget manager.
        Winget = "winget",
        /// Chocolatey.
        Choco = "choco",
        /// Scoop.
        Scoop = "scoop",
        /// The dpkg database read without a manager.
        Dpkg = "dpkg",
        /// The RPM database read without a manager.
        Rpm = "rpm",
        /// Any other manager.
        Other = "other",
    }
}

text_enum! {
    /// Kind of an account.
    AccountKind {
        /// User.
        #[default]
        User = "user",
        /// Group.
        Group = "group",
    }
}

text_enum! {
    /// Level of a log event, unified across systems.
    LogLevel {
        /// Critical, alert or emergency.
        Critical = "critical",
        /// Error.
        Error = "error",
        /// Warning.
        Warning = "warning",
        /// Notice or information.
        #[default]
        Info = "info",
        /// Debugging.
        Debug = "debug",
    }
}

text_enum! {
    /// State of a path in the index or the working tree of a repository.
    GitFileState {
        /// Not modified.
        #[default]
        Unmodified = "unmodified",
        /// Modified.
        Modified = "modified",
        /// Added.
        Added = "added",
        /// Deleted.
        Deleted = "deleted",
        /// Renamed.
        Renamed = "renamed",
        /// Copied.
        Copied = "copied",
        /// Type changed, for example from a file to a link.
        TypeChanged = "type_changed",
        /// Not merged.
        Unmerged = "unmerged",
        /// Not tracked.
        Untracked = "untracked",
        /// Ignored.
        Ignored = "ignored",
    }
}

text_enum! {
    /// Kind of an entry of the repository status.
    GitStatusKind {
        /// Tracked path.
        #[default]
        Tracked = "tracked",
        /// Untracked path.
        Untracked = "untracked",
        /// Ignored path.
        Ignored = "ignored",
        /// Path with a conflict.
        Conflict = "conflict",
    }
}

text_enum! {
    /// Kind of a merge conflict.
    GitConflictKind {
        /// Both sides modified.
        #[default]
        BothModified = "both_modified",
        /// Both sides added.
        BothAdded = "both_added",
        /// Both sides deleted.
        BothDeleted = "both_deleted",
        /// Added by the current side.
        AddedByUs = "added_by_us",
        /// Added by the other side.
        AddedByThem = "added_by_them",
        /// Deleted by the current side.
        DeletedByUs = "deleted_by_us",
        /// Deleted by the other side.
        DeletedByThem = "deleted_by_them",
        /// Renamed differently on both sides.
        RenameRename = "rename_rename",
        /// Renamed on one side and deleted on the other.
        RenameDelete = "rename_delete",
        /// A directory on one side and a file on the other.
        DirectoryFile = "directory_file",
        /// Binary content.
        Binary = "binary",
        /// Submodule.
        Submodule = "submodule",
    }
}

text_enum! {
    /// Verification status of a signature.
    SignatureStatus {
        /// Not signed.
        #[default]
        None = "none",
        /// Valid signature.
        Good = "good",
        /// Invalid signature.
        Bad = "bad",
        /// The key is not known.
        UnknownKey = "unknown_key",
        /// The format is not supported.
        Unsupported = "unsupported",
    }
}

text_enum! {
    /// Format of a signature.
    SignatureFormat {
        /// OpenPGP.
        #[default]
        Openpgp = "openpgp",
        /// SSH.
        Ssh = "ssh",
        /// X.509.
        X509 = "x509",
    }
}

text_enum! {
    /// State of a scheduled task.
    ScheduleState {
        /// Active.
        #[default]
        Active = "active",
        /// Paused.
        Paused = "paused",
        /// All runs done.
        Completed = "completed",
        /// Past its expiry time.
        Expired = "expired",
    }
}

text_enum! {
    /// Kind of a schedule trigger.
    TriggerKind {
        /// Cron expression.
        #[default]
        Cron = "cron",
        /// Fixed interval.
        Every = "every",
        /// Single point in time.
        At = "at",
    }
}

text_enum! {
    /// Outcome of a run of a scheduled task.
    ScheduleRunState {
        /// Succeeded.
        #[default]
        Ok = "ok",
        /// Failed.
        Failed = "failed",
        /// Ended by the time limit.
        TimedOut = "timed_out",
        /// Skipped because the previous run was still running.
        SkippedOverlap = "skipped_overlap",
        /// Missed while the service was not running.
        Missed = "missed",
        /// Cancelled.
        Cancelled = "cancelled",
    }
}

text_enum! {
    /// State of an operation of the journal as shown by `history`.
    OperationState {
        /// Can be undone.
        #[default]
        Available = "available",
        /// Can be undone for the remaining items only.
        Partial = "partial",
        /// Can no longer be undone.
        Expired = "expired",
        /// Has been undone.
        Undone = "undone",
    }
}

text_enum! {
    /// Scope of an undo entry.
    UndoScope {
        /// One operation.
        #[default]
        Op = "op",
        /// Operations back to a point.
        To = "to",
        /// The history of one file.
        File = "file",
        /// Selected paths of an operation.
        Paths = "paths",
    }
}

text_enum! {
    /// Source of a program card.
    ProgramSource {
        /// The search path.
        #[default]
        Path = "path",
        /// A package database.
        Package = "package",
        /// An installed application.
        App = "app",
        /// A program catalog.
        Catalog = "catalog",
    }
}

text_enum! {
    /// Test framework.
    TestFramework {
        /// Bun.
        #[default]
        Bun = "bun",
        /// Vitest.
        Vitest = "vitest",
        /// Jest.
        Jest = "jest",
        /// The pytest framework.
        Pytest = "pytest",
        /// Go tests.
        Go = "go",
        /// Cargo tests.
        Cargo = "cargo",
        /// Tests run by the `test` script of `package.json` through npm.
        Npm = "npm",
        /// A command given by the user.
        Custom = "custom",
    }
}

text_enum! {
    /// Outcome of a test run.
    TestRunStatus {
        /// All tests passed.
        #[default]
        Passed = "passed",
        /// Some tests failed.
        Failed = "failed",
        /// The framework did not start or the code did not compile.
        Error = "error",
        /// Ended by the time limit.
        TimedOut = "timed_out",
        /// No tests were found.
        NoTests = "no_tests",
    }
}

text_enum! {
    /// Outcome of one test.
    TestStatus {
        /// Passed.
        #[default]
        Passed = "passed",
        /// Failed.
        Failed = "failed",
        /// Skipped.
        Skipped = "skipped",
        /// Could not run.
        Error = "error",
        /// Marked as not yet written.
        Todo = "todo",
    }
}

text_enum! {
    /// Format of a coverage report.
    CoverageFormat {
        /// LCOV.
        #[default]
        Lcov = "lcov",
        /// Cobertura XML.
        Cobertura = "cobertura",
        /// Istanbul JSON.
        Istanbul = "istanbul",
        /// Go cover profile.
        Go = "go",
        /// LLVM coverage export.
        Llvm = "llvm",
    }
}

text_enum! {
    /// Kind of a finding.
    FindingKind {
        /// Type checking.
        Types = "types",
        /// Linting.
        #[default]
        Lint = "lint",
        /// Formatting.
        Format = "format",
        /// Complexity.
        Complexity = "complexity",
        /// Duplicated code.
        Duplication = "duplication",
        /// Leftover conflict markers.
        ConflictMarkers = "conflict_markers",
        /// White space errors.
        Whitespace = "whitespace",
        /// Large files.
        LargeFiles = "large_files",
        /// Binary files.
        Binary = "binary",
        /// Secrets in content.
        Secrets = "secrets",
        /// Commit subject rules.
        Subject = "subject",
        /// Any other kind.
        Other = "other",
    }
}

text_enum! {
    /// Severity of a finding.
    FindingSeverity {
        /// Error.
        #[default]
        Error = "error",
        /// Warning.
        Warning = "warning",
        /// Information; hints and notes of tools map here.
        Info = "info",
    }
}

text_enum! {
    /// Outcome of one check of an analysis.
    CheckStatus {
        /// Passed.
        #[default]
        Ok = "ok",
        /// Failed.
        Failed = "failed",
        /// Skipped.
        Skipped = "skipped",
        /// The tool is missing.
        Missing = "missing",
    }
}

text_enum! {
    /// Check performed by an analysis tool.
    AnalyzeCheck {
        /// Type checking.
        Types = "types",
        /// Linting.
        #[default]
        Lint = "lint",
        /// Formatting.
        Format = "format",
    }
}

text_enum! {
    /// Type of a value in the Windows registry.
    RegistryType {
        /// `REG_SZ`.
        #[default]
        String = "string",
        /// `REG_EXPAND_SZ`.
        ExpandString = "expand_string",
        /// `REG_MULTI_SZ`.
        MultiString = "multi_string",
        /// `REG_DWORD`.
        Dword = "dword",
        /// `REG_QWORD`.
        Qword = "qword",
        /// `REG_BINARY`.
        Binary = "binary",
        /// `REG_NONE`.
        None = "none",
        /// Any other type.
        Other = "other",
    }
}

text_enum! {
    /// View of the Windows registry.
    RegistryView {
        /// The 64-bit view.
        #[default]
        Bits64 = "64",
        /// The 32-bit view.
        Bits32 = "32",
    }
}

text_enum! {
    /// Kind of a network interface.
    InterfaceKind {
        /// Wired Ethernet.
        #[default]
        Ethernet = "ethernet",
        /// Wireless.
        Wireless = "wireless",
        /// Loopback.
        Loopback = "loopback",
        /// Virtual interface.
        Virtual = "virtual",
        /// Any other kind.
        Other = "other",
    }
}

text_enum! {
    /// Kind of the text changed by a rename.
    RenameEditKind {
        /// Code.
        #[default]
        Code = "code",
        /// Comment.
        Comment = "comment",
        /// Text literal.
        String = "string",
        /// Import.
        Import = "import",
    }
}

text_enum! {
    /// Section of a repository profile.
    ProfileSection {
        /// Build and test commands.
        #[default]
        Commands = "commands",
        /// Entry points.
        EntryPoints = "entry_points",
        /// Files changed most often.
        Hotspots = "hotspots",
        /// Manifests.
        Manifests = "manifests",
    }
}

text_enum! {
    /// Kind of a tag.
    GitTagKind {
        /// Reference without an object.
        #[default]
        Lightweight = "lightweight",
        /// Tag object with a message.
        Annotated = "annotated",
    }
}

text_enum! {
    /// Kind of a preserved copy of a deleted reference.
    GitTrashKind {
        /// Deleted branch.
        #[default]
        Branch = "branch",
        /// Dropped stash entry.
        Stash = "stash",
    }
}

text_enum! {
    /// Outcome of an update of a reference.
    GitRefStatus {
        /// Created.
        #[default]
        New = "new",
        /// Updated by a fast-forward.
        Updated = "updated",
        /// Updated by force.
        Forced = "forced",
        /// Deleted.
        Deleted = "deleted",
        /// Already up to date.
        UpToDate = "up_to_date",
        /// Rejected.
        Rejected = "rejected",
        /// Failed.
        Error = "error",
    }
}

text_enum! {
    /// Status of an effect.
    EffectStatus {
        /// The effect was applied.
        #[default]
        Ok = "ok",
        /// The effect was planned only (preview).
        Planned = "planned",
        /// The effect continues in the background.
        Running = "running",
        /// The effect failed.
        Failed = "failed",
        /// The effect was rolled back.
        RolledBack = "rolled_back",
    }
}

object_struct! {
    /// Block of lines moved within a difference.
    MovedBlock {
        /// File the lines were removed from.
        from_path: Path,
        /// First removed line, from 1.
        from_line: u64,
        /// File the lines were added to.
        to_path: Path,
        /// First added line, from 1.
        to_line: u64,
        /// Number of lines.
        lines: u64,
    }
}

object_struct! {
    /// Image carried by a content record.
    ContentImage {
        /// Media type, for example `image/png`.
        mime: String,
        /// Width in pixels.
        width: Option<u64>,
        /// Height in pixels.
        height: Option<u64>,
        /// Image data.
        data: Option<Bytes>,
    }
}

object_struct! {
    /// Symbol enclosing a match.
    MatchContainer {
        /// Name of the symbol.
        name: String,
        /// Kind of the symbol.
        kind: SymbolKind,
        /// First line, from 1.
        line: u64,
        /// Last line.
        end_line: Option<u64>,
    }
}

object_struct! {
    /// Related position in a file.
    RelatedLocation {
        /// File.
        path: Path,
        /// Line, from 1.
        line: u64,
        /// Last line.
        end_line: Option<u64>,
        /// Explanation.
        message: Option<String>,
    }
}

object_struct! {
    /// How much of one output stream is shown.
    OutputShown {
        /// Lines shown from the start.
        head: u64,
        /// Lines shown from the end.
        tail: u64,
        /// Lines left out between them.
        omitted_lines: u64,
        /// Lines removed by output filters.
        filtered: u64,
    }
}

object_struct! {
    /// How much of the output of a run is shown.
    ExecShown {
        /// Standard output.
        stdout: Option<OutputShown>,
        /// Standard error.
        stderr: Option<OutputShown>,
    }
}

object_struct! {
    /// Error found in the output of a known tool; every field is one line.
    ExecDiagnostic {
        /// File named by the tool.
        file: Option<String>,
        /// Line, from 1.
        line: Option<u64>,
        /// Column, from 1.
        col: Option<u64>,
        /// Code of the error, for example `E0308` or `TS2322`.
        code: Option<String>,
        /// Message.
        message: String,
        /// Severity.
        severity: DiagnosticSeverity,
        /// Places the same error was reported at, when more than one.
        count: Option<u64>,
    }
}

object_struct! {
    /// Errors and warnings found in the output of a known tool.
    ExecDiagnostics {
        /// Tool whose format was recognised: `cargo`, `pytest`, `tsc`, `gcc`
        /// (gcc or clang) or `go`.
        tool: String,
        /// Number of errors in the output.
        errors: u64,
        /// Number of warnings in the output.
        warnings: u64,
        /// The first distinct errors, at most 10.
        first: Vec<ExecDiagnostic>,
    }
}

object_struct! {
    /// Progress of a run.
    Progress {
        /// Percentage, when the output shows one.
        percent: Option<f64>,
        /// Last progress line.
        line: String,
    }
}

object_struct! {
    /// One batch of a run over a record stream.
    Batch {
        /// Batch number, from 1.
        no: u64,
        /// Arguments of the batch.
        args: Vec<String>,
        /// Exit code.
        exit_code: i64,
        /// Time taken.
        elapsed: Duration,
    }
}

object_struct! {
    /// Coverage of one file.
    CoverageFile {
        /// File.
        path: Path,
        /// Percentage of covered lines.
        lines_pct: f64,
        /// Ranges of uncovered lines.
        uncovered: Option<Vec<Range>>,
    }
}

object_struct! {
    /// Code coverage of a test run.
    Coverage {
        /// Percentage of covered lines.
        lines_pct: f64,
        /// Percentage of covered branches.
        branches_pct: Option<f64>,
        /// Percentage of covered functions.
        functions_pct: Option<f64>,
        /// Percentage of covered statements.
        statements_pct: Option<f64>,
        /// Coverage per file.
        files: Option<Vec<CoverageFile>>,
        /// Report file written by the framework.
        report_file: Option<Path>,
        /// Format of the report file.
        format: Option<CoverageFormat>,
    }
}

object_struct! {
    /// One check of an analysis.
    AnalysisCheck {
        /// Check, for example `lint`.
        check: String,
        /// Tool that performed it.
        tool: Option<String>,
        /// Outcome.
        status: CheckStatus,
        /// Number of findings.
        findings: Option<u64>,
        /// Time taken.
        duration: Option<Duration>,
    }
}

object_struct! {
    /// Trigger of a scheduled task.
    Trigger {
        /// Kind of trigger.
        kind: TriggerKind,
        /// Cron expression, interval or point in time as written.
        expression: String,
        /// Time zone of the trigger.
        timezone: Option<String>,
    }
}

object_struct! {
    /// File changed by a commit.
    CommitFile {
        /// Path in the repository.
        path: String,
        /// Change.
        status: GitFileState,
        /// Path before a rename.
        old_path: Option<String>,
        /// Lines added.
        added: Option<u64>,
        /// Lines removed.
        removed: Option<u64>,
        /// Binary content.
        binary: Option<bool>,
    }
}

object_struct! {
    /// Signature of a commit or a tag.
    CommitSignature {
        /// Verification status.
        status: SignatureStatus,
        /// Format.
        format: Option<SignatureFormat>,
        /// Key identifier or fingerprint.
        key: Option<String>,
        /// Signer.
        signer: Option<String>,
    }
}

object_struct! {
    /// Change statistics of a commit.
    CommitStats {
        /// Files changed.
        files: u64,
        /// Lines added.
        added: u64,
        /// Lines removed.
        removed: u64,
    }
}

object_struct! {
    /// Status of a submodule.
    Submodule {
        /// The recorded commit changed.
        commit_changed: bool,
        /// Tracked content modified.
        modified: bool,
        /// Untracked content present.
        untracked: bool,
    }
}

object_struct! {
    /// Active limit of the journal.
    JournalLimit {
        /// Limit: `steps`, `age` or `size`.
        name: String,
        /// Value, for example an int, a duration or a size; `unlimited` as
        /// an enumeration value.
        value: Value,
        /// Where the value comes from: `default`, `system`, `user`,
        /// `project`, `session` or `floor`.
        source: String,
        /// Scope of a value set during work: `session` or `user`.
        scope: Option<String>,
        /// Protection floor that the value cannot go below.
        floor: Option<Value>,
        /// Entries that expire because of a lowered limit.
        expires: Option<u64>,
    }
}

record_struct! {
    /// File system entry: file, directory, link or another object.
    ///
    /// For an entry that does not exist (`exists: false` in the result of
    /// `info`), `size` is zero and `modified` is the Unix epoch.
    Entry (Entry) {
        /// Absolute path in canonical form.
        path: Path,
        /// Native bytes of the path; present only when the path is not valid
        /// Unicode, in which case `path` shows the invalid bytes as U+FFFD.
        raw_path: Option<Bytes>,
        /// Last component of the path, in NFC.
        name: String,
        /// Name without the last extension.
        stem: Option<String>,
        /// Last extension, without the leading period.
        extension: Option<String>,
        /// Parent directory.
        parent: Option<Path>,
        /// Native bytes of the parent directory; present only when it is not
        /// valid Unicode.
        raw_parent: Option<Bytes>,
        /// Type of the entry.
        kind as "type": EntryType,
        /// Whether the entry exists; set in the result of `info`.
        exists: Option<bool>,
        /// Size in bytes; for directories as reported by the system.
        size: Size,
        /// Time of the last modification.
        modified: Instant,
        /// Time of creation.
        created: Option<Instant>,
        /// Time of the last access.
        accessed: Option<Instant>,
        /// Permission bits in octal, for example `0644`.
        mode: Option<String>,
        /// Owning account: name and identifier.
        owner: Option<String>,
        /// Owning group.
        group: Option<String>,
        /// Access control entries.
        acl: Option<Vec<String>>,
        /// Read-only.
        readonly: Option<bool>,
        /// Hidden.
        hidden: Option<bool>,
        /// Target of a link.
        link_target: Option<Path>,
        /// Native bytes of the target of a link; present only when it is not
        /// valid Unicode.
        raw_link_target: Option<Bytes>,
        /// Kind of a link.
        link_type: Option<LinkType>,
        /// Major device number of a device node.
        major: Option<u64>,
        /// Minor device number of a device node.
        minor: Option<u64>,
        /// Number of hard links.
        links: Option<u64>,
        /// Mount point of the volume.
        volume: Option<Path>,
        /// File identifier on the volume.
        file_id: Option<String>,
        /// Programming or markup language.
        language: Option<String>,
        /// Media type.
        mime: Option<String>,
        /// Binary content.
        binary: Option<bool>,
        /// Detected encoding of a text file.
        encoding: Option<String>,
        /// Line endings of a text file.
        line_ending: Option<LineEnding>,
        /// Whether the text starts with a byte order mark.
        bom: Option<bool>,
        /// Number of lines.
        lines: Option<u64>,
        /// Content hash, for example `sha256:…`.
        hash: Option<String>,
        /// Entity tag of the content at the time of reading.
        etag: Option<String>,
        /// Name as bytes when it is not valid Unicode or differs from the
        /// NFC form.
        raw_name: Option<Bytes>,
        /// Path after resolving links.
        real_path: Option<Path>,
        /// Actual spelling on a volume that ignores case.
        case_alias: Option<String>,
        /// Portability problems of the name, for example
        /// `windows_reserved_name`.
        portability: Option<Vec<String>>,
        /// Depth below the root of the listing.
        depth: Option<u64>,
        /// Path relative to the root of the listing.
        relative: Option<String>,
        /// Native bytes of the relative path, with `/` separators; present
        /// only when it is not valid Unicode.
        raw_relative: Option<Bytes>,
        /// Total size of a directory tree.
        total_size: Option<Size>,
        /// Number of files in a directory tree.
        file_count: Option<u64>,
        /// Similarity rank of a fuzzy search.
        rank: Option<f64>,
        /// Number of matches in the file.
        match_count: Option<u64>,
        /// Extended attributes.
        xattrs: Option<IndexMap<String, String>>,
        /// Alternate data streams.
        streams: Option<Vec<String>>,
        /// File flags such as `immutable`.
        flags: Option<Vec<String>>,
        /// Details requested with `details:`, by kind.
        details: Option<IndexMap<String, Value>>,
        /// Error that kept the entry from being read completely.
        error: Option<String>,
    }
}

record_struct! {
    /// Text content of a file, decoded to UTF-8.
    Content (Content) {
        /// File.
        path: Path,
        /// Native bytes of the path; present only when it is not valid
        /// Unicode.
        raw_path: Option<Bytes>,
        /// Text, possibly limited to a range.
        text: String,
        /// Number of lines in `text`.
        lines: u64,
        /// Size of the file in bytes.
        size: Size,
        /// Entity tag of the content.
        etag: String,
        /// Encoding of the file.
        encoding: String,
        /// Whether the file starts with a byte order mark.
        bom: bool,
        /// Line endings of the file.
        line_ending: LineEnding,
        /// Range of lines in `text`.
        range: Option<Range>,
        /// Ranges of lines when several were requested.
        ranges: Option<Vec<Range>>,
        /// Total number of lines of the file.
        total_lines: Option<u64>,
        /// The file did not change since the given entity tag.
        unchanged: Option<bool>,
        /// The text was cut at a limit.
        truncated: Option<bool>,
        /// Binary content.
        binary: Option<bool>,
        /// Image content.
        image: Option<ContentImage>,
        /// Page of a document, from 1.
        page: Option<u64>,
        /// Number of pages of the document.
        pages_total: Option<u64>,
        /// Cells of a notebook.
        cells: Option<Vec<Cell>>,
        /// Symbols of the outline view.
        symbols: Option<Vec<Symbol>>,
        /// The content given with `since:` was not available.
        since_missing: Option<bool>,
        /// Difference against the content given with `since:`.
        diff: Option<Vec<Hunk>>,
        /// Compressed file the text was read from.
        decompressed_from: Option<Path>,
        /// View used, for example `outline`.
        view: Option<String>,
        /// Full identifier of the repository revision the content was read
        /// from.
        revision: Option<String>,
    }
}

record_struct! {
    /// One line of a file.
    Line (Line) {
        /// File.
        path: Path,
        /// Native bytes of the path; present only when it is not valid
        /// Unicode.
        raw_path: Option<Bytes>,
        /// Line number, from 1.
        no: u64,
        /// Text without the line ending.
        text: String,
        /// Line ending.
        line_ending: LineEnding,
        /// Entity tag of the file.
        etag: String,
        /// Byte offset of the line from the start of the file.
        start: Option<u64>,
        /// Byte offset after the line, without the line ending.
        end: Option<u64>,
        /// Index of the requested range the line belongs to.
        range_index: Option<u64>,
        /// Encoding of the file.
        encoding: Option<String>,
    }
}

record_struct! {
    /// Match of a search.
    ///
    /// `start` and `end` are byte offsets from the start of the file;
    /// `column` counts Unicode scalar values from 1.
    Match (Match) {
        /// File.
        path: Path,
        /// Native bytes of the path; present only when it is not valid
        /// Unicode.
        raw_path: Option<Bytes>,
        /// Line number, from 1.
        no: u64,
        /// Column of the match, from 1.
        column: u64,
        /// Byte offset of the match from the start of the file.
        start: u64,
        /// Byte offset after the match.
        end: u64,
        /// Matched text.
        text: String,
        /// Whole line.
        line: String,
        /// Entity tag of the file.
        etag: String,
        /// Capture groups by number; `null` for a group that did not match.
        groups: Option<Vec<Option<String>>>,
        /// Named capture groups.
        named: Option<IndexMap<String, Option<String>>>,
        /// Lines before the match.
        before: Option<Vec<String>>,
        /// Lines after the match.
        after: Option<Vec<String>>,
        /// Number of matches in the file (`output: counts`).
        count: Option<u64>,
        /// Index of the pattern in a list of patterns.
        pattern: Option<u64>,
        /// Enclosing symbol.
        container: Option<MatchContainer>,
        /// Engine that evaluated the pattern.
        regex_engine: Option<RegexEngine>,
        /// Last line of a multi-line match.
        end_line: Option<u64>,
        /// Column after the match on the last line.
        end_column: Option<u64>,
        /// Only counted, not shown in full.
        summarized: Option<bool>,
    }
}

record_struct! {
    /// Value at a key path in a structured document.
    Item (Item) {
        /// File.
        path: Path,
        /// Format of the document, for example `json`.
        format: String,
        /// Key path in canonical form.
        key: String,
        /// Value.
        value: Value,
        /// Type of the value in the document.
        value_type: ValueType,
        /// Entity tag of the file.
        etag: String,
        /// Line, from 1.
        line: Option<u64>,
        /// Column, from 1.
        column: Option<u64>,
        /// Last line.
        end_line: Option<u64>,
        /// Byte offset of the value from the start of the file.
        start: Option<u64>,
        /// Byte offset after the value.
        end: Option<u64>,
        /// Comment attached to the value.
        comment: Option<String>,
        /// Index of the document in a file with several documents.
        document: Option<u64>,
        /// Anchor of the value.
        anchor: Option<String>,
        /// Anchor this value is an alias of.
        alias_of: Option<String>,
        /// Tag of the value.
        tag: Option<String>,
    }
}

record_struct! {
    /// Row of a table, a CSV file or a query.
    Row (Row) {
        /// File.
        path: Path,
        /// Row number, from 1.
        no: u64,
        /// Cells by column name.
        cells: IndexMap<String, Value>,
        /// Table or statement the row comes from.
        table: Option<String>,
    }
}

record_struct! {
    /// Node of a syntax tree.
    Node (Node) {
        /// File.
        path: Path,
        /// Language.
        language: String,
        /// Kind of node in the grammar.
        kind: String,
        /// Byte offset from the start of the file.
        start: u64,
        /// Byte offset after the node.
        end: u64,
        /// First line, from 1.
        line: u64,
        /// First column, from 1.
        column: u64,
        /// Last line.
        end_line: u64,
        /// Text of the node.
        text: String,
        /// Entity tag of the file.
        etag: String,
        /// Column after the node on the last line.
        end_column: Option<u64>,
        /// Captured metavariables of the pattern.
        captures: Option<IndexMap<String, String>>,
    }
}

record_struct! {
    /// Symbol of source code.
    Symbol (Symbol) {
        /// Name.
        name: String,
        /// Kind.
        kind: SymbolKind,
        /// File.
        path: Path,
        /// First line, from 1.
        line: u64,
        /// Column, from 1.
        column: u64,
        /// Last line.
        end_line: u64,
        /// Language.
        language: String,
        /// Source of the information.
        source: SymbolSource,
        /// Entity tag of the file.
        etag: String,
        /// Column after the symbol on the last line.
        end_column: Option<u64>,
        /// Enclosing symbol.
        container: Option<String>,
        /// Signature.
        signature: Option<String>,
        /// Text of the definition.
        text: Option<String>,
        /// Exported from its module.
        exported: Option<bool>,
        /// Number of references.
        references: Option<u64>,
    }
}

record_struct! {
    /// Reference to a symbol.
    Reference (Reference) {
        /// Name of the symbol.
        name: String,
        /// File.
        path: Path,
        /// Line, from 1.
        line: u64,
        /// Column, from 1.
        column: u64,
        /// Role.
        role: ReferenceRole,
        /// Source of the information.
        source: SymbolSource,
        /// Entity tag of the file.
        etag: String,
        /// Column after the reference.
        end_column: Option<u64>,
        /// Text of the line.
        text: Option<String>,
        /// Enclosing symbol.
        container: Option<String>,
    }
}

record_struct! {
    /// Diagnostic of a compiler, a linter or a language server.
    Diagnostic (Diagnostic) {
        /// File.
        path: Path,
        /// Line, from 1.
        line: u64,
        /// Column, from 1.
        column: u64,
        /// Severity.
        severity: DiagnosticSeverity,
        /// Message.
        message: String,
        /// Tool or server that reported it.
        source: String,
        /// Code of the diagnostic.
        code: Option<String>,
        /// Last line.
        end_line: Option<u64>,
        /// Column after the fragment on the last line.
        end_column: Option<u64>,
        /// Language.
        language: Option<String>,
        /// Related positions.
        related: Option<Vec<RelatedLocation>>,
    }
}

record_struct! {
    /// Cell of a notebook.
    Cell (Cell) {
        /// Notebook file.
        path: Path,
        /// Position of the cell, from 0.
        index: u64,
        /// Type of the cell.
        cell_type: CellType,
        /// Source of the cell.
        source: String,
        /// Entity tag of the notebook file.
        etag: String,
        /// Identifier of the cell.
        id: Option<String>,
        /// Outputs as stored in the notebook.
        outputs: Option<Vec<Value>>,
        /// Number and kinds of outputs, without their content.
        outputs_summary: Option<String>,
        /// Execution counter.
        execution_count: Option<u64>,
        /// Metadata of the cell.
        metadata: Option<IndexMap<String, Value>>,
    }
}

record_struct! {
    /// Hunk of a unified difference.
    Hunk (Hunk) {
        /// File after the change.
        path: Path,
        /// Native bytes of the path; present only when it is not valid
        /// Unicode.
        raw_path: Option<Bytes>,
        /// First line before, from 1.
        old_start: u64,
        /// Number of lines before.
        old_len: u64,
        /// First line after, from 1.
        new_start: u64,
        /// Number of lines after.
        new_len: u64,
        /// Lines with the prefixes `+`, `-` and space, and the marker line
        /// `\ No newline at end of file`.
        lines: Vec<String>,
        /// Stable identifier, `path#number`.
        id: Option<String>,
        /// Section heading after `@@`.
        header: Option<String>,
        /// File before a rename.
        old_path: Option<Path>,
        /// Native bytes of `old_path`; present only when it is not valid
        /// Unicode.
        raw_old_path: Option<Bytes>,
        /// Status of the file.
        status: Option<HunkStatus>,
        /// Binary content.
        binary: Option<bool>,
        /// Mode before.
        old_mode: Option<String>,
        /// Mode after.
        new_mode: Option<String>,
        /// White space was ignored.
        ignore_whitespace: Option<bool>,
        /// First six hexadecimal digits of the SHA-256 hash of the hunk
        /// lines; part of the identifier `path#number:fingerprint`.
        fingerprint: Option<String>,
        /// The hunk is in the index (staged) rather than in the worktree.
        staged: Option<bool>,
        /// The hunk adds an untracked file.
        untracked: Option<bool>,
        /// Unit of the difference.
        unit: Option<DiffUnit>,
        /// Blocks of lines moved within the difference.
        moved: Option<Vec<MovedBlock>>,
    }
}

/// Line of a hunk that marks a missing line feed at the end of the preceding
/// line, as in a unified difference.
pub const NO_NEWLINE_MARKER: &str = "\\ No newline at end of file";

impl Hunk {
    /// Index of the first line that has neither a valid prefix nor is the
    /// marker [`NO_NEWLINE_MARKER`], if any.
    ///
    /// ```
    /// use nurt_contracts::record::Hunk;
    ///
    /// let hunk = Hunk {
    ///     lines: vec!["+new".to_owned(), "old".to_owned()],
    ///     ..Hunk::default()
    /// };
    /// assert_eq!(hunk.first_invalid_line(), Some(1));
    /// ```
    #[must_use]
    pub fn first_invalid_line(&self) -> Option<usize> {
        self.lines.iter().position(|line| {
            !(line.starts_with('+')
                || line.starts_with('-')
                || line.starts_with(' ')
                || line == NO_NEWLINE_MARKER)
        })
    }
}

record_struct! {
    /// Change of one file within an effect.
    Change (Change) {
        /// File.
        path: Path,
        /// Native bytes of the path; present only when it is not valid
        /// Unicode, in which case `path` shows the invalid bytes as U+FFFD.
        raw_path: Option<Bytes>,
        /// Action.
        action: ChangeAction,
        /// Previous path of a rename.
        from: Option<Path>,
        /// Native bytes of `from`; present only when it is not valid Unicode.
        raw_from: Option<Bytes>,
        /// New path of a move or a copy.
        to: Option<Path>,
        /// Native bytes of `to`; present only when it is not valid Unicode.
        raw_to: Option<Bytes>,
        /// Size before.
        size_before: Option<Size>,
        /// Size after.
        size_after: Option<Size>,
        /// Hash before.
        hash_before: Option<String>,
        /// Hash after.
        hash_after: Option<String>,
        /// Lines added.
        added: Option<u64>,
        /// Lines removed.
        removed: Option<u64>,
        /// Replacements made.
        replacements: Option<u64>,
        /// Rows inserted, updated or deleted by SQL statements.
        rows_changed: Option<u64>,
        /// Notes about the change, for example `crlf_preserved`.
        flags: Option<Vec<String>>,
        /// Encoding before.
        encoding_before: Option<String>,
        /// Encoding after.
        encoding_after: Option<String>,
        /// Line endings before.
        line_ending_before: Option<LineEnding>,
        /// Line endings after.
        line_ending_after: Option<LineEnding>,
        /// Byte order mark before.
        bom_before: Option<bool>,
        /// Byte order mark after.
        bom_after: Option<bool>,
        /// Target of a created link.
        link_target: Option<Path>,
        /// Native bytes of `link_target`; present only when it is not valid
        /// Unicode.
        raw_link_target: Option<Bytes>,
        /// How the previous content was kept.
        store: Option<String>,
        /// Reason why the change cannot be undone, for example
        /// `no_space_for_copy`.
        irreversible: Option<Irreversible>,
        /// Difference of the content.
        diff: Option<Vec<Hunk>>,
        /// State before a change of the action `state`, for example
        /// `running`.
        before: Option<String>,
        /// State after a change of the action `state`.
        after: Option<String>,
    }
}

impl Change {
    /// Change of a target with no details.
    ///
    /// For targets outside the file system, `path` holds an identifier of the
    /// form `<provider>:<name>` (for example `service:nginx`), the action is
    /// [`ChangeAction::State`] and `before` and `after` hold the states.
    ///
    /// ```
    /// use nurt_contracts::record::{Change, ChangeAction};
    ///
    /// let change = Change::new("/srv/app/a.go", ChangeAction::Modify);
    /// assert_eq!(change.size_after, None);
    /// assert_eq!(
    ///     serde_json::to_value(&change).unwrap(),
    ///     serde_json::json!({"path": "/srv/app/a.go", "action": "modify"})
    /// );
    /// ```
    #[must_use]
    pub fn new(path: impl Into<Path>, action: ChangeAction) -> Self {
        Self {
            path: path.into(),
            action,
            ..Self::default()
        }
    }
}

record_struct! {
    /// Result of a finished run of a program.
    ///
    /// `exit_code` is `-1` when the process ended by a signal or a limit;
    /// `signal` and `killed_by` tell which.
    Exec (Exec) {
        /// Arguments as given.
        command: Vec<String>,
        /// Program after resolution in the search path.
        program: Path,
        /// Working directory.
        cwd: Path,
        /// Process identifier.
        pid: u64,
        /// Exit code.
        exit_code: i64,
        /// Ended by the time limit.
        timed_out: bool,
        /// Ended by the idle limit.
        stalled: bool,
        /// Start time.
        started: Instant,
        /// Time taken.
        elapsed: Duration,
        /// Standard output after filters.
        stdout: String,
        /// Standard error after filters.
        stderr: String,
        /// Bytes of standard output.
        stdout_bytes: u64,
        /// Bytes of standard error.
        stderr_bytes: u64,
        /// Lines of standard output.
        stdout_lines: u64,
        /// Lines of standard error.
        stderr_lines: u64,
        /// Encoding the output was decoded from, for example `utf-8`.
        decoded_as: String,
        /// Files changed by the run.
        changes: Vec<Change>,
        /// How changes were detected.
        tracking: Tracking,
        /// Directories watched for changes.
        watch: Vec<Path>,
        /// How the program was started.
        launcher: Option<Launcher>,
        /// Signal that ended the process.
        signal: Option<Signal>,
        /// Number of that signal.
        signal_number: Option<i64>,
        /// Who ended the process.
        killed_by: Option<KilledBy>,
        /// Resource limit reached.
        limit_hit: Option<LimitHit>,
        /// End time.
        finished: Option<Instant>,
        /// Processor time of the process group.
        cpu_time: Option<Duration>,
        /// Peak memory of the process group.
        max_rss: Option<Size>,
        /// What was shown of the output.
        shown: Option<ExecShown>,
        /// File with the merged output.
        output_file: Option<Path>,
        /// File with standard output.
        stdout_file: Option<Path>,
        /// File with standard error.
        stderr_file: Option<Path>,
        /// Cursor for reading further output.
        output_cursor: Option<String>,
        /// Hash of the complete output.
        output_sha256: Option<String>,
        /// Lines matched by `summary:`.
        summary_lines: Option<Vec<String>>,
        /// Last progress.
        progress: Option<Progress>,
        /// Previous content kept before the run.
        preimage: Option<Preimage>,
        /// Number of files kept.
        preimage_entries: Option<u64>,
        /// Bytes kept.
        preimage_bytes: Option<Size>,
        /// Effects outside the file system recognised in the arguments.
        external_effect: Option<Vec<String>>,
        /// Batches of a run over a record stream.
        batches: Option<Vec<Batch>>,
        /// Number of failed batches.
        failed_batches: Option<u64>,
        /// Names of the environment variables set explicitly.
        env_keys: Option<Vec<String>>,
        /// Job identifier when the run continued in the background.
        job: Option<String>,
        /// Errors and warnings of a recognised tool.
        diagnostics: Option<ExecDiagnostics>,
        /// One line about a successful run: exit code, time, lines and bytes.
        digest: Option<String>,
        /// Sentence with the commands that read what the response leaves out
        /// from the output file.
        read_hint: Option<String>,
    }
}

record_struct! {
    /// Background job.
    Job (Job) {
        /// Identifier, for example `j7`.
        id: String,
        /// Arguments.
        command: Vec<String>,
        /// Working directory.
        cwd: Path,
        /// Process identifier.
        pid: u64,
        /// State.
        state: JobState,
        /// Start time.
        started: Instant,
        /// Time since the start or until the end.
        elapsed: Duration,
        /// Label.
        label: Option<String>,
        /// Description.
        description: Option<String>,
        /// Program after resolution.
        program: Option<Path>,
        /// Exit code after the end.
        exit_code: Option<i64>,
        /// Signal that ended the process.
        signal: Option<Signal>,
        /// Who ended the process.
        killed_by: Option<KilledBy>,
        /// End time.
        finished: Option<Instant>,
        /// Last progress.
        progress: Option<Progress>,
        /// Standard output, when requested.
        stdout: Option<String>,
        /// Standard error, when requested.
        stderr: Option<String>,
        /// Bytes of standard output.
        stdout_bytes: Option<u64>,
        /// Bytes of standard error.
        stderr_bytes: Option<u64>,
        /// Lines of standard output.
        stdout_lines: Option<u64>,
        /// Lines of standard error.
        stderr_lines: Option<u64>,
        /// File with the merged output.
        output_file: Option<Path>,
        /// File with standard output.
        stdout_file: Option<Path>,
        /// File with standard error.
        stderr_file: Option<Path>,
        /// Cursor for reading further output.
        output_cursor: Option<String>,
        /// Job this one repeats.
        retry_of: Option<String>,
        /// Number of the journal operation of the job.
        op: Option<u64>,
        /// Files changed, after the end.
        changes: Option<Vec<Change>>,
        /// Session that started the job.
        session: Option<String>,
        /// Total time suspended.
        suspended_total: Option<Duration>,
    }
}

record_struct! {
    /// Process of the operating system.
    Process (Process) {
        /// Process identifier.
        pid: u64,
        /// Parent process identifier.
        parent_pid: u64,
        /// Name.
        name: String,
        /// Arguments.
        command: Vec<String>,
        /// Account name.
        user: String,
        /// Resident memory.
        memory: Size,
        /// Start time.
        started: Instant,
        /// State.
        state: ProcessState,
        /// Executable file.
        executable: Option<Path>,
        /// Account identifier: UID or SID.
        uid: Option<String>,
        /// Processor use in percent of one core.
        cpu: Option<f64>,
        /// Number of threads.
        threads: Option<u64>,
        /// Working directory.
        cwd: Option<Path>,
        /// Child processes.
        children: Option<Vec<u64>>,
        /// Depth in the process tree.
        depth: Option<u64>,
        /// Raw attributes of the system, outside the stability guarantee.
        os_raw: Option<IndexMap<String, Value>>,
    }
}

record_struct! {
    /// Network port.
    Port (Port) {
        /// Protocol.
        protocol: Protocol,
        /// Local address and port, `address:port`.
        local: String,
        /// Local address.
        address: String,
        /// Local port.
        port: u64,
        /// State.
        state: PortState,
        /// Remote address and port.
        remote: Option<String>,
        /// Remote address.
        remote_address: Option<String>,
        /// Remote port.
        remote_port: Option<u64>,
        /// Owning process.
        pid: Option<u64>,
        /// Name of the owning process.
        process: Option<String>,
        /// Address family.
        family: Option<AddressFamily>,
    }
}

record_struct! {
    /// Mounted disk.
    Disk (Disk) {
        /// Mount point.
        mount: Path,
        /// Device.
        device: String,
        /// File system.
        filesystem: String,
        /// Total size.
        total: Size,
        /// Free size available to the account.
        free: Size,
        /// Used size.
        used: Size,
        /// Type.
        kind as "type": DiskType,
        /// Names differ by case.
        case_sensitive: bool,
        /// Mounted read-only.
        readonly: bool,
        /// Label.
        label: Option<String>,
        /// Total number of inodes.
        inodes_total: Option<u64>,
        /// Free inodes.
        inodes_free: Option<u64>,
        /// Mount options.
        options: Option<Vec<String>>,
        /// Raw attributes of the system, outside the stability guarantee.
        os_raw: Option<IndexMap<String, Value>>,
    }
}

record_struct! {
    /// Description of the device.
    HostInfo (Host) {
        /// Operating system.
        os: OsKind,
        /// Version of the operating system.
        os_version: String,
        /// Processor architecture, for example `x86_64`.
        architecture: String,
        /// Logical processors.
        cpus: u64,
        /// Total memory.
        memory_total: Size,
        /// Host name.
        hostname: String,
        /// Current account.
        user: String,
        /// Home directory.
        home: Path,
        /// Directory for temporary files.
        temp: Path,
        /// Local time zone.
        timezone: String,
        /// Path style: `posix` or `windows`.
        path_style: String,
        /// Version of this implementation.
        nurt_version: String,
        /// Features available, for example `reflink` and `journal`.
        features: Vec<String>,
        /// Kernel version.
        kernel: Option<String>,
        /// Free memory.
        memory_free: Option<Size>,
        /// Account identifier.
        uid: Option<String>,
        /// Groups of the account.
        groups: Option<Vec<String>>,
        /// Locale.
        locale: Option<String>,
        /// Time since boot.
        uptime: Option<Duration>,
        /// Boot time.
        boot_time: Option<Instant>,
        /// Programs found in the search path.
        programs: Option<Vec<ProgramLocation>>,
        /// Virtualisation, when detected.
        virtualization: Option<String>,
        /// State of the background service of the instance.
        service: Option<ServiceAvailability>,
    }
}

record_struct! {
    /// Environment variable; the value is never masked.
    EnvironmentVariable (EnvironmentVariable) {
        /// Name.
        name: String,
        /// Value.
        value: String,
        /// Scope.
        scope: EnvironmentScope,
    }
}

record_struct! {
    /// Service of the operating system.
    Service (Service) {
        /// Name.
        name: String,
        /// State.
        state: ServiceState,
        /// Start type.
        start_type: StartType,
        /// Service manager.
        manager: ServiceManager,
        /// Display name.
        display_name: Option<String>,
        /// Raw state reported by the manager.
        state_raw: Option<String>,
        /// Main process.
        pid: Option<u64>,
        /// Description.
        description: Option<String>,
        /// Scope.
        scope: Option<ServiceScope>,
        /// Unit file, property list or program of the service.
        path: Option<Path>,
        /// Account the service runs as.
        account: Option<String>,
        /// Raw attributes of the system, outside the stability guarantee.
        os_raw: Option<IndexMap<String, Value>>,
    }
}

record_struct! {
    /// Installed or available package.
    Package (Package) {
        /// Name.
        name: String,
        /// Installed or offered version.
        version: String,
        /// Package manager.
        manager: PackageManager,
        /// Installed.
        installed: bool,
        /// Newer version available.
        available_version: Option<String>,
        /// Description.
        description: Option<String>,
        /// Architecture.
        architecture: Option<String>,
        /// Installed size.
        size: Option<Size>,
        /// Repository.
        source: Option<String>,
        /// Number of files.
        files_count: Option<u64>,
    }
}

record_struct! {
    /// User or group account.
    Account (Account) {
        /// Name.
        name: String,
        /// Kind.
        kind: AccountKind,
        /// Identifier: UID, GID or SID.
        id: String,
        /// Home directory.
        home: Option<Path>,
        /// Login shell.
        shell: Option<Path>,
        /// Groups of a user or members of a group.
        groups: Option<Vec<String>>,
        /// Disabled.
        disabled: Option<bool>,
        /// Full name.
        full_name: Option<String>,
        /// System account.
        system: Option<bool>,
        /// Last login.
        last_login: Option<Instant>,
        /// Raw attributes of the system, outside the stability guarantee.
        os_raw: Option<IndexMap<String, Value>>,
    }
}

record_struct! {
    /// Event of a system log.
    LogEvent (LogEvent) {
        /// Time.
        time: Instant,
        /// Program, provider or subsystem that logged the event.
        source: String,
        /// Level.
        level: LogLevel,
        /// Message.
        message: String,
        /// Event identifier.
        id: Option<String>,
        /// Host name.
        host: Option<String>,
        /// Unit, channel or category.
        unit: Option<String>,
        /// Process identifier.
        pid: Option<u64>,
        /// Boot identifier.
        boot_id: Option<String>,
        /// Position for paging and resuming.
        cursor: Option<String>,
        /// Other fields.
        fields: Option<IndexMap<String, String>>,
    }
}

record_struct! {
    /// Commit of a repository.
    Commit (Commit) {
        /// Full object identifier.
        id: String,
        /// Abbreviated identifier.
        short_id: String,
        /// Author name.
        author: String,
        /// Author time.
        time: Instant,
        /// Complete message.
        message: String,
        /// First line of the message.
        subject: String,
        /// Parent identifiers.
        parents: Vec<String>,
        /// Author e-mail.
        author_email: Option<String>,
        /// Committer name.
        committer: Option<String>,
        /// Committer e-mail.
        committer_email: Option<String>,
        /// Commit time.
        commit_time: Option<Instant>,
        /// Names of references that point to the commit.
        refs: Option<Vec<String>>,
        /// Signature.
        signature: Option<CommitSignature>,
        /// Change statistics.
        stats: Option<CommitStats>,
        /// Trailers of the message.
        trailers: Option<IndexMap<String, String>>,
        /// Tree identifier.
        tree: Option<String>,
        /// Files changed.
        files: Option<Vec<CommitFile>>,
        /// Branch that received the commit, in the result of `git.commit`.
        branch: Option<String>,
    }
}

record_struct! {
    /// Status of a path in a repository.
    GitStatus (GitStatus) {
        /// Path in the repository.
        path: String,
        /// Bytes of `path`; present only when it is not valid Unicode.
        raw_path: Option<Bytes>,
        /// State in the index.
        index: GitFileState,
        /// State in the working tree.
        worktree: GitFileState,
        /// Kind of entry.
        kind: GitStatusKind,
        /// Root of the repository.
        repo: Path,
        /// Path before a rename.
        renamed_from: Option<String>,
        /// Bytes of `renamed_from`; present only when it is not valid Unicode.
        raw_renamed_from: Option<Bytes>,
        /// Kind of conflict.
        conflict: Option<GitConflictKind>,
        /// Status of a submodule.
        submodule: Option<Submodule>,
        /// Mode before.
        mode_before: Option<String>,
        /// Mode after.
        mode_after: Option<String>,
        /// Lines added.
        added: Option<u64>,
        /// Lines removed.
        removed: Option<u64>,
        /// Similarity of a rename in percent.
        similarity: Option<u64>,
    }
}

record_struct! {
    /// Branch of a repository.
    Branch (Branch) {
        /// Name.
        name: String,
        /// Checked out in this working tree.
        current: bool,
        /// Commit identifier.
        commit: String,
        /// Remote-tracking branch.
        remote: bool,
        /// Upstream branch.
        upstream: Option<String>,
        /// Commits ahead of the upstream.
        ahead: Option<u64>,
        /// Commits behind the upstream.
        behind: Option<u64>,
        /// The upstream no longer exists.
        upstream_gone: Option<bool>,
        /// Working tree where the branch is checked out.
        worktree: Option<Path>,
        /// Subject of the last commit.
        subject: Option<String>,
        /// Time of the last commit.
        time: Option<Instant>,
    }
}

record_struct! {
    /// Scheduled task.
    Schedule (Schedule) {
        /// Identifier, for example `s3`.
        id: String,
        /// Trigger.
        trigger: Trigger,
        /// Next run.
        next_run: Instant,
        /// The task runs.
        active: bool,
        /// Action: the canonical program text.
        action: String,
        /// Label.
        label: Option<String>,
        /// Last run.
        last_run: Option<Instant>,
        /// Outcome of the last run.
        last_result: Option<ScheduleRunState>,
        /// State.
        state: Option<ScheduleState>,
        /// Runs so far.
        runs: Option<u64>,
        /// Maximum number of runs.
        max_runs: Option<u64>,
        /// Expiry time.
        expires: Option<Instant>,
        /// Handling of runs missed while the service was not running.
        on_missed: Option<String>,
        /// Tolerance for a late start.
        misfire: Option<Duration>,
        /// Handling of a run that starts while the previous one runs.
        overlap: Option<String>,
        /// Time limit of a run.
        timeout: Option<Duration>,
        /// Creation time.
        created: Option<Instant>,
        /// Session that created the task.
        session: Option<String>,
        /// Kind of action, for example `pipeline`.
        action_kind: Option<String>,
        /// Why the task is not active, for example `service_not_running`.
        reason: Option<String>,
    }
}

object_struct! {
    /// Item of a journal entry: one target of the operation.
    OperationItem {
        /// Target of the item.
        path: Path,
        /// Native bytes of the target; present only when it is not valid
        /// Unicode.
        raw_path: Option<Bytes>,
        /// Action on the target, the name of the journal target action, for
        /// example `create`, `modify` or `rename`.
        action: String,
        /// Preserve mode of the previous content, for example `clone` or
        /// `none`.
        mode: String,
        /// Hash of the content before the operation; absent when there was
        /// none.
        before_hash: Option<String>,
        /// Hash of the content after the operation; absent when there is
        /// none.
        after_hash: Option<String>,
        /// The item has been undone.
        undone: bool,
        /// The preserved previous content of the item has been released.
        released: bool,
    }
}

record_struct! {
    /// Entry of the operation journal.
    ///
    /// The record shows no expiry time and nothing from which one could be
    /// derived.
    Operation (Operation) {
        /// Operation number, one sequence per account, from 1.
        id: u64,
        /// Commit time.
        time: Instant,
        /// Command as named in the catalog, for example `replace` or
        /// `git.stash`.
        kind: String,
        /// Changing action of a command with several actions, for example
        /// `push`.
        action: Option<String>,
        /// One-line summary of the effect.
        summary: String,
        /// State.
        state: OperationState,
        /// Number of items.
        files: u64,
        /// Logical bytes changed.
        bytes: Size,
        /// Effects that cannot be undone; empty when everything can.
        irreversible: Vec<IrreversibleEntry>,
        /// Several targets in one operation.
        bulk: bool,
        /// Session of the call.
        session: Option<String>,
        /// Operations this one undid; empty for other operations.
        undo_of: Vec<u64>,
        /// Undo operation this one redid.
        redo_of: Option<u64>,
        /// Scope of an undo operation.
        undo_scope: Option<UndoScope>,
        /// Hash of the canonical bound program, `sha256:` and 64 hexadecimal
        /// digits.
        ast_hash: String,
        /// Hash of the manifest of the entry.
        manifest_sha256: String,
        /// Hash of the manifest of the previous entry of the account;
        /// absent for the first entry.
        chain: Option<String>,
        /// Protected from expiry by steps, age and size, but not from the need
        /// for space.
        pinned: bool,
        /// Items of the entry; present only when details are requested, and
        /// empty for an expired entry.
        items: Option<Vec<OperationItem>>,
    }
}

record_struct! {
    /// Response that an HTTP request received.
    HttpResponse (Response) {
        /// Requested URL.
        url: String,
        /// Status code.
        status: u64,
        /// Headers by lower-case name; repeated headers joined, `set-cookie`
        /// as a list.
        headers: IndexMap<String, Value>,
        /// Body decoded to text.
        body: String,
        /// Bytes received.
        bytes: Size,
        /// Time taken.
        elapsed: Duration,
        /// URL after redirects.
        final_url: String,
        /// File the body was saved to.
        saved: Option<Path>,
        /// Media type.
        content_type: Option<String>,
        /// Character set.
        charset: Option<String>,
        /// The body was cut.
        truncated: Option<bool>,
        /// Offset of the body in characters.
        offset: Option<u64>,
        /// Total characters of the body.
        total_chars: Option<u64>,
        /// URLs of the redirects.
        redirects: Option<Vec<String>>,
        /// Served from the cache.
        from_cache: Option<bool>,
        /// Not changed since the given time or tag.
        unchanged: Option<bool>,
        /// Entity tag of the response.
        etag: Option<String>,
        /// Last modification reported.
        last_modified: Option<Instant>,
        /// Address of the remote host.
        remote_address: Option<String>,
    }
}

record_struct! {
    /// Volume as seen by the journal.
    Volume (Volume) {
        /// Mount point.
        mount: Path,
        /// File system.
        filesystem: String,
        /// Total size.
        total: Size,
        /// Free size.
        free: Size,
        /// Free size if the journal held nothing.
        free_without_journal: Size,
        /// Size the journal can release.
        reclaimable: Size,
        /// Reserve kept free.
        reserve: Size,
        /// Soft threshold of free space.
        soft: Size,
        /// Bytes held by the journal.
        journal_bytes: Size,
        /// The journal is active on the volume.
        journal: bool,
        /// Why the journal is not active.
        journal_reason: Option<String>,
    }
}

record_struct! {
    /// Description of a program.
    ProgramCard (ProgramCard) {
        /// Name.
        name: String,
        /// One sentence.
        summary: String,
        /// Main source.
        source: ProgramSource,
        /// Installed.
        installed: bool,
        /// Instructions are not required before use.
        guide_optional: bool,
        /// Sources the card was assembled from.
        sources: Vec<String>,
        /// Category.
        category: Option<String>,
        /// Subcategory.
        subcategory: Option<String>,
        /// Path.
        path: Option<Path>,
        /// Version.
        version: Option<String>,
        /// Description.
        description: Option<String>,
        /// Instructions of use.
        guide: Option<String>,
        /// Name of instructions shared by several programs.
        guide_shared: Option<String>,
        /// Package.
        package: Option<String>,
        /// Home page.
        homepage: Option<String>,
        /// Licence.
        license: Option<String>,
        /// Other names that resolve to the program.
        aliases: Option<Vec<String>>,
        /// Time of the last verification.
        verified: Option<Instant>,
        /// Tags.
        tags: Option<Vec<String>>,
        /// Operating systems.
        platforms: Option<Vec<String>>,
        /// Relevance in a search.
        score: Option<f64>,
    }
}

record_struct! {
    /// Member of an archive.
    ArchiveEntry (ArchiveEntry) {
        /// Archive file.
        archive: Path,
        /// Last component of the member path.
        name: String,
        /// Size after extraction.
        size: Size,
        /// Type.
        kind as "type": EntryType,
        /// Full path inside the archive.
        path: Option<String>,
        /// Size in the archive.
        packed: Option<Size>,
        /// Modification time.
        modified: Option<Instant>,
        /// Permission bits in octal.
        mode: Option<String>,
        /// Target of a link.
        link_target: Option<String>,
        /// Compression method.
        compression: Option<String>,
        /// Encrypted.
        encrypted: Option<bool>,
        /// CRC-32 checksum.
        crc32: Option<String>,
        /// Format of the archive.
        format: Option<String>,
    }
}

record_struct! {
    /// Test run.
    TestRun (TestRun) {
        /// Identifier, for example `run-3`.
        id: String,
        /// Framework.
        framework: TestFramework,
        /// Outcome.
        status: TestRunStatus,
        /// Tests passed.
        passed: u64,
        /// Tests failed.
        failed: u64,
        /// Tests skipped.
        skipped: u64,
        /// Tests that could not run.
        errors: u64,
        /// Time taken.
        duration: Duration,
        /// Arguments of the command.
        command: Vec<String>,
        /// Start time.
        started: Instant,
        /// Working directory.
        cwd: Path,
        /// Version of the framework.
        framework_version: Option<String>,
        /// Tests marked as not yet written.
        todo: Option<u64>,
        /// Tests that passed only on retry.
        flaky: Option<u64>,
        /// Fingerprint of the state of the code.
        fingerprint: Option<String>,
        /// Selected targets.
        targets: Option<Vec<String>>,
        /// Name filter.
        filter: Option<String>,
        /// Exit code of the framework.
        exit_code: Option<i64>,
        /// File with the complete output.
        output_file: Option<Path>,
        /// Run this one repeats.
        retry_of: Option<String>,
        /// Journal operation of the run.
        op: Option<u64>,
        /// Code coverage.
        coverage: Option<Coverage>,
    }
}

record_struct! {
    /// Result of one test.
    TestResult (TestResult) {
        /// Test run.
        run: String,
        /// Stable identifier, for example `tests/a_test.py::test_x`.
        id: String,
        /// Name.
        name: String,
        /// Outcome.
        status: TestStatus,
        /// Suite.
        suite: Option<String>,
        /// Time taken.
        duration: Option<Duration>,
        /// Failure message.
        message: Option<String>,
        /// File of the test.
        path: Option<Path>,
        /// Line, from 1.
        line: Option<u64>,
        /// Column, from 1.
        column: Option<u64>,
        /// Stack trace.
        stack: Option<String>,
        /// Standard output of the test.
        stdout: Option<String>,
        /// Standard error of the test.
        stderr: Option<String>,
        /// Number of retries.
        retries: Option<u64>,
    }
}

record_struct! {
    /// Finding of an analysis or a review.
    Finding (Finding) {
        /// File.
        path: Path,
        /// Kind.
        kind: FindingKind,
        /// Severity.
        severity: FindingSeverity,
        /// Message.
        message: String,
        /// Line, from 1.
        line: Option<u64>,
        /// Column, from 1.
        column: Option<u64>,
        /// Last line.
        end_line: Option<u64>,
        /// Column after the fragment on the last line.
        end_column: Option<u64>,
        /// Tool that reported it.
        tool: Option<String>,
        /// Identifier, for example `an-3#17`.
        id: Option<String>,
        /// Analysis, for example `an-3`.
        analysis: Option<String>,
        /// Rule of the tool, for example `no-unused-vars`.
        rule: Option<String>,
        /// The tool can fix it.
        fixable: Option<bool>,
        /// Lines of the fragment.
        sample: Option<Vec<String>>,
        /// Measured value, for example a complexity.
        value: Option<f64>,
        /// Threshold exceeded.
        threshold: Option<f64>,
        /// Related positions, for example the other copy of a duplicate.
        related: Option<Vec<RelatedLocation>>,
        /// Identifier of the hunk of a review.
        hunk: Option<String>,
        /// What the finding is about; for a commit message `path` is the
        /// root of the repository.
        target: Option<FindingTarget>,
        /// Number of occurrences aggregated in this finding.
        count: Option<u64>,
    }
}

record_struct! {
    /// Effect of a changing or running stage, as a record for the stages
    /// after the effect.
    ///
    /// The complete effect is described by the effect report; this record
    /// carries the parts that later stages filter and check.
    Effect (Effect) {
        /// Status.
        status: EffectStatus,
        /// Command, for example `replace`.
        kind: String,
        /// Number of the journal operation.
        op: Option<u64>,
        /// Number of targets.
        targets: Option<u64>,
        /// Counters of the summary, for example `files_modified`.
        summary: Option<IndexMap<String, u64>>,
        /// Changed files.
        changes: Option<Vec<Change>>,
        /// Codes of unusual effects, for example `mass_delete`.
        notable: Option<Vec<String>>,
        /// Effects that cannot be undone.
        irreversible: Option<Vec<IrreversibleEntry>>,
        /// Run of a program.
        exec: Option<Exec>,
        /// Background job.
        job: Option<Job>,
        /// Test run.
        test: Option<TestRun>,
        /// Analysis.
        analysis: Option<Analysis>,
        /// Target of an effect outside the file system, for example
        /// `service:nginx`.
        target: Option<String>,
        /// Action on that target, for example `restart`.
        action: Option<String>,
        /// State before.
        before: Option<Value>,
        /// State after.
        after: Option<Value>,
        /// Effect on a repository.
        git: Option<GitEffect>,
    }
}

object_struct! {
    /// Progress of an operation with several steps, such as a rebase.
    GitStep {
        /// Steps done.
        done: u64,
        /// Steps in total.
        total: u64,
    }
}

record_struct! {
    /// Effect of an operation on a repository.
    GitEffect (GitEffect) {
        /// Root of the repository.
        repo: Path,
        /// Outcome.
        state: GitEffectState,
        /// Commit of `HEAD` before.
        head_before: Option<String>,
        /// Commit of `HEAD` after.
        head_after: Option<String>,
        /// Current branch after the operation.
        branch: Option<String>,
        /// Updates of references.
        ref_updates: Option<Vec<GitRefUpdate>>,
        /// Conflicts left in the working tree.
        conflicts: Option<Vec<GitConflict>>,
        /// Operation left in progress.
        in_progress: Option<GitInProgress>,
        /// Progress of an operation with several steps.
        step: Option<GitStep>,
        /// Objects written to the object database.
        objects_written: Option<u64>,
        /// Objects received from a remote.
        objects_received: Option<u64>,
        /// Bytes received from a remote.
        bytes_received: Option<Size>,
        /// Bytes sent to a remote.
        bytes_sent: Option<Size>,
        /// Current branch before the operation.
        branch_before: Option<String>,
        /// `HEAD` points to a commit, not to a branch.
        detached: Option<bool>,
        /// Upstream branch of the current branch.
        upstream: Option<String>,
        /// Commits of the current branch not in the upstream.
        ahead: Option<u64>,
        /// Commits of the upstream not in the current branch.
        behind: Option<u64>,
        /// The configured upstream no longer exists.
        upstream_gone: Option<bool>,
        /// Commit being merged while a merge is in progress.
        merge_head: Option<String>,
        /// Numbers of changed paths by state.
        counts: Option<GitCounts>,
        /// Summary of the index after staging.
        index: Option<GitIndexSummary>,
        /// Checkpoint created by the operation.
        checkpoint: Option<String>,
        /// Checkpoint saved before a restore or a switch.
        checkpoint_before: Option<String>,
        /// Hash of the repository state (`HEAD`, merge state, index tree and
        /// targets) for the `expect_state` parameter of a later call.
        state_token: Option<String>,
    }
}

object_struct! {
    /// Numbers of changed paths of a repository by state.
    GitCounts {
        /// Paths with changes in the index.
        staged: u64,
        /// Paths with changes in the worktree not in the index.
        unstaged: u64,
        /// Untracked paths.
        untracked: u64,
        /// Paths with conflicts.
        conflicts: u64,
    }
}

object_struct! {
    /// Summary of the index after staging.
    GitIndexSummary {
        /// Files with staged changes.
        files: u64,
        /// Lines added.
        added: u64,
        /// Lines removed.
        removed: u64,
    }
}

record_struct! {
    /// Value in the Windows registry.
    RegistryValue (RegistryValue) {
        /// Key, for example `registry:/HKCU/Software/Example`.
        path: Path,
        /// Name; empty for the default value of the key.
        name: String,
        /// Type.
        kind as "type": RegistryType,
        /// Value: text, int, list of texts or bytes.
        value: Value,
        /// Size in bytes.
        size: Size,
        /// Registry view.
        view: Option<RegistryView>,
    }
}

record_struct! {
    /// State and limits of the operation journal.
    JournalState (JournalState) {
        /// The journal is active.
        active: bool,
        /// Number of entries.
        entries: u64,
        /// Bytes held.
        bytes: Size,
        /// Active limits with their sources.
        limits: Vec<JournalLimit>,
        /// Volume, when one was selected.
        volume: Option<Path>,
        /// Time of the oldest entry.
        oldest: Option<Instant>,
        /// Number of the oldest entry.
        oldest_id: Option<u64>,
        /// Number of the newest entry.
        newest_id: Option<u64>,
        /// Pinned entries.
        pinned: Option<u64>,
        /// Why the journal is not active.
        reason: Option<String>,
    }
}

record_struct! {
    /// Result of an expression.
    Evaluation (Value) {
        /// Value.
        value: Value,
        /// Language type of the value.
        kind as "type": String,
    }
}

record_struct! {
    /// Group of records with the same key.
    Group (Group) {
        /// Key: one value, or a map for several keys.
        key: Value,
        /// Number of records.
        count: u64,
        /// The records, when requested.
        items: Option<Vec<Value>>,
    }
}

record_struct! {
    /// Statistics of a field.
    FieldSummary (Summary) {
        /// Number of records.
        count: u64,
        /// Field.
        field: Option<String>,
        /// Group key.
        group: Option<Value>,
        /// Sum.
        sum: Option<Value>,
        /// Smallest value.
        min: Option<Value>,
        /// Largest value.
        max: Option<Value>,
        /// Average.
        average: Option<Value>,
    }
}

record_struct! {
    /// Field of the records in a pipeline.
    FieldInfo (FieldInfo) {
        /// Field name.
        field: String,
        /// Language type.
        kind as "type": String,
        /// Example value.
        example: Option<Value>,
        /// Present in every record.
        required: Option<bool>,
        /// Number of records that have it.
        records: Option<u64>,
    }
}

record_struct! {
    /// Management object of Windows.
    CimObject (CimObject) {
        /// Class.
        class: String,
        /// Object path.
        path: String,
        /// Properties.
        properties: IndexMap<String, Value>,
    }
}

record_struct! {
    /// Line annotated with the commit that last changed it.
    BlameLine (BlameLine) {
        /// File.
        path: String,
        /// Line number, from 1.
        no: u64,
        /// Commit identifier.
        commit: String,
        /// Abbreviated identifier.
        short_id: String,
        /// Author.
        author: String,
        /// Author time.
        time: Instant,
        /// Text of the line.
        text: String,
        /// Author e-mail.
        author_email: Option<String>,
        /// Path in that commit.
        original_path: Option<String>,
        /// Line number in that commit.
        original_no: Option<u64>,
        /// The commit is a boundary of the history.
        boundary: Option<bool>,
    }
}

record_struct! {
    /// Description of a symbol at a position.
    Hover (Hover) {
        /// File.
        path: Path,
        /// Line, from 1.
        line: u64,
        /// Column, from 1.
        column: u64,
        /// Source of the information.
        source: SymbolSource,
        /// Signature.
        signature: Option<String>,
        /// Documentation.
        documentation: Option<String>,
    }
}

record_struct! {
    /// Code action offered by a language server.
    CodeAction (CodeAction) {
        /// File.
        path: Path,
        /// Line, from 1.
        line: u64,
        /// Title.
        title: String,
        /// Source of the information.
        source: SymbolSource,
        /// Last line.
        end_line: Option<u64>,
        /// Kind, for example `quickfix`.
        kind: Option<String>,
    }
}

record_struct! {
    /// Supported language and its server.
    Language (Language) {
        /// Language.
        language: String,
        /// File extensions.
        extensions: Vec<String>,
        /// A server was found.
        server_found: bool,
        /// Version of the built-in grammar.
        grammar_version: Option<String>,
        /// Server program.
        server: Option<String>,
        /// The server runs.
        server_running: Option<bool>,
        /// Process of the server.
        server_pid: Option<u64>,
        /// Root directory of the server.
        server_root: Option<Path>,
    }
}

record_struct! {
    /// Planned edit of a rename.
    RenameEdit (RenameEdit) {
        /// File.
        path: Path,
        /// Line, from 1.
        line: u64,
        /// Column, from 1.
        column: u64,
        /// Last line.
        end_line: u64,
        /// Column after the fragment.
        end_column: u64,
        /// Text before.
        old_text: String,
        /// Text after.
        new_text: String,
        /// Kind of text.
        kind: RenameEditKind,
        /// Source of the information.
        source: SymbolSource,
    }
}

record_struct! {
    /// Entry of a repository profile.
    ProfileEntry (ProfileEntry) {
        /// Section.
        section: ProfileSection,
        /// Name.
        name: String,
        /// File.
        path: Option<Path>,
        /// Command.
        command: Option<String>,
        /// Where the entry comes from, for example a manifest.
        origin: Option<String>,
        /// Number of changes in the history.
        changes: Option<u64>,
        /// Time of the last change.
        last_change: Option<Instant>,
    }
}

record_struct! {
    /// Result of a reachability test; times in milliseconds.
    PingResult (PingResult) {
        /// Host as given.
        host: String,
        /// Address used.
        address: String,
        /// Requests sent.
        sent: u64,
        /// Replies received.
        received: u64,
        /// Loss in percent.
        loss: f64,
        /// Shortest round trip.
        rtt_min: Option<f64>,
        /// Average round trip.
        rtt_avg: Option<f64>,
        /// Longest round trip.
        rtt_max: Option<f64>,
        /// Standard deviation of the round trip.
        rtt_stddev: Option<f64>,
    }
}

record_struct! {
    /// DNS resource record.
    DnsRecord (DnsRecord) {
        /// Name.
        name: String,
        /// Type, for example `AAAA`.
        kind as "type": String,
        /// Data.
        value: String,
        /// Time to live.
        ttl: Duration,
        /// Server that answered.
        server: Option<String>,
    }
}

record_struct! {
    /// Network interface.
    NetworkInterface (NetworkInterface) {
        /// Name.
        name: String,
        /// Addresses with prefix length.
        addresses: Vec<String>,
        /// The interface is up.
        up: bool,
        /// Hardware address.
        mac: Option<String>,
        /// Maximum transmission unit.
        mtu: Option<u64>,
        /// Kind.
        kind: Option<InterfaceKind>,
        /// Default gateway.
        gateway: Option<String>,
    }
}

record_struct! {
    /// Location of a program.
    ProgramLocation (ProgramLocation) {
        /// Name.
        name: String,
        /// The program was found.
        found: bool,
        /// Path that the name resolves to.
        path: Option<Path>,
        /// Version.
        version: Option<String>,
        /// Source.
        source: Option<ProgramSource>,
        /// Every match in the search path, in order of resolution.
        all_paths: Option<Vec<Path>>,
    }
}

record_struct! {
    /// Category of programs.
    ProgramCategory (ProgramCategory) {
        /// Name.
        name: String,
        /// Number of programs.
        count: u64,
        /// Source.
        source: String,
        /// Title.
        title: Option<String>,
        /// Description.
        description: Option<String>,
        /// Subcategories.
        subcategories: Option<Vec<String>>,
    }
}

record_struct! {
    /// One run of a scheduled task.
    ScheduleRun (ScheduleRun) {
        /// Scheduled task.
        schedule_id: String,
        /// Start time.
        started: Instant,
        /// Outcome.
        exit_state: ScheduleRunState,
        /// Planned time.
        trigger_time: Instant,
        /// Time taken.
        elapsed: Option<Duration>,
        /// Journal operation of the run.
        operation: Option<u64>,
        /// Message.
        message: Option<String>,
    }
}

record_struct! {
    /// Verdict of an analysis.
    Analysis (Analysis) {
        /// Identifier, for example `an-3`.
        id: String,
        /// The analysis passed.
        passed: bool,
        /// Lowest severity that fails the analysis.
        fail_on: String,
        /// Errors found.
        errors: u64,
        /// Warnings found.
        warnings: u64,
        /// Information found.
        infos: u64,
        /// Checks performed.
        checks: Vec<AnalysisCheck>,
        /// Analysed paths.
        paths: Vec<Path>,
        /// Only changed files were analysed.
        changed_only: bool,
        /// Start time.
        started: Instant,
        /// Time taken.
        duration: Duration,
        /// Fingerprint of the state of the code.
        fingerprint: Option<String>,
        /// Share of duplicated lines in the analysed files, in percent.
        duplication_percent: Option<f64>,
        /// Journal operation of the analysis.
        op: Option<u64>,
    }
}

record_struct! {
    /// Analysis tool found in a project.
    AnalyzeTool (AnalyzeTool) {
        /// Tool.
        tool: String,
        /// Language.
        language: String,
        /// Check performed.
        check: AnalyzeCheck,
        /// The tool was found.
        found: bool,
        /// Version.
        version: Option<String>,
        /// Configuration file.
        config: Option<Path>,
        /// Command.
        command: Option<Vec<String>>,
    }
}

record_struct! {
    /// Tag of a repository.
    GitTag (GitTag) {
        /// Name.
        name: String,
        /// Identifier of the tagged object.
        target: String,
        /// Kind.
        kind: GitTagKind,
        /// Tagger.
        tagger: Option<String>,
        /// Time.
        time: Option<Instant>,
        /// Message.
        message: Option<String>,
        /// Signature.
        signature: Option<CommitSignature>,
    }
}

record_struct! {
    /// Working tree of a repository.
    GitWorktree (GitWorktree) {
        /// Directory.
        path: Path,
        /// Commit identifier of `HEAD`.
        head: String,
        /// `HEAD` is detached.
        detached: bool,
        /// Bare repository.
        bare: bool,
        /// The main working tree.
        main: bool,
        /// Branch.
        branch: Option<String>,
        /// Lock reason.
        locked: Option<String>,
        /// Can be pruned.
        prunable: Option<bool>,
    }
}

record_struct! {
    /// Stash entry.
    GitStash (GitStash) {
        /// Position, 0 for the newest.
        index: u64,
        /// Commit identifier.
        id: String,
        /// Message.
        message: String,
        /// Time.
        time: Instant,
        /// Untracked files included.
        includes_untracked: bool,
        /// Branch the entry was made on.
        branch: Option<String>,
    }
}

record_struct! {
    /// Checkpoint of a working tree.
    GitCheckpoint (GitCheckpoint) {
        /// Identifier, for example `cp-2`.
        id: String,
        /// Commit identifier.
        commit: String,
        /// Time.
        time: Instant,
        /// Reference, for example `refs/nurt/checkpoints/cp-2`.
        reference as "ref": String,
        /// The index was included.
        index_included: bool,
        /// Label.
        label: Option<String>,
        /// Number of files.
        files: Option<u64>,
    }
}

record_struct! {
    /// Update of a reference by a transfer.
    GitRefUpdate (GitRefUpdate) {
        /// Local reference.
        reference as "ref": String,
        /// Outcome.
        status: GitRefStatus,
        /// Remote.
        remote: String,
        /// Remote reference.
        remote_ref: Option<String>,
        /// Identifier before.
        old: Option<String>,
        /// Identifier after.
        new: Option<String>,
        /// Reason of a rejection, for example `non_fast_forward`.
        reason: Option<String>,
    }
}

record_struct! {
    /// Conflict left by a merge.
    GitConflict (GitConflict) {
        /// Path in the repository.
        path: String,
        /// Bytes of `path`; present only when it is not valid Unicode.
        raw_path: Option<Bytes>,
        /// Kind.
        kind: GitConflictKind,
        /// The file contains conflict markers.
        markers: bool,
        /// Object identifier of the base.
        base: Option<String>,
        /// Object identifier of the current side.
        ours: Option<String>,
        /// Object identifier of the other side.
        theirs: Option<String>,
    }
}

record_struct! {
    /// Preserved copy of a deleted branch or a dropped stash entry.
    GitTrashEntry (GitTrashEntry) {
        /// Name of the deleted branch or stash entry.
        name: String,
        /// Kind.
        kind: GitTrashKind,
        /// Reference that keeps the copy, for example `refs/nurt/trash/feature`.
        reference as "ref": String,
        /// Commit identifier.
        commit: String,
        /// Time of the deletion.
        time: Instant,
        /// First line of the message of the commit.
        subject: Option<String>,
    }
}

macro_rules! typed_records {
    ($( $variant:ident ($ty:ident) = $kind:literal ),+ $(,)?) => {
        /// Typed record of any kind.
        ///
        /// The JSON form is `{"kind": "entry", "record": {…}}`.
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
        #[serde(tag = "kind", content = "record", deny_unknown_fields)]
        #[non_exhaustive]
        pub enum TypedRecord {
            $(
                #[doc = concat!("Record of kind `", $kind, "`.")]
                #[serde(rename = $kind)]
                $variant(Box<$ty>),
            )+
        }

        impl TypedRecord {
            /// Kinds that have a typed structure, in a stable order.
            pub const KINDS: &'static [RecordKind] = &[$(RecordKind::$variant),+];

            /// Names and language types of the fields of a record kind;
            /// `None` for a kind without a typed structure.
            #[must_use]
            pub const fn field_types(kind: RecordKind) -> Option<FieldTypes> {
                match kind {
                    $(RecordKind::$variant => Some($ty::FIELD_TYPES),)+
                    _ => None,
                }
            }

            /// Kind of the record.
            #[must_use]
            pub const fn kind(&self) -> RecordKind {
                match self {
                    $(Self::$variant(_) => RecordKind::$variant,)+
                }
            }
        }

        impl From<TypedRecord> for Record {
            fn from(record: TypedRecord) -> Self {
                match record {
                    $(TypedRecord::$variant(inner) => Record::from(*inner),)+
                }
            }
        }

        impl TryFrom<Record> for TypedRecord {
            type Error = RecordError;

            fn try_from(record: Record) -> Result<Self, Self::Error> {
                match record.kind {
                    $(RecordKind::$variant => $ty::try_from(record).map(|inner| Self::$variant(Box::new(inner))),)+
                    kind => Err(RecordError::Untyped { kind }),
                }
            }
        }

        $(
            impl From<$ty> for TypedRecord {
                fn from(record: $ty) -> Self {
                    Self::$variant(Box::new(record))
                }
            }
        )+
    };
}

typed_records! {
    Entry(Entry) = "entry",
    Content(Content) = "content",
    Line(Line) = "line",
    Match(Match) = "match",
    Item(Item) = "item",
    Row(Row) = "row",
    Node(Node) = "node",
    Symbol(Symbol) = "symbol",
    Reference(Reference) = "reference",
    Diagnostic(Diagnostic) = "diagnostic",
    Cell(Cell) = "cell",
    Hunk(Hunk) = "hunk",
    Change(Change) = "change",
    Exec(Exec) = "exec",
    Job(Job) = "job",
    Process(Process) = "process",
    Port(Port) = "port",
    Disk(Disk) = "disk",
    Host(HostInfo) = "host",
    EnvironmentVariable(EnvironmentVariable) = "environment_variable",
    Service(Service) = "service",
    Package(Package) = "package",
    Account(Account) = "account",
    LogEvent(LogEvent) = "log_event",
    Commit(Commit) = "commit",
    GitStatus(GitStatus) = "git_status",
    Branch(Branch) = "branch",
    Schedule(Schedule) = "schedule",
    Operation(Operation) = "operation",
    Response(HttpResponse) = "response",
    Volume(Volume) = "volume",
    ProgramCard(ProgramCard) = "program_card",
    ArchiveEntry(ArchiveEntry) = "archive_entry",
    TestRun(TestRun) = "test_run",
    TestResult(TestResult) = "test_result",
    Finding(Finding) = "finding",
    Effect(Effect) = "effect",
    RegistryValue(RegistryValue) = "registry_value",
    JournalState(JournalState) = "journal_state",
    Value(Evaluation) = "value",
    Group(Group) = "group",
    Summary(FieldSummary) = "summary",
    FieldInfo(FieldInfo) = "field_info",
    CimObject(CimObject) = "cim_object",
    BlameLine(BlameLine) = "blame_line",
    Hover(Hover) = "hover",
    CodeAction(CodeAction) = "code_action",
    Language(Language) = "language",
    RenameEdit(RenameEdit) = "rename_edit",
    ProfileEntry(ProfileEntry) = "profile_entry",
    PingResult(PingResult) = "ping_result",
    DnsRecord(DnsRecord) = "dns_record",
    NetworkInterface(NetworkInterface) = "network_interface",
    ProgramLocation(ProgramLocation) = "program_location",
    ProgramCategory(ProgramCategory) = "program_category",
    ScheduleRun(ScheduleRun) = "schedule_run",
    Analysis(Analysis) = "analysis",
    AnalyzeTool(AnalyzeTool) = "analyze_tool",
    GitTag(GitTag) = "git_tag",
    GitWorktree(GitWorktree) = "git_worktree",
    GitStash(GitStash) = "git_stash",
    GitCheckpoint(GitCheckpoint) = "git_checkpoint",
    GitRefUpdate(GitRefUpdate) = "git_ref_update",
    GitConflict(GitConflict) = "git_conflict",
    GitEffect(GitEffect) = "git_effect",
    GitTrashEntry(GitTrashEntry) = "git_trash_entry",
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            path: Path::from("/home/user/a.txt"),
            name: "a.txt".to_owned(),
            kind: EntryType::File,
            size: Size::from_bytes(3),
            modified: Instant::UNIX_EPOCH,
            ..Entry::default()
        }
    }

    fn field_error(record: Record) -> RecordError {
        Entry::try_from(record).unwrap_err()
    }

    #[test]
    fn name_that_is_not_unicode_survives_as_raw_bytes_in_records_and_json() {
        let raw = Bytes::new(vec![b'z', 0xff, b'.', b't', b'x', b't']);
        let source = Entry {
            raw_name: Some(raw.clone()),
            ..entry()
        };
        let record = Record::from(source.clone());
        assert_eq!(record.get("raw_name"), Some(&Value::Bytes(raw.clone())));
        assert_eq!(Entry::try_from(record).unwrap(), source);
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json["raw_name"], serde_json::json!(raw.to_base64()));
        assert_eq!(serde_json::from_value::<Entry>(json).unwrap(), source);
    }

    #[test]
    fn conversion_reports_missing_unknown_and_mistyped_fields() {
        let missing = field_error(Record::new(RecordKind::Entry));
        assert_eq!(
            missing,
            RecordError::MissingField {
                field: "path".to_owned()
            }
        );
        let mut record = Record::from(entry());
        record.set("bogus", Value::Int(1));
        assert_eq!(
            field_error(record),
            RecordError::UnknownField {
                field: "bogus".to_owned()
            }
        );
        let mut record = Record::from(entry());
        record.set("size", Value::Text("big".to_owned()));
        assert_eq!(
            field_error(record),
            RecordError::FieldType {
                field: "size".to_owned(),
                expected: "size".to_owned(),
                found: "text".to_owned()
            }
        );
        let mut record = Record::from(entry());
        record.set("modified", Value::Text("yesterday".to_owned()));
        assert!(matches!(field_error(record), RecordError::FieldType { .. }));
        assert_eq!(
            Entry::try_from(Record::new(RecordKind::Line)).unwrap_err(),
            RecordError::KindMismatch {
                expected: RecordKind::Entry,
                found: RecordKind::Line
            }
        );
    }

    #[test]
    fn errors_inside_nested_records_carry_the_full_field_path() {
        let record = Record::new(RecordKind::Effect)
            .with("status", Value::Enum("ok".to_owned()))
            .with("kind", Value::Text("replace".to_owned()))
            .with(
                "changes",
                Value::List(vec![Value::Record(Record::new(RecordKind::Change))]),
            );
        assert_eq!(
            Effect::try_from(record).unwrap_err(),
            RecordError::MissingField {
                field: "changes.0.path".to_owned()
            }
        );
    }

    #[test]
    fn field_types_name_the_language_type_of_every_field() {
        let entry = TypedRecord::field_types(RecordKind::Entry).unwrap();
        let type_of = |name: &str| {
            entry
                .iter()
                .find(|(field, _)| *field == name)
                .map(|(_, ty)| *ty)
        };
        assert_eq!(type_of("size"), Some("size"));
        assert_eq!(type_of("modified"), Some("instant"));
        assert_eq!(type_of("created"), Some("instant"));
        assert_eq!(type_of("exists"), Some("bool"));
        assert_eq!(type_of("path"), Some("path"));
        assert_eq!(type_of("name"), Some("text"));
        assert_eq!(type_of("links"), Some("int"));
        assert_eq!(type_of("type"), Some("enum"));
        assert_eq!(type_of("no_such_field"), None);
        let job = TypedRecord::field_types(RecordKind::Job).unwrap();
        assert!(job.contains(&("elapsed", "duration")));
        assert!(job.contains(&("command", "list")));
        assert_eq!(TypedRecord::field_types(RecordKind::Map), None);
    }

    #[test]
    fn every_typed_kind_has_field_types() {
        for kind in TypedRecord::KINDS {
            assert!(TypedRecord::field_types(*kind).is_some(), "{kind}");
        }
    }

    #[test]
    fn counts_above_the_signed_range_use_big_integers_and_negative_counts_are_rejected() {
        let source = Entry {
            links: Some(u64::MAX),
            ..entry()
        };
        let record = Record::from(source.clone());
        assert_eq!(
            record.get("links"),
            Some(&Value::BigInt(u64::MAX.to_string()))
        );
        assert_eq!(Entry::try_from(record).unwrap(), source);
        let mut record = Record::from(entry());
        record.set("links", Value::Int(-1));
        assert_eq!(
            field_error(record),
            RecordError::FieldType {
                field: "links".to_owned(),
                expected: "count".to_owned(),
                found: "int".to_owned()
            }
        );
    }

    #[test]
    fn sizes_accept_non_negative_integers_and_null_means_absent() {
        let mut record = Record::from(entry());
        record.set("size", Value::Int(7));
        record.set("exists", Value::Null);
        let read = Entry::try_from(record).unwrap();
        assert_eq!(read.size, Size::from_bytes(7));
        assert_eq!(read.exists, None);
        let mut record = Record::from(entry());
        record.set("size", Value::Int(-1));
        assert!(matches!(field_error(record), RecordError::FieldType { .. }));
        assert_eq!(Record::from(entry()).get("stem"), None);
    }

    #[test]
    fn typed_records_cover_every_kind_except_map_and_round_trip_through_json() {
        assert_eq!(TypedRecord::KINDS.len() + 1, RecordKind::ALL.len());
        assert!(!TypedRecord::KINDS.contains(&RecordKind::Map));
        let typed = TypedRecord::from(entry());
        assert_eq!(typed.kind(), RecordKind::Entry);
        let json = serde_json::to_value(&typed).unwrap();
        assert_eq!(json["kind"], "entry");
        assert_eq!(json["record"]["type"], "file");
        assert_eq!(serde_json::from_value::<TypedRecord>(json).unwrap(), typed);
        assert_eq!(
            TypedRecord::try_from(Record::new(RecordKind::Map)).unwrap_err(),
            RecordError::Untyped {
                kind: RecordKind::Map
            }
        );
        assert_eq!(Record::from(typed), Record::from(entry()));
    }

    #[test]
    fn enumerations_parse_their_stable_names_only() {
        assert_eq!(
            "directory".parse::<EntryType>().unwrap(),
            EntryType::Directory
        );
        assert!("dir".parse::<EntryType>().is_err());
        assert!("File".parse::<EntryType>().is_err());
        assert_eq!(EntryType::ALL.len(), 6);
        assert_eq!("fifo".parse::<EntryType>().unwrap(), EntryType::Fifo);
        assert_eq!("device".parse::<EntryType>().unwrap(), EntryType::Device);
        assert_eq!(RegistryView::Bits32.as_str(), "32");
        assert_eq!("64".parse::<RegistryView>().unwrap(), RegistryView::Bits64);
    }

    #[test]
    fn record_errors_map_to_the_codes_of_the_contract() {
        let unknown = Error::from(RecordError::UnknownField {
            field: "x".to_owned(),
        });
        assert_eq!(unknown.code(), ErrorCode::UnknownField);
        let missing = Error::from(RecordError::MissingField {
            field: "x".to_owned(),
        });
        assert_eq!(missing.code(), ErrorCode::Type);
    }

    #[test]
    fn hunk_lines_need_a_diff_prefix() {
        let valid = Hunk {
            lines: vec!["+a".to_owned(), "-b".to_owned(), " c".to_owned()],
            ..Hunk::default()
        };
        assert_eq!(valid.first_invalid_line(), None);
        let invalid = Hunk {
            lines: vec![" c".to_owned(), String::new()],
            ..Hunk::default()
        };
        assert_eq!(invalid.first_invalid_line(), Some(1));
    }

    #[test]
    fn hunk_lines_admit_the_no_newline_marker_only_in_full() {
        let marked = Hunk {
            lines: vec![
                "-a".to_owned(),
                NO_NEWLINE_MARKER.to_owned(),
                "+a".to_owned(),
            ],
            ..Hunk::default()
        };
        assert_eq!(marked.first_invalid_line(), None);
        let altered = Hunk {
            lines: vec!["\\ No newline".to_owned()],
            ..Hunk::default()
        };
        assert_eq!(altered.first_invalid_line(), Some(0));
    }
}
