// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Settings of a language server session.

use std::sync::Arc;

use nurt_bind::Catalog;
use nurt_contracts::config::Config;
use nurt_contracts::spec::StageSpec;
use nurt_help::catalog::Catalog as HelpCatalog;
use nurt_syntax::parse::Variant;
use serde_json::Value;

/// Section of the syntax settings in the initialization options.
const SYNTAX: &str = "syntax";

/// Key of the syntax variant inside the syntax settings.
const VARIANT: &str = "variant";

/// Key of the tolerant reading of other variants inside the syntax settings.
const TOLERANT: &str = "tolerant";

/// Section of the correction settings in the initialization options.
const CORRECTION: &str = "correction";

/// Key that switches correction on and off inside the correction settings.
const ENABLED: &str = "enabled";

/// Settings of a language server session.
///
/// With a binding catalog ([`Options::from_catalog`]) the diagnostics
/// include the corrections and the errors of binding; with command
/// signatures only ([`Options::new`]) they include the corrections of
/// notation and the errors of reading. With the help catalog
/// ([`Options::with_help`]) hover shows the help of commands and of error
/// codes.
///
/// The client overrides three settings with its initialization options:
/// `syntax.variant` (`"a"`, `"b"` or `"c"`), `syntax.tolerant` and
/// `correction.enabled` (`true` or `false`). Each is accepted as a nested
/// object such as `{"syntax": {"variant": "b"}}` and as a dotted key such
/// as `{"syntax.variant": "b"}`.
///
/// # Examples
///
/// ```
/// use nurt_langserver::Options;
/// use nurt_syntax::parse::Variant;
///
/// let options = Options::new(Vec::new()).with_variant(Variant::B);
/// assert_eq!(options.variant, Variant::B);
/// assert!(options.commands.is_empty() && options.catalog.is_none());
/// assert!(options.config.correction.enabled);
/// ```
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Syntax variant of the program files when the client selects none.
    pub variant: Variant,
    /// Command signatures, used for completion, hover, signatures and
    /// highlighting.
    pub commands: Vec<StageSpec>,
    /// Catalog that programs are bound to; without it the diagnostics stop
    /// at the correction of notation.
    pub catalog: Option<Arc<Catalog>>,
    /// Catalog with the documentation that hover describes commands and
    /// error codes from.
    pub help: Option<Arc<HelpCatalog>>,
    /// Configuration of correction and binding. Its syntax variant is kept
    /// equal to [`Options::variant`].
    pub config: Config,
}

impl Options {
    /// Creates options with the command signatures of a catalog, variant A
    /// and the default configuration.
    #[must_use]
    pub fn new(commands: Vec<StageSpec>) -> Self {
        Self {
            variant: Variant::A,
            commands,
            ..Self::default()
        }
    }

    /// Creates options that bind programs to `catalog` and take the command
    /// signatures from it.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use nurt_bind::Catalog;
    /// use nurt_langserver::Options;
    ///
    /// let catalog = Catalog::load_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog"))?;
    /// let options = Options::from_catalog(catalog);
    /// assert!(options.commands.iter().any(|command| command.name == "find"));
    /// # Ok::<(), nurt_bind::CatalogError>(())
    /// ```
    #[must_use]
    pub fn from_catalog(catalog: Catalog) -> Self {
        Self {
            commands: catalog.commands().to_vec(),
            catalog: Some(Arc::new(catalog)),
            ..Self::new(Vec::new())
        }
    }

    /// Returns the options with the catalog that hover takes help from.
    #[must_use]
    pub fn with_help(mut self, help: HelpCatalog) -> Self {
        self.help = Some(Arc::new(help));
        self
    }

    /// Returns the options with another default syntax variant.
    #[must_use]
    pub fn with_variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self.config.syntax.variant = variant;
        self
    }

    /// Returns the options with another configuration; its syntax variant
    /// becomes the default variant.
    #[must_use]
    pub fn with_config(mut self, config: Config) -> Self {
        self.variant = config.syntax.variant;
        self.config = config;
        self
    }
}

/// Settings of a session after the initialization options of the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Settings {
    /// Syntax variant of the documents.
    pub(crate) variant: Variant,
    /// Whether a statement written in another variant is read and corrected.
    pub(crate) tolerant: bool,
    /// Whether notation is corrected.
    pub(crate) correction: bool,
}

/// Reads the settings from the initialization options of the client.
///
/// A missing or `null` value keeps the value of `defaults`. Every value of
/// the wrong form keeps it as well and adds a warning for the client.
pub(crate) fn settings(options: Option<&Value>, defaults: &Options) -> (Settings, Vec<String>) {
    let mut warnings = Vec::new();
    let variant = match read(options, SYNTAX, VARIANT) {
        None => defaults.variant,
        Some(value) => variant_of(value).unwrap_or_else(|| {
            warnings.push(format!(
                "unknown syntax variant {}; use \"a\", \"b\" or \"c\"; variant \"{}\" is used",
                shown(value),
                variant_name(defaults.variant)
            ));
            defaults.variant
        }),
    };
    let tolerant = flag(
        options,
        (SYNTAX, TOLERANT),
        defaults.config.syntax.tolerant,
        &mut warnings,
    );
    let correction = flag(
        options,
        (CORRECTION, ENABLED),
        defaults.config.correction.enabled,
        &mut warnings,
    );
    let settings = Settings {
        variant,
        tolerant,
        correction,
    };
    (settings, warnings)
}

/// Returns the value of `section.key`, nested or dotted, unless it is
/// missing or `null`.
fn read<'a>(options: Option<&'a Value>, section: &str, key: &str) -> Option<&'a Value> {
    let options = options?;
    let dotted = format!("{section}.{key}");
    let value = options
        .get(section)
        .and_then(|settings| settings.get(key))
        .or_else(|| options.get(dotted.as_str()))?;
    (!value.is_null()).then_some(value)
}

fn flag(
    options: Option<&Value>,
    (section, key): (&str, &str),
    default: bool,
    warnings: &mut Vec<String>,
) -> bool {
    match read(options, section, key) {
        None => default,
        Some(Value::Bool(value)) => *value,
        Some(other) => {
            warnings.push(format!(
                "invalid value {} of {section}.{key}; use true or false; {default} is used",
                shown(other)
            ));
            default
        }
    }
}

fn variant_of(value: &Value) -> Option<Variant> {
    match value.as_str()?.to_ascii_lowercase().as_str() {
        "a" => Some(Variant::A),
        "b" => Some(Variant::B),
        "c" => Some(Variant::C),
        _ => None,
    }
}

/// Returns a value as the client wrote it, a text without quotes.
fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Returns the option value that selects `variant`.
fn variant_name(variant: Variant) -> &'static str {
    match variant {
        Variant::A => "a",
        Variant::B => "b",
        Variant::C => "c",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn read_settings(options: &Value, defaults: &Options) -> (Settings, Vec<String>) {
        settings(Some(options), defaults)
    }

    #[test]
    fn reads_nested_and_dotted_variant_keys() {
        let defaults = Options::default();
        let nested = read_settings(&json!({"syntax": {"variant": "b"}}), &defaults);
        assert_eq!(nested.0.variant, Variant::B);
        let dotted = read_settings(&json!({"syntax.variant": "C"}), &defaults);
        assert_eq!(dotted.0.variant, Variant::C);
        let first = read_settings(
            &json!({"syntax": {"variant": "a"}}),
            &defaults.clone().with_variant(Variant::B),
        );
        assert_eq!((first.0.variant, first.1.len()), (Variant::A, 0));
    }

    #[test]
    fn keeps_the_defaults_without_a_selection() {
        let defaults = Options::new(Vec::new()).with_variant(Variant::B);
        let (none, warnings) = settings(None, &defaults);
        assert_eq!(
            none,
            Settings {
                variant: Variant::B,
                tolerant: true,
                correction: true
            }
        );
        assert!(warnings.is_empty());
        let empty = read_settings(&json!({"syntax": {}}), &defaults);
        assert_eq!(empty.0.variant, Variant::B);
        let null = read_settings(&json!({"syntax.variant": null}), &defaults);
        assert_eq!((null.0.variant, null.1.len()), (Variant::B, 0));
    }

    #[test]
    fn reads_tolerant_syntax_and_correction_switches() {
        let defaults = Options::default();
        let nested = read_settings(
            &json!({"syntax": {"tolerant": false}, "correction": {"enabled": false}}),
            &defaults,
        );
        assert_eq!((nested.0.tolerant, nested.0.correction), (false, false));
        let dotted = read_settings(
            &json!({"syntax.tolerant": false, "correction.enabled": true}),
            &defaults,
        );
        assert_eq!((dotted.0.tolerant, dotted.0.correction), (false, true));
    }

    #[test]
    fn warns_about_values_of_the_wrong_form() {
        let defaults = Options::default();
        let (unknown, warnings) = read_settings(&json!({"syntax": {"variant": "d"}}), &defaults);
        assert_eq!(unknown.variant, Variant::A);
        assert_eq!(
            warnings,
            ["unknown syntax variant d; use \"a\", \"b\" or \"c\"; variant \"a\" is used"]
        );
        let (_, number) = read_settings(&json!({"syntax.variant": 2}), &defaults);
        assert_eq!(number.len(), 1);
        let (switches, warnings) = read_settings(
            &json!({"syntax": {"tolerant": "no"}, "correction.enabled": 0}),
            &defaults,
        );
        assert!(switches.tolerant && switches.correction);
        assert_eq!(
            warnings,
            [
                "invalid value no of syntax.tolerant; use true or false; true is used",
                "invalid value 0 of correction.enabled; use true or false; true is used"
            ]
        );
    }

    #[test]
    fn keeps_the_variant_of_the_configuration_in_step() {
        let mut config = Config::default();
        config.syntax.variant = Variant::C;
        let options = Options::new(Vec::new()).with_config(config);
        assert_eq!(options.variant, Variant::C);
        let options = options.with_variant(Variant::B);
        assert_eq!(options.config.syntax.variant, Variant::B);
    }
}
