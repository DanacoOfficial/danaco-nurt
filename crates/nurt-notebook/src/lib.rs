// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Jupyter notebooks for Danaco Nurt.
//!
//! The crate reads and edits cells of `.ipynb` notebooks (nbformat 4) while
//! preserving key order, and exports notebooks to scripts and Markdown.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
