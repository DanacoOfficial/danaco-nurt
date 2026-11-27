// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The commands `network.route_add`, `network.route_remove` and
//! `network.dns_set`.

mod dns;
mod route;

use std::sync::Arc;

use nurt_contracts::stage::StageImpl;

pub use self::dns::{
    DNS_PREFIX, DnsBackendKind, DnsSet, DnsState, parse_dns_text, parse_resolvconf,
};
pub use self::route::{
    ROUTE_PREFIX, Route, RouteChange, parse_ip_json, parse_netstat, parse_windows,
};

/// Every stage of the network commands, in catalog order.
#[must_use]
pub fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![
        Arc::new(RouteChange::ADD),
        Arc::new(RouteChange::REMOVE),
        Arc::new(DnsSet),
    ]
}

/// Whether `text` can be an interface name or alias: no quotes, no leading
/// dash, no control characters.
#[must_use]
pub fn valid_interface(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && !text.starts_with('-')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-' | ' '))
}
