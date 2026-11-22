// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Classification enumerations shared by the catalog, the syntax trees and the
//! report.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Kind of effect a stage has on the world, recorded in the bound tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    /// Reads state only; no effect outside the process.
    Read,
    /// Changes local files or content; recorded in the journal and reversible.
    Local,
    /// Has an effect outside the local file system (network, services, remote
    /// systems); not reversible by the journal.
    External,
    /// Runs a user program whose effects are detected and classified after it
    /// finishes.
    Process,
}

/// Category of a command in the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Starts a pipeline by producing records.
    Source,
    /// Transforms a record stream.
    Transform,
    /// Checks a record stream or an effect.
    Check,
    /// Changes files, content or other local state.
    Change,
    /// Runs a program.
    Run,
    /// Inspects or administers the operating system.
    System,
    /// Inspects or manages the operation journal.
    Journal,
    /// Provides help and descriptions.
    Help,
}

/// Reversibility of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Reversibility {
    /// The command only reads; there is nothing to reverse.
    Read,
    /// The command is always reversible from the journal.
    Reversible,
    /// The command is reversible when the previous content was preserved;
    /// otherwise the report marks it as irreversible.
    ReversibleIfPreserved,
    /// The command is irreversible by nature.
    Irreversible,
}

/// How a command is implemented with respect to external programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Implementation {
    /// Fully built into the executable.
    Builtin,
    /// Runs a program supplied by the user.
    UserProgram,
    /// Acts as a client of an external language server.
    LanguageServer,
    /// Wraps the package manager of the operating system.
    PackageManager,
    /// Uses a component of the operating system that has no public
    /// programming interface; explicitly marked in the catalog.
    SystemProgram,
}

/// Notation variant of the text form of a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyntaxVariant {
    /// Variant A, the word notation.
    #[default]
    A,
    /// Variant B, the call notation.
    B,
    /// Variant C, the named notation.
    C,
}
