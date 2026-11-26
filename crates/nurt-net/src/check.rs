// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Network targets reported before execution.
//!
//! The static check of a network command describes every address it will
//! contact as an element of the `network` list: the address after
//! normalization, its host, port and scheme, the proxy that will be used,
//! pinned addresses, header names, the kind of body and, when resolution is
//! requested, the IP addresses with their class from the IANA registries of
//! special-purpose addresses (RFC 6890 and its updates). The entries state
//! facts only; the console allows every target, and an embedding application
//! applies its own rules to them.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use indexmap::IndexMap;
use nurt_contracts::env::Host;
use nurt_contracts::error::Result;
use nurt_contracts::stage::NetworkTarget;
use nurt_contracts::value::Value;

use crate::client::{NameResolver, Pins, ProxyChoice, Target, effective_proxy};

/// Class of an IP address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressClass {
    /// Class: `public`, `private`, `loopback`, `link_local`, `unspecified`,
    /// `multicast`, `broadcast`, `shared`, `documentation`, `benchmarking`,
    /// `reserved`, `ietf_protocol`, `ipv4_mapped`, `ipv4_translated`,
    /// `6to4`, `teredo`, `discard` or `unique_local`.
    pub class: &'static str,
    /// IPv4 address carried inside an IPv6 address, with its own class.
    pub embedded: Option<(Ipv4Addr, &'static str)>,
}

/// Classifies an IP address.
///
/// ```
/// use std::net::IpAddr;
/// use nurt_net::check::classify;
///
/// let mapped: IpAddr = "::ffff:127.0.0.1".parse().unwrap();
/// let class = classify(mapped);
/// assert_eq!(class.class, "ipv4_mapped");
/// assert_eq!(class.embedded.unwrap().1, "loopback");
/// assert_eq!(classify("203.0.113.7".parse().unwrap()).class, "documentation");
/// ```
#[must_use]
pub fn classify(ip: IpAddr) -> AddressClass {
    match ip {
        IpAddr::V4(ip) => AddressClass {
            class: classify_v4(ip),
            embedded: None,
        },
        IpAddr::V6(ip) => classify_v6(ip),
    }
}

fn in_v4(ip: Ipv4Addr, network: [u8; 4], prefix: u32) -> bool {
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    u32::from(ip) & mask == u32::from(Ipv4Addr::from(network)) & mask
}

fn classify_v4(ip: Ipv4Addr) -> &'static str {
    const TABLE: &[([u8; 4], u32, &str)] = &[
        ([0, 0, 0, 0], 32, "unspecified"),
        ([255, 255, 255, 255], 32, "broadcast"),
        ([0, 0, 0, 0], 8, "reserved"),
        ([10, 0, 0, 0], 8, "private"),
        ([100, 64, 0, 0], 10, "shared"),
        ([127, 0, 0, 0], 8, "loopback"),
        ([169, 254, 0, 0], 16, "link_local"),
        ([172, 16, 0, 0], 12, "private"),
        ([192, 0, 0, 0], 24, "ietf_protocol"),
        ([192, 0, 2, 0], 24, "documentation"),
        ([192, 88, 99, 0], 24, "6to4"),
        ([192, 168, 0, 0], 16, "private"),
        ([198, 18, 0, 0], 15, "benchmarking"),
        ([198, 51, 100, 0], 24, "documentation"),
        ([203, 0, 113, 0], 24, "documentation"),
        ([224, 0, 0, 0], 4, "multicast"),
        ([240, 0, 0, 0], 4, "reserved"),
    ];
    TABLE
        .iter()
        .find(|(network, prefix, _)| in_v4(ip, *network, *prefix))
        .map_or("public", |(_, _, class)| class)
}

fn in_v6(ip: Ipv6Addr, network: [u16; 8], prefix: u32) -> bool {
    let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
    u128::from(ip) & mask == u128::from(Ipv6Addr::from(network)) & mask
}

fn low_v4(ip: Ipv6Addr) -> Ipv4Addr {
    let [.., a, b, c, d] = ip.octets();
    Ipv4Addr::new(a, b, c, d)
}

fn embed(class: &'static str, ip: Ipv4Addr) -> AddressClass {
    AddressClass {
        class,
        embedded: Some((ip, classify_v4(ip))),
    }
}

fn classify_v6(ip: Ipv6Addr) -> AddressClass {
    if in_v6(ip, [0, 0, 0, 0, 0, 0xffff, 0, 0], 96) {
        return embed("ipv4_mapped", low_v4(ip));
    }
    if in_v6(ip, [0x64, 0xff9b, 0, 0, 0, 0, 0, 0], 96) {
        return embed("ipv4_translated", low_v4(ip));
    }
    if in_v6(ip, [0x2001, 0, 0, 0, 0, 0, 0, 0], 32) {
        let client = !u32::from(low_v4(ip));
        return embed("teredo", Ipv4Addr::from(client));
    }
    if in_v6(ip, [0x2002, 0, 0, 0, 0, 0, 0, 0], 16) {
        let [_, _, a, b, c, d, ..] = ip.octets();
        return embed("6to4", Ipv4Addr::new(a, b, c, d));
    }
    const TABLE: &[([u16; 8], u32, &str)] = &[
        ([0, 0, 0, 0, 0, 0, 0, 0], 128, "unspecified"),
        ([0, 0, 0, 0, 0, 0, 0, 1], 128, "loopback"),
        ([0x100, 0, 0, 0, 0, 0, 0, 0], 64, "discard"),
        ([0x2001, 0x2, 0, 0, 0, 0, 0, 0], 48, "benchmarking"),
        ([0x2001, 0xdb8, 0, 0, 0, 0, 0, 0], 32, "documentation"),
        ([0x2001, 0, 0, 0, 0, 0, 0, 0], 23, "ietf_protocol"),
        ([0xfc00, 0, 0, 0, 0, 0, 0, 0], 7, "unique_local"),
        ([0xfe80, 0, 0, 0, 0, 0, 0, 0], 10, "link_local"),
        ([0xff00, 0, 0, 0, 0, 0, 0, 0], 8, "multicast"),
        ([0, 0, 0, 0, 0, 0, 0, 0], 8, "reserved"),
    ];
    let class = TABLE
        .iter()
        .find(|(network, prefix, _)| in_v6(ip, *network, *prefix))
        .map_or("public", |(_, _, class)| class);
    AddressClass {
        class,
        embedded: None,
    }
}

/// Address of a request as known before execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressFact {
    /// An address given in the program.
    Fixed(String),
    /// An address computed during execution, with the text of its
    /// expression.
    Dynamic(String),
}

/// Facts about one request, taken from the arguments of a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestFacts {
    /// Index of the stage in the pipeline.
    pub stage: u32,
    /// Command name.
    pub command: String,
    /// The request runs later, inside a scheduled action.
    pub deferred: bool,
    /// Method in lower case.
    pub method: String,
    /// Address.
    pub address: AddressFact,
    /// Redirects are followed.
    pub follow_redirects: bool,
    /// Largest number of redirects.
    pub max_redirects: u32,
    /// Retries.
    pub retries: u32,
    /// Requested proxy.
    pub proxy: ProxyChoice,
    /// Pinned addresses.
    pub pins: Pins,
    /// The certificate is verified.
    pub verify_tls: bool,
    /// Header names.
    pub header_names: Vec<String>,
    /// Kind of body: `none`, `text`, `json`, `file`, `form` or `multipart`.
    pub body: String,
    /// Path the body is saved to.
    pub save: Option<String>,
    /// Cache mode.
    pub cache: String,
}

/// Network facts of one request: the target for [`NetworkTarget`] lists and
/// the `network` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkFacts {
    /// Target, absent for dynamic addresses.
    pub target: Option<NetworkTarget>,
    /// Entry of the `network` list.
    pub entry: Value,
}

/// Describes one request.
///
/// The `http.proxy` setting is passed as `configured_proxy`, and the
/// environment variables are read through `host`. Names are resolved only
/// when a resolver is given. Addresses written as IP addresses are always
/// classified.
///
/// # Errors
///
/// `E_ARG` when the address or the proxy is not valid.
pub fn describe(
    facts: &RequestFacts,
    host: &dyn Host,
    configured_proxy: Option<&str>,
    resolver: Option<&dyn NameResolver>,
) -> Result<NetworkFacts> {
    let mut entry = IndexMap::new();
    let text = |value: &str| Value::Text(value.to_owned());
    entry.insert("stage".to_owned(), Value::Int(i64::from(facts.stage)));
    entry.insert("command".to_owned(), text(&facts.command));
    entry.insert("deferred".to_owned(), Value::Bool(facts.deferred));
    entry.insert("method".to_owned(), text(&facts.method));
    let target = match &facts.address {
        AddressFact::Fixed(input) => Some(Target::parse(input)?),
        AddressFact::Dynamic(_) => None,
    };
    let proxy = match &target {
        Some(target) => effective_proxy(&facts.proxy, configured_proxy, host, target)?
            .map(|url| url.as_str().to_owned()),
        None => None,
    };
    match (&target, &facts.address) {
        (Some(target), _) => address_fields(&mut entry, target),
        (None, AddressFact::Dynamic(expr)) => {
            entry.insert("url".to_owned(), Value::Null);
            entry.insert("expr".to_owned(), text(expr));
            entry.insert("dynamic".to_owned(), Value::Bool(true));
        }
        (None, AddressFact::Fixed(_)) => {}
    }
    entry.insert(
        "follow_redirects".to_owned(),
        Value::Bool(facts.follow_redirects),
    );
    entry.insert(
        "max_redirects".to_owned(),
        Value::Int(i64::from(facts.max_redirects)),
    );
    entry.insert("retries".to_owned(), Value::Int(i64::from(facts.retries)));
    entry.insert(
        "proxy".to_owned(),
        proxy.as_deref().map_or(Value::Null, text),
    );
    entry.insert("pinned".to_owned(), Value::Bool(!facts.pins.is_empty()));
    entry.insert("verify_tls".to_owned(), Value::Bool(facts.verify_tls));
    entry.insert(
        "header_names".to_owned(),
        Value::List(
            facts
                .header_names
                .iter()
                .map(|name| text(&name.to_ascii_lowercase()))
                .collect(),
        ),
    );
    entry.insert("body".to_owned(), text(&facts.body));
    entry.insert(
        "save".to_owned(),
        facts.save.as_deref().map_or(Value::Null, text),
    );
    entry.insert("cache".to_owned(), text(&facts.cache));
    let offline = facts.cache == "only";
    entry.insert(
        "network_access".to_owned(),
        text(if offline { "no" } else { "yes" }),
    );
    let (resolved, addresses) = match (&target, offline) {
        (Some(target), false) => addresses(target, &facts.pins, resolver),
        _ => (false, Vec::new()),
    };
    entry.insert("resolved".to_owned(), Value::Bool(resolved));
    entry.insert("addresses".to_owned(), Value::List(addresses));
    Ok(NetworkFacts {
        target: target.map(|target| NetworkTarget {
            scheme: target.scheme().to_owned(),
            host: target.host(),
            port: Some(target.port()),
            url: Some(target.display_url()),
        }),
        entry: Value::Map(entry),
    })
}

fn address_fields(entry: &mut IndexMap<String, Value>, target: &Target) {
    let text = |value: &str| Value::Text(value.to_owned());
    let input = if target.has_userinfo() {
        target.display_url()
    } else {
        target.input().to_owned()
    };
    entry.insert("url".to_owned(), text(&target.display_url()));
    entry.insert("url_input".to_owned(), text(&input));
    entry.insert("scheme".to_owned(), text(target.scheme()));
    entry.insert(
        "scheme_assumed".to_owned(),
        Value::Bool(target.scheme_assumed()),
    );
    entry.insert("host".to_owned(), text(&target.host()));
    entry.insert("host_kind".to_owned(), text(target.host_kind().as_str()));
    entry.insert("port".to_owned(), Value::Int(i64::from(target.port())));
    entry.insert("userinfo".to_owned(), Value::Bool(target.has_userinfo()));
    entry.insert("dynamic".to_owned(), Value::Bool(false));
}

fn addresses(
    target: &Target,
    pins: &Pins,
    resolver: Option<&dyn NameResolver>,
) -> (bool, Vec<Value>) {
    let found: Option<Vec<IpAddr>> = target
        .ip()
        .map(|ip| vec![ip])
        .or_else(|| {
            pins.get(&target.host(), target.port())
                .map(<[IpAddr]>::to_vec)
        })
        .or_else(|| {
            resolver.and_then(|resolver| resolver.resolve(&target.host(), target.port()).ok())
        });
    match found {
        Some(found) => (true, found.into_iter().map(address_value).collect()),
        None => (false, Vec::new()),
    }
}

fn address_value(ip: IpAddr) -> Value {
    let class = classify(ip);
    let mut map = IndexMap::new();
    map.insert("ip".to_owned(), Value::Text(ip.to_string()));
    let family = if ip.is_ipv4() { "ipv4" } else { "ipv6" };
    map.insert("family".to_owned(), Value::Text(family.to_owned()));
    map.insert("class".to_owned(), Value::Text(class.class.to_owned()));
    if let Some((embedded, embedded_class)) = class.embedded {
        let mut inner = IndexMap::new();
        inner.insert("ip".to_owned(), Value::Text(embedded.to_string()));
        inner.insert("class".to_owned(), Value::Text(embedded_class.to_owned()));
        map.insert("embedded".to_owned(), Value::Map(inner));
    }
    Value::Map(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(text: &str) -> &'static str {
        classify(text.parse().unwrap()).class
    }

    #[test]
    fn description_of_an_address_with_credentials_does_not_repeat_them() {
        let context = nurt_testkit::ctx::TestContext::new();
        let ctx = context.ctx();
        let facts = RequestFacts {
            stage: 0,
            command: "http.request".to_owned(),
            deferred: false,
            method: "get".to_owned(),
            address: AddressFact::Fixed("https://user:secret@example.test/a".to_owned()),
            follow_redirects: true,
            max_redirects: 10,
            retries: 0,
            proxy: ProxyChoice::None,
            pins: Pins::new(),
            verify_tls: true,
            header_names: Vec::new(),
            body: "none".to_owned(),
            save: None,
            cache: "bypass".to_owned(),
        };
        let described = describe(&facts, ctx.host(), None, None).unwrap();
        assert!(!format!("{:?}", described.entry).contains("secret"));
        let target = described.target.unwrap();
        assert_eq!(target.url.as_deref(), Some("https://example.test/a"));
    }

    #[test]
    fn classifies_ipv4_special_ranges() {
        let cases = [
            ("0.0.0.0", "unspecified"),
            ("0.1.2.3", "reserved"),
            ("10.0.0.5", "private"),
            ("100.64.1.1", "shared"),
            ("127.0.0.1", "loopback"),
            ("169.254.169.254", "link_local"),
            ("172.31.255.255", "private"),
            ("172.32.0.1", "public"),
            ("192.0.0.8", "ietf_protocol"),
            ("192.0.2.1", "documentation"),
            ("192.168.1.1", "private"),
            ("198.19.0.1", "benchmarking"),
            ("224.0.0.1", "multicast"),
            ("250.0.0.1", "reserved"),
            ("255.255.255.255", "broadcast"),
            ("8.8.8.8", "public"),
        ];
        for (ip, expected) in cases {
            assert_eq!(class(ip), expected, "{ip}");
        }
    }

    #[test]
    fn classifies_ipv6_special_ranges() {
        let cases = [
            ("::", "unspecified"),
            ("::1", "loopback"),
            ("100::1", "discard"),
            ("2001:2::1", "benchmarking"),
            ("2001:db8::1", "documentation"),
            ("2001:10::1", "ietf_protocol"),
            ("fd00::1", "unique_local"),
            ("fe80::1", "link_local"),
            ("ff02::1", "multicast"),
            ("2606:4700::1", "public"),
        ];
        for (ip, expected) in cases {
            assert_eq!(class(ip), expected, "{ip}");
        }
    }

    #[test]
    fn reports_embedded_ipv4_addresses() {
        let nat64 = classify("64:ff9b::a00:5".parse().unwrap());
        assert_eq!(nat64.class, "ipv4_translated");
        assert_eq!(
            nat64.embedded,
            Some((Ipv4Addr::new(10, 0, 0, 5), "private"))
        );
        let six_to_four = classify("2002:7f00:1::1".parse().unwrap());
        assert_eq!(six_to_four.embedded.map(|e| e.1), Some("loopback"));
        let teredo = classify("2001:0:4136:e378:8000:63bf:3fff:fdd2".parse().unwrap());
        assert_eq!(teredo.class, "teredo");
        assert_eq!(
            teredo.embedded.map(|e| e.0),
            Some(Ipv4Addr::new(192, 0, 2, 45))
        );
    }
}
