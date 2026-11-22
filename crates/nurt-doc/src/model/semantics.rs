// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use nurt_contracts::value::Value;

use super::{DocTree, NodeId};

/// Meaning of a name that several members of one map or element share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Repeated {
    /// The last member counts (duplicate keys in JSON, YAML, INI, property
    /// files and environment files).
    LastWins,
    /// The members form a group addressed by index (`dep[2]`, repeated XML
    /// elements); a path without an index is ambiguous and fails with
    /// `E_KEY_CONFLICT`.
    Conflict,
}

/// Rules of a format for resolving key paths and building values.
///
/// ```
/// use nurt_doc::model::{FlatSemantics, FormatSemantics, MapSemantics, Repeated};
///
/// assert_eq!(MapSemantics::LAST_WINS.repeated(), Repeated::LastWins);
/// assert!(FlatSemantics.flat_keys());
/// ```
pub trait FormatSemantics: Send + Sync {
    /// Meaning of a name shared by several members.
    fn repeated(&self) -> Repeated;

    /// Returns `true` when member names are flat dotted keys (`server.port`
    /// in property files), so that consecutive name segments of a key path
    /// are joined with `.` into one name.
    fn flat_keys(&self) -> bool {
        false
    }

    /// Returns `true` when the member `node` is selected by `name`.
    ///
    /// The default compares the node name exactly; XML overrides it to
    /// match qualified names and `{uri}local` names.
    fn name_matches(&self, tree: &DocTree, node: NodeId, name: &str) -> bool {
        tree[node].name.as_deref() == Some(name)
    }

    /// Form of one document value in which the meaning of a change is
    /// compared.
    ///
    /// The meaning guard passes the expected value of a changed document and
    /// the value read back from its bytes through this function before
    /// comparing them, so distinctions that documents of the format do not
    /// keep are not reported as differences. A format whose text has no
    /// types maps every scalar to text; a format whose groups of repeated
    /// members read as a single member when one remains maps a list of one
    /// element to that element. The default returns the value unchanged.
    ///
    /// ```
    /// use nurt_contracts::value::Value;
    /// use nurt_doc::model::{FormatSemantics, MapSemantics};
    ///
    /// assert_eq!(MapSemantics::LAST_WINS.canonical(Value::Int(1)), Value::Int(1));
    /// ```
    fn canonical(&self, value: Value) -> Value {
        value
    }
}

/// Semantics of formats whose maps nest (JSON, YAML, TOML, INI sections,
/// property lists, tables).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MapSemantics {
    repeated: Repeated,
}

impl MapSemantics {
    /// Duplicate names: the last member counts.
    pub const LAST_WINS: Self = Self {
        repeated: Repeated::LastWins,
    };

    /// Duplicate names form an indexed group.
    pub const CONFLICT: Self = Self {
        repeated: Repeated::Conflict,
    };
}

impl FormatSemantics for MapSemantics {
    fn repeated(&self) -> Repeated {
        self.repeated
    }
}

/// Semantics of flat key-value formats (property files, environment files):
/// `server.port` names the single member `"server.port"`, and the last of
/// duplicate members counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FlatSemantics;

impl FormatSemantics for FlatSemantics {
    fn repeated(&self) -> Repeated {
        Repeated::LastWins
    }

    fn flat_keys(&self) -> bool {
        true
    }
}
