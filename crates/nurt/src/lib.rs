// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rust interface of Danaco Nurt, a command shell and language where every
//! change can be undone.
//!
//! A [`Nurt`] client runs programs of the Nurt command language through a
//! `nurt serve` process; an [`Embedded`] engine runs them inside the calling
//! process. Both return effect reports. Requests are
//! [`Request`] envelopes (`nurt-request/1`); responses are [`Response`]
//! envelopes (`nurt-response/1`) holding the report and the rendered text.
//!
//! Two kinds of failure are kept apart. A language error (syntax, binding,
//! assertion, stale target) is part of a successful response and is read
//! with [`Response::error`]; it carries a stable `E_*` code, the position and
//! ready fixes. A call error ([`Error`]) means that the call itself failed:
//! the executable was not found, the process ended or the protocol was
//! broken.
//!
//! # Features
//!
//! - `embedded` (default): the [`Embedded`] engine, built on `nurt-core`.
//! - `process` (default): the client of `nurt serve --stdio` and of
//!   connections given by the application. It depends only on `serde_json`.
//!   An application that needs only this client disables the default
//!   features and enables `process`.
//! - `cli`: the `nurt` executable, which runs `nurt_cli::main_with_args`.
//!   `cargo install nurt --features cli` installs it.
//!
//! # Examples
//!
//! ```no_run
//! # #[cfg(feature = "process")]
//! # fn main() -> Result<(), nurt::Error> {
//! use nurt::{Format, Nurt, Request};
//!
//! let nurt = Nurt::builder().spawn()?;
//! let request = Request::program(r#"search /oldName\(/ "/home/user/src" glob: "**/*.go""#)
//!     .preview(true)
//!     .format(Format::Text);
//! let response = nurt.run(&request)?;
//! if let Some(error) = response.error() {
//!     for fix in error.fix() {
//!         let _candidate = fix;
//!     }
//! } else if let Some(text) = response.text() {
//!     assert!(!text.is_empty());
//! }
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "process"))]
//! # fn main() {}
//! ```
//!
//! Requests are plain values and can be built and inspected without a
//! server:
//!
//! ```
//! use nurt::{Request, Variant};
//!
//! let request = Request::program("find(\"/home/user/src\")").variant(Variant::B);
//! assert_eq!(
//!     request.to_json(),
//!     r#"{"nurt":"1.0","program":"find(\"/home/user/src\")","variant":"b"}"#
//! );
//! ```

#![deny(missing_docs)]
#![forbid(unsafe_code)]

#[cfg(feature = "process")]
mod client;
#[cfg(feature = "embedded")]
mod embedded;
mod error;
#[cfg(feature = "process")]
mod process;
mod request;
mod response;

#[cfg(feature = "process")]
pub use crate::client::{Builder, Describe, Execution, Nurt};
#[cfg(feature = "embedded")]
pub use crate::embedded::{Embedded, EmbeddedBuilder, EmbeddedExecution};
pub use crate::error::{Error, ErrorKind, Status};
#[cfg(feature = "process")]
pub use crate::process::Transport;
pub use crate::request::{
    DiffMode, ENVELOPE_VERSION, Format, Language, Request, Variant, Verbosity,
};
pub use crate::response::{Event, LanguageError, Response};
