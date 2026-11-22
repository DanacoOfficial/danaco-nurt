// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Document node tree with byte spans, positions, comments and style.
//!
//! A format parses its text into a [`DocTree`] with a [`TreeBuilder`]. Every
//! node records the byte span of the whole node, of its key and of its value,
//! the comment attached to it and the [`Style`] it is written in. Key paths
//! are resolved on the tree by [`DocTree::resolve`]; the differences between
//! formats (repeated names, flat keys, XML names) are described by a
//! [`FormatSemantics`] value. A [`DocHandle`] combines the tree with the
//! operations that change the document.
//!
//! ```
//! use nurt_contracts::keypath::KeyPath;
//! use nurt_contracts::value::Value;
//! use nurt_doc::model::{MapSemantics, NodeKind, NodeSpec, Span, TreeBuilder};
//!
//! let text = br#"{"a": 1}"#;
//! let mut builder = TreeBuilder::new(text);
//! let root = builder.node(None, NodeSpec::new(NodeKind::Map, Span::new(0, 8)));
//! builder.node(
//!     Some(root),
//!     NodeSpec::new(NodeKind::Scalar, Span::new(1, 7))
//!         .with_name("a")
//!         .with_key_span(Span::new(1, 4))
//!         .with_value_span(Span::new(6, 7))
//!         .with_value(Value::Int(1)),
//! );
//! let tree = builder.finish();
//! let found = tree.resolve(&KeyPath::parse("a")?, &MapSemantics::LAST_WINS)?;
//! assert_eq!(found[0].key.to_string(), "a");
//! assert_eq!(tree[found[0].node].start.line, 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod errors;
mod filter;
mod handle;
mod position;
mod resolve;
mod semantics;
mod span;
mod tree;

pub use errors::{ambiguous_key, doc_parse, key_conflict, key_not_found};
pub use handle::{DocHandle, SetOptions};
pub use position::LineIndex;
pub use resolve::Resolved;
pub use semantics::{FlatSemantics, FormatSemantics, MapSemantics, Repeated};
pub use span::Span;
pub use tree::{
    DocNode, DocTree, Layout, NodeId, NodeKind, NodeSpec, ScalarStyle, Style, TreeBuilder,
};
