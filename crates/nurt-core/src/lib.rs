// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Engine facade of Danaco Nurt.
//!
//! The engine connects syntax, correction, binding, planning, execution,
//! journal and reporting behind a single request interface. It owns the
//! configuration, sessions and result cursors and is shared by the command
//! line interface, the C ABI, the protocol servers and the language bindings.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
