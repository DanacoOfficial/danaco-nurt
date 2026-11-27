// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The hidden subcommand `elevated-exec`.
//!
//! The elevation backends (`sudo`, `doas`, `run0`) start `nurt elevated-exec`
//! with the rights of the administrator. The subcommand reads one request in
//! the request format of the protocol from the standard input, refuses to run
//! unless its identity is the administrator (or the account named by
//! `as_user`), executes the request in the process, writes the response to the
//! standard output and exits. It is not listed in the usage and the
//! documentation of the subcommands, and takes no arguments.

use nurt_contracts::envelope::Request;
use nurt_core::elevation::{EXIT_BAD_REQUEST, EXIT_NOT_ADMINISTRATOR, is_administrator};

use crate::app::{EngineSource, Io};
use crate::exit;

/// Runs the subcommand and returns its exit code.
///
/// The exit code is `0` when a response was written (also a response that
/// reports an error), `126` when the identity is not the administrator, `2`
/// when no valid request was read and `1` when the engine cannot be opened.
pub(crate) fn run(io: &mut Io<'_>, source: &dyn EngineSource) -> u8 {
    let mut bytes = Vec::new();
    if let Err(error) = io.input.read_to_end(&mut bytes) {
        return refuse(
            io,
            EXIT_BAD_REQUEST,
            &format!("cannot read the request: {error}"),
        );
    }
    let request: Request = match serde_json::from_slice(&bytes) {
        Ok(request) => request,
        Err(error) => {
            return refuse(io, EXIT_BAD_REQUEST, &format!("invalid request: {error}"));
        }
    };
    if let Err(error) = request.validate() {
        return refuse(io, EXIT_BAD_REQUEST, &format!("invalid request: {error}"));
    }
    let user = source.host().info().user.clone();
    let switched = request.options.as_user.as_deref() == Some(user.as_str());
    if !is_administrator() && !switched {
        return refuse(
            io,
            EXIT_NOT_ADMINISTRATOR,
            "elevated-exec runs only with the identity of the administrator",
        );
    }
    nurt_core::elevation::reset_environment();
    let engine = match source.open_elevated() {
        Ok(engine) => engine,
        Err(error) => return refuse(io, exit::FAILURE, &error.to_string()),
    };
    let response = engine.elevated_exec(&request);
    match serde_json::to_writer(&mut *io.out, &response)
        .and_then(|()| io.out.write_all(b"\n").map_err(serde_json::Error::io))
    {
        Ok(()) => exit::SUCCESS,
        Err(error) => refuse(
            io,
            exit::FAILURE,
            &format!("cannot write the response: {error}"),
        ),
    }
}

fn refuse(io: &mut Io<'_>, code: u8, message: &str) -> u8 {
    let _ = writeln!(io.err, "nurt: {message}");
    code
}
