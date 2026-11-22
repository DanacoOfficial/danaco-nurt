// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! XML names and namespaces.
//!
//! Names follow the `Name` production of XML 1.0 (fifth edition); namespace
//! prefixes are resolved according to Namespaces in XML 1.0. A prefix that
//! is not declared leaves the name without a namespace instead of failing
//! the parse, so documents written for tools without namespace support
//! remain editable.

use super::tree::{NodeId, XmlTree};

/// Namespace bound to the reserved prefix `xml`.
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// Namespace bound to the reserved prefix `xmlns`.
pub const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

/// Attribute that declares the default namespace.
const DEFAULT_DECLARATION: &str = "xmlns";

/// Prefix of attributes that declare a namespace prefix.
const PREFIX_DECLARATION: &str = "xmlns:";

/// `true` for characters allowed as the first character of a name.
pub(crate) fn is_name_start_char(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}'
        | '\u{D8}'..='\u{F6}'
        | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}'
        | '\u{37F}'..='\u{1FFF}'
        | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}'
        | '\u{2C00}'..='\u{2FEF}'
        | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}'
        | '\u{FDF0}'..='\u{FFFD}'
        | '\u{10000}'..='\u{EFFFF}')
}

/// `true` for characters allowed after the first character of a name.
pub(crate) fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c,
            '-' | '.' | '0'..='9' | '\u{B7}'
            | '\u{300}'..='\u{36F}'
            | '\u{203F}'..='\u{2040}')
}

/// `true` when `name` matches the XML `Name` production.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::xml::is_valid_name;
///
/// assert!(is_valid_name("pom:project"));
/// assert!(is_valid_name("_id-2.x"));
/// assert!(!is_valid_name("2nd"));
/// assert!(!is_valid_name("a b"));
/// ```
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_name_start_char) && chars.all(is_name_char)
}

/// Splits a qualified name into prefix and local part at the first colon.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::xml::split_qname;
///
/// assert_eq!(split_qname("pom:project"), (Some("pom"), "project"));
/// assert_eq!(split_qname("project"), (None, "project"));
/// ```
pub fn split_qname(name: &str) -> (Option<&str>, &str) {
    match name.split_once(':') {
        Some((prefix, local)) if !prefix.is_empty() && !local.is_empty() => (Some(prefix), local),
        _ => (None, name),
    }
}

/// Expanded name written `{uri}local` in key paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExpandedName<'a> {
    pub(crate) namespace: Option<&'a str>,
    pub(crate) local: &'a str,
}

/// Parses the Clark notation `{uri}local`; `None` for other names.
pub(crate) fn parse_clark(name: &str) -> Option<ExpandedName<'_>> {
    let rest = name.strip_prefix('{')?;
    let (uri, local) = rest.split_once('}')?;
    Some(ExpandedName {
        namespace: (!uri.is_empty()).then_some(uri),
        local,
    })
}

/// Namespace declared by an attribute name: `Some(None)` for the default
/// namespace, `Some(Some(prefix))` for a prefix, `None` for other
/// attributes.
pub(crate) fn declared_prefix(attribute: &str) -> Option<Option<&str>> {
    if attribute == DEFAULT_DECLARATION {
        return Some(None);
    }
    attribute.strip_prefix(PREFIX_DECLARATION).map(Some)
}

impl XmlTree {
    /// Namespace bound to `prefix` (`None`: the default namespace) in the
    /// scope of `element`.
    ///
    /// Declarations are read from `element` and its ancestors; the reserved
    /// prefixes `xml` and `xmlns` are always bound. An empty default
    /// namespace declaration (`xmlns=""`) yields `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse(r#"<a xmlns:p="urn:p"><p:b/></a>"#)?;
    /// let b = tree.child_elements(tree.root()).next().unwrap();
    /// assert_eq!(tree.lookup_namespace(b, Some("p")).as_deref(), Some("urn:p"));
    /// assert_eq!(tree.lookup_namespace(b, None), None);
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn lookup_namespace(&self, element: NodeId, prefix: Option<&str>) -> Option<String> {
        match prefix {
            Some("xml") => return Some(XML_NAMESPACE.to_owned()),
            Some("xmlns") => return Some(XMLNS_NAMESPACE.to_owned()),
            _ => {}
        }
        let mut current = Some(element);
        while let Some(id) = current {
            if let Some(found) = self.declaration_in(id, prefix) {
                return found;
            }
            current = self.parent(id);
        }
        None
    }

    /// Declaration of `prefix` on `element` itself: `Some(namespace)` when
    /// declared there, `None` when the element does not declare it.
    fn declaration_in(&self, element: NodeId, prefix: Option<&str>) -> Option<Option<String>> {
        let el = self.element(element)?;
        el.attributes.iter().find_map(|attribute| {
            let name = &self.text()[attribute.name.clone()];
            (declared_prefix(name)? == prefix).then(|| {
                let value = self.attribute_value(attribute).unwrap_or_default();
                (!value.is_empty()).then_some(value)
            })
        })
    }

    /// Namespace of the element name, from its prefix or the default
    /// namespace in scope.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse(r#"<project xmlns="urn:pom"/>"#)?;
    /// assert_eq!(tree.namespace(tree.root()).as_deref(), Some("urn:pom"));
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn namespace(&self, element: NodeId) -> Option<String> {
        let (prefix, _) = split_qname(self.name(element));
        self.lookup_namespace(element, prefix)
    }

    /// `true` when a child element of `parent` matches the key segment
    /// `name`.
    ///
    /// `{uri}local` compares expanded names. `prefix:local` matches the same
    /// written name or an element whose expanded name equals the one of the
    /// segment, the prefix being resolved in the scope of the candidate.
    /// An unprefixed name matches the same written name.
    pub(crate) fn name_matches(&self, candidate: NodeId, segment: &str) -> bool {
        let written = self.name(candidate);
        if written == segment {
            return true;
        }
        if let Some(expanded) = parse_clark(segment) {
            let (_, local) = split_qname(written);
            return local == expanded.local
                && self.namespace(candidate).as_deref() == expanded.namespace;
        }
        let (Some(prefix), local) = split_qname(segment) else {
            return false;
        };
        let (_, candidate_local) = split_qname(written);
        candidate_local == local
            && self
                .lookup_namespace(candidate, Some(prefix))
                .is_some_and(|uri| self.namespace(candidate).as_deref() == Some(uri.as_str()))
    }

    /// `true` when `candidate`, a child of `parent`, is in the namespace of
    /// `parent` under the parent's prefix and has the local name `local`.
    ///
    /// Elements created under a prefixed parent receive the parent's
    /// prefix, so an unprefixed key segment also finds them.
    pub(crate) fn matches_in_parent_prefix(&self, candidate: NodeId, local: &str) -> bool {
        let Some(parent) = self.parent(candidate) else {
            return false;
        };
        let (Some(parent_prefix), _) = split_qname(self.name(parent)) else {
            return false;
        };
        split_qname(self.name(candidate)) == (Some(parent_prefix), local)
    }

    /// Written name for a new element or attribute called `requested`
    /// in the scope of the existing element `scope`.
    ///
    /// An unprefixed element name takes `parent_prefix`, the prefix of the
    /// new element's parent, so the new element stays in the parent's
    /// namespace; a name in Clark notation is written with a prefix bound to
    /// its namespace in scope. Returns `None` when the namespace has no
    /// prefix in scope, which would require a new namespace declaration.
    pub(crate) fn written_name(
        &self,
        scope: NodeId,
        parent_prefix: Option<&str>,
        requested: &str,
        is_element: bool,
    ) -> Option<String> {
        if let Some(expanded) = parse_clark(requested) {
            return self.prefixed_for(scope, &expanded, is_element);
        }
        if let (Some(prefix), _) = split_qname(requested) {
            return self
                .lookup_namespace(scope, Some(prefix))
                .map(|_| requested.to_owned());
        }
        match parent_prefix {
            Some(prefix) if is_element => Some(format!("{prefix}:{requested}")),
            _ => Some(requested.to_owned()),
        }
    }

    /// Written name for an expanded name in the scope of `parent`.
    fn prefixed_for(
        &self,
        parent: NodeId,
        expanded: &ExpandedName<'_>,
        is_element: bool,
    ) -> Option<String> {
        let default = self.lookup_namespace(parent, None);
        if is_element && default.as_deref() == expanded.namespace {
            return Some(expanded.local.to_owned());
        }
        let uri = expanded.namespace?;
        self.prefixes_in_scope(parent)
            .into_iter()
            .find(|prefix| self.lookup_namespace(parent, Some(prefix)).as_deref() == Some(uri))
            .map(|prefix| format!("{prefix}:{}", expanded.local))
    }

    /// Prefixes declared on `element` and its ancestors, nearest first.
    fn prefixes_in_scope(&self, element: NodeId) -> Vec<String> {
        let mut prefixes = Vec::new();
        let mut current = Some(element);
        while let Some(id) = current {
            if let Some(el) = self.element(id) {
                for attribute in &el.attributes {
                    let name = &self.text()[attribute.name.clone()];
                    if let Some(Some(prefix)) = declared_prefix(name) {
                        if !prefixes.iter().any(|known| known == prefix) {
                            prefixes.push(prefix.to_owned());
                        }
                    }
                }
            }
            current = self.parent(id);
        }
        prefixes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clark_notation_splits_namespace_and_local_name() {
        let name = parse_clark("{urn:x}item").unwrap();
        assert_eq!(name.namespace, Some("urn:x"));
        assert_eq!(name.local, "item");
        assert_eq!(parse_clark("{}item").unwrap().namespace, None);
        assert!(parse_clark("item").is_none());
    }

    #[test]
    fn namespace_declarations_are_recognized() {
        assert_eq!(declared_prefix("xmlns"), Some(None));
        assert_eq!(declared_prefix("xmlns:p"), Some(Some("p")));
        assert_eq!(declared_prefix("xmlnsx"), None);
    }

    #[test]
    fn qualified_name_without_local_part_is_unprefixed() {
        assert_eq!(split_qname(":a"), (None, ":a"));
        assert_eq!(split_qname("a:"), (None, "a:"));
    }
}
