// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Text processing for Danaco Nurt.
//!
//! The crate implements character encodings and their detection, line ending
//! handling, literal and regular expression search, differences, patches and
//! content hashes.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod codec;
pub mod codec_service;
pub mod detect;
pub mod diff;
pub mod edit_plan;
pub mod eol;
pub mod fuzzy_match;
pub mod hash;
pub mod patch;
pub mod search;
pub mod template;

pub use codec_service::TextCodec;
