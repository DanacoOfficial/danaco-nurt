// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-report
// Responsibility:   Effect report of Danaco Nurt.
// Dependencies:     options, render, shape
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Effect report of Danaco Nurt.
//!
//! The crate builds `nurt-report/1` reports from planned and executed effects
//! and renders them as text, JSON, Markdown and compact output. It applies the
//! rules for notable effects, irreversibility and bulk summaries.
//!
//! Rendering has two steps. [`shape`] reduces a report to the verbosity,
//! limits and field selection of a call; [`render`] writes the result in the
//! requested format; [`respond`] builds the response envelope with the reduced
//! report and the rendered text.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod assemble;
pub mod rules;

mod compact;
mod facts;
mod lines;
mod markdown;
mod options;
mod render;
mod shape;
mod text;
mod tree;
mod units;

pub use options::{DEFAULT_DIFF_MAX_HUNKS, DEFAULT_DIFF_MAX_LINES, RenderOptions};
pub use render::{render, respond};
pub use shape::{Omitted, Shaped, shape};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
