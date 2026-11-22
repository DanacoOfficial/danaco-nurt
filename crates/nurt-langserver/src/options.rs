// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Settings of a language server session.

use nurt_contracts::spec::StageSpec;
use nurt_syntax::parse::Variant;
use serde_json::Value;

/// Key of the syntax settings in the initialization options.
const SYNTAX_KEY: &str = "syntax";

/// Key of the variant inside the syntax settings.
const VARIANT_KEY: &str = "variant";

/// Dotted form of the variant key, as used by editor settings.
const DOTTED_VARIANT_KEY: &str = "syntax.variant";

/// Settings of a language server session.
///
/// The client selects another syntax variant with the initialization option
/// named `syntax.variant`, whose value is `"a"`, `"b"` or `"c"`. The option
/// is accepted as a nested object such as `{"syntax": {"variant": "b"}}` and
/// as a dotted key such as `{"syntax.variant": "b"}`.
///
/// # Examples
///
/// ```
/// use nurt_langserver::Options;
/// use nurt_syntax::parse::Variant;
///
/// let options = Options::new(Vec::new()).with_variant(Variant::B);
/// assert_eq!(options.variant, Variant::B);
/// assert!(options.commands.is_empty());
/// ```
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Syntax variant of the program files when the client selects none.
    pub variant: Variant,
    /// Command signatures of the catalog, used for completion and hover.
    pub commands: Vec<StageSpec>,
}

impl Options {
    /// Creates options with the command signatures of a catalog and
    /// variant A.
    #[must_use]
    pub fn new(commands: Vec<StageSpec>) -> Self {
        Self {
            variant: Variant::A,
            commands,
        }
    }

    /// Returns the options with another default syntax variant.
    #[must_use]
    pub fn with_variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }
}

/// Reads the syntax variant from the initialization options of the client.
///
/// Returns `default` when the options select no variant, together with a
/// warning when the selected value is not a variant name.
pub(crate) fn variant_from(options: Option<&Value>, default: Variant) -> (Variant, Option<String>) {
    let selected = options.and_then(|options| {
        options
            .get(SYNTAX_KEY)
            .and_then(|syntax| syntax.get(VARIANT_KEY))
            .or_else(|| options.get(DOTTED_VARIANT_KEY))
    });
    let name = match selected {
        None | Some(Value::Null) => return (default, None),
        Some(Value::String(name)) => name.to_ascii_lowercase(),
        Some(other) => other.to_string(),
    };
    match name.as_str() {
        "a" => (Variant::A, None),
        "b" => (Variant::B, None),
        "c" => (Variant::C, None),
        _ => {
            let warning = format!(
                "unknown syntax variant {name}; use \"a\", \"b\" or \"c\"; variant \"{}\" is used",
                variant_name(default)
            );
            (default, Some(warning))
        }
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

    #[test]
    fn reads_nested_and_dotted_variant_keys() {
        let nested = json!({"syntax": {"variant": "b"}});
        assert_eq!(variant_from(Some(&nested), Variant::A), (Variant::B, None));
        let dotted = json!({"syntax.variant": "C"});
        assert_eq!(variant_from(Some(&dotted), Variant::A), (Variant::C, None));
        let first = json!({"syntax": {"variant": "a"}});
        assert_eq!(variant_from(Some(&first), Variant::B), (Variant::A, None));
    }

    #[test]
    fn keeps_the_default_without_a_selection() {
        assert_eq!(variant_from(None, Variant::B), (Variant::B, None));
        let empty = json!({"syntax": {}});
        assert_eq!(variant_from(Some(&empty), Variant::C), (Variant::C, None));
        let null = json!({"syntax.variant": null});
        assert_eq!(variant_from(Some(&null), Variant::A), (Variant::A, None));
    }

    #[test]
    fn warns_about_unknown_variants() {
        let unknown = json!({"syntax": {"variant": "d"}});
        let (variant, warning) = variant_from(Some(&unknown), Variant::A);
        assert_eq!(variant, Variant::A);
        assert_eq!(
            warning.as_deref(),
            Some("unknown syntax variant d; use \"a\", \"b\" or \"c\"; variant \"a\" is used")
        );
        let number = json!({"syntax.variant": 2});
        assert!(variant_from(Some(&number), Variant::C).1.is_some());
    }
}
