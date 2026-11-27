// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-net/tests/ping_icmp.rs
// Responsibility:   `network.ping`: argument checks, and echo requests to the loopback.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `network.ping`: argument checks, and echo requests to the loopback
//! address where the system allows ICMP datagram sockets.

#[path = "support/exec_ctx.rs"]
mod exec_ctx;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::PingResult;
use nurt_contracts::value::{Duration as ValueDuration, Value};
use nurt_net::ping::{COMMAND, Ping};

/// Time to wait for each reply of the loopback interface.
const REPLY_WAIT_MS: i64 = 1000;

#[test]
#[ignore = "requires ICMP datagram sockets allowed by net.ipv4.ping_group_range"]
fn ping_loopback() {
    let args = exec_ctx::args(
        COMMAND,
        &[
            ("host", exec_ctx::text("127.0.0.1")),
            ("count", Value::Int(2)),
            (
                "timeout",
                Value::Duration(ValueDuration::from_millis(REPLY_WAIT_MS)),
            ),
        ],
    );
    let records = match exec_ctx::run(&Ping::new(), &args, &exec_ctx::ctx()) {
        Err(error) if error.code() == ErrorCode::Privilege => {
            assert!(!error.fix.is_empty(), "{error:?}");
            return;
        }
        result => result.unwrap(),
    };
    let result = PingResult::try_from(records[0].clone()).unwrap();
    assert_eq!(result.address, "127.0.0.1");
    assert_eq!(result.sent, 2);
    assert_eq!(result.received, 2);
    assert_eq!(result.loss, 0.0);
    assert!(result.rtt_min.unwrap() <= result.rtt_max.unwrap());
}

#[test]
fn ping_rejects_zero_count() {
    let args = exec_ctx::args(
        COMMAND,
        &[
            ("host", exec_ctx::text("127.0.0.1")),
            ("count", Value::Int(0)),
        ],
    );
    let error = exec_ctx::run(&Ping::new(), &args, &exec_ctx::ctx()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn ping_rejects_empty_host() {
    let args = exec_ctx::args(COMMAND, &[("host", exec_ctx::text("  "))]);
    let error = exec_ctx::run(&Ping::new(), &args, &exec_ctx::ctx()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
