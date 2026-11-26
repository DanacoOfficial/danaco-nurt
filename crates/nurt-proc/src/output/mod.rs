// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Spool files, decoding and response views of program output.

use std::sync::Arc;

use crate::iface::SpoolFactory;

mod cursor;
mod decode;
mod filter;
mod formats;
mod progress;
mod select;
mod spool;
mod window;

#[cfg(test)]
mod tests;

/// Spool files in the `spool` directory of the state directory.
pub(crate) fn spool_factory() -> Arc<dyn SpoolFactory> {
    Arc::new(spool::SpoolDir::default())
}
