// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Syntax of `registry:` paths and their mapping to provider keys.
//!
//! A path has the form `registry:/<root>/<key>/…/<name>`. The root is one of
//! `HKLM`, `HKCU`, `HKCR`, `HKU` and `HKCC`, or the full name of the
//! predefined key (`HKEY_LOCAL_MACHINE` and so on), which is normalised to the
//! short form. The last segment names a key or a value; which one is decided
//! against the registry by [`RegistryProvider::resolve`](super::RegistryProvider::resolve).
//! The segment `(default)` addresses the default value of a key, whose name
//! is empty.
//!
//! Key names may contain `/`, which the language uses as the separator, so a
//! segment escapes `%` as `%25` and `/` as `%2F`. No other character is
//! escaped.

use std::fmt;

use nurt_contracts::fs::{ProviderKey, ProviderScheme};

/// Scheme of the provider, without the trailing colon.
pub const SCHEME: &str = "registry";

/// Prefix of every path of the provider.
pub const PREFIX: &str = "registry:/";

/// Last segment that addresses the default value of a key.
pub const DEFAULT_VALUE_SEGMENT: &str = "(default)";

const SEPARATOR: char = '/';
const ESCAPE: char = '%';
const ESCAPED_PERCENT: &str = "%25";
const ESCAPED_SEPARATOR: &str = "%2F";

/// Predefined root key of the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hive {
    /// `HKEY_LOCAL_MACHINE`, written `HKLM`.
    LocalMachine,
    /// `HKEY_CURRENT_USER`, written `HKCU`.
    CurrentUser,
    /// `HKEY_CLASSES_ROOT`, written `HKCR`.
    ClassesRoot,
    /// `HKEY_USERS`, written `HKU`.
    Users,
    /// `HKEY_CURRENT_CONFIG`, written `HKCC`.
    CurrentConfig,
}

impl Hive {
    /// Every root key, in the order of the predefined handles.
    pub const ALL: [Self; 5] = [
        Self::ClassesRoot,
        Self::CurrentUser,
        Self::LocalMachine,
        Self::Users,
        Self::CurrentConfig,
    ];

    /// Short name used in paths, for example `HKLM`.
    pub const fn short_name(self) -> &'static str {
        match self {
            Self::LocalMachine => "HKLM",
            Self::CurrentUser => "HKCU",
            Self::ClassesRoot => "HKCR",
            Self::Users => "HKU",
            Self::CurrentConfig => "HKCC",
        }
    }

    /// Full name of the predefined key, for example `HKEY_LOCAL_MACHINE`.
    pub const fn full_name(self) -> &'static str {
        match self {
            Self::LocalMachine => "HKEY_LOCAL_MACHINE",
            Self::CurrentUser => "HKEY_CURRENT_USER",
            Self::ClassesRoot => "HKEY_CLASSES_ROOT",
            Self::Users => "HKEY_USERS",
            Self::CurrentConfig => "HKEY_CURRENT_CONFIG",
        }
    }

    /// Root key named by its short or full name, ignoring case.
    ///
    /// ```
    /// use nurt_sys::registry::path::Hive;
    ///
    /// assert_eq!(Hive::from_name("hkey_current_user"), Some(Hive::CurrentUser));
    /// assert_eq!(Hive::from_name("HKCU"), Some(Hive::CurrentUser));
    /// assert_eq!(Hive::from_name("HKEY_PERFORMANCE_DATA"), None);
    /// ```
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|hive| {
            name.eq_ignore_ascii_case(hive.short_name())
                || name.eq_ignore_ascii_case(hive.full_name())
        })
    }
}

impl fmt::Display for Hive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.short_name())
    }
}

/// Compares two registry names the way the registry does: without regard
/// to case.
pub fn names_equal(left: &str, right: &str) -> bool {
    left.chars()
        .flat_map(char::to_uppercase)
        .eq(right.chars().flat_map(char::to_uppercase))
}

/// Path of a key: a root key and the names of the keys below it.
///
/// Names keep the spelling they were given; comparisons ignore case.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyPath {
    hive: Hive,
    keys: Vec<String>,
}

impl KeyPath {
    /// Path of `keys` below `hive`.
    pub fn new(hive: Hive, keys: Vec<String>) -> Self {
        Self { hive, keys }
    }

    /// Path of the root key itself.
    pub fn root(hive: Hive) -> Self {
        Self::new(hive, Vec::new())
    }

    /// Root key.
    pub fn hive(&self) -> Hive {
        self.hive
    }

    /// Names of the keys below the root key.
    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    /// Whether the path names a root key.
    pub fn is_root(&self) -> bool {
        self.keys.is_empty()
    }

    /// Name of the last key, or the short name of the root key.
    pub fn name(&self) -> &str {
        self.keys
            .last()
            .map_or(self.hive.short_name(), String::as_str)
    }

    /// Parent key; `None` for a root key.
    pub fn parent(&self) -> Option<Self> {
        let (_, parent) = self.keys.split_last()?;
        Some(Self::new(self.hive, parent.to_vec()))
    }

    /// Path of the subkey `name`.
    pub fn child(&self, name: &str) -> Self {
        let mut keys = self.keys.clone();
        keys.push(name.to_owned());
        Self::new(self.hive, keys)
    }

    /// The first `depth` keys of the path (`0` gives the root key).
    pub fn prefix(&self, depth: usize) -> Self {
        let depth = depth.min(self.keys.len());
        Self::new(
            self.hive,
            self.keys.get(..depth).unwrap_or_default().to_vec(),
        )
    }

    /// Whether `self` is `ancestor` or lies below it, ignoring case.
    pub fn starts_with(&self, ancestor: &Self) -> bool {
        self.hive == ancestor.hive
            && self.keys.len() >= ancestor.keys.len()
            && self
                .keys
                .iter()
                .zip(&ancestor.keys)
                .all(|(left, right)| names_equal(left, right))
    }

    /// Whether both paths name the same key, ignoring case.
    pub fn same_key(&self, other: &Self) -> bool {
        self.keys.len() == other.keys.len() && self.starts_with(other)
    }

    /// Native form used by the Win32 interface: names joined by `\`, without
    /// the root key.
    pub fn native_subkey(&self) -> String {
        self.keys.join("\\")
    }

    /// Path in the language form, for example `registry:/HKCU/Software`.
    pub fn to_language_string(&self) -> String {
        let mut text = String::from(PREFIX);
        text.push_str(self.hive.short_name());
        for key in &self.keys {
            text.push(SEPARATOR);
            text.push_str(&escape_segment(key));
        }
        text
    }

    /// Path of a value of this key in the language form; the default value
    /// is written as `(default)`.
    pub fn value_language_string(&self, name: &str) -> String {
        let mut text = self.to_language_string();
        text.push(SEPARATOR);
        if name.is_empty() {
            text.push_str(DEFAULT_VALUE_SEGMENT);
        } else {
            text.push_str(&escape_segment(name));
        }
        text
    }

    /// Provider key addressing this key.
    pub fn to_provider_key(&self) -> ProviderKey {
        self.provider_key(None)
    }

    /// Provider key addressing the value `name` of this key.
    pub fn value_provider_key(&self, name: &str) -> ProviderKey {
        self.provider_key(Some(name.to_owned()))
    }

    fn provider_key(&self, value: Option<String>) -> ProviderKey {
        let mut segments = Vec::with_capacity(self.keys.len() + 1);
        segments.push(self.hive.short_name().to_owned());
        segments.extend(self.keys.iter().cloned());
        ProviderKey {
            scheme: ProviderScheme(SCHEME.to_owned()),
            segments,
            value,
        }
    }
}

impl fmt::Display for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_language_string())
    }
}

/// Key or value addressed by a provider key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// A key.
    Key(KeyPath),
    /// A value of a key; the empty name is the default value.
    Value(KeyPath, String),
}

impl Address {
    /// Key of the address: the key itself or the key holding the value.
    pub fn key(&self) -> &KeyPath {
        match self {
            Self::Key(key) | Self::Value(key, _) => key,
        }
    }

    /// Provider key of the address.
    pub fn to_provider_key(&self) -> ProviderKey {
        match self {
            Self::Key(key) => key.to_provider_key(),
            Self::Value(key, name) => key.value_provider_key(name),
        }
    }

    /// Address of a provider key of the `registry` scheme.
    ///
    /// # Errors
    ///
    /// [`PathError::Scheme`] for another scheme and [`PathError::Root`] when
    /// the first segment is not a root key.
    pub fn from_provider_key(key: &ProviderKey) -> Result<Self, PathError> {
        if key.scheme.0 != SCHEME {
            return Err(PathError::Scheme);
        }
        let (root, keys) = key.segments.split_first().ok_or(PathError::Empty)?;
        let hive = Hive::from_name(root).ok_or_else(|| PathError::Root(root.clone()))?;
        let path = KeyPath::new(hive, keys.to_vec());
        Ok(match &key.value {
            Some(name) => Self::Value(path, name.clone()),
            None => Self::Key(path),
        })
    }

    /// Path in the language form.
    pub fn to_language_string(&self) -> String {
        match self {
            Self::Key(key) => key.to_language_string(),
            Self::Value(key, name) => key.value_language_string(name),
        }
    }
}

/// Parsed `registry:` path whose last segment is not yet known to name a key
/// or a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryPath {
    hive: Hive,
    segments: Vec<String>,
    default_value: bool,
}

impl RegistryPath {
    /// Parses a path in the language form.
    ///
    /// ```
    /// use nurt_sys::registry::path::{Hive, RegistryPath};
    ///
    /// let path = RegistryPath::parse("registry:/HKEY_CURRENT_USER/Environment/EDITOR")?;
    /// assert_eq!(path.hive(), Hive::CurrentUser);
    /// assert_eq!(path.segments(), ["Environment", "EDITOR"]);
    /// assert_eq!(path.to_string(), "registry:/HKCU/Environment/EDITOR");
    /// # Ok::<(), nurt_sys::registry::path::PathError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`PathError`] when the prefix, the root key, a segment or an escape
    /// sequence is not valid.
    pub fn parse(text: &str) -> Result<Self, PathError> {
        let rest = strip_prefix_ignore_case(text, PREFIX).ok_or(PathError::Scheme)?;
        let rest = rest.strip_suffix(SEPARATOR).unwrap_or(rest);
        let mut parts = rest.split(SEPARATOR);
        let root = parts
            .next()
            .filter(|root| !root.is_empty())
            .ok_or(PathError::Empty)?;
        let hive = Hive::from_name(root).ok_or_else(|| PathError::Root(root.to_owned()))?;
        let mut segments = parts.map(unescape_segment).collect::<Result<Vec<_>, _>>()?;
        let default_value = segments
            .last()
            .is_some_and(|last| last.eq_ignore_ascii_case(DEFAULT_VALUE_SEGMENT));
        if default_value {
            segments.pop();
        }
        Ok(Self {
            hive,
            segments,
            default_value,
        })
    }

    /// Root key.
    pub fn hive(&self) -> Hive {
        self.hive
    }

    /// Segments below the root key, unescaped, without a final `(default)`.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// Whether the path ends with `(default)`.
    pub fn is_default_value(&self) -> bool {
        self.default_value
    }

    /// The path read as a key.
    ///
    /// Returns `None` when the path ends with `(default)`.
    pub fn as_key(&self) -> Option<KeyPath> {
        (!self.default_value).then(|| KeyPath::new(self.hive, self.segments.clone()))
    }

    /// The path read as a value: the last segment is the value name, or the
    /// default value for `(default)`.
    ///
    /// Returns `None` for a root key without `(default)`.
    pub fn as_value(&self) -> Option<(KeyPath, String)> {
        if self.default_value {
            return Some((
                KeyPath::new(self.hive, self.segments.clone()),
                String::new(),
            ));
        }
        let (name, keys) = self.segments.split_last()?;
        Some((KeyPath::new(self.hive, keys.to_vec()), name.clone()))
    }
}

impl fmt::Display for RegistryPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = KeyPath::new(self.hive, self.segments.clone());
        if self.default_value {
            f.write_str(&key.value_language_string(""))
        } else {
            f.write_str(&key.to_language_string())
        }
    }
}

/// Error in the syntax of a `registry:` path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// The path does not start with `registry:/`.
    #[error("path must start with `registry:/`")]
    Scheme,
    /// The path names no root key.
    #[error("path must name a root key such as `HKCU` or `HKLM`")]
    Empty,
    /// The first segment is not a root key.
    #[error("unknown root key `{0}`; use HKLM, HKCU, HKCR, HKU or HKCC")]
    Root(String),
    /// A segment between two separators is empty.
    #[error("empty key name; remove the repeated `/`")]
    EmptySegment,
    /// A `%` is not followed by `25` or `2F`.
    #[error("invalid escape in `{0}`; write `%25` for `%` and `%2F` for `/`")]
    Escape(String),
}

/// Escapes a key or value name for use as one path segment.
///
/// ```
/// use nurt_sys::registry::path::escape_segment;
///
/// assert_eq!(escape_segment("application/json"), "application%2Fjson");
/// assert_eq!(escape_segment("100%"), "100%25");
/// ```
pub fn escape_segment(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for character in name.chars() {
        match character {
            ESCAPE => escaped.push_str(ESCAPED_PERCENT),
            SEPARATOR => escaped.push_str(ESCAPED_SEPARATOR),
            other => escaped.push(other),
        }
    }
    escaped
}

fn unescape_segment(segment: &str) -> Result<String, PathError> {
    if segment.is_empty() {
        return Err(PathError::EmptySegment);
    }
    let mut name = String::with_capacity(segment.len());
    let mut rest = segment;
    while let Some(index) = rest.find(ESCAPE) {
        let (head, tail) = rest.split_at(index);
        name.push_str(head);
        let code = tail.get(..ESCAPED_PERCENT.len()).unwrap_or(tail);
        if code.eq_ignore_ascii_case(ESCAPED_PERCENT) {
            name.push(ESCAPE);
        } else if code.eq_ignore_ascii_case(ESCAPED_SEPARATOR) {
            name.push(SEPARATOR);
        } else {
            return Err(PathError::Escape(segment.to_owned()));
        }
        rest = tail.get(code.len()..).unwrap_or_default();
    }
    name.push_str(rest);
    Ok(name)
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| text.get(prefix.len()..))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_path_parse_roots_aliases_case_insensitive() {
        let cases = [
            ("registry:/HKLM/SOFTWARE", Hive::LocalMachine),
            ("registry:/hkey_local_machine/SOFTWARE", Hive::LocalMachine),
            ("REGISTRY:/hkcu/SOFTWARE", Hive::CurrentUser),
            ("registry:/HKEY_CLASSES_ROOT/SOFTWARE", Hive::ClassesRoot),
            ("registry:/Hku/SOFTWARE", Hive::Users),
            (
                "registry:/HKEY_CURRENT_CONFIG/SOFTWARE",
                Hive::CurrentConfig,
            ),
        ];
        for (text, hive) in cases {
            let path = RegistryPath::parse(text).unwrap();
            assert_eq!(path.hive(), hive, "{text}");
            assert_eq!(path.segments(), ["SOFTWARE"]);
            assert_eq!(
                path.to_string(),
                format!("registry:/{}/SOFTWARE", hive.short_name())
            );
        }
        let lower = RegistryPath::parse("registry:/HKLM/software/microsoft").unwrap();
        let upper = RegistryPath::parse("registry:/HKLM/SOFTWARE/Microsoft").unwrap();
        assert!(lower.as_key().unwrap().same_key(&upper.as_key().unwrap()));
        assert_eq!(lower.segments(), ["software", "microsoft"]);
        assert_eq!(
            RegistryPath::parse("registry:/HKEY_PERFORMANCE_DATA/x"),
            Err(PathError::Root("HKEY_PERFORMANCE_DATA".to_owned()))
        );
        assert_eq!(RegistryPath::parse("env:/PATH"), Err(PathError::Scheme));
        assert_eq!(RegistryPath::parse("registry:/"), Err(PathError::Empty));
    }

    #[test]
    fn registry_path_default_value_addressing() {
        let path = RegistryPath::parse("registry:/HKCU/Software/Example/(Default)").unwrap();
        assert!(path.is_default_value());
        assert_eq!(path.as_key(), None);
        let (key, name) = path.as_value().unwrap();
        assert_eq!(key.to_language_string(), "registry:/HKCU/Software/Example");
        assert_eq!(name, "");
        assert_eq!(
            path.to_string(),
            "registry:/HKCU/Software/Example/(default)"
        );
        let provider = key.value_provider_key(&name);
        assert_eq!(provider.segments, ["HKCU", "Software", "Example"]);
        assert_eq!(provider.value.as_deref(), Some(""));
        let address = Address::from_provider_key(&provider).unwrap();
        assert_eq!(
            address.to_language_string(),
            "registry:/HKCU/Software/Example/(default)"
        );
    }

    #[test]
    fn root_path_reads_as_key_only() {
        let path = RegistryPath::parse("registry:/HKLM/").unwrap();
        assert!(path.as_key().unwrap().is_root());
        assert_eq!(path.as_value(), None);
        assert_eq!(path.as_key().unwrap().name(), "HKLM");
        assert_eq!(path.as_key().unwrap().parent(), None);
    }

    #[test]
    fn escaped_separator_round_trips() {
        let path =
            RegistryPath::parse("registry:/HKCR/MIME/Database/Content Type/application%2fjson")
                .unwrap();
        assert_eq!(path.segments().last().unwrap(), "application/json");
        assert_eq!(
            path.to_string(),
            "registry:/HKCR/MIME/Database/Content Type/application%2Fjson"
        );
        assert_eq!(unescape_segment("100%25").unwrap(), "100%");
        assert!(matches!(
            RegistryPath::parse("registry:/HKCU/a%zz"),
            Err(PathError::Escape(_))
        ));
        assert!(matches!(
            RegistryPath::parse("registry:/HKCU/a%2"),
            Err(PathError::Escape(_))
        ));
        assert_eq!(
            RegistryPath::parse("registry:/HKCU//x"),
            Err(PathError::EmptySegment)
        );
    }

    #[test]
    fn key_path_relations_ignore_case() {
        let key = KeyPath::new(Hive::CurrentUser, vec!["Software".into(), "Ä".into()]);
        let other = KeyPath::new(Hive::CurrentUser, vec!["SOFTWARE".into(), "ä".into()]);
        assert!(key.same_key(&other));
        assert!(key.starts_with(&KeyPath::root(Hive::CurrentUser)));
        assert!(!key.starts_with(&KeyPath::root(Hive::LocalMachine)));
        assert_eq!(key.prefix(1).keys(), ["Software"]);
        assert_eq!(key.prefix(9), key);
        assert_eq!(key.native_subkey(), "Software\\Ä");
        assert_eq!(key.child("x").name(), "x");
        assert_eq!(Hive::Users.to_string(), "HKU");
        assert_eq!(key.to_string(), "registry:/HKCU/Software/Ä");
    }

    #[test]
    fn provider_key_of_another_scheme_is_rejected() {
        let mut key = KeyPath::root(Hive::Users).to_provider_key();
        key.scheme = ProviderScheme("env".to_owned());
        assert_eq!(Address::from_provider_key(&key), Err(PathError::Scheme));
        key.scheme = ProviderScheme(SCHEME.to_owned());
        key.segments.clear();
        assert_eq!(Address::from_provider_key(&key), Err(PathError::Empty));
        key.segments.push("HKXX".to_owned());
        assert_eq!(
            Address::from_provider_key(&key),
            Err(PathError::Root("HKXX".to_owned()))
        );
    }
}
