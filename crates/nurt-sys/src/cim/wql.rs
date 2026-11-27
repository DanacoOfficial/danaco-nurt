// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::cim::wql
// Responsibility:   WQL queries built from the parameters of `windows.cim`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! WQL queries built from the parameters of `windows.cim`.
//!
//! The class and the property names are restricted to identifiers, so they
//! cannot change the structure of the statement. The condition is passed to
//! WMI as written after checks that keep it a single `WHERE` clause of a data
//! query: no control characters, balanced string literals and none of the
//! association query forms.

use nurt_contracts::error::{Error, ErrorCode, Result};

/// Separator of namespace segments accepted in the parameter.
const NAMESPACE_SEPARATOR: char = '/';

/// Separator of namespace segments expected by `IWbemLocator::ConnectServer`.
const WMI_NAMESPACE_SEPARATOR: char = '\\';

/// Description of a valid identifier, reported as `expected`.
const IDENTIFIER_RULE: &str = "identifier matching [A-Za-z_][A-Za-z0-9_]*";

/// Keyword pairs that start association and reference queries, which are
/// not data queries and are not accepted in a condition.
const FORBIDDEN_PAIRS: [(&str, &str); 2] = [("ASSOCIATORS", "OF"), ("REFERENCES", "OF")];

/// Validated query of `windows.cim`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CimQuery {
    /// CIM class.
    pub(crate) class: String,
    /// WQL condition without the `WHERE` keyword.
    pub(crate) condition: Option<String>,
    /// Namespace as given, with `/` separators.
    pub(crate) namespace: String,
    /// Properties to read in order; empty for all properties.
    pub(crate) properties: Vec<String>,
}

impl CimQuery {
    /// Validates the parameters of the command.
    ///
    /// An empty or blank condition is treated as absent, an empty list of
    /// properties selects all properties, and repeated property names are
    /// kept once.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` with `expected` when the class, a property or a
    /// namespace segment is not an identifier, or when the condition holds a
    /// control character, an unterminated string literal or an association
    /// query keyword.
    pub(crate) fn new(
        class: &str,
        condition: Option<&str>,
        namespace: &str,
        properties: &[String],
    ) -> Result<Self> {
        check_identifier("class", class)?;
        let mut selected: Vec<String> = Vec::with_capacity(properties.len());
        for property in properties {
            check_identifier("properties", property)?;
            if !selected
                .iter()
                .any(|known| known.eq_ignore_ascii_case(property))
            {
                selected.push(property.clone());
            }
        }
        check_namespace(namespace)?;
        let condition = match condition.map(str::trim) {
            Some(text) if !text.is_empty() => {
                check_condition(text)?;
                Some(text.to_owned())
            }
            _ => None,
        };
        Ok(Self {
            class: class.to_owned(),
            condition,
            namespace: namespace.to_owned(),
            properties: selected,
        })
    }

    /// The WQL statement: `SELECT <properties|*> FROM <class> [WHERE <condition>]`.
    #[cfg(any(windows, test))]
    pub(crate) fn wql(&self) -> String {
        let columns = if self.properties.is_empty() {
            "*".to_owned()
        } else {
            self.properties.join(", ")
        };
        let mut statement = format!("SELECT {columns} FROM {}", self.class);
        if let Some(condition) = &self.condition {
            statement.push_str(" WHERE ");
            statement.push_str(condition);
        }
        statement
    }

    /// The namespace in the form expected by WMI, with `\` separators.
    #[cfg(any(windows, test))]
    pub(crate) fn wmi_namespace(&self) -> String {
        wmi_namespace(&self.namespace)
    }
}

/// Converts a namespace written with `/` separators to the form expected by
/// WMI; `\` separators are kept.
#[cfg(any(windows, test))]
pub(crate) fn wmi_namespace(namespace: &str) -> String {
    namespace.replace(NAMESPACE_SEPARATOR, &WMI_NAMESPACE_SEPARATOR.to_string())
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|rest| rest.is_ascii_alphanumeric() || rest == '_')
}

fn invalid(param: &str, found: &str, message: String, expected: &str) -> Error {
    Error::new(ErrorCode::InvalidValue, message)
        .with_found(found)
        .with_expected(serde_json::Value::from(expected))
        .with_detail("param", serde_json::Value::from(param))
}

fn check_identifier(param: &str, text: &str) -> Result<()> {
    if is_identifier(text) {
        return Ok(());
    }
    Err(invalid(
        param,
        text,
        format!("`{param}` must be a CIM name such as Win32_Service, not `{text}`"),
        IDENTIFIER_RULE,
    ))
}

fn check_namespace(namespace: &str) -> Result<()> {
    let valid = namespace
        .split([NAMESPACE_SEPARATOR, WMI_NAMESPACE_SEPARATOR])
        .all(is_identifier);
    if valid {
        return Ok(());
    }
    Err(invalid(
        "namespace",
        namespace,
        format!("`namespace` must be a path of CIM names such as root/cimv2, not `{namespace}`"),
        "names matching [A-Za-z_][A-Za-z0-9_]* separated by /",
    ))
}

/// Words of the condition outside string literals, in upper case.
///
/// Returns `None` when a string literal is not terminated. WQL string
/// literals are delimited by `'` or `"` and use `\` to escape the next
/// character.
fn words_outside_literals(condition: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in condition.chars() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == delimiter {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
        }
        if ch.is_ascii_alphanumeric() || ch == '_' {
            word.push(ch.to_ascii_uppercase());
        } else if !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
    }
    if quote.is_some() {
        return None;
    }
    if !word.is_empty() {
        words.push(word);
    }
    Some(words)
}

fn check_condition(condition: &str) -> Result<()> {
    if let Some(control) = condition.chars().find(|ch| ch.is_control()) {
        return Err(invalid(
            "where",
            condition,
            format!(
                "`where` must not contain control characters; remove U+{:04X}",
                u32::from(control)
            ),
            "WQL condition on one line",
        ));
    }
    let Some(words) = words_outside_literals(condition) else {
        return Err(invalid(
            "where",
            condition,
            "`where` has an unterminated string literal; close every ' and \"".to_owned(),
            "WQL condition with balanced quotes",
        ));
    };
    let forbidden = words.windows(2).find_map(|pair| {
        FORBIDDEN_PAIRS
            .iter()
            .find(|(first, second)| pair[0] == *first && pair[1] == *second)
    });
    if let Some((first, second)) = forbidden {
        return Err(invalid(
            "where",
            condition,
            format!("`where` must be a condition on properties, not a `{first} {second}` query"),
            "WQL condition of a data query",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Class, condition, namespace, properties and the parameter reported
    /// in the error.
    type InvalidCase<'a> = (&'a str, Option<&'a str>, &'a str, &'a [&'a str], &'a str);

    fn query(
        class: &str,
        condition: Option<&str>,
        namespace: &str,
        properties: &[&str],
    ) -> Result<CimQuery> {
        let properties: Vec<String> = properties.iter().map(|name| (*name).to_owned()).collect();
        CimQuery::new(class, condition, namespace, &properties)
    }

    #[test]
    fn cim_wql_builder_select_where_properties() {
        let all = query("Win32_OperatingSystem", None, "root/cimv2", &[]).unwrap();
        assert_eq!(all.wql(), "SELECT * FROM Win32_OperatingSystem");

        let some = query(
            "Win32_Service",
            Some("State = 'Running'"),
            "root/cimv2",
            &["Name", "State", "name"],
        )
        .unwrap();
        assert_eq!(
            some.wql(),
            "SELECT Name, State FROM Win32_Service WHERE State = 'Running'"
        );

        let blank = query("Win32_Process", Some("   "), "root/cimv2", &[]).unwrap();
        assert_eq!(blank.condition, None);
        assert_eq!(blank.wql(), "SELECT * FROM Win32_Process");

        let trimmed = query("Win32_Process", Some(" ProcessId = 4 "), "root/cimv2", &[]).unwrap();
        assert_eq!(
            trimmed.wql(),
            "SELECT * FROM Win32_Process WHERE ProcessId = 4"
        );
    }

    #[test]
    fn cim_wql_rejects_invalid_identifiers_and_control_chars() {
        let cases: [InvalidCase<'_>; 9] = [
            ("Win32_Service; DROP", None, "root/cimv2", &[], "class"),
            ("1Class", None, "root/cimv2", &[], "class"),
            ("", None, "root/cimv2", &[], "class"),
            (
                "Win32_Service",
                None,
                "root/cimv2",
                &["Name,State"],
                "properties",
            ),
            ("Win32_Service", None, "root/../cimv2", &[], "namespace"),
            ("Win32_Service", None, "", &[], "namespace"),
            (
                "Win32_Service",
                Some("Name = 'a'\nOR 1=1"),
                "root/cimv2",
                &[],
                "where",
            ),
            (
                "Win32_Service",
                Some("Name = 'unterminated"),
                "root/cimv2",
                &[],
                "where",
            ),
            (
                "Win32_Service",
                Some("Name = 'x' ASSOCIATORS  of {Win32_Service}"),
                "root/cimv2",
                &[],
                "where",
            ),
        ];
        for (class, condition, namespace, properties, param) in cases {
            let error = query(class, condition, namespace, properties).unwrap_err();
            assert_eq!(
                error.code(),
                ErrorCode::InvalidValue,
                "{class} {condition:?}"
            );
            assert_eq!(error.details["param"], param, "{class} {condition:?}");
            assert!(error.expected.is_some());
        }
    }

    #[test]
    fn cim_wql_accepts_keywords_and_quotes_inside_literals() {
        let accepted = [
            "Name = 'references of x'",
            "Caption = \"it's\"",
            "Name = 'a\\'b' AND State <> \"Stopped\"",
            "PathName LIKE '%associators of%'",
        ];
        for condition in accepted {
            let built = query("Win32_Service", Some(condition), "root/cimv2", &[]).unwrap();
            assert_eq!(built.condition.as_deref(), Some(condition));
        }
    }

    #[test]
    fn cim_namespace_slash_to_backslash() {
        let built = query("MSFT_Disk", None, "root/Microsoft/Windows/Storage", &[]).unwrap();
        assert_eq!(built.wmi_namespace(), "root\\Microsoft\\Windows\\Storage");
        assert_eq!(wmi_namespace("root\\cimv2"), "root\\cimv2");
        assert_eq!(wmi_namespace("root/cimv2"), "root\\cimv2");
        assert!(query("Win32_Bios", None, "root\\cimv2", &[]).is_ok());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
