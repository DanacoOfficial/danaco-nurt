// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Configuration `nurt-config/1`: keys, defaults, levels, precedence and trust.
//!
//! A configuration is assembled from seven levels, from the weakest to the
//! strongest (see [`ConfigLevel`]): built-in defaults, the system file, the
//! user file, the project file `.nurt.toml`, the file named by `NURT_CONFIG`,
//! `NURT_*` environment variables and run-time settings. A higher level
//! overrides a lower one key by key: tables merge, scalars and arrays are
//! replaced as a whole.
//!
//! A project file comes from a repository and is not trusted. It may set only
//! `scope.base`, `scope.project_roots`, `report.*` and `run.*`; `journal.*` is
//! honoured only for projects listed in `scope.trusted_projects`. Trust keys
//! (`scope.trust*`) and `service.*` in a project file are ignored and reported
//! as `trust_config_ignored`, so that a repository can never trust itself. The
//! rule is the pure function [`allowed_at`]; [`resolve`] applies it while
//! merging the levels and records the level that set every key.
//!
//! Protection floors (`journal.floor.*`) are configuration keys like any
//! other; raising lower limits to the floor is part of the retention rules of
//! the journal, which receives the effective values together with their
//! levels from [`Resolved::source`].
//!
//! # Examples
//!
//! ```
//! use nurt_contracts::config::{Config, ConfigLayer, ConfigLevel, resolve};
//!
//! let user: serde_json::Map<String, serde_json::Value> =
//!     serde_json::from_str(r#"{"journal": {"max_ops": 50}}"#).unwrap();
//! let layers = [ConfigLayer::new(ConfigLevel::User, "nurt.toml", user)];
//! let resolved = resolve(&layers, false).unwrap();
//!
//! assert_eq!(resolved.config.journal.max_ops.to_string(), "50");
//! assert_eq!(resolved.source("journal.max_ops"), ConfigLevel::User);
//! assert_eq!(resolved.source("journal.max_age"), ConfigLevel::Builtin);
//! assert_eq!(Config::default().journal.max_ops.to_string(), "30");
//! ```

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value as Json, json};

use crate::envelope::Privilege;
use crate::kinds::SyntaxVariant;

/// Format identifier of configuration files.
pub const CONFIG_FORMAT: &str = "nurt-config/1";

/// File name of the system and user configuration files.
pub const CONFIG_FILE_NAME: &str = "nurt.toml";

/// File name of the project configuration file.
pub const PROJECT_FILE_NAME: &str = ".nurt.toml";

/// Environment variable naming an additional configuration file.
pub const CONFIG_FILE_VAR: &str = "NURT_CONFIG";

/// Separator of list values in environment variables (`:`; `;` on Windows).
pub const ENV_LIST_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// Lowest step limit applied to values that come from a project file, even
/// when no explicit floor is configured.
pub const PROJECT_FLOOR_MAX_OPS: StepLimit = StepLimit::Limited(10);

/// Lowest age limit applied to values that come from a project file, even
/// when no explicit floor is configured (five minutes).
pub const PROJECT_FLOOR_MAX_AGE: AgeLimit =
    AgeLimit::Limited(DurationSetting::from_millis(5 * 60 * 1000));

/// Keys a project file may set (`*` stands for every key of the section).
pub const PROJECT_KEYS: &[&str] = &["scope.base", "scope.project_roots", "report.*", "run.*"];

/// Keys a project file may set only when the project is trusted.
pub const TRUSTED_PROJECT_KEYS: &[&str] = &["journal.*"];

/// Keys that a project file can never set; they are ignored and reported as
/// `trust_config_ignored`. `scope.trust*` is also refused at the run-time
/// level.
pub const TRUST_KEYS: &[&str] = &["scope.trust*", "service.*", "privilege.*"];

/// Tables whose keys are chosen by the user (names of variables, languages,
/// mount points) rather than fixed by the format.
pub const OPEN_TABLES: &[&str] = &["run.env_defaults", "code.servers", "journal.volume"];

/// Error of parsing a configuration value from its text form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingParseError {
    /// Description of the problem, with the accepted forms.
    pub message: String,
}

impl SettingParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SettingParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SettingParseError {}

/// Value read by the deserializers of the text-based setting types.
enum Raw {
    Number(i128),
    Text(String),
}

struct RawVisitor;

impl Visitor<'_> for RawVisitor {
    type Value = Raw;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a number or a string")
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Raw, E> {
        Ok(Raw::Number(i128::from(value)))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Raw, E> {
        Ok(Raw::Number(i128::from(value)))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Raw, E> {
        Ok(Raw::Text(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Raw, E> {
        Ok(Raw::Text(value))
    }
}

/// Implements `Deserialize` through `from_number` and `FromStr`.
macro_rules! deserialize_number_or_text {
    ($ty:ty) => {
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                match deserializer.deserialize_any(RawVisitor)? {
                    Raw::Number(number) => <$ty>::from_number(number),
                    Raw::Text(text) => text.parse::<$ty>(),
                }
                .map_err(de::Error::custom)
            }
        }
    };
}

/// Implements `Serialize` through `Display`.
macro_rules! serialize_as_text {
    ($ty:ty) => {
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
    };
}

/// Implements `JsonSchema` with a fixed schema.
macro_rules! fixed_schema {
    ($ty:ty, $name:literal, $schema:expr) => {
        impl JsonSchema for $ty {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                std::borrow::Cow::Borrowed($name)
            }

            fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
                schema_of($schema)
            }
        }
    };
}

fn schema_of(value: Json) -> Schema {
    match value {
        Json::Object(map) => Schema::from(map),
        _ => Schema::from(true),
    }
}

const DURATION_PATTERN: &str = "^([0-9]+(ms|s|m|h|d|w))+$";
const SIZE_PATTERN: &str = "^[0-9]+(\\.[0-9]+)?(b|kb|mb|gb|tb|kib|mib|gib|tib)?$";
const PERCENT_PATTERN: &str = "^[0-9]+(\\.[0-9]{1,2})?%$";
const SIZE_OR_PERCENT_PATTERN: &str =
    "^([0-9]+(\\.[0-9]{1,2})?%|[0-9]+(\\.[0-9]+)?(b|kb|mb|gb|tb|kib|mib|gib|tib)?)$";
const SIZE_LIMIT_PATTERN: &str = "^(unlimited|[0-9]+(\\.[0-9]{1,2})?%(free)?|[0-9]+(\\.[0-9]+)?(b|kb|mb|gb|tb|kib|mib|gib|tib)?)$";
const BYTE_LIMIT_PATTERN: &str = "^(unlimited|[0-9]+(\\.[0-9]+)?(b|kb|mb|gb|tb|kib|mib|gib|tib)?)$";
const AGE_LIMIT_PATTERN: &str = "^(unlimited|([0-9]+(ms|s|m|h|d|w))+)$";

const UNLIMITED: &str = "unlimited";

/// Largest number of digits after the decimal point accepted in a size.
const MAX_SIZE_FRACTION_DIGITS: usize = 18;

/// Number of hundredths in one percent, the unit in which [`Percent`] is
/// stored.
const HUNDREDTHS_PER_PERCENT: u32 = 100;

/// Largest number of decimal places of a percentage.
const PERCENT_DECIMALS: usize = 2;

const DURATION_UNITS: [(&str, u64); 6] = [
    ("w", 7 * 24 * 3_600_000),
    ("d", 24 * 3_600_000),
    ("h", 3_600_000),
    ("m", 60_000),
    ("s", 1_000),
    ("ms", 1),
];

const SIZE_UNITS: [(&str, u64); 8] = [
    ("tib", 1 << 40),
    ("gib", 1 << 30),
    ("mib", 1 << 20),
    ("kib", 1 << 10),
    ("tb", 1_000_000_000_000),
    ("gb", 1_000_000_000),
    ("mb", 1_000_000),
    ("kb", 1_000),
];

/// Duration written with units, as in the language: `ms`, `s`, `m`, `h`, `d`
/// and `w`, also combined (`1h30m`). Stored in milliseconds and written back
/// with the largest unit that represents it exactly (`90s`, `30m`, `1d`).
///
/// # Examples
///
/// ```
/// use nurt_contracts::config::DurationSetting;
///
/// let wait: DurationSetting = "1h30m".parse().unwrap();
/// assert_eq!(wait.as_millis(), 5_400_000);
/// assert_eq!(wait.to_string(), "90m");
/// assert!("30".parse::<DurationSetting>().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct DurationSetting {
    millis: u64,
}

impl DurationSetting {
    /// Duration of the given number of milliseconds.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self { millis }
    }

    /// Duration of the given number of seconds.
    #[must_use]
    pub const fn from_secs(secs: u64) -> Self {
        Self {
            millis: secs.saturating_mul(1000),
        }
    }

    /// Length in milliseconds.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.millis
    }

    /// Length as a standard library duration.
    #[must_use]
    pub const fn as_std(self) -> std::time::Duration {
        std::time::Duration::from_millis(self.millis)
    }

    fn from_number(_number: i128) -> Result<Self, SettingParseError> {
        Err(SettingParseError::new(
            "a duration needs a unit: ms, s, m, h, d or w (for example \"30s\")",
        ))
    }
}

impl From<DurationSetting> for std::time::Duration {
    fn from(value: DurationSetting) -> Self {
        value.as_std()
    }
}

impl FromStr for DurationSetting {
    type Err = SettingParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_duration_millis(text).map(Self::from_millis)
    }
}

impl fmt::Display for DurationSetting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.millis == 0 {
            return f.write_str("0s");
        }
        let (unit, factor) = DURATION_UNITS
            .iter()
            .copied()
            .find(|(_, factor)| self.millis.is_multiple_of(*factor))
            .unwrap_or(("ms", 1));
        write!(f, "{}{unit}", self.millis / factor)
    }
}

serialize_as_text!(DurationSetting);
deserialize_number_or_text!(DurationSetting);
fixed_schema!(
    DurationSetting,
    "DurationSetting",
    json!({
        "description": "Duration with units ms, s, m, h, d or w, for example \"30m\", \"1500ms\" or \"1h30m\".",
        "type": "string",
        "pattern": DURATION_PATTERN
    })
);

fn parse_duration_millis(text: &str) -> Result<u64, SettingParseError> {
    let invalid = || {
        SettingParseError::new(format!(
            "invalid duration \"{text}\": expected digits followed by ms, s, m, h, d or w, for example \"30s\" or \"1h30m\""
        ))
    };
    if text.is_empty() {
        return Err(invalid());
    }
    let mut total: u64 = 0;
    let mut rest = text;
    while !rest.is_empty() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let (number, tail) = rest.split_at(digits);
        let letters = tail.bytes().take_while(u8::is_ascii_lowercase).count();
        let (unit, next) = tail.split_at(letters);
        let factor = DURATION_UNITS
            .iter()
            .find(|(name, _)| *name == unit)
            .map(|(_, factor)| *factor)
            .ok_or_else(invalid)?;
        let value: u64 = number.parse().map_err(|_| invalid())?;
        total = value
            .checked_mul(factor)
            .and_then(|part| total.checked_add(part))
            .ok_or_else(invalid)?;
        rest = next;
    }
    Ok(total)
}

/// Size in bytes written with a unit: decimal `b`, `kb`, `mb`, `gb`, `tb`
/// (powers of 1000) or binary `kib`, `mib`, `gib`, `tib` (powers of 1024);
/// a fraction is allowed (`1.5tb`) and a bare integer means bytes. Written
/// back with the largest unit that represents it exactly.
///
/// # Examples
///
/// ```
/// use nurt_contracts::config::SizeSetting;
///
/// let size: SizeSetting = "2gib".parse().unwrap();
/// assert_eq!(size.bytes(), 2 * 1024 * 1024 * 1024);
/// assert_eq!("1.5kb".parse::<SizeSetting>().unwrap().bytes(), 1500);
/// assert_eq!(SizeSetting::from_bytes(1 << 20).to_string(), "1mib");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct SizeSetting {
    bytes: u64,
}

impl SizeSetting {
    /// Size of the given number of bytes.
    #[must_use]
    pub const fn from_bytes(bytes: u64) -> Self {
        Self { bytes }
    }

    /// Size of the given number of binary gibibytes.
    #[must_use]
    pub const fn from_gib(gib: u64) -> Self {
        Self {
            bytes: gib.saturating_mul(1 << 30),
        }
    }

    /// Size of the given number of binary mebibytes.
    #[must_use]
    pub const fn from_mib(mib: u64) -> Self {
        Self {
            bytes: mib.saturating_mul(1 << 20),
        }
    }

    /// Size in bytes.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    fn from_number(number: i128) -> Result<Self, SettingParseError> {
        u64::try_from(number)
            .map(Self::from_bytes)
            .map_err(|_| SettingParseError::new(format!("size {number} is out of range")))
    }
}

impl FromStr for SizeSetting {
    type Err = SettingParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_size_bytes(text).map(Self::from_bytes)
    }
}

impl fmt::Display for SizeSetting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unit = SIZE_UNITS
            .iter()
            .copied()
            .filter(|(_, factor)| self.bytes != 0 && self.bytes.is_multiple_of(*factor))
            .max_by_key(|(_, factor)| *factor);
        match unit {
            Some((name, factor)) => write!(f, "{}{name}", self.bytes / factor),
            None => write!(f, "{}b", self.bytes),
        }
    }
}

serialize_as_text!(SizeSetting);
deserialize_number_or_text!(SizeSetting);
fixed_schema!(
    SizeSetting,
    "SizeSetting",
    json!({
        "description": "Size with a unit b, kb, mb, gb, tb (powers of 1000) or kib, mib, gib, tib (powers of 1024), for example \"2gib\"; an integer means bytes.",
        "anyOf": [
            { "type": "string", "pattern": SIZE_PATTERN },
            { "type": "integer", "minimum": 0 }
        ]
    })
);

fn parse_size_bytes(text: &str) -> Result<u64, SettingParseError> {
    let invalid = || {
        SettingParseError::new(format!(
            "invalid size \"{text}\": expected a number with b, kb, mb, gb, tb, kib, mib, gib or tib, for example \"2gib\""
        ))
    };
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let factor = if unit.is_empty() {
        1
    } else {
        SIZE_UNITS
            .iter()
            .chain(std::iter::once(&("b", 1)))
            .find(|(name, _)| *name == unit)
            .map(|(_, factor)| *factor)
            .ok_or_else(invalid)?
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let all_digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty()
        || !all_digits(whole)
        || !all_digits(fraction)
        || (number.contains('.') && fraction.is_empty())
        || fraction.len() > MAX_SIZE_FRACTION_DIGITS
    {
        return Err(invalid());
    }
    let whole: u128 = whole.parse().map_err(|_| invalid())?;
    let fraction_value: u128 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().map_err(|_| invalid())?
    };
    let exponent = u32::try_from(fraction.len()).map_err(|_| invalid())?;
    let scale = 10_u128.pow(exponent);
    let bytes = whole
        .checked_mul(scale)
        .and_then(|scaled| scaled.checked_add(fraction_value))
        .and_then(|scaled| scaled.checked_mul(u128::from(factor)))
        .map(|scaled| scaled / scale)
        .ok_or_else(invalid)?;
    u64::try_from(bytes).map_err(|_| invalid())
}

/// Percentage with up to two decimal places, stored in hundredths of a
/// percent (`1500` is 15 %), written as `15%` or `12.5%`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::config::Percent;
///
/// let share: Percent = "12.5%".parse().unwrap();
/// assert_eq!(share.hundredths(), 1250);
/// assert_eq!(share.to_string(), "12.5%");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Percent {
    hundredths: u32,
}

impl Percent {
    /// Percentage of the given number of whole percent.
    #[must_use]
    pub const fn from_percent(percent: u32) -> Self {
        Self {
            hundredths: percent.saturating_mul(HUNDREDTHS_PER_PERCENT),
        }
    }

    /// Percentage in hundredths of a percent.
    #[must_use]
    pub const fn hundredths(self) -> u32 {
        self.hundredths
    }

    fn in_range(self, min: u32, max: u32, what: &str) -> Result<Self, SettingParseError> {
        if self.hundredths >= min * HUNDREDTHS_PER_PERCENT
            && self.hundredths <= max * HUNDREDTHS_PER_PERCENT
        {
            Ok(self)
        } else {
            Err(SettingParseError::new(format!(
                "{what} {self} is out of range {min}%..{max}%"
            )))
        }
    }
}

impl FromStr for Percent {
    type Err = SettingParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || {
            SettingParseError::new(format!(
                "invalid percentage \"{text}\": expected for example \"15%\" or \"12.5%\""
            ))
        };
        let number = text.strip_suffix('%').ok_or_else(invalid)?;
        let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
        let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        if !digits(whole)
            || (number.contains('.') && !digits(fraction))
            || fraction.len() > PERCENT_DECIMALS
        {
            return Err(invalid());
        }
        let whole: u32 = whole.parse().map_err(|_| invalid())?;
        let fraction_value: u32 = match fraction.len() {
            0 => 0,
            1 => fraction.parse::<u32>().map_err(|_| invalid())? * 10,
            _ => fraction.parse().map_err(|_| invalid())?,
        };
        let hundredths = whole
            .checked_mul(HUNDREDTHS_PER_PERCENT)
            .and_then(|value| value.checked_add(fraction_value))
            .ok_or_else(invalid)?;
        Ok(Self { hundredths }).and_then(|value| value.in_range(0, 100, "percentage"))
    }
}

impl fmt::Display for Percent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.hundredths / HUNDREDTHS_PER_PERCENT;
        let rest = self.hundredths % HUNDREDTHS_PER_PERCENT;
        if rest == 0 {
            write!(f, "{whole}%")
        } else if rest.is_multiple_of(10) {
            write!(f, "{whole}.{}%", rest / 10)
        } else {
            write!(f, "{whole}.{rest:02}%")
        }
    }
}

impl Percent {
    fn from_number(_number: i128) -> Result<Self, SettingParseError> {
        Err(SettingParseError::new(
            "a percentage is written as a string, for example \"15%\"",
        ))
    }
}

serialize_as_text!(Percent);
deserialize_number_or_text!(Percent);
fixed_schema!(
    Percent,
    "Percent",
    json!({
        "description": "Percentage with up to two decimal places, for example \"15%\".",
        "type": "string",
        "pattern": PERCENT_PATTERN
    })
);

/// Absolute size or percentage of the volume capacity (space reserve and
/// thresholds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeOrPercent {
    /// Absolute size.
    Size(SizeSetting),
    /// Percentage of the volume capacity.
    Percent(Percent),
}

impl SizeOrPercent {
    fn from_number(number: i128) -> Result<Self, SettingParseError> {
        SizeSetting::from_number(number).map(Self::Size)
    }
}

impl FromStr for SizeOrPercent {
    type Err = SettingParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.ends_with('%') {
            text.parse().map(Self::Percent)
        } else {
            text.parse().map(Self::Size)
        }
    }
}

impl fmt::Display for SizeOrPercent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size(size) => size.fmt(f),
            Self::Percent(percent) => percent.fmt(f),
        }
    }
}

serialize_as_text!(SizeOrPercent);
deserialize_number_or_text!(SizeOrPercent);
fixed_schema!(
    SizeOrPercent,
    "SizeOrPercent",
    json!({
        "description": "Size (for example \"5gib\") or percentage of the volume capacity (for example \"15%\").",
        "anyOf": [
            { "type": "string", "pattern": SIZE_OR_PERCENT_PATTERN },
            { "type": "integer", "minimum": 0 }
        ]
    })
);

/// Step limit of the journal: the number of committed changing operations,
/// `1..=100000`, or `unlimited`. Written as an integer or the word
/// `unlimited`; `0` is not accepted.
///
/// # Examples
///
/// ```
/// use nurt_contracts::config::StepLimit;
///
/// assert_eq!("30".parse::<StepLimit>().unwrap(), StepLimit::Limited(30));
/// assert_eq!("unlimited".parse::<StepLimit>().unwrap(), StepLimit::Unlimited);
/// assert!("0".parse::<StepLimit>().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepLimit {
    /// Limit in steps, `1..=100000`.
    Limited(u32),
    /// No step limit.
    Unlimited,
}

impl StepLimit {
    /// Largest accepted step limit.
    pub const MAX: u32 = 100_000;

    fn from_number(number: i128) -> Result<Self, SettingParseError> {
        match u32::try_from(number) {
            Ok(steps) if (1..=Self::MAX).contains(&steps) => Ok(Self::Limited(steps)),
            _ => Err(SettingParseError::new(format!(
                "step limit {number} is out of range 1..{}; use \"unlimited\" for no limit",
                Self::MAX
            ))),
        }
    }
}

impl FromStr for StepLimit {
    type Err = SettingParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == UNLIMITED {
            return Ok(Self::Unlimited);
        }
        let number: i128 = text.parse().map_err(|_| {
            SettingParseError::new(format!(
                "invalid step limit \"{text}\": expected an integer 1..{} or \"unlimited\"",
                Self::MAX
            ))
        })?;
        Self::from_number(number)
    }
}

impl fmt::Display for StepLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limited(steps) => write!(f, "{steps}"),
            Self::Unlimited => f.write_str(UNLIMITED),
        }
    }
}

impl Serialize for StepLimit {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Limited(steps) => serializer.serialize_u32(*steps),
            Self::Unlimited => serializer.serialize_str(UNLIMITED),
        }
    }
}

deserialize_number_or_text!(StepLimit);
fixed_schema!(
    StepLimit,
    "StepLimit",
    json!({
        "description": "Number of steps 1..100000, or \"unlimited\".",
        "anyOf": [
            { "type": "integer", "minimum": 1, "maximum": StepLimit::MAX },
            { "const": UNLIMITED }
        ]
    })
);

/// Age limit of the journal: a duration in the range `10s..=365d`, or
/// `unlimited`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::config::AgeLimit;
///
/// assert_eq!("30m".parse::<AgeLimit>().unwrap().to_string(), "30m");
/// assert!("5s".parse::<AgeLimit>().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgeLimit {
    /// Limit, `10s..=365d`.
    Limited(DurationSetting),
    /// No age limit.
    Unlimited,
}

impl AgeLimit {
    /// Shortest accepted age limit (ten seconds).
    pub const MIN: DurationSetting = DurationSetting::from_secs(10);
    /// Longest accepted age limit (365 days).
    pub const MAX: DurationSetting = DurationSetting::from_secs(365 * 24 * 3600);

    fn from_number(number: i128) -> Result<Self, SettingParseError> {
        DurationSetting::from_number(number).map(Self::Limited)
    }
}

impl FromStr for AgeLimit {
    type Err = SettingParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == UNLIMITED {
            return Ok(Self::Unlimited);
        }
        let age: DurationSetting = text.parse()?;
        if age < Self::MIN || age > Self::MAX {
            return Err(SettingParseError::new(format!(
                "age limit {age} is out of range {}..{}; use \"unlimited\" for no limit",
                Self::MIN,
                Self::MAX
            )));
        }
        Ok(Self::Limited(age))
    }
}

impl fmt::Display for AgeLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limited(age) => age.fmt(f),
            Self::Unlimited => f.write_str(UNLIMITED),
        }
    }
}

serialize_as_text!(AgeLimit);
deserialize_number_or_text!(AgeLimit);
fixed_schema!(
    AgeLimit,
    "AgeLimit",
    json!({
        "description": "Duration 10s..365d (for example \"30m\"), or \"unlimited\".",
        "type": "string",
        "pattern": AGE_LIMIT_PATTERN
    })
);

/// Size budget of the journal on one volume: absolute size (at least 1 MiB),
/// percentage of the capacity (`1%..=90%`), percentage of the free space
/// including journal data (`1%free..=100%free`), or `unlimited`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::config::{Percent, SizeLimit};
///
/// assert_eq!("10%".parse::<SizeLimit>().unwrap(), SizeLimit::OfCapacity(Percent::from_percent(10)));
/// assert_eq!("50%free".parse::<SizeLimit>().unwrap().to_string(), "50%free");
/// assert!("95%".parse::<SizeLimit>().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeLimit {
    /// Absolute logical bytes.
    Bytes(SizeSetting),
    /// Percentage of the volume capacity, clamped to
    /// `journal.max_size_min..=journal.max_size_max`.
    OfCapacity(Percent),
    /// Percentage of the free space including journal data, clamped like
    /// [`SizeLimit::OfCapacity`].
    OfFree(Percent),
    /// No size budget; only the space reserve limits the journal.
    Unlimited,
}

impl SizeLimit {
    /// Smallest accepted absolute budget (1 MiB).
    pub const MIN_BYTES: u64 = 1 << 20;

    /// Smallest share of the capacity or of the free space, in percent.
    const SHARE_MIN_PERCENT: u32 = 1;

    /// Largest share of the volume capacity, in percent.
    const CAPACITY_SHARE_MAX_PERCENT: u32 = 90;

    /// Largest share of the free space, in percent.
    const FREE_SHARE_MAX_PERCENT: u32 = 100;

    fn from_number(number: i128) -> Result<Self, SettingParseError> {
        SizeSetting::from_number(number).and_then(Self::bytes)
    }

    fn bytes(size: SizeSetting) -> Result<Self, SettingParseError> {
        if size.bytes() < Self::MIN_BYTES {
            return Err(SettingParseError::new(format!(
                "size budget {size} is below the minimum {}",
                SizeSetting::from_bytes(Self::MIN_BYTES)
            )));
        }
        Ok(Self::Bytes(size))
    }
}

impl FromStr for SizeLimit {
    type Err = SettingParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == UNLIMITED {
            Ok(Self::Unlimited)
        } else if let Some(percent) = text.strip_suffix("free") {
            let share: Percent = percent.parse()?;
            share
                .in_range(
                    Self::SHARE_MIN_PERCENT,
                    Self::FREE_SHARE_MAX_PERCENT,
                    "free space share",
                )
                .map(Self::OfFree)
        } else if text.ends_with('%') {
            let share: Percent = text.parse()?;
            share
                .in_range(
                    Self::SHARE_MIN_PERCENT,
                    Self::CAPACITY_SHARE_MAX_PERCENT,
                    "capacity share",
                )
                .map(Self::OfCapacity)
        } else {
            text.parse().and_then(Self::bytes)
        }
    }
}

impl fmt::Display for SizeLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes(size) => size.fmt(f),
            Self::OfCapacity(percent) => percent.fmt(f),
            Self::OfFree(percent) => write!(f, "{percent}free"),
            Self::Unlimited => f.write_str(UNLIMITED),
        }
    }
}

serialize_as_text!(SizeLimit);
deserialize_number_or_text!(SizeLimit);
fixed_schema!(
    SizeLimit,
    "SizeLimit",
    json!({
        "description": "Size (for example \"20gib\"), percentage of the volume capacity (\"10%\"), percentage of the free space (\"50%free\") or \"unlimited\".",
        "anyOf": [
            { "type": "string", "pattern": SIZE_LIMIT_PATTERN },
            { "type": "integer", "minimum": SizeLimit::MIN_BYTES }
        ]
    })
);

/// Size limit that can be switched off: absolute size or `unlimited`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ByteLimit {
    /// Absolute size.
    Bytes(SizeSetting),
    /// No limit.
    Unlimited,
}

impl ByteLimit {
    fn from_number(number: i128) -> Result<Self, SettingParseError> {
        SizeSetting::from_number(number).map(Self::Bytes)
    }
}

impl FromStr for ByteLimit {
    type Err = SettingParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == UNLIMITED {
            Ok(Self::Unlimited)
        } else {
            text.parse().map(Self::Bytes)
        }
    }
}

impl fmt::Display for ByteLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes(size) => size.fmt(f),
            Self::Unlimited => f.write_str(UNLIMITED),
        }
    }
}

serialize_as_text!(ByteLimit);
deserialize_number_or_text!(ByteLimit);
fixed_schema!(
    ByteLimit,
    "ByteLimit",
    json!({
        "description": "Size (for example \"256mib\") or \"unlimited\".",
        "anyOf": [
            { "type": "string", "pattern": BYTE_LIMIT_PATTERN },
            { "type": "integer", "minimum": 0 }
        ]
    })
);

/// Handling of an operation larger than the size budget of the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Oversize {
    /// Perform the operation without keeping the previous content; the report
    /// marks it as irreversible (`too_large_for_journal`).
    #[default]
    Skip,
    /// Keep the entry above the budget; other entries expire first.
    Keep,
}

/// Clock that measures the age of journal entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgeClock {
    /// Wall-clock time; entries age while the computer sleeps or is off.
    #[default]
    Wall,
    /// Time the computer is running; sleep and power-off do not count.
    Active,
}

/// Strategy of keeping the previous content of an overwritten file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OverwriteStrategy {
    /// Keep the old file by exchanging names (or a backup name or hard link),
    /// falling back to a clone or a copy.
    #[default]
    Exchange,
    /// Keep a clone when the volume supports it, otherwise a copy.
    CloneOrCopy,
}

/// Detection of file changes made by a program started with `run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TrackChanges {
    /// Scan of the watched paths, accelerated by file system events.
    #[default]
    Auto,
    /// Scan of the watched paths before and after the program.
    Scan,
    /// File system events only.
    Events,
    /// No detection.
    #[serde(rename = "none")]
    Disabled,
}

/// Keeping the previous content of files before a program runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PreimageMode {
    /// Clone or copy the declared and watched files within the configured
    /// budget and limits.
    #[default]
    Auto,
    /// Do not keep previous content; changed files are reported as
    /// irreversible.
    Off,
}

/// Supervision of programs that continue in the background after the command
/// line interface exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Supervisor {
    /// A long-lived process supervises background programs: the embedding
    /// process, or a detached supervisor started by the command line.
    #[default]
    Auto,
    /// Background programs are available only inside a long-lived process.
    #[serde(rename = "none")]
    Disabled,
}

/// Language of messages and help.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Language {
    /// English.
    #[default]
    En,
    /// Polish.
    Pl,
}

/// Colour output for people.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ColorMode {
    /// Colours when the output is a terminal and `NO_COLOR` is not set.
    #[default]
    Auto,
    /// Always colours.
    Always,
    /// Never colours.
    Never,
}

/// Line ending of newly created text files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    /// `\n`.
    #[default]
    Lf,
    /// `\r\n`.
    Crlf,
}

/// YAML version assumed when a document does not declare one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
pub enum YamlVersion {
    /// YAML 1.1 (`yes`, `no`, `on`, `off` are booleans).
    #[serde(rename = "1.1")]
    V1_1,
    /// YAML 1.2.
    #[default]
    #[serde(rename = "1.2")]
    V1_2,
}

/// Source of information about installed programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProgramSource {
    /// Programs found in `PATH`.
    Path,
    /// Databases of package managers.
    Package,
    /// Applications registered with the operating system.
    App,
    /// Manual pages and `whatis`.
    Man,
    /// External catalogs in the format `nurt-program-catalog/1`.
    Catalog,
}

/// Verification of SSH host keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KnownHosts {
    /// Only hosts present in the known hosts files.
    #[default]
    Strict,
    /// Unknown hosts are accepted and recorded; changed keys are refused.
    AcceptNew,
}

/// Holder of the secret key for OpenPGP signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OpenPgpSource {
    /// A running gpg-agent.
    GpgAgent,
    /// The key file named by `git.signing.key_file`.
    KeyFile,
}

/// Installation mode of the background service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceMode {
    /// Service of the user account (on Linux with lingering enabled, so that
    /// it runs without a login session).
    #[default]
    User,
    /// System service; recommended for servers.
    System,
}

/// Level of diagnostic logs of the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Errors only.
    Error,
    /// Errors and warnings.
    #[default]
    Warn,
    /// Informational messages.
    Info,
    /// Debugging messages.
    Debug,
    /// All messages.
    Trace,
}

/// Mode of the language server allowance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServerAllowMode {
    /// Every configured server may be started.
    #[default]
    All,
    /// No server is started; code commands use syntax analysis only.
    Off,
}

/// Language servers that may be started: all, none, or the listed languages.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ServerAllow {
    /// `"all"` or `"off"`.
    Mode(ServerAllowMode),
    /// Names of languages whose servers may be started.
    Languages(Vec<String>),
}

impl Default for ServerAllow {
    fn default() -> Self {
        Self::Mode(ServerAllowMode::All)
    }
}

/// External catalog of programs: a path, or a table with options.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum CatalogSource {
    /// Path of a catalog file or directory.
    Path(String),
    /// Catalog with options.
    Entry(CatalogEntry),
}

/// External catalog of programs with options.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct CatalogEntry {
    /// Path of the catalog file or directory.
    pub path: String,
    /// Order among catalogs; a higher priority wins when catalogs describe
    /// the same program. Default `0`.
    #[serde(default)]
    pub priority: i32,
    /// Whether the catalog is used. Default `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

const fn default_true() -> bool {
    true
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// Complete configuration `nurt-config/1`.
///
/// Every key has a built-in default, so an empty file is a valid
/// configuration. Keys are grouped in sections that correspond to tables of
/// the TOML file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct Config {
    /// Text notation of programs.
    pub syntax: SyntaxConfig,
    /// Correction of the written form of programs.
    pub correction: CorrectionConfig,
    /// Base directory, project directories and trusted projects.
    pub scope: ScopeConfig,
    /// Journal of changes: retention, floors and volumes.
    pub journal: JournalConfig,
    /// Free space reserve and the space service.
    pub space: SpaceConfig,
    /// Programs started with `run`.
    pub run: RunConfig,
    /// Output for people and models.
    pub output: OutputConfig,
    /// Thresholds of the effect report.
    pub report: ReportConfig,
    /// Continuation of shortened results: lifetime and space.
    pub continuation: ContinuationConfig,
    /// Text encodings and line endings.
    pub text: TextConfig,
    /// Resource limits.
    pub limits: LimitsConfig,
    /// Limits of the loops of the language.
    #[serde(rename = "loop")]
    pub loop_limits: LoopConfig,
    /// Limits of the functions of the language.
    pub function: FunctionConfig,
    /// File system behaviour.
    pub fs: FsConfig,
    /// Source code analysis and language servers.
    pub code: CodeConfig,
    /// Structured documents.
    pub documents: DocumentsConfig,
    /// SQLite databases.
    pub sql: SqlConfig,
    /// Installed programs and external program catalogs.
    pub programs: ProgramsConfig,
    /// Proxy used by the HTTP client.
    pub http: HttpConfig,
    /// Git repositories.
    pub git: GitConfig,
    /// Tests and analysis.
    pub verify: VerifyConfig,
    /// Extensions.
    pub ext: ExtConfig,
    /// Background service.
    pub service: ServiceConfig,
    /// State directory.
    pub state: StateConfig,
    /// Diagnostic logs.
    pub log: LogConfig,
    /// Session recorded in the journal.
    pub session: SessionConfig,
    /// Privilege elevation.
    pub privilege: PrivilegeConfig,
}

/// Section `[syntax]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct SyntaxConfig {
    /// Key `syntax.variant`, variable `NURT_SYNTAX`: notation variant of the
    /// text form (`a`, `b` or `c`). Default `a`.
    pub variant: SyntaxVariant,
    /// Key `syntax.tolerant`, variable `NURT_SYNTAX_TOLERANT`: also accept the
    /// notation of the other variants. Default `true`.
    pub tolerant: bool,
}

impl Default for SyntaxConfig {
    fn default() -> Self {
        Self {
            variant: SyntaxVariant::A,
            tolerant: true,
        }
    }
}

/// Section `[correction]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct CorrectionConfig {
    /// Key `correction.enabled`, variable `NURT_CORRECTION`: correct the
    /// written form of programs. Default `true`.
    pub enabled: bool,
    /// Key `correction.max_read`: correction budget of programs that only
    /// read. Default `8`.
    pub max_read: u32,
    /// Key `correction.max_effect`: correction budget of programs that change
    /// something. Default `4`.
    pub max_effect: u32,
    /// Key `correction.require_first_char`: fuzzy correction of names keeps
    /// the first character. Default `true`.
    pub require_first_char: bool,
    /// Key `correction.candidates_ttl`: lifetime of candidates that can be
    /// accepted after an ambiguous program. Default `10m`.
    pub candidates_ttl: DurationSetting,
}

impl Default for CorrectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_read: 8,
            max_effect: 4,
            require_first_char: true,
            candidates_ttl: DurationSetting::from_secs(10 * 60),
        }
    }
}

/// Section `[scope]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct ScopeConfig {
    /// Key `scope.base`, variable `NURT_BASE`: absolute directory for
    /// relative paths. Default none.
    pub base: Option<String>,
    /// Key `scope.project_roots`, variable `NURT_PROJECT_ROOTS` (list
    /// separated by `:`, `;` on Windows): project directories; changes outside
    /// them are reported as `outside_project`. Default empty.
    pub project_roots: Vec<String>,
    /// Key `scope.trusted_projects`, variable `NURT_TRUSTED_PROJECTS`:
    /// projects whose `.nurt.toml` may change `journal.*`. Set only in the
    /// system or user file, the `NURT_CONFIG` file or the variable; never in a
    /// project file or at run time. Default empty.
    pub trusted_projects: Vec<String>,
}

/// Section `[journal]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct JournalConfig {
    /// Key `journal.enabled`, variable `NURT_JOURNAL`: record changes in the
    /// journal. Default `true`.
    pub enabled: bool,
    /// Key `journal.max_ops`, variable `NURT_JOURNAL_MAX_OPS`: number of
    /// recent changing operations that can be undone (`1..100000` or
    /// `unlimited`). Default `30`.
    pub max_ops: StepLimit,
    /// Key `journal.max_age`, variable `NURT_JOURNAL_MAX_AGE`: how long the
    /// journal keeps previous content (`10s..365d` or `unlimited`). Default
    /// `30m`.
    pub max_age: AgeLimit,
    /// Key `journal.max_size`, variable `NURT_JOURNAL_MAX_SIZE`: size budget
    /// per volume (size, percentage of capacity, percentage of free space or
    /// `unlimited`). Default `unlimited`; the space reserve always applies.
    pub max_size: SizeLimit,
    /// Key `journal.max_size_min`: lower clamp of percentage budgets. Default
    /// `2gib`.
    pub max_size_min: SizeSetting,
    /// Key `journal.max_size_max`: upper clamp of percentage budgets. Default
    /// `50gib`.
    pub max_size_max: SizeSetting,
    /// Key `journal.max_size_total`: budget summed over all volumes. Default
    /// `unlimited`.
    pub max_size_total: ByteLimit,
    /// Key `journal.max_op_size`, variable `NURT_JOURNAL_MAX_OP_SIZE`: limit
    /// of a single operation. Default `unlimited`.
    pub max_op_size: ByteLimit,
    /// Key `journal.oversize`, variable `NURT_JOURNAL_OVERSIZE`: operation
    /// larger than the budget (`skip` or `keep`). Default `skip`.
    pub oversize: Oversize,
    /// Key `journal.age_clock`: clock of entry age (`wall` or `active`).
    /// Default `wall`.
    pub age_clock: AgeClock,
    /// Keys `journal.floor.*`: protection floors.
    pub floor: JournalFloor,
    /// Key `journal.pin_max_age`: default duration of `journal.pin`. Default
    /// `24h`.
    pub pin_max_age: DurationSetting,
    /// Key `journal.overwrite`, variable `NURT_JOURNAL_OVERWRITE`: keeping
    /// overwritten files (`exchange` or `clone_or_copy`). Default `exchange`.
    pub overwrite: OverwriteStrategy,
    /// Key `journal.count_undo_as_op`: an undo counts as a step. Default
    /// `true`.
    pub count_undo_as_op: bool,
    /// Key `journal.history_keep`: number of expired entries whose metrics are
    /// kept for `history`. Default `1000`.
    pub history_keep: u32,
    /// Key `journal.dir_name`: name of the journal directory on each volume.
    /// Default `.nurt-journal`.
    pub dir_name: String,
    /// Keys `journal.volumes.*`: journal on network and removable volumes.
    pub volumes: VolumeClasses,
    /// Keys `journal.volume."<mount point>".*`: settings of one volume.
    /// Default empty.
    pub volume: IndexMap<String, VolumeOverride>,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_ops: StepLimit::Limited(30),
            max_age: AgeLimit::Limited(DurationSetting::from_secs(30 * 60)),
            max_size: SizeLimit::Unlimited,
            max_size_min: SizeSetting::from_gib(2),
            max_size_max: SizeSetting::from_gib(50),
            max_size_total: ByteLimit::Unlimited,
            max_op_size: ByteLimit::Unlimited,
            oversize: Oversize::Skip,
            age_clock: AgeClock::Wall,
            floor: JournalFloor::default(),
            pin_max_age: DurationSetting::from_secs(24 * 3600),
            overwrite: OverwriteStrategy::Exchange,
            count_undo_as_op: true,
            history_keep: 1000,
            dir_name: ".nurt-journal".to_owned(),
            volumes: VolumeClasses::default(),
            volume: IndexMap::new(),
        }
    }
}

/// Section `[journal.floor]`: lowest limits that other settings cannot go
/// below. Lower values are raised to the floor and the change is noted in
/// `journal.status`. Values that come from a project file are raised at least
/// to [`PROJECT_FLOOR_MAX_OPS`] and [`PROJECT_FLOOR_MAX_AGE`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct JournalFloor {
    /// Key `journal.floor.max_ops`: lowest step limit. Default none.
    pub max_ops: Option<StepLimit>,
    /// Key `journal.floor.max_age`: lowest age limit. Default none.
    pub max_age: Option<AgeLimit>,
}

/// Section `[journal.volumes]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct VolumeClasses {
    /// Key `journal.volumes.network`, variable `NURT_JOURNAL_NETWORK`: keep a
    /// journal on network volumes. Default `false`.
    pub network: bool,
    /// Key `journal.volumes.removable`, variable `NURT_JOURNAL_REMOVABLE`:
    /// keep a journal on removable volumes. Default `false`.
    pub removable: bool,
}

/// Table `[journal.volume."<mount point>"]`: settings of one volume; absent
/// keys are inherited.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct VolumeOverride {
    /// Journal enabled on this volume.
    pub enabled: Option<bool>,
    /// Space reserve of this volume.
    pub reserve: Option<SizeOrPercent>,
    /// Soft threshold of this volume.
    pub soft: Option<SizeOrPercent>,
    /// Step limit on this volume.
    pub max_ops: Option<StepLimit>,
    /// Age limit on this volume.
    pub max_age: Option<AgeLimit>,
    /// Size budget on this volume.
    pub max_size: Option<SizeLimit>,
}

/// Section `[space]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct SpaceConfig {
    /// Key `space.reserve`, variable `NURT_SPACE_RESERVE`: free space that
    /// the journal never takes (size or percentage of capacity). Default
    /// `15%`.
    pub reserve: SizeOrPercent,
    /// Key `space.reserve_min`: lower clamp of a percentage reserve. Default
    /// `5gib`.
    pub reserve_min: SizeSetting,
    /// Key `space.reserve_max`: upper clamp of a percentage reserve. Default
    /// `50gib`.
    pub reserve_max: SizeSetting,
    /// Key `space.soft`, variable `NURT_SPACE_SOFT`: soft threshold at which
    /// the service starts releasing journal data. Default none: reserve plus
    /// the larger of 5 % and 2 GiB, no more than reserve plus 20 GiB.
    pub soft: Option<SizeOrPercent>,
    /// Key `space.margin`: margin above the reserve. Default none: the
    /// smaller of 1 % and 1 GiB.
    pub margin: Option<SizeOrPercent>,
    /// Key `space.poll_ms`, variable `NURT_SPACE_POLL_MS`: period of free
    /// space measurement in milliseconds. Default `250`.
    pub poll_ms: u32,
    /// Key `space.poll_fast_ms`: measurement period near a threshold, in
    /// milliseconds. Default `50`.
    pub poll_fast_ms: u32,
    /// Key `space.heartbeat_ms`: heartbeat period of the service, in
    /// milliseconds. Default `500`.
    pub heartbeat_ms: u32,
    /// Key `space.heartbeat_max_age_ms`: age after which a heartbeat is stale,
    /// in milliseconds. Default `2000`.
    pub heartbeat_max_age_ms: u32,
    /// Key `space.sync_budget_ms`: budget of synchronous release before a
    /// write, in milliseconds. Default `1500`.
    pub sync_budget_ms: u32,
    /// Key `space.settle_ms`: wait for free space to grow after a release, in
    /// milliseconds. Default `1000`.
    pub settle_ms: u32,
    /// Key `space.quota_aware`: take quotas of the account into account.
    /// Default `true`.
    pub quota_aware: bool,
}

impl Default for SpaceConfig {
    fn default() -> Self {
        Self {
            reserve: SizeOrPercent::Percent(Percent::from_percent(15)),
            reserve_min: SizeSetting::from_gib(5),
            reserve_max: SizeSetting::from_gib(50),
            soft: None,
            margin: None,
            poll_ms: 250,
            poll_fast_ms: 50,
            heartbeat_ms: 500,
            heartbeat_max_age_ms: 2000,
            sync_budget_ms: 1500,
            settle_ms: 1000,
            quota_aware: true,
        }
    }
}

/// Section `[run]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct RunConfig {
    /// Key `run.timeout`, variable `NURT_RUN_TIMEOUT`: time limit of a
    /// program. Default `120s`.
    pub timeout: DurationSetting,
    /// Key `run.wait`, variable `NURT_RUN_WAIT`: time to wait in the
    /// foreground before a program continues in the background. Default `3s`.
    pub wait: DurationSetting,
    /// Key `run.idle_timeout`: time without output and processor activity
    /// after which a program is reported as stalled. Default `60s`.
    pub idle_timeout: DurationSetting,
    /// Key `run.kill_grace`: time between the termination request and killing
    /// the process group. Default `5s`.
    pub kill_grace: DurationSetting,
    /// Table `run.env_defaults`: variables added to the environment of every
    /// program. Default `PYTHONUTF8`, `PYTHONIOENCODING`, `NO_COLOR`, `TERM`,
    /// `GIT_TERMINAL_PROMPT`, `PAGER`, `GIT_PAGER`, `DEBIAN_FRONTEND`.
    pub env_defaults: IndexMap<String, String>,
    /// Key `run.track_changes`: detection of file changes (`auto`, `scan`,
    /// `events` or `none`). Default `auto`.
    pub track_changes: TrackChanges,
    /// Key `run.track_ignore`: names excluded from change detection. Default
    /// `[".git"]`.
    pub track_ignore: Vec<String>,
    /// Key `run.preimage`: keep previous content before a program runs
    /// (`auto` or `off`). Default `auto`.
    pub preimage: PreimageMode,
    /// Key `run.preimage_budget`: time budget of keeping previous content.
    /// Default `1500ms`.
    pub preimage_budget: DurationSetting,
    /// Key `run.preimage_max_entries`: largest number of files kept. Default
    /// `20000`.
    pub preimage_max_entries: u32,
    /// Key `run.preimage_copy_max`: largest amount copied when cloning is not
    /// available. Default `256mib`.
    pub preimage_copy_max: SizeSetting,
    /// Key `run.preimage_exclude`: names never kept. Default `node_modules`,
    /// `.git`, `target`, `build`, `dist`, `.venv`, `__pycache__`.
    pub preimage_exclude: Vec<String>,
    /// Key `run.irreversible_patterns`: argument patterns added to the
    /// built-in table of programs with effects outside the computer, for
    /// example `"deploy-tool publish"`; tokens are separated by spaces, `*`
    /// matches any token and `a|b` matches alternatives. The built-in table
    /// cannot be removed. Default empty.
    pub irreversible_patterns: Vec<String>,
    /// Key `run.parallel`: default number of batches run at the same time.
    /// Default `1`.
    pub parallel: u32,
    /// Key `run.spool_max`: largest output kept per program: its first part
    /// and, up to a quarter of the limit (at most 64 MiB), its last part; the
    /// bytes between them are omitted, marked in the file and reported.
    /// `unlimited` keeps everything. Default `1gib`.
    pub spool_max: ByteLimit,
    /// Key `run.supervisor`: supervision of background programs (`auto` or
    /// `none`). Default `auto`.
    pub supervisor: Supervisor,
    /// Key `run.waits_threshold`: waiting time above which a program is listed
    /// among the waits of a checked program. Default `3s`.
    pub waits_threshold: DurationSetting,
}

impl Default for RunConfig {
    fn default() -> Self {
        let env_defaults = [
            ("PYTHONUTF8", "1"),
            ("PYTHONIOENCODING", "utf-8"),
            ("NO_COLOR", "1"),
            ("TERM", "dumb"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("PAGER", "cat"),
            ("GIT_PAGER", "cat"),
            ("DEBIAN_FRONTEND", "noninteractive"),
        ]
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
        Self {
            timeout: DurationSetting::from_secs(120),
            wait: DurationSetting::from_secs(3),
            idle_timeout: DurationSetting::from_secs(60),
            kill_grace: DurationSetting::from_secs(5),
            env_defaults,
            track_changes: TrackChanges::Auto,
            track_ignore: strings(&[".git"]),
            preimage: PreimageMode::Auto,
            preimage_budget: DurationSetting::from_millis(1500),
            preimage_max_entries: 20_000,
            preimage_copy_max: SizeSetting::from_mib(256),
            preimage_exclude: strings(&[
                "node_modules",
                ".git",
                "target",
                "build",
                "dist",
                ".venv",
                "__pycache__",
            ]),
            irreversible_patterns: Vec::new(),
            parallel: 1,
            spool_max: ByteLimit::Bytes(SizeSetting::from_gib(1)),
            supervisor: Supervisor::Auto,
            waits_threshold: DurationSetting::from_secs(3),
        }
    }
}

/// Section `[output]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct OutputConfig {
    /// Key `output.language`, variable `NURT_LANGUAGE`: language of messages
    /// and help (`en` or `pl`). Default `en`.
    pub language: Language,
    /// Key `output.color`, variable `NURT_COLOR`: colours for people (`auto`,
    /// `always` or `never`). Default `auto`.
    pub color: ColorMode,
    /// Key `output.head`: first lines of program output shown in a response.
    /// Default `20`.
    pub head: u32,
    /// Key `output.tail`: last lines of program output shown in a response.
    /// Default `40`.
    pub tail: u32,
    /// Key `output.max_bytes`: bytes of text of one program output stream
    /// shown in a response; `0` is no limit. Default `4096`.
    pub max_bytes: u32,
    /// Key `output.spool_ttl`: lifetime of files with complete program output.
    /// Default `1h`.
    pub spool_ttl: DurationSetting,
    /// Key `output.results_dir`, variable `NURT_RESULTS_DIR`: directory of
    /// complete results saved to files. Default none: `results` in the state
    /// directory.
    pub results_dir: Option<String>,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            language: Language::En,
            color: ColorMode::Auto,
            head: 20,
            tail: 40,
            max_bytes: 4096,
            spool_ttl: DurationSetting::from_secs(3600),
            results_dir: None,
        }
    }
}

/// Section `[continuation]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct ContinuationConfig {
    /// Key `continuation.max_age`: a continuation older than this expires.
    /// Default `1h`.
    pub max_age: DurationSetting,
    /// Key `continuation.max_count`: most continuations kept; the oldest is
    /// removed first. Default `50`.
    pub max_count: u32,
    /// Key `continuation.max_size`: most space of all kept results; the
    /// oldest is removed first. Default `256mib`.
    pub max_size: SizeSetting,
}

impl Default for ContinuationConfig {
    fn default() -> Self {
        Self {
            max_age: DurationSetting::from_secs(3600),
            max_count: 50,
            max_size: SizeSetting::from_bytes(256 << 20),
        }
    }
}

/// Section `[report]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct ReportConfig {
    /// Key `report.mass_threshold`: number of targets from which an operation
    /// is reported as a bulk operation. Default `200`.
    pub mass_threshold: u32,
    /// Key `report.diff_max_hunks`: hunks in the sample of differences.
    /// Default `20`.
    pub diff_max_hunks: u32,
    /// Key `report.diff_max_lines`: lines in the sample of differences.
    /// Default `200`.
    pub diff_max_lines: u32,
    /// Keys `report.notable.*`: thresholds of notable effects.
    pub notable: NotableThresholds,
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            mass_threshold: 200,
            diff_max_hunks: 20,
            diff_max_lines: 200,
            notable: NotableThresholds::default(),
        }
    }
}

/// Section `[report.notable]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct NotableThresholds {
    /// Key `report.notable.mass_files`: deleted files from which
    /// `mass_delete` is reported. Default `100`.
    pub mass_files: u32,
    /// Key `report.notable.mass_bytes`: deleted bytes from which
    /// `mass_delete` is reported. Default `1gib`.
    pub mass_bytes: SizeSetting,
    /// Key `report.notable.mass_ratio`: share of a directory deleted from
    /// which `mass_delete` is reported. Default `0.8`.
    pub mass_ratio: f64,
    /// Key `report.notable.shrink_ratio`: share of its size by which a file
    /// of at least 1 KiB shrinks from which `size_shrunk` is reported.
    /// Default `0.5`.
    pub shrink_ratio: f64,
    /// Key `report.notable.per_file_repl`: replacements in one file from which
    /// `many_per_file` is reported. Default `100`.
    pub per_file_repl: u32,
    /// Key `report.notable.targets_high`: targets from which
    /// `target_count_high` is reported. Default `500`.
    pub targets_high: u32,
}

impl Default for NotableThresholds {
    fn default() -> Self {
        Self {
            mass_files: 100,
            mass_bytes: SizeSetting::from_gib(1),
            mass_ratio: 0.8,
            shrink_ratio: 0.5,
            per_file_repl: 100,
            targets_high: 500,
        }
    }
}

/// Section `[text]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct TextConfig {
    /// Key `text.legacy_encodings`: legacy encodings considered when a file is
    /// not valid UTF-8, in order. Default `windows-1250`, `ibm852`,
    /// `iso-8859-2`, `windows-1252`.
    pub legacy_encodings: Vec<String>,
    /// Key `text.new_file_line_ending`: line ending of new files (`lf` or
    /// `crlf`). Default `lf`.
    pub new_file_line_ending: LineEnding,
}

impl Default for TextConfig {
    fn default() -> Self {
        Self {
            legacy_encodings: strings(&["windows-1250", "ibm852", "iso-8859-2", "windows-1252"]),
            new_file_line_ending: LineEnding::Lf,
        }
    }
}

/// Section `[limits]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct LimitsConfig {
    /// Key `limits.max_records`, variable `NURT_MAX_RECORDS`: largest number
    /// of records held in memory by one stage; more is an error, never a
    /// silent cut. Default `5000000`.
    pub max_records: u64,
    /// Key `limits.max_file_bytes_text`: largest file read as text. Default
    /// `1gib`.
    pub max_file_bytes_text: SizeSetting,
    /// Key `limits.threads`, variable `NURT_THREADS`: worker threads. Default
    /// none: the number of processor cores.
    pub threads: Option<u32>,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_records: 5_000_000,
            max_file_bytes_text: SizeSetting::from_gib(1),
            threads: None,
        }
    }
}

/// Section `[loop]`: limits of a `repeat` or a `while` without `limit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct LoopConfig {
    /// Key `loop.max_iterations`: largest number of iterations of a loop
    /// without `limit`; reaching it ends the loop with `E_LIMIT` in `repeat`
    /// and normally in `while`. Default `10000`.
    pub max_iterations: u64,
    /// Key `loop.max_time`: longest time of a loop without `limit`. Default
    /// `10m`.
    pub max_time: DurationSetting,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 10_000,
            max_time: DurationSetting::from_millis(600_000),
        }
    }
}

/// Section `[function]`: limits of the functions defined with `define`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct FunctionConfig {
    /// Key `function.max_depth`: deepest nesting of calls of functions;
    /// deeper recursion ends with `E_LIMIT`. From 1 to
    /// [`FUNCTION_DEPTH_LIMIT`]; default `64`.
    #[serde(deserialize_with = "function_depth")]
    #[schemars(range(min = 1, max = 1024))]
    pub max_depth: u32,
}

/// Largest value of `function.max_depth`: calls of functions nest on the stack
/// of the executor, and a larger depth could overflow it before `E_LIMIT`.
pub const FUNCTION_DEPTH_LIMIT: u32 = 1024;

fn function_depth<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let depth = u32::deserialize(deserializer)?;
    if (1..=FUNCTION_DEPTH_LIMIT).contains(&depth) {
        Ok(depth)
    } else {
        Err(serde::de::Error::custom(format!(
            "function.max_depth is from 1 to {FUNCTION_DEPTH_LIMIT}, found {depth}"
        )))
    }
}

impl Default for FunctionConfig {
    fn default() -> Self {
        Self { max_depth: 64 }
    }
}

/// Section `[fs]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct FsConfig {
    /// Key `fs.sharing_retry_ms`: time of retries after a sharing violation
    /// (Windows), in milliseconds. Default `2000`.
    pub sharing_retry_ms: u32,
}

impl Default for FsConfig {
    fn default() -> Self {
        Self {
            sharing_retry_ms: 2000,
        }
    }
}

/// Section `[code]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct CodeConfig {
    /// Table `code.servers."<language>"`: language servers. Default
    /// `pyright-langserver --stdio` (Python), `typescript-language-server
    /// --stdio` (TypeScript and JavaScript), `gopls` (Go), `rust-analyzer`
    /// (Rust), `clangd` (C and C++).
    pub servers: IndexMap<String, LanguageServerConfig>,
    /// Key `code.servers_allow`: servers that may be started (`all`, `off` or
    /// a list of languages). Default `all`.
    pub servers_allow: ServerAllow,
    /// Key `code.server_idle`: time after which an unused server stops.
    /// Default `10m`.
    pub server_idle: DurationSetting,
    /// Key `code.server_retry`: time before a failed server is started again.
    /// Default `5m`.
    pub server_retry: DurationSetting,
    /// Key `code.max_servers`: servers running at the same time. Default `4`.
    pub max_servers: u32,
    /// Key `code.timeout`: time limit of a server request. Default `20s`.
    pub timeout: DurationSetting,
    /// Key `code.timeout_max`: largest time limit a command may request.
    /// Default `2m`.
    pub timeout_max: DurationSetting,
    /// Key `code.index_max_files`: largest number of files indexed for
    /// syntax analysis. Default `20000`.
    pub index_max_files: u32,
    /// Key `code.file_bytes_max`: largest file analysed. Default `1mib`.
    pub file_bytes_max: SizeSetting,
    /// Key `code.diagnostics_quiet`: time without new diagnostics after which
    /// diagnostics are considered complete (at least `50ms`). Default
    /// `1500ms`.
    pub diagnostics_quiet: DurationSetting,
}

impl Default for CodeConfig {
    fn default() -> Self {
        let servers = [
            ("python", &["pyright-langserver", "--stdio"][..]),
            ("typescript", &["typescript-language-server", "--stdio"][..]),
            ("javascript", &["typescript-language-server", "--stdio"][..]),
            ("go", &["gopls"][..]),
            ("rust", &["rust-analyzer"][..]),
            ("c", &["clangd"][..]),
            ("cpp", &["clangd"][..]),
        ]
        .iter()
        .map(|(language, command)| ((*language).to_owned(), LanguageServerConfig::new(command)))
        .collect();
        Self {
            servers,
            servers_allow: ServerAllow::default(),
            server_idle: DurationSetting::from_secs(10 * 60),
            server_retry: DurationSetting::from_secs(5 * 60),
            max_servers: 4,
            timeout: DurationSetting::from_secs(20),
            timeout_max: DurationSetting::from_secs(120),
            index_max_files: 20_000,
            file_bytes_max: SizeSetting::from_mib(1),
            diagnostics_quiet: DurationSetting::from_millis(1500),
        }
    }
}

/// Table `[code.servers."<language>"]`: one language server.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct LanguageServerConfig {
    /// Program and arguments; the program is found in `PATH`.
    pub command: Vec<String>,
    /// Initialization options sent to the server. Default none.
    pub init: Option<Json>,
}

impl LanguageServerConfig {
    /// Server started with the given program and arguments.
    #[must_use]
    pub fn new(command: &[&str]) -> Self {
        Self {
            command: strings(command),
            init: None,
        }
    }
}

/// Section `[documents]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct DocumentsConfig {
    /// Key `documents.max_bytes`: largest structured document read. Default
    /// `256mib`.
    pub max_bytes: SizeSetting,
    /// Key `documents.yaml_version`: YAML version of documents without a
    /// directive (`1.1` or `1.2`). Default `1.2`.
    pub yaml_version: YamlVersion,
    /// Key `documents.xml_external_entities`: resolve external entities of XML
    /// documents. Default `false`.
    pub xml_external_entities: bool,
}

impl Default for DocumentsConfig {
    fn default() -> Self {
        Self {
            max_bytes: SizeSetting::from_mib(256),
            yaml_version: YamlVersion::V1_2,
            xml_external_entities: false,
        }
    }
}

/// Section `[sql]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct SqlConfig {
    /// Key `sql.busy_timeout`: wait for a locked SQLite database. Default
    /// `5s`.
    pub busy_timeout: DurationSetting,
}

impl Default for SqlConfig {
    fn default() -> Self {
        Self {
            busy_timeout: DurationSetting::from_secs(5),
        }
    }
}

/// Section `[programs]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct ProgramsConfig {
    /// Key `programs.catalogs`, variable `NURT_PROGRAMS_CATALOGS`: external
    /// catalogs of programs in the format `nurt-program-catalog/1`; without
    /// them the commands describe installed programs. Default empty.
    pub catalogs: Vec<CatalogSource>,
    /// Key `programs.index_dir`: directory of the program index. Default none:
    /// the cache directory of the user.
    pub index_dir: Option<String>,
    /// Key `programs.help_timeout`: time limit of reading the help of a
    /// program on request. Default `3s`.
    pub help_timeout: DurationSetting,
    /// Key `programs.sources_disabled`: sources of program information that
    /// are not used (`path`, `package`, `app`, `man`, `catalog`). Default
    /// empty.
    pub sources_disabled: Vec<ProgramSource>,
}

impl Default for ProgramsConfig {
    fn default() -> Self {
        Self {
            catalogs: Vec::new(),
            index_dir: None,
            help_timeout: DurationSetting::from_secs(3),
            sources_disabled: Vec::new(),
        }
    }
}

/// Section `[http]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct HttpConfig {
    /// Key `http.proxy`, variable `NURT_HTTP_PROXY` (also `HTTP_PROXY`,
    /// `HTTPS_PROXY`, `NO_PROXY`): proxy of HTTP requests and Git fetches.
    /// Default none: the system setting.
    pub proxy: Option<String>,
}

/// Section `[git]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct GitConfig {
    /// Key `git.trash_refs`: keep deleted branches under `refs/nurt/trash/`.
    /// Default `false`.
    pub trash_refs: bool,
    /// Keys `git.hooks.*`.
    pub hooks: GitHooksConfig,
    /// Keys `git.ssh.*`.
    pub ssh: GitSshConfig,
    /// Keys `git.signing.*`.
    pub signing: GitSigningConfig,
    /// Keys `git.checkpoint.*`.
    pub checkpoint: GitCheckpointConfig,
    /// Keys `git.status.*`.
    pub status: GitStatusConfig,
    /// Keys `git.network.*`.
    pub network: GitNetworkConfig,
}

/// Section `[git.hooks]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct GitHooksConfig {
    /// Key `git.hooks.timeout`: time limit of a repository hook. Default
    /// `10m`.
    pub timeout: DurationSetting,
}

impl Default for GitHooksConfig {
    fn default() -> Self {
        Self {
            timeout: DurationSetting::from_secs(10 * 60),
        }
    }
}

/// Section `[git.ssh]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct GitSshConfig {
    /// Key `git.ssh.known_hosts`: verification of host keys (`strict` or
    /// `accept_new`). Default `strict`.
    pub known_hosts: KnownHosts,
    /// Key `git.ssh.identity_files`: private key files. Default empty: the
    /// standard identity files of the user.
    pub identity_files: Vec<String>,
}

/// Section `[git.signing]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct GitSigningConfig {
    /// Key `git.signing.key_file`: key file for signing commits when the
    /// repository configuration does not name one. Default none.
    pub key_file: Option<String>,
    /// Key `git.signing.openpgp_source`: the only holder of the secret key
    /// used for OpenPGP signatures (`gpg_agent` or `key_file`). Default none:
    /// gpg-agent first, then the key file.
    pub openpgp_source: Option<OpenPgpSource>,
}

/// Section `[git.checkpoint]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct GitCheckpointConfig {
    /// Key `git.checkpoint.keep`: checkpoints kept per repository. Default
    /// `50`.
    pub keep: u32,
}

impl Default for GitCheckpointConfig {
    fn default() -> Self {
        Self { keep: 50 }
    }
}

/// Section `[git.status]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct GitStatusConfig {
    /// Key `git.status.limit`: entries listed by `git.status` before paging.
    /// Default `200`.
    pub limit: u32,
}

impl Default for GitStatusConfig {
    fn default() -> Self {
        Self { limit: 200 }
    }
}

/// Section `[git.network]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct GitNetworkConfig {
    /// Key `git.network.timeout`: time limit of a network operation. Default
    /// `5m`.
    pub timeout: DurationSetting,
}

impl Default for GitNetworkConfig {
    fn default() -> Self {
        Self {
            timeout: DurationSetting::from_secs(5 * 60),
        }
    }
}

/// Section `[verify]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct VerifyConfig {
    /// Key `verify.history_max`: test runs kept for comparison. Default `50`.
    pub history_max: u32,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        Self { history_max: 50 }
    }
}

/// Section `[ext]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct ExtConfig {
    /// Key `ext.dir`, variable `NURT_EXT_DIR`: directory of extensions.
    /// Default none: `ext` in the configuration directory.
    pub dir: Option<String>,
    /// Key `ext.native`: allow native extension libraries besides WebAssembly.
    /// Default `false`.
    pub native: bool,
}

/// Section `[service]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct ServiceConfig {
    /// Key `service.mode`: installation mode of the service (`user` or
    /// `system`). Default `user`.
    pub mode: ServiceMode,
}

/// Section `[state]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct StateConfig {
    /// Key `state.dir`, variable `NURT_STATE_DIR`: state directory. Default
    /// none: the state directory of the operating system.
    pub dir: Option<String>,
}

/// Section `[log]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct LogConfig {
    /// Key `log.level`, variable `NURT_LOG`: level of diagnostic logs
    /// (`error`, `warn`, `info`, `debug`, `trace`). Default `warn`.
    pub level: LogLevel,
}

/// Section `[privilege]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct PrivilegeConfig {
    /// Key `privilege.default`: value of the call parameter `privilege` when
    /// a call does not give it; the embedding engine may set it. Default
    /// `account`.
    pub default: Privilege,
    /// Key `privilege.allowed`: when `false`, `elevated` and `auto` end with
    /// `E_PRIVILEGE` and the reason `disabled_by_configuration`; the switch
    /// with which the embedding engine applies its policy. Default `true`.
    pub allowed: bool,
    /// Key `privilege.backends`: allowed elevation backends in order of
    /// trial (`self`, `broker`, `sudo`, `doas`, `run0`). Default none: the
    /// list of the system, `["self", "broker", "sudo", "doas", "run0"]` on
    /// Linux, `["self", "broker", "sudo"]` on macOS and `["self", "broker"]`
    /// on Windows.
    pub backends: Option<Vec<String>>,
    /// Key `privilege.group`: group allowed to connect to the service
    /// `nurt-elevation`. Default none: `sudo` or `wheel` on Linux, `admin` on
    /// macOS.
    pub group: Option<String>,
}

impl Default for PrivilegeConfig {
    fn default() -> Self {
        Self {
            default: Privilege::Account,
            allowed: true,
            backends: None,
            group: None,
        }
    }
}

/// Section `[session]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
pub struct SessionConfig {
    /// Key `session.id`, variable `NURT_SESSION`: session identifier recorded
    /// in the journal. Default none.
    pub id: Option<String>,
    /// Key `session.ttl`: time without calls after which session settings
    /// expire. Default `24h`.
    pub ttl: DurationSetting,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            id: None,
            ttl: DurationSetting::from_secs(24 * 3600),
        }
    }
}

/// Level of the configuration, in increasing precedence.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ConfigLevel {
    /// Built-in defaults.
    Builtin,
    /// System file: `/etc/nurt/nurt.toml`,
    /// `/Library/Application Support/Nurt/nurt.toml` or
    /// `%PROGRAMDATA%\Nurt\nurt.toml`.
    System,
    /// User file: `$XDG_CONFIG_HOME/nurt/nurt.toml`,
    /// `~/Library/Application Support/Nurt/nurt.toml` or
    /// `%APPDATA%\Nurt\nurt.toml`.
    User,
    /// Project file `.nurt.toml`; limited to the keys allowed by
    /// [`allowed_at`].
    Project,
    /// File named by `NURT_CONFIG`, or the configuration given by an
    /// embedding application when it opens the engine.
    ConfigFile,
    /// `NURT_*` environment variables, read when the process starts.
    Environment,
    /// Run-time settings: invocation parameters, request options and
    /// `journal.limits`.
    Runtime,
}

impl ConfigLevel {
    /// All levels, in increasing precedence.
    pub const ALL: [Self; 7] = [
        Self::Builtin,
        Self::System,
        Self::User,
        Self::Project,
        Self::ConfigFile,
        Self::Environment,
        Self::Runtime,
    ];

    /// Stable identifier of the level.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::System => "system",
            Self::User => "user",
            Self::Project => "project",
            Self::ConfigFile => "config_file",
            Self::Environment => "environment",
            Self::Runtime => "runtime",
        }
    }
}

impl fmt::Display for ConfigLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Reason why a key of a configuration layer was not applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConfigNoteKind {
    /// A trust key or a service key outside the levels that may set it.
    TrustConfigIgnored,
    /// A journal key in the file of a project that is not trusted.
    JournalConfigIgnored,
    /// A key that a project file may not set.
    ProjectConfigIgnored,
    /// A key that the format does not define.
    UnknownKey,
}

impl ConfigNoteKind {
    /// Code of the notable effect that reports the note, when there is one.
    #[must_use]
    pub const fn notable(self) -> Option<&'static str> {
        match self {
            Self::TrustConfigIgnored => Some("trust_config_ignored"),
            Self::JournalConfigIgnored => Some("journal_config_ignored"),
            Self::ProjectConfigIgnored => Some("project_config_ignored"),
            Self::UnknownKey => None,
        }
    }
}

/// Whether a key may be set at a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KeyAccess {
    /// The key is applied.
    Allowed,
    /// The key is applied only when the project is trusted.
    TrustedProjectOnly,
    /// The key is ignored and reported.
    Ignored(ConfigNoteKind),
}

impl KeyAccess {
    /// Decision for a project that is or is not trusted.
    ///
    /// # Errors
    ///
    /// Returns the kind of the note when the key is not applied.
    pub const fn permits(self, project_trusted: bool) -> Result<(), ConfigNoteKind> {
        match self {
            Self::Allowed => Ok(()),
            Self::TrustedProjectOnly if project_trusted => Ok(()),
            Self::TrustedProjectOnly => Err(ConfigNoteKind::JournalConfigIgnored),
            Self::Ignored(kind) => Err(kind),
        }
    }
}

/// Whether the dotted `key` may be set at `level`.
///
/// Every key may be set at every level except the following:
///
/// * trust keys (`scope.trust*`) are ignored in a project file and at run
///   time, so that neither a repository nor a request can grant trust;
/// * `service.*` and `privilege.*` are ignored in a project file;
/// * a project file may set `scope.base`, `scope.project_roots`, `report.*`
///   and `run.*`, and `journal.*` only for a trusted project; other keys are
///   ignored.
///
/// # Examples
///
/// ```
/// use nurt_contracts::config::{ConfigLevel, ConfigNoteKind, KeyAccess, allowed_at};
///
/// assert_eq!(allowed_at(ConfigLevel::Project, "run.timeout"), KeyAccess::Allowed);
/// assert_eq!(
///     allowed_at(ConfigLevel::Project, "journal.max_ops"),
///     KeyAccess::TrustedProjectOnly
/// );
/// assert_eq!(
///     allowed_at(ConfigLevel::Project, "scope.trusted_projects"),
///     KeyAccess::Ignored(ConfigNoteKind::TrustConfigIgnored)
/// );
/// assert_eq!(allowed_at(ConfigLevel::User, "scope.trusted_projects"), KeyAccess::Allowed);
/// ```
#[must_use]
pub fn allowed_at(level: ConfigLevel, key: &str) -> KeyAccess {
    let mut segments = key.split('.');
    let section = segments.next().unwrap_or_default();
    let field = segments.next().unwrap_or_default();
    access(level, section, field)
}

fn access(level: ConfigLevel, section: &str, field: &str) -> KeyAccess {
    let trust_key = section == "scope" && field.starts_with("trust");
    if trust_key && matches!(level, ConfigLevel::Project | ConfigLevel::Runtime) {
        return KeyAccess::Ignored(ConfigNoteKind::TrustConfigIgnored);
    }
    if level != ConfigLevel::Project {
        return KeyAccess::Allowed;
    }
    match section {
        "service" | "privilege" => KeyAccess::Ignored(ConfigNoteKind::TrustConfigIgnored),
        "report" | "run" => KeyAccess::Allowed,
        "scope" if matches!(field, "base" | "project_roots") => KeyAccess::Allowed,
        "journal" => KeyAccess::TrustedProjectOnly,
        _ => KeyAccess::Ignored(ConfigNoteKind::ProjectConfigIgnored),
    }
}

/// Values of one configuration source: a parsed file, the environment or
/// run-time settings. Values are the tables of the TOML file converted to
/// JSON (TOML tables become objects).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigLayer {
    /// Level of the source.
    pub level: ConfigLevel,
    /// Where the values come from: a file path, `environment` or `runtime`.
    pub origin: String,
    /// Tables and values of the source.
    pub values: Map<String, Json>,
}

impl ConfigLayer {
    /// Layer of the given level, origin and values.
    #[must_use]
    pub fn new(level: ConfigLevel, origin: impl Into<String>, values: Map<String, Json>) -> Self {
        Self {
            level,
            origin: origin.into(),
            values,
        }
    }
}

/// Key of a layer that was not applied.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct ConfigNote {
    /// Why the key was not applied.
    pub kind: ConfigNoteKind,
    /// Dotted key, with quoted segments where needed.
    pub key: String,
    /// Level of the layer.
    pub level: ConfigLevel,
    /// Origin of the layer.
    pub origin: String,
}

/// Configuration error: a value that does not match its key, or a table where
/// a value is expected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigError {
    /// Dotted key, when the error concerns one key.
    pub key: Option<String>,
    /// Level of the layer, when known.
    pub level: Option<ConfigLevel>,
    /// Origin of the layer, when known.
    pub origin: Option<String>,
    /// Description of the problem.
    pub message: String,
}

impl ConfigError {
    fn general(message: impl Into<String>) -> Self {
        Self {
            key: None,
            level: None,
            origin: None,
            message: message.into(),
        }
    }

    fn at(layer: &ConfigLayer, key: &str, message: impl Into<String>) -> Self {
        Self {
            key: Some(key.to_owned()),
            level: Some(layer.level),
            origin: Some(layer.origin.clone()),
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(origin) = &self.origin {
            write!(f, "{origin}: ")?;
        }
        if let Some(key) = &self.key {
            write!(f, "{key}: ")?;
        }
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Result of merging configuration layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Resolved {
    /// Effective configuration.
    pub config: Config,
    /// Level that set each key, by dotted key; keys not listed keep the
    /// built-in default.
    pub sources: IndexMap<String, ConfigLevel>,
    /// Keys that were not applied, in the order of the layers.
    pub notes: Vec<ConfigNote>,
}

impl Resolved {
    /// Level that set the dotted `key`; [`ConfigLevel::Builtin`] when no
    /// layer set it.
    #[must_use]
    pub fn source(&self, key: &str) -> ConfigLevel {
        self.sources
            .get(key)
            .copied()
            .unwrap_or(ConfigLevel::Builtin)
    }
}

/// Merges configuration layers over the built-in defaults.
///
/// Layers are applied in increasing level; layers of the same level keep
/// their order (a later one wins). Every leaf key is first checked with
/// [`allowed_at`] (keys under `journal.*` of a project file need
/// `project_trusted`); keys that are not applied, and keys the format does
/// not define, are returned as notes. Tables merge key by key; scalars and
/// arrays replace the lower value, except that `journal.floor.max_ops` and
/// `journal.floor.max_age` keep the highest value of all layers, because a
/// floor can only raise protection.
///
/// # Errors
///
/// Returns [`ConfigError`] with the key, level and origin when a value does
/// not match the type of its key.
pub fn resolve(layers: &[ConfigLayer], project_trusted: bool) -> Result<Resolved, ConfigError> {
    let defaults = default_values()?;
    let known = KnownKeys::new(&defaults);
    let mut merged = defaults.clone();
    let mut sources = IndexMap::new();
    let mut notes = Vec::new();
    let mut ordered: Vec<&ConfigLayer> = layers.iter().collect();
    ordered.sort_by_key(|layer| layer.level);
    for layer in ordered {
        for (segments, value) in leaves(&layer.values) {
            let key = display_key(&segments);
            match gate(&known, layer, &segments, &key, project_trusted)? {
                Some(kind) => notes.push(ConfigNote {
                    kind,
                    key,
                    level: layer.level,
                    origin: layer.origin.clone(),
                }),
                None => {
                    probe(&defaults, &segments, value.clone())
                        .map_err(|message| ConfigError::at(layer, &key, message))?;
                    if lowers_floor(&merged, &segments, &value) {
                        continue;
                    }
                    set_path(&mut merged, &segments, value);
                    sources.insert(key, layer.level);
                }
            }
        }
    }
    let config = serde_json::from_value(Json::Object(merged))
        .map_err(|error| ConfigError::general(error.to_string()))?;
    Ok(Resolved {
        config,
        sources,
        notes,
    })
}

/// Trusted projects declared by the layers that may declare trust.
///
/// The project file is excluded and trust keys at run time are ignored, so
/// the result can be used to decide whether the project file is trusted
/// before [`resolve`] is called.
///
/// # Errors
///
/// Returns [`ConfigError`] when a layer other than the project file has an
/// invalid value.
pub fn trusted_projects(layers: &[ConfigLayer]) -> Result<Vec<String>, ConfigError> {
    let without_project: Vec<ConfigLayer> = layers
        .iter()
        .filter(|layer| layer.level != ConfigLevel::Project)
        .cloned()
        .collect();
    resolve(&without_project, false).map(|resolved| resolved.config.scope.trusted_projects)
}

/// Every leaf key of the format, dotted, in the order of the sections.
///
/// Keys of open tables ([`OPEN_TABLES`]) are listed by the name of the table.
///
/// # Errors
///
/// Returns [`ConfigError`] when the defaults cannot be represented, which
/// does not happen for a correct build.
pub fn known_keys() -> Result<Vec<String>, ConfigError> {
    let defaults = default_values()?;
    let mut keys = Vec::new();
    for (segments, _) in leaves(&defaults) {
        let key = display_key(&segments);
        let open = OPEN_TABLES
            .iter()
            .find(|table| key.starts_with(&format!("{table}.")));
        match open {
            Some(table) if !keys.iter().any(|known: &String| known == table) => {
                keys.push((*table).to_owned());
            }
            Some(_) => {}
            None => keys.push(key),
        }
    }
    for table in OPEN_TABLES {
        if !keys.iter().any(|known| known == table) {
            keys.push((*table).to_owned());
        }
    }
    Ok(keys)
}

fn default_values() -> Result<Map<String, Json>, ConfigError> {
    match serde_json::to_value(Config::default()) {
        Ok(Json::Object(map)) => Ok(map),
        Ok(_) => Err(ConfigError::general("defaults are not a table")),
        Err(error) => Err(ConfigError::general(error.to_string())),
    }
}

fn gate(
    known: &KnownKeys,
    layer: &ConfigLayer,
    segments: &[String],
    key: &str,
    project_trusted: bool,
) -> Result<Option<ConfigNoteKind>, ConfigError> {
    match known.classify(segments, key) {
        KeyKind::Unknown => return Ok(Some(ConfigNoteKind::UnknownKey)),
        KeyKind::Table => {
            return Err(ConfigError::at(
                layer,
                key,
                "expected a table, found a value",
            ));
        }
        KeyKind::Value => {}
    }
    let section = segments.first().map(String::as_str).unwrap_or_default();
    let field = segments.get(1).map(String::as_str).unwrap_or_default();
    Ok(access(layer.level, section, field)
        .permits(project_trusted)
        .err())
}

/// Whether `value` for a key of `journal.floor.*` is lower than the floor
/// that a lower level already set: a floor can only raise protection, so the
/// higher floor stays.
fn lowers_floor(merged: &Map<String, Json>, segments: &[String], value: &Json) -> bool {
    let [section, table, field] = segments else {
        return false;
    };
    if section != "journal" || table != "floor" {
        return false;
    }
    let Some(current) = merged
        .get(section)
        .and_then(|journal| journal.get(table))
        .and_then(|floor| floor.get(field))
    else {
        return false;
    };
    let parsed = |json: &Json| -> Option<(u8, u64)> {
        match field.as_str() {
            "max_ops" => serde_json::from_value::<StepLimit>(json.clone())
                .ok()
                .map(|limit| match limit {
                    StepLimit::Limited(steps) => (0, u64::from(steps)),
                    StepLimit::Unlimited => (1, 0),
                }),
            "max_age" => serde_json::from_value::<AgeLimit>(json.clone())
                .ok()
                .map(|limit| match limit {
                    AgeLimit::Limited(age) => (0, age.as_millis()),
                    AgeLimit::Unlimited => (1, 0),
                }),
            _ => None,
        }
    };
    matches!((parsed(value), parsed(current)), (Some(new), Some(old)) if new < old)
}

fn probe(defaults: &Map<String, Json>, segments: &[String], value: Json) -> Result<(), String> {
    let mut candidate = defaults.clone();
    set_path(&mut candidate, segments, value);
    serde_json::from_value::<Config>(Json::Object(candidate))
        .map(|_| ())
        .map_err(|error| error.to_string())
}

enum KeyKind {
    Value,
    Table,
    Unknown,
}

struct KnownKeys {
    values: HashSet<String>,
    tables: HashSet<String>,
    volume_fields: HashSet<String>,
    server_fields: HashSet<String>,
}

/// Names of the fields of a table entry, from its serialized default.
fn field_names<T: Serialize + Default>() -> HashSet<String> {
    match serde_json::to_value(T::default()) {
        Ok(Json::Object(fields)) => fields.into_iter().map(|(name, _)| name).collect(),
        _ => HashSet::new(),
    }
}

impl KnownKeys {
    fn new(defaults: &Map<String, Json>) -> Self {
        let mut known = Self {
            values: HashSet::new(),
            tables: HashSet::new(),
            volume_fields: field_names::<VolumeOverride>(),
            server_fields: field_names::<LanguageServerConfig>(),
        };
        known.walk(&mut Vec::new(), defaults);
        known
    }

    fn walk(&mut self, prefix: &mut Vec<String>, map: &Map<String, Json>) {
        for (name, value) in map {
            prefix.push(name.clone());
            let key = display_key(prefix);
            if let Json::Object(inner) = value {
                self.tables.insert(key);
                self.walk(prefix, inner);
            } else {
                self.values.insert(key);
            }
            prefix.pop();
        }
    }

    /// Whether a key inside an open table names a field of its entry:
    /// `journal.volume."<mount>".<field>` and `code.servers."<language>".<field>`
    /// have fixed fields, `run.env_defaults` takes any name.
    fn is_entry_field(&self, segments: &[String]) -> bool {
        let [section, table, _, field, ..] = segments else {
            return true;
        };
        match (section.as_str(), table.as_str()) {
            ("journal", "volume") => self.volume_fields.contains(field),
            ("code", "servers") => self.server_fields.contains(field),
            _ => true,
        }
    }

    fn classify(&self, segments: &[String], key: &str) -> KeyKind {
        let open = OPEN_TABLES.iter().any(|table| {
            let table: Vec<&str> = table.split('.').collect();
            segments.len() >= table.len()
                && segments
                    .iter()
                    .zip(&table)
                    .all(|(segment, name)| segment == name)
        });
        if open {
            return if self.is_entry_field(segments) {
                KeyKind::Value
            } else {
                KeyKind::Unknown
            };
        }
        if self.values.contains(key) {
            KeyKind::Value
        } else if self.tables.contains(key) {
            KeyKind::Table
        } else {
            KeyKind::Unknown
        }
    }
}

fn leaves(map: &Map<String, Json>) -> Vec<(Vec<String>, Json)> {
    fn walk(prefix: &mut Vec<String>, map: &Map<String, Json>, out: &mut Vec<(Vec<String>, Json)>) {
        for (name, value) in map {
            prefix.push(name.clone());
            match value {
                Json::Object(inner) => walk(prefix, inner, out),
                other => out.push((prefix.clone(), other.clone())),
            }
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    walk(&mut Vec::new(), map, &mut out);
    out
}

fn set_path(root: &mut Map<String, Json>, segments: &[String], value: Json) {
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let mut current = root;
    for name in parents {
        let slot = current
            .entry(name.clone())
            .or_insert_with(|| Json::Object(Map::new()));
        if !slot.is_object() {
            *slot = Json::Object(Map::new());
        }
        let Json::Object(next) = slot else {
            return;
        };
        current = next;
    }
    current.insert(last.clone(), value);
}

/// Writes key segments as a dotted key, quoting segments that are not bare
/// keys of TOML (for example mount points).
///
/// Every component that names a configuration key in a message or a source
/// uses this form.
///
/// ```
/// use nurt_contracts::config::display_key;
///
/// let segments = ["journal", "volume", "/mnt/data", "max_size"].map(String::from);
/// assert_eq!(display_key(&segments), "journal.volume.\"/mnt/data\".max_size");
/// ```
#[must_use]
pub fn display_key(segments: &[String]) -> String {
    let bare = |segment: &str| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    segments
        .iter()
        .map(|segment| {
            if bare(segment) {
                segment.clone()
            } else {
                let escaped = segment.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{escaped}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Kind of value of an environment variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvKind {
    /// Text, passed as a string to the key.
    Text,
    /// `1`, `true`, `yes`, `on` or `0`, `false`, `no`, `off`
    /// (case-insensitive).
    Bool,
    /// Integer.
    Integer,
    /// List separated by [`ENV_LIST_SEPARATOR`]; empty items are skipped.
    List,
}

/// Environment variable that sets a configuration key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct EnvVar {
    /// Name of the variable.
    pub name: &'static str,
    /// Dotted key it sets.
    pub key: &'static str,
    /// Kind of value.
    pub kind: EnvKind,
}

const fn env(name: &'static str, key: &'static str, kind: EnvKind) -> EnvVar {
    EnvVar { name, key, kind }
}

/// Environment variables of the configuration. `NURT_CONFIG` names a file and
/// is not listed.
pub const ENV_VARS: &[EnvVar] = &[
    env("NURT_SYNTAX", "syntax.variant", EnvKind::Text),
    env("NURT_SYNTAX_TOLERANT", "syntax.tolerant", EnvKind::Bool),
    env("NURT_CORRECTION", "correction.enabled", EnvKind::Bool),
    env("NURT_BASE", "scope.base", EnvKind::Text),
    env("NURT_PROJECT_ROOTS", "scope.project_roots", EnvKind::List),
    env(
        "NURT_TRUSTED_PROJECTS",
        "scope.trusted_projects",
        EnvKind::List,
    ),
    env("NURT_JOURNAL", "journal.enabled", EnvKind::Bool),
    env("NURT_JOURNAL_MAX_OPS", "journal.max_ops", EnvKind::Text),
    env("NURT_JOURNAL_MAX_AGE", "journal.max_age", EnvKind::Text),
    env("NURT_JOURNAL_MAX_SIZE", "journal.max_size", EnvKind::Text),
    env(
        "NURT_JOURNAL_MAX_OP_SIZE",
        "journal.max_op_size",
        EnvKind::Text,
    ),
    env("NURT_JOURNAL_OVERSIZE", "journal.oversize", EnvKind::Text),
    env("NURT_JOURNAL_OVERWRITE", "journal.overwrite", EnvKind::Text),
    env(
        "NURT_JOURNAL_NETWORK",
        "journal.volumes.network",
        EnvKind::Bool,
    ),
    env(
        "NURT_JOURNAL_REMOVABLE",
        "journal.volumes.removable",
        EnvKind::Bool,
    ),
    env("NURT_SPACE_RESERVE", "space.reserve", EnvKind::Text),
    env("NURT_SPACE_SOFT", "space.soft", EnvKind::Text),
    env("NURT_SPACE_POLL_MS", "space.poll_ms", EnvKind::Integer),
    env("NURT_RUN_TIMEOUT", "run.timeout", EnvKind::Text),
    env("NURT_RUN_WAIT", "run.wait", EnvKind::Text),
    env("NURT_LANGUAGE", "output.language", EnvKind::Text),
    env("NURT_COLOR", "output.color", EnvKind::Text),
    env("NURT_RESULTS_DIR", "output.results_dir", EnvKind::Text),
    env("NURT_MAX_RECORDS", "limits.max_records", EnvKind::Integer),
    env("NURT_THREADS", "limits.threads", EnvKind::Integer),
    env("NURT_PROGRAMS_CATALOGS", "programs.catalogs", EnvKind::List),
    env("NURT_HTTP_PROXY", "http.proxy", EnvKind::Text),
    env("NURT_EXT_DIR", "ext.dir", EnvKind::Text),
    env("NURT_STATE_DIR", "state.dir", EnvKind::Text),
    env("NURT_LOG", "log.level", EnvKind::Text),
    env("NURT_SESSION", "session.id", EnvKind::Text),
];

/// Builds the environment layer from process variables.
///
/// Only the variables of [`ENV_VARS`] are read; others are ignored. Lists
/// are split by `list_separator` (normally [`ENV_LIST_SEPARATOR`]).
///
/// # Errors
///
/// Returns [`ConfigError`] when a boolean variable has another value.
///
/// # Examples
///
/// ```
/// use nurt_contracts::config::{ConfigLevel, env_layer, resolve};
///
/// let layer = env_layer([("NURT_JOURNAL_MAX_OPS", "50"), ("HOME", "/home/user")], ':').unwrap();
/// let resolved = resolve(&[layer], false).unwrap();
/// assert_eq!(resolved.config.journal.max_ops.to_string(), "50");
/// assert_eq!(resolved.source("journal.max_ops"), ConfigLevel::Environment);
/// ```
pub fn env_layer<I, K, V>(vars: I, list_separator: char) -> Result<ConfigLayer, ConfigError>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let mut layer = ConfigLayer::new(ConfigLevel::Environment, "environment", Map::new());
    for (name, value) in vars {
        let Some(var) = ENV_VARS.iter().find(|var| var.name == name.as_ref()) else {
            continue;
        };
        let value = env_value(var, value.as_ref(), list_separator)
            .map_err(|message| ConfigError::at(&layer, var.key, message))?;
        let segments: Vec<String> = var.key.split('.').map(str::to_owned).collect();
        set_path(&mut layer.values, &segments, value);
    }
    Ok(layer)
}

fn env_value(var: &EnvVar, text: &str, list_separator: char) -> Result<Json, String> {
    match var.kind {
        EnvKind::Text => Ok(Json::from(text)),
        EnvKind::Integer => {
            let trimmed = text.trim();
            Ok(trimmed.parse::<i64>().map_or_else(
                |_| {
                    trimmed
                        .parse::<u64>()
                        .map_or_else(|_| Json::from(text), Json::from)
                },
                Json::from,
            ))
        }
        EnvKind::List => Ok(Json::from(
            text.split(list_separator)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(Json::from)
                .collect::<Vec<_>>(),
        )),
        EnvKind::Bool => match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(Json::Bool(true)),
            "0" | "false" | "no" | "off" => Ok(Json::Bool(false)),
            _ => Err(format!(
                "{} must be 1, true, yes, on, 0, false, no or off, found \"{text}\"",
                var.name
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_sizes_with_the_largest_exact_unit_of_both_families() {
        let cases = [
            (1_000_000_000_000_u64, "1tb"),
            (4_000_000_000, "4gb"),
            (1_500_000_000_000, "1500gb"),
            (1 << 41, "2tib"),
            (1_000_000, "1mb"),
            (1_500, "1500b"),
        ];
        for (bytes, text) in cases {
            let size = SizeSetting::from_bytes(bytes);
            assert_eq!(size.to_string(), text);
            assert_eq!(text.parse::<SizeSetting>().unwrap(), size);
        }
    }

    #[test]
    fn size_and_duration_extremes_round_trip() {
        let size = SizeSetting::from_bytes(u64::MAX);
        assert_eq!(size.to_string().parse::<SizeSetting>().unwrap(), size);
        let duration = DurationSetting::from_millis(u64::MAX);
        assert_eq!(
            duration.to_string().parse::<DurationSetting>().unwrap(),
            duration
        );
        assert!("18446744073709551616b".parse::<SizeSetting>().is_err());
        assert!("18446744073709551616ms".parse::<DurationSetting>().is_err());
        assert!("30500000000000000w".parse::<DurationSetting>().is_err());
    }

    #[test]
    fn environment_integer_accepts_the_full_unsigned_range() {
        let layer = env_layer([("NURT_MAX_RECORDS", "18446744073709551615")], ':').unwrap();
        let resolved = resolve(&[layer], false).unwrap();
        assert_eq!(resolved.config.limits.max_records, u64::MAX);
    }

    #[test]
    fn environment_integer_that_is_not_a_number_is_reported_with_its_key() {
        let layer = env_layer([("NURT_SPACE_POLL_MS", "fast")], ':').unwrap();
        let error = resolve(&[layer], false).unwrap_err();
        assert_eq!(error.key.as_deref(), Some("space.poll_ms"));
        assert_eq!(error.level, Some(ConfigLevel::Environment));
    }

    #[test]
    fn project_file_cannot_set_journal_keys_of_an_untrusted_project() {
        let values: Map<String, Json> =
            serde_json::from_str(r#"{"journal": {"max_ops": 5}, "output": {"head": 3}}"#).unwrap();
        let layers = [ConfigLayer::new(ConfigLevel::Project, ".nurt.toml", values)];
        let untrusted = resolve(&layers, false).unwrap();
        assert_eq!(untrusted.config.journal.max_ops, StepLimit::Limited(30));
        let kinds: Vec<_> = untrusted.notes.iter().map(|note| note.kind).collect();
        assert_eq!(
            kinds,
            [
                ConfigNoteKind::JournalConfigIgnored,
                ConfigNoteKind::ProjectConfigIgnored
            ]
        );
        let trusted = resolve(&layers, true).unwrap();
        assert_eq!(trusted.config.journal.max_ops, StepLimit::Limited(5));
        assert_eq!(trusted.source("journal.max_ops"), ConfigLevel::Project);
    }

    #[test]
    fn trust_keys_are_ignored_at_project_and_runtime_levels() {
        for level in [ConfigLevel::Project, ConfigLevel::Runtime] {
            let values: Map<String, Json> =
                serde_json::from_str(r#"{"scope": {"trusted_projects": ["/home/user/repo"]}}"#)
                    .unwrap();
            let layers = [ConfigLayer::new(level, "layer", values)];
            assert!(trusted_projects(&layers).unwrap().is_empty());
        }
    }

    #[test]
    fn age_limit_bounds_are_inclusive() {
        assert!("10s".parse::<AgeLimit>().is_ok());
        assert!("9s".parse::<AgeLimit>().is_err());
        assert!("365d".parse::<AgeLimit>().is_ok());
        assert!("365d1ms".parse::<AgeLimit>().is_err());
        assert!("52w1d".parse::<AgeLimit>().is_ok());
    }

    #[test]
    fn unknown_keys_are_reported_and_the_other_keys_are_applied() {
        let values: Map<String, Json> =
            serde_json::from_str(r#"{"nosuch": {"x": 1}, "output": {"head": 7}}"#).unwrap();
        let layers = [ConfigLayer::new(ConfigLevel::User, "nurt.toml", values)];
        let resolved = resolve(&layers, false).unwrap();
        assert_eq!(resolved.config.output.head, 7);
        assert_eq!(resolved.source("output.head"), ConfigLevel::User);
        assert_eq!(resolved.notes.len(), 1);
        assert_eq!(resolved.notes[0].kind, ConfigNoteKind::UnknownKey);
        assert_eq!(resolved.notes[0].key, "nosuch.x");
    }

    #[test]
    fn a_value_given_for_a_table_names_the_key_level_and_origin() {
        let values: Map<String, Json> = serde_json::from_str(r#"{"journal": 5}"#).unwrap();
        let layers = [ConfigLayer::new(ConfigLevel::User, "nurt.toml", values)];
        let error = resolve(&layers, false).unwrap_err();
        assert_eq!(error.key.as_deref(), Some("journal"));
        assert_eq!(error.level, Some(ConfigLevel::User));
        assert_eq!(error.origin.as_deref(), Some("nurt.toml"));
    }

    #[test]
    fn higher_levels_win_regardless_of_the_order_of_the_layers() {
        let layer = |level, text: &str| {
            let values: Map<String, Json> = serde_json::from_str(text).unwrap();
            ConfigLayer::new(level, "layer", values)
        };
        let layers = [
            layer(ConfigLevel::Environment, r#"{"journal": {"max_ops": 50}}"#),
            layer(
                ConfigLevel::User,
                r#"{"journal": {"max_ops": 40}, "run": {"timeout": "10s"}}"#,
            ),
            layer(ConfigLevel::User, r#"{"run": {"timeout": "20s"}}"#),
        ];
        let resolved = resolve(&layers, false).unwrap();
        assert_eq!(resolved.config.journal.max_ops, StepLimit::Limited(50));
        assert_eq!(resolved.source("journal.max_ops"), ConfigLevel::Environment);
        assert_eq!(resolved.config.run.timeout.as_millis(), 20_000);
    }

    #[test]
    fn settings_of_one_volume_are_keyed_by_the_mount_point() {
        let values: Map<String, Json> =
            serde_json::from_str(r#"{"journal": {"volume": {"/mnt/a.b": {"max_ops": 5}}}}"#)
                .unwrap();
        let key = r#"journal.volume."/mnt/a.b".max_ops"#;
        let user = [ConfigLayer::new(
            ConfigLevel::User,
            "nurt.toml",
            values.clone(),
        )];
        let resolved = resolve(&user, false).unwrap();
        let volume = resolved.config.journal.volume.get("/mnt/a.b").unwrap();
        assert_eq!(volume.max_ops, Some(StepLimit::Limited(5)));
        assert_eq!(resolved.source(key), ConfigLevel::User);
        let project = [ConfigLayer::new(ConfigLevel::Project, ".nurt.toml", values)];
        let ignored = resolve(&project, false).unwrap();
        assert!(ignored.config.journal.volume.is_empty());
        assert_eq!(ignored.notes[0].kind, ConfigNoteKind::JournalConfigIgnored);
        assert_eq!(ignored.notes[0].key, key);
    }

    #[test]
    fn keys_are_written_with_quotes_where_toml_requires_them() {
        let segments = ["a", "b c", "d\"e", "f\\g", ""].map(String::from);
        assert_eq!(display_key(&segments), r#"a."b c"."d\"e"."f\\g"."""#);
    }

    #[test]
    fn environment_layer_reads_lists_booleans_and_rejects_other_booleans() {
        let layer = env_layer(
            [
                ("NURT_PROJECT_ROOTS", "/home/user/a::/home/user/b: "),
                ("NURT_SYNTAX_TOLERANT", "NO"),
                ("NURT_THREADS", " 4 "),
            ],
            ':',
        )
        .unwrap();
        let resolved = resolve(&[layer], false).unwrap();
        assert_eq!(
            resolved.config.scope.project_roots,
            ["/home/user/a", "/home/user/b"]
        );
        assert!(!resolved.config.syntax.tolerant);
        assert_eq!(resolved.config.limits.threads, Some(4));
        let error = env_layer([("NURT_SYNTAX_TOLERANT", "maybe")], ':').unwrap_err();
        assert_eq!(error.key.as_deref(), Some("syntax.tolerant"));
    }

    #[test]
    fn every_environment_variable_sets_a_known_key() {
        let known = known_keys().unwrap();
        for var in ENV_VARS {
            assert!(known.iter().any(|key| key == var.key), "{}", var.key);
        }
        assert!(known.iter().any(|key| key == "journal.pin_max_age"));
        assert!(known.iter().any(|key| key == "journal.volume"));
        assert!(!known.iter().any(|key| key.starts_with("journal.volume.")));
    }
}
