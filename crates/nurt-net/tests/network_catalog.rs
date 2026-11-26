// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `http.download` and the `network.*` commands against the catalog: the
//! parameters of the catalog entries and the static check of each stage.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers fail the test on an unexpected result"
)]

use nurt_contracts::config::Config;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::spec::StageSpec;
use nurt_contracts::stage::{ArgValue, BoundArgs, CheckCtx, InputKinds, StageImpl, StaticInfo};
use nurt_contracts::value::{Duration as ValueDuration, Value};
use nurt_net::dns::Resolve;
use nurt_net::http::download::Download;
use nurt_net::interfaces::Interfaces;
use nurt_net::ping::Ping;
use nurt_testkit::host::TestHost;

/// Catalog entry of `command` in `catalog/network.toml`.
fn entry(command: &str) -> toml::Value {
    let path = format!(
        "{}/../../catalog/network.toml",
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR"))
    );
    let text = std::fs::read_to_string(path).unwrap();
    let catalog: toml::Table = text.parse().unwrap();
    catalog["command"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"].as_str() == Some(command))
        .expect("the command is in the catalog")
        .clone()
}

fn parameter_names(command: &str) -> Vec<String> {
    entry(command)
        .get("param")
        .and_then(toml::Value::as_array)
        .map(|params| {
            params
                .iter()
                .map(|param| param["name"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn args(command: &str, values: &[(&str, Value)]) -> BoundArgs {
    values
        .iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn check(stage: &dyn StageImpl, args: &BoundArgs) -> Result<StaticInfo> {
    let command = stage.descriptor().command;
    let spec: StageSpec = entry(&command).try_into().unwrap();
    let host = TestHost::native();
    let config = Config::default();
    stage.check(args, &CheckCtx::new(&spec, &host, &config))
}

#[test]
fn catalog_parameters_match_commands() {
    assert_eq!(
        parameter_names("http.download"),
        ["url", "to", "resume", "hash", "headers", "timeout"]
    );
    assert_eq!(
        parameter_names("network.ping"),
        ["host", "count", "timeout"]
    );
    assert_eq!(
        parameter_names("network.resolve"),
        ["name", "type", "server"]
    );
    assert!(parameter_names("network.interfaces").is_empty());
    for (command, output) in [
        ("http.download", "effect"),
        ("network.ping", "ping_result"),
        ("network.resolve", "dns_record"),
        ("network.interfaces", "network_interface"),
    ] {
        assert_eq!(entry(command)["output"][0].as_str(), Some(output));
    }
}

#[test]
fn stages_are_sources_without_input() {
    let stages: [&dyn StageImpl; 4] =
        [&Download, &Ping::new(), &Resolve::new(), &Interfaces::new()];
    for stage in stages {
        let descriptor = stage.descriptor();
        assert!(descriptor.source, "{}", descriptor.command);
        assert_eq!(descriptor.input, InputKinds::None);
    }
    assert!(Download.as_effect().is_some() && Download.as_read().is_none());
    assert!(Ping::new().as_read().is_some());
    assert!(Resolve::new().as_read().is_some());
    assert!(Interfaces::new().as_read().is_some());
}

#[test]
fn download_check_reports_address_and_target() {
    let target = std::env::temp_dir().join("archive.tar.gz");
    let info = check(
        &Download,
        &args(
            "http.download",
            &[
                ("url", text("http://127.0.0.1:8080/archive.tar.gz")),
                ("to", Value::Path(target.display().to_string())),
            ],
        ),
    )
    .unwrap();
    let network = &info.network[0];
    assert_eq!(network.scheme, "http");
    assert_eq!(network.host, "127.0.0.1");
    assert_eq!(network.port, Some(8080));
    assert_eq!(
        network.url.as_deref(),
        Some("http://127.0.0.1:8080/archive.tar.gz")
    );
    assert_eq!(info.paths.len(), 1);
}

#[test]
fn download_check_rejects_invalid_arguments() {
    let target = Value::Path(std::env::temp_dir().join("a.bin").display().to_string());
    let bad_hash = args(
        "http.download",
        &[
            ("url", text("https://example.test/a.bin")),
            ("to", target.clone()),
            ("hash", text("crc32:00000000")),
        ],
    );
    assert_eq!(
        check(&Download, &bad_hash).unwrap_err().code(),
        ErrorCode::InvalidValue
    );
    let relative = args(
        "http.download",
        &[
            ("url", text("https://example.test/a.bin")),
            ("to", Value::Path("a.bin".to_owned())),
        ],
    );
    assert!(check(&Download, &relative).is_err());
    let bad_url = args(
        "http.download",
        &[("url", text("ftp://example.test/a.bin")), ("to", target)],
    );
    assert!(check(&Download, &bad_url).is_err());
}

#[test]
fn ping_check_reports_icmp_target() {
    let info = check(
        &Ping::new(),
        &args("network.ping", &[("host", text("192.0.2.1"))]),
    )
    .unwrap();
    assert_eq!(info.network[0].scheme, "icmp");
    assert_eq!(info.network[0].host, "192.0.2.1");
    assert_eq!(info.network[0].port, None);

    let zero = args(
        "network.ping",
        &[
            ("host", text("192.0.2.1")),
            ("timeout", Value::Duration(ValueDuration::from_millis(0))),
        ],
    );
    assert_eq!(
        check(&Ping::new(), &zero).unwrap_err().code(),
        ErrorCode::InvalidValue
    );
}

#[test]
fn resolve_check_reports_explicit_server() {
    let info = check(
        &Resolve::new(),
        &args(
            "network.resolve",
            &[
                ("name", text("example.test")),
                ("server", text("192.0.2.53:5353")),
            ],
        ),
    )
    .unwrap();
    assert_eq!(info.network[0].scheme, "dns");
    assert_eq!(info.network[0].host, "192.0.2.53");
    assert_eq!(info.network[0].port, Some(5353));
    assert_eq!(info.details["name"], text("example.test"));

    let named = check(
        &Resolve::new(),
        &args(
            "network.resolve",
            &[
                ("name", text("example.test")),
                ("server", text("ns.example.test")),
            ],
        ),
    )
    .unwrap();
    assert_eq!(named.network[0].host, "ns.example.test");
    assert_eq!(named.network[0].port, Some(53));

    let system = check(
        &Resolve::new(),
        &args("network.resolve", &[("name", text("example.test"))]),
    )
    .unwrap();
    assert!(system.network.is_empty());
}

#[test]
fn resolve_check_rejects_unknown_type_and_invalid_name() {
    let unknown = args(
        "network.resolve",
        &[
            ("name", text("example.test")),
            ("type", Value::Enum("srv".to_owned())),
        ],
    );
    assert_eq!(
        check(&Resolve::new(), &unknown).unwrap_err().code(),
        ErrorCode::InvalidValue
    );
    let invalid = args("network.resolve", &[("name", text(&"a".repeat(64)))]);
    assert_eq!(
        check(&Resolve::new(), &invalid).unwrap_err().code(),
        ErrorCode::InvalidValue
    );
    let wrong_type = args(
        "network.resolve",
        &[("name", text("example.test")), ("type", Value::Int(1))],
    );
    assert_eq!(
        check(&Resolve::new(), &wrong_type).unwrap_err().code(),
        ErrorCode::Type
    );
}
