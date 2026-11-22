// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! SSH transport and commit signatures for Danaco Nurt.
//!
//! The crate provides the built-in `ssh://` transport used by fetch, clone,
//! pull and push, with key and SSH agent authentication, and creates and
//! verifies OpenPGP and SSH commit signatures.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
