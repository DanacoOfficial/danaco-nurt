// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::lexical
// Responsibility:   Lexical correction rules.
// Dependencies:     front, rules
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Lexical correction rules.
//!
//! The rules `L01` to `L24` correct the notation of a program: names, quotes,
//! brackets, pipes, separators, white space and the notation of paths. Each
//! rule has one certainty class (see [`RULES`]). Rules that need the type of
//! an argument place (`L01`, `L02`, `L05`, `L06`, `L19`, `L22`, `L23`, `L24`
//! and the joining of units for `L17`) are functions called by the binder;
//! the other rules run in [`front`], from the program text to a surface tree.
//!
//! Rules apply to names and syntax only. The bytes of literals, numbers, the
//! stages that select targets, the kind of a changing stage, the parameters of
//! the effect mode and the base directory are never changed.

pub mod front;
mod literal;
mod locate;
pub mod names;
pub mod paths;
pub mod repairs;
mod rules;
pub(crate) mod scan;
pub mod spacing;
pub mod tokens;
pub mod tree;

pub use front::{Front, front, front_for_binding};
pub use rules::{RULES, Rule, RuleInfo, number_corrections};

/// Shape of a token that may follow the arguments of a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ArgumentShape {
    /// Bare word, such as a command name or a value name.
    Word,
    /// Text in quotes.
    Text,
    /// Regular expression.
    Regex,
    /// Number, size or duration.
    Number,
}

/// View of the command catalog needed by the lexical rules.
///
/// The binder implements it on the loaded catalog; tests implement it on a
/// fixed list of commands.
pub trait CatalogView {
    /// Whether `name` is the name of a command, such as `find` or
    /// `git.status`.
    fn is_command(&self, name: &str) -> bool;

    /// Names of every command, in a stable order.
    fn command_names(&self) -> &[&str];

    /// Whether the command changes something or runs a program.
    fn is_mutating(&self, name: &str) -> bool;

    /// Whether the command accepts a positional argument of the given shape at
    /// the given position, counted from 0.
    fn can_take_argument(&self, command: &str, position: usize, shape: ArgumentShape) -> bool;
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
