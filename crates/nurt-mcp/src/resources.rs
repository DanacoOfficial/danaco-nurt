// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Resources of the server under the `nurt://` scheme.
//!
//! | URI | Content |
//! |---|---|
//! | `nurt://help/{topic}` | built-in help of a topic |
//! | `nurt://schema/{name}` | published JSON schema: `ast`, `bound`, `report`, `request`, `config`, `tool` |
//! | `nurt://describe/core` | core description layer |
//! | `nurt://describe/group/{name}` | description layer of a group of commands |

use rmcp::model::{Resource, ResourceTemplate};

use crate::backend::{DescribeSubject, SchemaName};

/// Media type of help and description layers.
pub(crate) const TEXT_MIME: &str = "text/plain";

/// Media type of JSON schemas.
pub(crate) const SCHEMA_MIME: &str = "application/schema+json";

const SCHEME: &str = "nurt://";
const HELP_PREFIX: &str = "help/";
const SCHEMA_PREFIX: &str = "schema/";
const DESCRIBE_CORE: &str = "describe/core";
const DESCRIBE_GROUP_PREFIX: &str = "describe/group/";

/// A resource named by a `nurt://` URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NurtResource {
    /// Built-in help of a topic.
    Help(String),
    /// A published JSON schema.
    Schema(SchemaName),
    /// A description layer.
    Describe(DescribeSubject),
}

impl NurtResource {
    /// Parses a resource URI; `None` for a URI that names no resource.
    pub(crate) fn parse(uri: &str) -> Option<Self> {
        let path = uri.strip_prefix(SCHEME)?;
        if let Some(topic) = path.strip_prefix(HELP_PREFIX) {
            return segment(topic).map(Self::Help);
        }
        if let Some(name) = path.strip_prefix(SCHEMA_PREFIX) {
            return SchemaName::from_name(name).map(Self::Schema);
        }
        if path == DESCRIBE_CORE {
            return Some(Self::Describe(DescribeSubject::Core));
        }
        if let Some(name) = path.strip_prefix(DESCRIBE_GROUP_PREFIX) {
            return segment(name).map(|name| Self::Describe(DescribeSubject::Group(name)));
        }
        None
    }

    /// Media type of the content.
    pub(crate) fn mime_type(&self) -> &'static str {
        match self {
            Self::Schema(_) => SCHEMA_MIME,
            Self::Help(_) | Self::Describe(_) => TEXT_MIME,
        }
    }
}

/// Decodes one non-empty path segment with percent-encoded octets.
fn segment(text: &str) -> Option<String> {
    if text.is_empty() || text.contains('/') {
        return None;
    }
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'%' {
            let high = bytes.get(index + 1).copied().and_then(hex_value)?;
            let low = bytes.get(index + 2).copied().and_then(hex_value)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(byte);
            index += 1;
        }
    }
    String::from_utf8(decoded)
        .ok()
        .filter(|text| !text.is_empty())
}

fn hex_value(digit: u8) -> Option<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

/// Resources with a fixed URI.
pub(crate) fn fixed_resources() -> Vec<Resource> {
    let mut resources: Vec<Resource> = SchemaName::ALL
        .iter()
        .map(|name| {
            Resource::new(
                format!("{SCHEME}{SCHEMA_PREFIX}{}", name.as_str()),
                format!("schema-{}", name.as_str()),
            )
            .with_title(format!("Schema {}", name.as_str()))
            .with_description(format!(
                "Published JSON schema `{}` of Danaco Nurt.",
                name.as_str()
            ))
            .with_mime_type(SCHEMA_MIME)
        })
        .collect();
    resources.push(
        Resource::new(format!("{SCHEME}{DESCRIBE_CORE}"), "describe-core")
            .with_title("Core description")
            .with_description(
                "Core description layer of Danaco Nurt: rules, syntax, core commands and examples.",
            )
            .with_mime_type(TEXT_MIME),
    );
    resources
}

/// Templates of resources with a parameter.
pub(crate) fn resource_templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate::new(format!("{SCHEME}{HELP_PREFIX}{{topic}}"), "help")
            .with_title("Help")
            .with_description(
                "Built-in help of a command, group, error code, `syntax` or `corrections`.",
            )
            .with_mime_type(TEXT_MIME),
        ResourceTemplate::new(format!("{SCHEME}{SCHEMA_PREFIX}{{name}}"), "schema")
            .with_title("Schema")
            .with_description(
                "Published JSON schema: `ast`, `bound`, `report`, `request`, `config` or `tool`.",
            )
            .with_mime_type(SCHEMA_MIME),
        ResourceTemplate::new(
            format!("{SCHEME}{DESCRIBE_GROUP_PREFIX}{{name}}"),
            "describe-group",
        )
        .with_title("Group description")
        .with_description("Description layer of a group of commands, for example `git`.")
        .with_mime_type(TEXT_MIME),
    ]
}

#[cfg(test)]
mod tests {
    use super::{NurtResource, fixed_resources, resource_templates, segment};
    use crate::backend::{DescribeSubject, SchemaName};

    #[test]
    fn parses_every_kind_of_uri() {
        assert_eq!(
            NurtResource::parse("nurt://help/replace"),
            Some(NurtResource::Help("replace".to_owned()))
        );
        assert_eq!(
            NurtResource::parse("nurt://schema/bound"),
            Some(NurtResource::Schema(SchemaName::Bound))
        );
        assert_eq!(
            NurtResource::parse("nurt://describe/core"),
            Some(NurtResource::Describe(DescribeSubject::Core))
        );
        assert_eq!(
            NurtResource::parse("nurt://describe/group/git"),
            Some(NurtResource::Describe(DescribeSubject::Group(
                "git".to_owned()
            )))
        );
    }

    #[test]
    fn rejects_uris_that_name_no_resource() {
        for uri in [
            "file:///etc/passwd",
            "nurt://help/",
            "nurt://help/a/b",
            "nurt://schema/catalog",
            "nurt://describe",
            "nurt://describe/group/",
            "nurt://other/x",
        ] {
            assert_eq!(NurtResource::parse(uri), None, "{uri}");
        }
    }

    #[test]
    fn decodes_percent_encoded_segments() {
        assert_eq!(segment("E_SYNTAX").as_deref(), Some("E_SYNTAX"));
        assert_eq!(segment("git%2Ecommit").as_deref(), Some("git.commit"));
        assert_eq!(segment("caf%C3%A9").as_deref(), Some("caf\u{e9}"));
        assert_eq!(segment("%2"), None);
        assert_eq!(segment("%zz"), None);
        assert_eq!(segment("%FF"), None);
        assert_eq!(segment("%2F"), Some("/".to_owned()));
    }

    #[test]
    fn media_types_follow_the_content() {
        assert_eq!(
            NurtResource::Schema(SchemaName::Ast).mime_type(),
            "application/schema+json"
        );
        assert_eq!(
            NurtResource::Help("find".to_owned()).mime_type(),
            "text/plain"
        );
    }

    #[test]
    fn listed_resources_parse_back() {
        let resources = fixed_resources();
        assert_eq!(resources.len(), SchemaName::ALL.len() + 1);
        for resource in resources {
            assert!(
                NurtResource::parse(&resource.uri).is_some(),
                "{}",
                resource.uri
            );
        }
        let templates: Vec<String> = resource_templates()
            .into_iter()
            .map(|template| template.uri_template)
            .collect();
        assert_eq!(
            templates,
            [
                "nurt://help/{topic}",
                "nurt://schema/{name}",
                "nurt://describe/group/{name}"
            ]
        );
    }
}
