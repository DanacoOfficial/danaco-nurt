// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! C ABI of the Danaco Nurt library.
//!
//! The crate exposes the engine through the functions declared in
//! `include/nurt.h`. Complex values cross the boundary as UTF-8 JSON, memory
//! returned by the library is released with `nurt_buf_free`, handles are
//! opaque and no panic crosses the boundary. Unsafe code is limited to the
//! boundary functions and every block documents its safety conditions.

#![deny(missing_docs)]
#![allow(
    unsafe_code,
    reason = "the C ABI boundary dereferences pointers passed by the caller"
)]
#![deny(clippy::undocumented_unsafe_blocks)]
