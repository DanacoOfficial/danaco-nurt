// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-tooldef::error
// Responsibility:   Error type of the crate.
// Dependencies:     thiserror
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Error type of the crate.

use thiserror::Error;

/// Error of rendering a tool definition or of reading the arguments of a
/// tool call.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ToolDefError {
    /// The description of the core group is empty.
    #[error("tool description is empty; pass the text of the core description")]
    EmptyDescription,
    /// A selected group has no description.
    #[error(
        "no description for group `{group}`; pass the text of the group description or remove the group"
    )]
    MissingGroupDescription {
        /// Name of the group.
        group: String,
    },
    /// A group name is empty or contains characters other than lowercase
    /// ASCII letters, digits and `-`.
    #[error(
        "invalid group name `{name}`; use lowercase letters, digits and `-`, or `core` and `all`"
    )]
    InvalidGroupName {
        /// Name given.
        name: String,
    },
    /// The catalog version is empty.
    #[error("catalog version is empty; pass the version of the command catalog or omit it")]
    EmptyCatalogVersion,
    /// The assembled description is longer than the limit given by the
    /// caller.
    #[error(
        "tool description has {chars} characters, above the limit of {limit}; select fewer groups or shorten the group descriptions"
    )]
    DescriptionTooLong {
        /// Characters of the assembled description.
        chars: usize,
        /// Limit given by the caller.
        limit: usize,
    },
    /// The name of a tool format is not known.
    #[error("unknown tool format `{name}`; expected one of {expected}")]
    UnknownFormat {
        /// Name given.
        name: String,
        /// Known names, separated by commas.
        expected: String,
    },
    /// The arguments of a tool call cannot be turned into a request.
    #[error("invalid tool arguments: {reason}")]
    InvalidArguments {
        /// What is wrong and how to correct it.
        reason: String,
    },
    /// The request schema has a construct the generator does not translate.
    #[error("unsupported request schema: {reason}")]
    SourceSchema {
        /// The construct and its location.
        reason: String,
    },
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
