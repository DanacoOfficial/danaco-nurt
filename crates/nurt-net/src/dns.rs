// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Name resolution.
//!
//! `network.resolve` sends DNS queries to the servers of the system
//! configuration, or to the server given in `server`, and returns the
//! records of the answer. Queries are built and parsed with the message
//! types of `hickory-proto`; the exchange itself is synchronous: one UDP
//! datagram per query, repeated over TCP when the answer is truncated
//! (RFC 1035, section 4.2, and RFC 7766).
//!
//! The system configuration is `/etc/resolv.conf` on Linux and macOS, read
//! below a [`SysRoot`], and the DNS servers of the network adapters on
//! Windows. Names are queried as given, without search domains.
//!
//! [`resolve_host`] resolves a host name to addresses through the resolver
//! of the operating system, which also consults the hosts file; `network.ping`
//! uses it.

use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::str::FromStr;
use std::time::{Duration, Instant};

use hickory_resolver::proto::op::{Message, MessageType, Query, ResponseCode};
use hickory_resolver::proto::rr::{Name, Record as Answer, RecordType};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::DnsRecord;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, NetworkTarget, ReadStage, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Duration as ValueDuration, Record, Value};

use crate::interfaces::SysRoot;

/// Name of the command in the catalog.
pub const COMMAND: &str = "network.resolve";

/// Port of DNS servers.
pub const DNS_PORT: u16 = 53;

const PARAM_NAME: &str = "name";
const PARAM_TYPE: &str = "type";
const PARAM_SERVER: &str = "server";

/// Time to wait for one answer when the configuration does not set one; the
/// default of the resolver of the C library.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Rounds over all servers when the configuration does not set a number.
const DEFAULT_ATTEMPTS: u32 = 2;
/// Upper bound of `options timeout:` in `resolv.conf`.
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound of `options attempts:` in `resolv.conf`.
const MAX_ATTEMPTS: u32 = 5;
/// Servers used from `resolv.conf`, as `MAXNS` of the C library.
const MAX_SERVERS: usize = 3;
/// Longest wait between two checks of cancellation.
const POLL_SLICE: Duration = Duration::from_millis(100);
/// Receive buffer for UDP answers.
const UDP_BUFFER: usize = 4096;
/// Milliseconds per second, for record lifetimes.
const MILLIS_PER_SECOND: i64 = 1000;

/// Type of the records asked for.
///
/// ```
/// use nurt_net::dns::QueryType;
///
/// assert_eq!("aaaa".parse::<QueryType>().map(QueryType::as_str), Ok("aaaa"));
/// assert!("srv".parse::<QueryType>().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryType {
    /// IPv4 addresses.
    A,
    /// IPv6 addresses.
    Aaaa,
    /// Mail exchangers.
    Mx,
    /// Text records.
    Txt,
    /// Name servers.
    Ns,
    /// Canonical name.
    Cname,
    /// The union of the other types, asked one query per type.
    Any,
}

impl QueryType {
    /// Every value in the order of the catalog.
    pub const ALL: &'static [QueryType] = &[
        Self::A,
        Self::Aaaa,
        Self::Mx,
        Self::Txt,
        Self::Ns,
        Self::Cname,
        Self::Any,
    ];

    /// Value as written in programs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::Aaaa => "aaaa",
            Self::Mx => "mx",
            Self::Txt => "txt",
            Self::Ns => "ns",
            Self::Cname => "cname",
            Self::Any => "any",
        }
    }

    fn record_types(self) -> &'static [RecordType] {
        match self {
            Self::A => &[RecordType::A],
            Self::Aaaa => &[RecordType::AAAA],
            Self::Mx => &[RecordType::MX],
            Self::Txt => &[RecordType::TXT],
            Self::Ns => &[RecordType::NS],
            Self::Cname => &[RecordType::CNAME],
            Self::Any => &[
                RecordType::A,
                RecordType::AAAA,
                RecordType::MX,
                RecordType::TXT,
                RecordType::NS,
                RecordType::CNAME,
            ],
        }
    }
}

impl FromStr for QueryType {
    type Err = Error;

    /// Parses a record type; the comparison ignores case.
    fn from_str(text: &str) -> Result<Self> {
        let lower = text.to_ascii_lowercase();
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str() == lower)
            .ok_or_else(|| {
                let names: Vec<&str> = Self::ALL.iter().map(|kind| kind.as_str()).collect();
                Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "unknown DNS record type `{text}`; use one of {}",
                        names.join(", ")
                    ),
                )
                .with_found(text)
                .with_expected(serde_json::json!(names))
            })
    }
}

/// Servers and limits of the resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolverConfig {
    /// Servers in the order they are asked.
    pub servers: Vec<SocketAddr>,
    /// Time to wait for one answer.
    pub timeout: Duration,
    /// Rounds over all servers before giving up.
    pub attempts: u32,
}

impl ResolverConfig {
    /// Configuration that asks only `server`.
    #[must_use]
    pub fn single(server: SocketAddr) -> Self {
        Self {
            servers: vec![server],
            timeout: DEFAULT_TIMEOUT,
            attempts: DEFAULT_ATTEMPTS,
        }
    }

    /// Reads the configuration from the text of a `resolv.conf` file.
    ///
    /// The lines `nameserver` (at most three) and the options `timeout:` and
    /// `attempts:` are used; other lines are ignored. Without a `nameserver`
    /// line the server is `127.0.0.1`, as in the C library.
    ///
    /// ```
    /// use std::time::Duration;
    /// use nurt_net::dns::ResolverConfig;
    ///
    /// let config = ResolverConfig::from_resolv_conf(
    ///     "nameserver 192.0.2.53\nnameserver 2001:db8::53\noptions timeout:2 attempts:3\n",
    /// );
    /// assert_eq!(config.servers.len(), 2);
    /// assert_eq!(config.timeout, Duration::from_secs(2));
    /// assert_eq!(config.attempts, 3);
    /// ```
    #[must_use]
    pub fn from_resolv_conf(text: &str) -> Self {
        let mut config = Self {
            servers: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
            attempts: DEFAULT_ATTEMPTS,
        };
        for line in text.lines() {
            let content = line.split(['#', ';']).next().unwrap_or_default();
            let mut words = content.split_whitespace();
            match words.next() {
                Some("nameserver") => {
                    let address = words
                        .next()
                        .map(|word| word.split('%').next().unwrap_or(word))
                        .and_then(|word| word.parse::<IpAddr>().ok());
                    if let Some(ip) = address
                        && config.servers.len() < MAX_SERVERS
                    {
                        config.servers.push(SocketAddr::new(ip, DNS_PORT));
                    }
                }
                Some("options") => words.for_each(|option| config.apply_option(option)),
                _ => {}
            }
        }
        if config.servers.is_empty() {
            config
                .servers
                .push(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DNS_PORT));
        }
        config
    }

    fn apply_option(&mut self, option: &str) {
        if let Some(seconds) = option
            .strip_prefix("timeout:")
            .and_then(|value| value.parse::<u64>().ok())
        {
            self.timeout = Duration::from_secs(seconds.max(1)).min(MAX_TIMEOUT);
        } else if let Some(attempts) = option
            .strip_prefix("attempts:")
            .and_then(|value| value.parse::<u32>().ok())
        {
            self.attempts = attempts.clamp(1, MAX_ATTEMPTS);
        }
    }

    /// Configuration of the system.
    ///
    /// On Linux and macOS the file `/etc/resolv.conf` below `root` is read;
    /// a missing file gives the defaults of [`from_resolv_conf`]. On Windows
    /// the DNS servers of the network adapters are used.
    ///
    /// # Errors
    ///
    /// The function fails with `E_IO` when the configuration exists but
    /// cannot be read.
    ///
    /// [`from_resolv_conf`]: ResolverConfig::from_resolv_conf
    pub fn system(root: &SysRoot) -> Result<Self> {
        system_config(root)
    }
}

#[cfg(unix)]
fn system_config(root: &SysRoot) -> Result<ResolverConfig> {
    let path = root.path("/etc/resolv.conf");
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(ResolverConfig::from_resolv_conf(&text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(ResolverConfig::from_resolv_conf(""))
        }
        Err(error) => Err(Error::new(
            ErrorCode::Io,
            format!("{} cannot be read: {error}", path.display()),
        )
        .with_detail("path", serde_json::Value::from(path.display().to_string()))),
    }
}

#[cfg(windows)]
fn system_config(_root: &SysRoot) -> Result<ResolverConfig> {
    let (config, options) = hickory_resolver::system_conf::read_system_conf().map_err(|error| {
        Error::new(
            ErrorCode::Io,
            format!("the DNS configuration of the system cannot be read: {error}"),
        )
    })?;
    let mut servers: Vec<SocketAddr> = config
        .name_servers()
        .iter()
        .map(|server| SocketAddr::new(server.ip, DNS_PORT))
        .collect();
    servers.dedup();
    if servers.is_empty() {
        servers.push(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DNS_PORT));
    }
    Ok(ResolverConfig {
        servers,
        timeout: options.timeout.min(MAX_TIMEOUT),
        attempts: u32::try_from(options.attempts)
            .unwrap_or(MAX_ATTEMPTS)
            .clamp(1, MAX_ATTEMPTS),
    })
}

/// Parses the `server` parameter: an address with an optional port, for
/// example `192.0.2.53`, `192.0.2.53:5353`, `2001:db8::53`,
/// `[2001:db8::53]:53`, or a host name with an optional port, resolved by the
/// system resolver.
///
/// # Errors
///
/// The function fails with `E_INVALID_VALUE` when the text is not an
/// address and the host name cannot be resolved.
///
/// # Examples
///
/// ```
/// use nurt_net::dns::parse_server;
///
/// assert_eq!(parse_server("192.0.2.53").unwrap().port(), 53);
/// assert_eq!(parse_server("[2001:db8::53]:5353").unwrap().port(), 5353);
/// ```
pub fn parse_server(text: &str) -> Result<SocketAddr> {
    if let Ok(address) = text.parse::<SocketAddr>() {
        return Ok(address);
    }
    if let Ok(ip) = text.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DNS_PORT));
    }
    let resolved = if text.contains(':') {
        text.to_socket_addrs()
    } else {
        (text, DNS_PORT).to_socket_addrs()
    };
    resolved
        .ok()
        .and_then(|mut addresses| addresses.next())
        .ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "DNS server `{text}` is not an address; give one such as 192.0.2.53 or [2001:db8::53]:53"
                ),
            )
            .with_found(text)
        })
}

/// Addresses of `host` from the resolver of the operating system, which
/// also reads the hosts file. An address given as text is returned as it is.
///
/// # Errors
///
/// The function fails with `E_NETWORK` (problem `dns`) when the name cannot
/// be resolved.
///
/// # Examples
///
/// ```
/// use std::net::{IpAddr, Ipv4Addr};
/// use nurt_net::dns::resolve_host;
///
/// let addresses = resolve_host("127.0.0.1").unwrap();
/// assert_eq!(addresses, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);
/// ```
pub fn resolve_host(host: &str) -> Result<Vec<IpAddr>> {
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = literal.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    let failure = |reason: String| {
        Error::new(
            ErrorCode::Network,
            format!("host `{host}` cannot be resolved: {reason}"),
        )
        .with_problem("dns")
        .with_found(host)
    };
    let mut addresses: Vec<IpAddr> = (host, 0)
        .to_socket_addrs()
        .map_err(|error| failure(error.to_string()))?
        .map(|address| address.ip())
        .collect();
    let mut seen = Vec::new();
    addresses.retain(|ip| {
        let new = !seen.contains(ip);
        seen.push(*ip);
        new
    });
    if addresses.is_empty() {
        return Err(failure("no address".to_owned()));
    }
    Ok(addresses)
}

/// Asks the servers of `config` for the records of `name` of type `kind`.
///
/// Every server is asked in turn, for `config.attempts` rounds, until one
/// answers. An answer without records of the type gives an empty list. For
/// [`QueryType::Any`] one query is sent per type and the records are merged
/// without duplicates, in the order A, AAAA, MX, TXT, NS, CNAME.
///
/// # Errors
///
/// The function fails with `E_INVALID_VALUE` when `name` is not a valid
/// domain name, with `E_NOT_FOUND` when a server answers that the name does
/// not exist, with `E_NETWORK` (problem `dns`) when no server answers or
/// every server reports a failure, and with `E_CANCELLED` or `E_TIMEOUT`
/// when the execution is cancelled or reaches its time limit.
///
/// # Examples
///
/// ```no_run
/// use nurt_net::dns::{QueryType, ResolverConfig, lookup};
///
/// # fn run(ctx: &nurt_contracts::stage::ExecCtx) -> nurt_contracts::error::Result<()> {
/// let config = ResolverConfig::single("192.0.2.53:53".parse().unwrap());
/// let records = lookup(&config, "example.org", QueryType::Mx, ctx)?;
/// assert!(records.iter().all(|record| record.kind == "MX"));
/// # Ok(())
/// # }
/// ```
pub fn lookup(
    config: &ResolverConfig,
    name: &str,
    kind: QueryType,
    ctx: &ExecCtx,
) -> Result<Vec<DnsRecord>> {
    let query_name = domain_name(name)?;
    let mut records: Vec<DnsRecord> = Vec::new();
    for record_type in kind.record_types() {
        let (message, server) = ask(config, &query_name, *record_type, ctx)?;
        if message.metadata.response_code == ResponseCode::NXDomain {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("the name `{name}` does not exist in DNS"),
            )
            .with_found(name)
            .with_detail("server", serde_json::Value::from(server_label(server))));
        }
        for answer in message
            .answers
            .iter()
            .filter(|answer| answer.record_type() == *record_type)
        {
            let record = answer_record(answer, server);
            if !records.contains(&record) {
                records.push(record);
            }
        }
    }
    Ok(records)
}

fn domain_name(name: &str) -> Result<Name> {
    let invalid = || {
        Error::new(
            ErrorCode::InvalidValue,
            format!("`{name}` is not a valid domain name"),
        )
        .with_found(name)
    };
    if name.trim().is_empty() {
        return Err(invalid());
    }
    let mut parsed = Name::from_utf8(name).map_err(|_| invalid())?;
    parsed.set_fqdn(true);
    Ok(parsed)
}

fn answer_record(answer: &Answer, server: SocketAddr) -> DnsRecord {
    let record_type = answer.record_type();
    let mut value = answer.data.to_string();
    if matches!(
        record_type,
        RecordType::MX | RecordType::NS | RecordType::CNAME
    ) {
        value = value.trim_end_matches('.').to_owned();
    }
    DnsRecord {
        name: answer.name.to_ascii().trim_end_matches('.').to_owned(),
        kind: record_type.to_string(),
        value,
        ttl: ValueDuration::from_millis(i64::from(answer.ttl) * MILLIS_PER_SECOND),
        server: Some(server_label(server)),
    }
}

/// Address of a server as reported in records: the address alone for port
/// 53, the socket address otherwise.
fn server_label(server: SocketAddr) -> String {
    if server.port() == DNS_PORT {
        server.ip().to_string()
    } else {
        server.to_string()
    }
}

fn network_error(message: String, server: SocketAddr) -> Error {
    Error::new(ErrorCode::Network, message)
        .with_problem("dns")
        .with_detail("server", serde_json::Value::from(server_label(server)))
}

fn is_interruption(error: &Error) -> bool {
    matches!(
        error.code(),
        ErrorCode::Cancelled | ErrorCode::Timeout | ErrorCode::Limit
    )
}

fn ask(
    config: &ResolverConfig,
    name: &Name,
    record_type: RecordType,
    ctx: &ExecCtx,
) -> Result<(Message, SocketAddr)> {
    let mut last = None;
    for _ in 0..config.attempts.max(1) {
        for server in &config.servers {
            ctx.checkpoint()?;
            match exchange(*server, name, record_type, config.timeout, ctx) {
                Ok(message) => match message.metadata.response_code {
                    ResponseCode::NoError | ResponseCode::NXDomain => {
                        return Ok((message, *server));
                    }
                    code => {
                        last = Some(network_error(
                            format!("DNS server {server} answered `{code}` for `{name}`"),
                            *server,
                        ));
                    }
                },
                Err(error) if is_interruption(&error) => return Err(error),
                Err(error) => last = Some(error),
            }
        }
    }
    Err(last.unwrap_or_else(|| {
        Error::new(
            ErrorCode::Network,
            "no DNS server is configured; give one with `server`",
        )
        .with_problem("dns")
    }))
}

fn exchange(
    server: SocketAddr,
    name: &Name,
    record_type: RecordType,
    timeout: Duration,
    ctx: &ExecCtx,
) -> Result<Message> {
    let mut query = Message::query();
    query.metadata.recursion_desired = true;
    query.add_query(Query::query(name.clone(), record_type));
    let id = query.metadata.id;
    let bytes = query.to_vec().map_err(|error| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("the query for `{name}` cannot be encoded: {error}"),
        )
    })?;
    let timeout = ctx.remaining().map_or(timeout, |left| left.min(timeout));
    let question = Question { name, record_type };
    let answer = udp_exchange(server, &bytes, id, &question, timeout, ctx)?;
    if answer.metadata.truncation {
        return tcp_exchange(server, &bytes, id, &question, timeout);
    }
    Ok(answer)
}

/// What a query asks, to recognise the answer to it.
struct Question<'a> {
    name: &'a Name,
    record_type: RecordType,
}

fn udp_exchange(
    server: SocketAddr,
    query: &[u8],
    id: u16,
    question: &Question<'_>,
    timeout: Duration,
    ctx: &ExecCtx,
) -> Result<Message> {
    let io_error = |error: io::Error| {
        network_error(
            format!("DNS server {server} cannot be reached: {error}"),
            server,
        )
    };
    let local: SocketAddr = match server {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    let socket = UdpSocket::bind(local).map_err(io_error)?;
    socket.connect(server).map_err(io_error)?;
    socket.send(query).map_err(io_error)?;
    let started = Instant::now();
    let mut buffer = vec![0_u8; UDP_BUFFER];
    loop {
        ctx.checkpoint()?;
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(network_error(
                format!(
                    "DNS server {server} did not answer within {} ms",
                    timeout.as_millis()
                ),
                server,
            ));
        }
        socket
            .set_read_timeout(Some(remaining.min(POLL_SLICE)))
            .map_err(io_error)?;
        match socket.recv(&mut buffer) {
            Ok(length) => {
                let answer = buffer
                    .get(..length)
                    .and_then(|bytes| Message::from_vec(bytes).ok());
                if let Some(answer) = answer.filter(|answer| is_answer_to(answer, id, question)) {
                    return Ok(answer);
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(io_error(error)),
        }
    }
}

fn tcp_exchange(
    server: SocketAddr,
    query: &[u8],
    id: u16,
    question: &Question<'_>,
    timeout: Duration,
) -> Result<Message> {
    let io_error = |error: io::Error| {
        network_error(
            format!("DNS server {server} cannot be reached over TCP: {error}"),
            server,
        )
    };
    let length = u16::try_from(query.len()).map_err(|_| {
        network_error(
            "the DNS query is longer than a TCP message".to_owned(),
            server,
        )
    })?;
    let mut stream = TcpStream::connect_timeout(&server, timeout).map_err(io_error)?;
    stream.set_read_timeout(Some(timeout)).map_err(io_error)?;
    stream.set_write_timeout(Some(timeout)).map_err(io_error)?;
    let mut framed = length.to_be_bytes().to_vec();
    framed.extend_from_slice(query);
    stream.write_all(&framed).map_err(io_error)?;
    let mut prefix = [0_u8; 2];
    stream.read_exact(&mut prefix).map_err(io_error)?;
    let mut body = vec![0_u8; usize::from(u16::from_be_bytes(prefix))];
    stream.read_exact(&mut body).map_err(io_error)?;
    Message::from_vec(&body)
        .ok()
        .filter(|answer| is_answer_to(answer, id, question))
        .ok_or_else(|| {
            network_error(
                format!("DNS server {server} sent an answer that cannot be read"),
                server,
            )
        })
}

/// Whether `message` answers the query with the identifier `id` and the
/// question `question`: an answer to another question, whose identifier a
/// forger or a late reply to an earlier query may repeat, is not accepted.
fn is_answer_to(message: &Message, id: u16, question: &Question<'_>) -> bool {
    message.metadata.id == id
        && message.metadata.message_type == MessageType::Response
        && matches!(
            message.queries.as_slice(),
            [asked] if asked.name() == question.name
                && asked.query_type() == question.record_type
        )
}

fn query_type(args: &BoundArgs) -> Result<QueryType> {
    match args.value(PARAM_TYPE) {
        None | Some(Value::Null) => Ok(QueryType::A),
        Some(Value::Enum(text) | Value::Text(text)) => text.parse(),
        Some(other) => Err(Error::new(
            ErrorCode::Type,
            format!(
                "`{COMMAND}` expects a record type for `{PARAM_TYPE}`, not {}",
                other.type_name()
            ),
        )),
    }
}

fn required_text<'a>(args: &'a BoundArgs, name: &str) -> Result<&'a str> {
    args.require(name)?;
    args.text(name)?.ok_or_else(|| {
        Error::new(
            ErrorCode::Type,
            format!("`{COMMAND}` expects text for `{name}`"),
        )
    })
}

/// Implementation of the `network.resolve` command.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_net::dns::Resolve;
///
/// assert_eq!(Resolve::new().descriptor().command, "network.resolve");
/// ```
#[derive(Debug, Clone, Default)]
pub struct Resolve {
    root: SysRoot,
}

impl Resolve {
    /// The command using the configuration of the host.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The command reading the system configuration below `root`.
    #[must_use]
    pub fn with_root(root: SysRoot) -> Self {
        Self { root }
    }
}

impl StageImpl for Resolve {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        query_type(args)?;
        let mut info = StaticInfo::default();
        if let Some(name) = args.text(PARAM_NAME)? {
            domain_name(name)?;
            info.details
                .insert(PARAM_NAME.to_owned(), Value::Text(name.to_owned()));
        }
        if let Some(server) = args.text(PARAM_SERVER)? {
            let literal = server.parse::<SocketAddr>().ok().or_else(|| {
                server
                    .parse::<IpAddr>()
                    .ok()
                    .map(|ip| SocketAddr::new(ip, DNS_PORT))
            });
            info.network.push(NetworkTarget {
                scheme: "dns".to_owned(),
                host: literal.map_or_else(|| server.to_owned(), |address| address.ip().to_string()),
                port: Some(literal.map_or(DNS_PORT, |address| address.port())),
                url: None,
            });
        }
        Ok(info)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Resolve {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let name = required_text(args, PARAM_NAME)?;
        let kind = query_type(args)?;
        let config = match args.text(PARAM_SERVER)? {
            Some(server) => ResolverConfig::single(parse_server(server)?),
            None => ResolverConfig::system(&self.root)?,
        };
        let records: Vec<Record> = lookup(&config, name, kind, ctx)?
            .into_iter()
            .map(Record::from)
            .collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

#[cfg(test)]
mod tests {
    use hickory_resolver::proto::rr::RData;
    use hickory_resolver::proto::rr::rdata::{A, CNAME, MX, TXT};

    use super::*;

    fn answer(name: &str, data: RData) -> Answer {
        Answer::from_rdata(Name::from_ascii(name).unwrap(), 300, data)
    }

    fn response(id: u16, questions: &[(&str, RecordType)]) -> Message {
        let mut message = Message::query();
        message.metadata.id = id;
        message.metadata.message_type = MessageType::Response;
        for (name, record_type) in questions {
            message.add_query(Query::query(Name::from_ascii(name).unwrap(), *record_type));
        }
        message
    }

    #[test]
    fn an_answer_must_repeat_the_question_of_the_query() {
        let name = Name::from_ascii("example.test.").unwrap();
        let question = Question {
            name: &name,
            record_type: RecordType::A,
        };
        let same = response(7, &[("EXAMPLE.test.", RecordType::A)]);
        assert!(is_answer_to(&same, 7, &question));
        let other_name = response(7, &[("other.test.", RecordType::A)]);
        assert!(!is_answer_to(&other_name, 7, &question));
        let other_type = response(7, &[("example.test.", RecordType::AAAA)]);
        assert!(!is_answer_to(&other_type, 7, &question));
        let no_question = response(7, &[]);
        assert!(!is_answer_to(&no_question, 7, &question));
        let two = response(
            7,
            &[
                ("example.test.", RecordType::A),
                ("other.test.", RecordType::A),
            ],
        );
        assert!(!is_answer_to(&two, 7, &question));
        let other_id = response(8, &[("example.test.", RecordType::A)]);
        assert!(!is_answer_to(&other_id, 7, &question));
    }

    #[test]
    fn query_type_parsing_ignores_case() {
        assert_eq!("MX".parse::<QueryType>().unwrap(), QueryType::Mx);
        let error = "srv".parse::<QueryType>().unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.found.as_deref(), Some("srv"));
    }

    #[test]
    fn resolv_conf_keeps_three_servers_and_clamps_options() {
        let config = ResolverConfig::from_resolv_conf(
            "# comment\nnameserver 192.0.2.1\nnameserver fe80::1%eth0\nnameserver 192.0.2.3\n\
             nameserver 192.0.2.4\noptions timeout:99 attempts:0 rotate\nsearch example.org\n",
        );
        assert_eq!(config.servers.len(), 3);
        assert_eq!(config.servers[1], "[fe80::1]:53".parse().unwrap());
        assert_eq!(config.timeout, MAX_TIMEOUT);
        assert_eq!(config.attempts, 1);
    }

    #[test]
    fn resolv_conf_without_servers_uses_localhost() {
        let config = ResolverConfig::from_resolv_conf("search example.org\n");
        assert_eq!(config.servers, vec!["127.0.0.1:53".parse().unwrap()]);
        assert_eq!(config.attempts, DEFAULT_ATTEMPTS);
    }

    #[test]
    fn missing_resolv_conf_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = ResolverConfig::system(&SysRoot::new(dir.path())).unwrap();
        assert_eq!(config, ResolverConfig::from_resolv_conf(""));
    }

    #[test]
    fn server_parsing_accepts_addresses_with_and_without_port() {
        assert_eq!(
            parse_server("192.0.2.53").unwrap(),
            "192.0.2.53:53".parse().unwrap()
        );
        assert_eq!(
            parse_server("2001:db8::53").unwrap(),
            "[2001:db8::53]:53".parse().unwrap()
        );
        assert_eq!(
            parse_server("127.0.0.1:5353").unwrap(),
            "127.0.0.1:5353".parse().unwrap()
        );
        let error = parse_server("not a server").unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn answers_are_formatted_without_trailing_dots() {
        let server: SocketAddr = "192.0.2.53:53".parse().unwrap();
        let mx = MX::new(10, Name::from_ascii("mail.example.org.").unwrap());
        let record = answer_record(&answer("example.org.", RData::MX(mx)), server);
        assert_eq!(record.name, "example.org");
        assert_eq!(record.kind, "MX");
        assert_eq!(record.value, "10 mail.example.org");
        assert_eq!(record.ttl, ValueDuration::from_millis(300_000));
        assert_eq!(record.server.as_deref(), Some("192.0.2.53"));

        let alias = CNAME(Name::from_ascii("target.example.org.").unwrap());
        let record = answer_record(&answer("www.example.org.", RData::CNAME(alias)), server);
        assert_eq!(record.value, "target.example.org");

        let text = TXT::new(vec!["v=spf1 -all".to_owned()]);
        let record = answer_record(&answer("example.org.", RData::TXT(text)), server);
        assert_eq!(record.value, "v=spf1 -all");

        let address = RData::A(A(Ipv4Addr::new(192, 0, 2, 7)));
        let other: SocketAddr = "127.0.0.1:5353".parse().unwrap();
        let record = answer_record(&answer("example.org.", address), other);
        assert_eq!(record.value, "192.0.2.7");
        assert_eq!(record.server.as_deref(), Some("127.0.0.1:5353"));
    }

    #[test]
    fn invalid_domain_names_are_rejected() {
        assert_eq!(
            domain_name(" ").unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        let long_label = "a".repeat(64);
        assert_eq!(
            domain_name(&long_label).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn host_literals_are_not_resolved() {
        assert_eq!(
            resolve_host("[::1]").unwrap(),
            vec![IpAddr::V6(Ipv6Addr::LOCALHOST)]
        );
    }
}
