// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Small element tree of an XML report.

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

/// Element of a report with its attributes, text and child elements.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Element {
    pub(crate) name: String,
    pub(crate) attributes: Vec<(String, String)>,
    /// Character data directly inside the element, concatenated.
    pub(crate) text: String,
    pub(crate) children: Vec<Element>,
}

impl Element {
    /// Value of an attribute.
    pub(crate) fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Child elements with the given name.
    pub(crate) fn children_named<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }
}

/// Tree read from a document and the reason the document ended early, when
/// it did. Elements still open at that point are closed, so that what was
/// read can be used.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Parsed {
    pub(crate) root: Element,
    pub(crate) error: Option<String>,
}

/// Reads a document into a tree under an unnamed root element.
pub(crate) fn parse(text: &str) -> Parsed {
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut error = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => stack.push(element(&start)),
            Ok(Event::Empty(start)) => attach(&mut stack, element(&start)),
            Ok(Event::End(_)) => {
                if stack.len() > 1 {
                    if let Some(done) = stack.pop() {
                        attach(&mut stack, done);
                    }
                }
            }
            Ok(Event::Text(chunk)) => push_text(&mut stack, &chunk.xml10_content()),
            Ok(Event::CData(chunk)) => push_text(&mut stack, &chunk.xml10_content()),
            Ok(Event::GeneralRef(reference)) => {
                push_text(&mut stack, &resolve(&reference));
            }
            Ok(Event::Eof) => {
                if stack.len() > 1 {
                    error = Some("the document ends inside an element".to_owned());
                }
                break;
            }
            Ok(_) => {}
            Err(failure) => {
                error = Some(failure.to_string());
                break;
            }
        }
    }
    while stack.len() > 1 {
        if let Some(done) = stack.pop() {
            attach(&mut stack, done);
        }
    }
    Parsed {
        root: stack.pop().unwrap_or_default(),
        error,
    }
}

fn element(start: &BytesStart<'_>) -> Element {
    let qname = start.name();
    let name: &str = qname.as_ref();
    let name = name.to_owned();
    let attributes = start
        .attributes()
        .flatten()
        .filter_map(|attribute| {
            let key: &str = attribute.key.as_ref();
            let key = key.to_owned();
            let value = attribute.normalized_value(XmlVersion::Implicit1_0).ok()?;
            Some((key, value.into_owned()))
        })
        .collect();
    Element {
        name,
        attributes,
        text: String::new(),
        children: Vec::new(),
    }
}

fn attach(stack: &mut [Element], done: Element) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(done);
    }
}

fn push_text(stack: &mut [Element], text: &str) {
    if let Some(current) = stack.last_mut() {
        current.text.push_str(text);
    }
}

fn resolve(reference: &quick_xml::events::BytesRef<'_>) -> String {
    if let Ok(Some(character)) = reference.resolve_char_ref() {
        return character.to_string();
    }
    match &**reference {
        "lt" => "<".to_owned(),
        "gt" => ">".to_owned(),
        "amp" => "&".to_owned(),
        "quot" => "\"".to_owned(),
        "apos" => "'".to_owned(),
        other => format!("&{other};"),
    }
}
