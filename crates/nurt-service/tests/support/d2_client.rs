// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-service/tests/support/d2_client.rs
// Responsibility:   Reference client of the journal liveness rule, written against the file and wire formats only.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reference client of the journal liveness rule, written against the file
//! and wire formats only.
//!
//! The journal keeps data only while the service is active: `service.hb`
//! holds a JSON object whose `beat` is at most 2000 ms old, its `pid`
//! exists, and `{"v":1,"op":"ping"}` on `service.sock` answers with
//! `"ok":true` and the same `pid` within 50 ms.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Largest heartbeat age of an active service.
pub(crate) const MAX_AGE_MS: i64 = 2_000;
const PING_TIMEOUT: Duration = Duration::from_millis(50);
const PING: &[u8] = b"{\"v\":1,\"op\":\"ping\"}\n";

/// Verdict of the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Active,
    Inactive(&'static str),
}

/// Applies the rule to the state directory at the instant `now`.
pub(crate) fn journal_active(state_dir: &Path, now: jiff::Timestamp) -> Verdict {
    let Ok(bytes) = std::fs::read(state_dir.join("service.hb")) else {
        return Verdict::Inactive("no_heartbeat");
    };
    let Ok(heartbeat) = serde_json::from_slice::<Value>(&bytes) else {
        return Verdict::Inactive("no_heartbeat");
    };
    let beat = heartbeat
        .get("beat")
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<jiff::Timestamp>().ok());
    let pid = heartbeat.get("pid").and_then(Value::as_u64);
    let (Some(beat), Some(pid)) = (beat, pid) else {
        return Verdict::Inactive("no_heartbeat");
    };
    let age_ms = now.as_millisecond() - beat.as_millisecond();
    if age_ms > MAX_AGE_MS {
        return Verdict::Inactive("stale_heartbeat");
    }
    if !u32::try_from(pid).is_ok_and(nurt_service::process::pid_alive) {
        return Verdict::Inactive("process_gone");
    }
    if ping(&state_dir.join("service.sock")) == Some(pid) {
        Verdict::Active
    } else {
        Verdict::Inactive("no_response")
    }
}

/// Steps and timing of one ping with the read limit `limit`, for the
/// message of a failed wait.
pub(crate) fn diagnose(state_dir: &Path, limit: Duration) -> String {
    let started = Instant::now();
    let mut stream = match UnixStream::connect(state_dir.join("service.sock")) {
        Ok(stream) => stream,
        Err(error) => return format!("connect failed after {:?}: {error}", started.elapsed()),
    };
    if let Err(error) = stream.set_read_timeout(Some(limit)) {
        return format!("set_read_timeout failed: {error}");
    }
    if let Err(error) = stream.write_all(PING) {
        return format!("write failed after {:?}: {error}", started.elapsed());
    }
    let mut line = String::new();
    match BufReader::new(stream).read_line(&mut line) {
        Ok(read) => format!("read {read} bytes after {:?}: {line:?}", started.elapsed()),
        Err(error) => format!("read failed after {:?}: {error}", started.elapsed()),
    }
}

fn ping(socket: &Path) -> Option<u64> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(PING_TIMEOUT)).ok()?;
    stream.write_all(PING).ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let reply: Value = serde_json::from_str(&line).ok()?;
    if reply.get("v") != Some(&Value::from(1)) || reply.get("ok") != Some(&Value::Bool(true)) {
        return None;
    }
    reply.get("result")?.get("pid")?.as_u64()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
