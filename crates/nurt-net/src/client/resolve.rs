// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Name resolution and pinned addresses.

use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::CancelToken;

/// Interval at which a pending lookup checks for cancellation.
const LOOKUP_POLL: Duration = Duration::from_millis(25);

/// Translates host names to IP addresses.
///
/// The client resolves every name itself and connects only to the addresses
/// returned here, so an implementation fixes the set of addresses a request
/// may reach. Tests substitute an implementation that answers from a table.
pub trait NameResolver: Send + Sync {
    /// Addresses of `host` for a connection to `port`, in preference order.
    ///
    /// # Errors
    ///
    /// An I/O error when the name cannot be resolved.
    fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<IpAddr>>;
}

/// Resolver of the operating system (`getaddrinfo` and its equivalents).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemResolver;

impl NameResolver for SystemResolver {
    fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<IpAddr>> {
        Ok((host, port)
            .to_socket_addrs()?
            .map(|address| address.ip())
            .collect())
    }
}

/// Addresses pinned to `host:port` authorities.
///
/// A request to a pinned authority connects only to the pinned addresses and
/// never resolves the name, also after a redirect to the same authority.
///
/// ```
/// use nurt_net::client::Pins;
///
/// let mut pins = Pins::new();
/// pins.insert("Example.test:443", &["192.0.2.7"]).unwrap();
/// let addresses = pins.get("example.test", 443).unwrap();
/// assert_eq!(addresses[0].to_string(), "192.0.2.7");
/// assert!(pins.get("example.test", 80).is_none());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pins {
    entries: BTreeMap<(String, u16), Vec<IpAddr>>,
}

impl Pins {
    /// No pinned addresses.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pins `addresses` to `authority`, written as `host:port` (`[ip]:port`
    /// for IPv6).
    ///
    /// # Errors
    ///
    /// `E_ARG` when the authority has no port or an address is not an IP
    /// address.
    pub fn insert<S: AsRef<str>>(&mut self, authority: &str, addresses: &[S]) -> Result<()> {
        let (host, port) = split_authority(authority)?;
        let parsed = addresses
            .iter()
            .map(|text| {
                let text = text.as_ref().trim();
                text.trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<IpAddr>()
                    .map_err(|_| {
                        Error::new(
                            ErrorCode::Arg,
                            format!("`{text}` pinned to `{authority}` is not an IP address"),
                        )
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        if parsed.is_empty() {
            return Err(Error::new(
                ErrorCode::Arg,
                format!("give at least one address for `{authority}`"),
            ));
        }
        self.entries.insert((host, port), parsed);
        Ok(())
    }

    /// Addresses pinned to `host` and `port`.
    #[must_use]
    pub fn get(&self, host: &str, port: u16) -> Option<&[IpAddr]> {
        self.entries
            .get(&(host.to_ascii_lowercase(), port))
            .map(Vec::as_slice)
    }

    /// Whether no address is pinned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn split_authority(authority: &str) -> Result<(String, u16)> {
    let invalid = || {
        Error::new(
            ErrorCode::Arg,
            format!("`{authority}` is not of the form host:port"),
        )
    };
    let (host, port) = authority.trim().rsplit_once(':').ok_or_else(invalid)?;
    let port = port.parse::<u16>().map_err(|_| invalid())?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return Err(invalid());
    }
    let host = url::Host::parse(host)
        .map_err(|_| invalid())?
        .to_string()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    Ok((host, port))
}

/// Resolves `host` on a helper thread, giving up when `wait` elapses or
/// `cancel` is cancelled; the lookup of the operating system itself cannot
/// be interrupted.
pub(crate) fn lookup(
    resolver: &Arc<dyn NameResolver>,
    host: &str,
    port: u16,
    wait: Duration,
    cancel: &CancelToken,
) -> Result<Vec<IpAddr>> {
    let (sender, receiver) = mpsc::channel();
    let worker = Arc::clone(resolver);
    let name = host.to_owned();
    thread::spawn(move || {
        let _ = sender.send(worker.resolve(&name, port));
    });
    let mut waited = Duration::ZERO;
    loop {
        cancel.check()?;
        match receiver.recv_timeout(LOOKUP_POLL) {
            Ok(Ok(addresses)) if addresses.is_empty() => return Err(dns_error(host, "no address")),
            Ok(Ok(addresses)) => return Ok(addresses),
            Ok(Err(error)) => return Err(dns_error(host, &error.to_string())),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(dns_error(host, "the resolver stopped"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                waited += LOOKUP_POLL;
                if waited >= wait {
                    return Err(Error::new(
                        ErrorCode::Timeout,
                        format!("resolving `{host}` exceeded the time limit"),
                    ));
                }
            }
        }
    }
}

fn dns_error(host: &str, reason: &str) -> Error {
    Error::new(
        ErrorCode::Network,
        format!("the name `{host}` cannot be resolved: {reason}"),
    )
    .with_problem("dns")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    struct Table(Vec<IpAddr>);

    impl NameResolver for Table {
        fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
            Ok(self.0.clone())
        }
    }

    struct Slow;

    impl NameResolver for Slow {
        fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
            thread::sleep(Duration::from_millis(300));
            Ok(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)])
        }
    }

    #[test]
    fn pins_accept_ipv6_authorities() {
        let mut pins = Pins::new();
        pins.insert("[::1]:8080", &["::1"]).unwrap();
        assert_eq!(pins.get("::1", 8080).unwrap().len(), 1);
    }

    #[test]
    fn pins_reject_malformed_entries() {
        let mut pins = Pins::new();
        assert_eq!(
            pins.insert("example.test", &["192.0.2.1"]).unwrap_err().code(),
            ErrorCode::Arg
        );
        assert_eq!(
            pins.insert("example.test:443", &["not-an-ip"]).unwrap_err().code(),
            ErrorCode::Arg
        );
        let empty: [&str; 0] = [];
        assert!(pins.insert("example.test:443", &empty).is_err());
    }

    #[test]
    fn lookup_reports_empty_answers_as_dns_problem() {
        let resolver: Arc<dyn NameResolver> = Arc::new(Table(Vec::new()));
        let error = lookup(
            &resolver,
            "a.test",
            80,
            Duration::from_secs(1),
            &CancelToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Network);
        assert_eq!(error.data().problem.as_deref(), Some("dns"));
    }

    #[test]
    fn lookup_gives_up_after_the_time_limit() {
        let resolver: Arc<dyn NameResolver> = Arc::new(Slow);
        let error = lookup(
            &resolver,
            "a.test",
            80,
            Duration::from_millis(50),
            &CancelToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Timeout);
    }

    #[test]
    fn system_resolver_answers_literal_addresses() {
        let addresses = SystemResolver.resolve("127.0.0.1", 80).unwrap();
        assert_eq!(addresses, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);
    }
}
