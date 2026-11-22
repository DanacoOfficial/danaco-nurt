// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reader of the Windows event log.
//!
//! Events are read through the Windows Event Log API (`wevtapi`): one query
//! per channel in reverse direction, filtered by an XPath expression built
//! from the parameters of `logs`, and rendered as event XML. The channel
//! streams are merged newest first and every event passes the final filter
//! of the crate. The modules that build queries, parse event XML and map
//! events do not call the operating system and are compiled on every
//! platform. Only the event source of the operating system, `WevtSource`, is
//! specific to Windows.
//!
//! | Parameter of `logs` | Event log |
//! |---|---|
//! | `unit` | channel; `System` and `Application` when absent |
//! | `source` | provider (`Provider/@Name`) |
//! | `level` | threshold on `Level` |
//! | `since`, `until` | `TimeCreated/@SystemTime` |
//! | `message` | formatted message |

pub mod channels;
pub mod level;
pub mod record;
pub mod source;
pub mod store;
#[cfg(windows)]
mod wevtapi;
pub mod xml;
pub mod xpath;

#[cfg(windows)]
pub use wevtapi::{EventHandle, WevtEvents, WevtSource};
