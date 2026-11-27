// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::ports::tests
// Responsibility:   Socket table held in memory.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::net::{Ipv6Addr, SocketAddr};
use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::record::{AddressFamily, Port, PortState, Protocol};
use nurt_contracts::report::Notable;
use nurt_contracts::stage::{ArgValue, BoundArgs, CancelToken, Demand, ExecCtx};
use nurt_contracts::value::Value;

use super::libproc::{
    INI_IPV4, INSI_FADDR, INSI_FPORT, INSI_LADDR, INSI_LPORT, INSI_VFLAG, SOCKET_FDINFO_SIZE,
    SOCKINFO_IN, SOCKINFO_TCP, SOI_KIND, SOI_PROTOCOL, TCPSI_STATE,
};
use super::win32::{
    TCP4_ROW_SIZE, TCP6_ROW_SIZE, TableKind, UDP4_ROW_SIZE, UDP6_ROW_SIZE, decode_table,
};
use super::{COMMAND, Ports, ProtocolFilter, SocketEntry, SocketSource, SocketTable, libproc};
use crate::processes::testing::{SystemClock, VirtualClock, context, run};

/// Socket table held in memory.
struct FakeSockets {
    entries: Vec<SocketEntry>,
    unattributed: u64,
}

impl SocketSource for FakeSockets {
    fn sockets(&self, _protocols: ProtocolFilter) -> Result<SocketTable> {
        Ok(SocketTable {
            entries: self.entries.clone(),
            unattributed: self.unattributed,
            hidden_processes: 0,
        })
    }
}

fn socket(protocol: Protocol, local: &str, remote: Option<&str>, state: PortState) -> SocketEntry {
    SocketEntry {
        protocol,
        local: local.parse().unwrap(),
        remote: remote.map(|remote| remote.parse().unwrap()),
        state,
        pid: Some(10),
        process: Some("server".to_owned()),
    }
}

fn ctx() -> ExecCtx {
    context(VirtualClock::at(0), Demand::default(), CancelToken::new())
}

fn call(state: &str, protocol: &str) -> BoundArgs {
    BoundArgs::new(COMMAND)
        .with(
            "state",
            ArgValue::Value(Value::Enum(state.to_owned())),
            false,
        )
        .with(
            "protocol",
            ArgValue::Value(Value::Enum(protocol.to_owned())),
            false,
        )
}

fn ports(source: FakeSockets, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Port>> {
    let stage = Ports::with_source(Arc::new(source));
    Ok(run(&stage, args, ctx)?
        .into_iter()
        .map(|record| Port::try_from(record).unwrap())
        .collect())
}

/// One listening TCP socket, one connected TCP socket, one unconnected and
/// one connected UDP socket.
fn mixed() -> FakeSockets {
    FakeSockets {
        entries: vec![
            socket(Protocol::Tcp, "0.0.0.0:80", None, PortState::Listen),
            socket(
                Protocol::Tcp,
                "10.0.0.2:80",
                Some("10.0.0.9:51000"),
                PortState::Established,
            ),
            socket(Protocol::Udp, "[::]:53", None, PortState::None),
            socket(
                Protocol::Udp,
                "10.0.0.2:41000",
                Some("10.0.0.1:53"),
                PortState::None,
            ),
        ],
        unattributed: 0,
    }
}

#[test]
fn ports_udp_state_none_and_listen_filter() {
    let listening = ports(mixed(), &BoundArgs::new(COMMAND), &ctx()).unwrap();
    let locals: Vec<&str> = listening.iter().map(|p| p.local.as_str()).collect();
    assert_eq!(locals, ["0.0.0.0:80", "[::]:53"]);
    assert_eq!(listening[1].state, PortState::None);
    assert_eq!(listening[1].family, Some(AddressFamily::Ipv6));
    assert_eq!(listening[1].address, "::");
    assert_eq!(listening[0].remote, None);

    let all = ports(mixed(), &call("all", "any"), &ctx()).unwrap();
    assert_eq!(all.len(), 4);
    let connected = all.iter().find(|p| p.port == 41000).unwrap();
    assert_eq!(connected.state, PortState::None);
    assert_eq!(connected.remote.as_deref(), Some("10.0.0.1:53"));
    assert_eq!(connected.remote_address.as_deref(), Some("10.0.0.1"));
    assert_eq!(connected.remote_port, Some(53));
    assert_eq!(connected.pid, Some(10));
    assert_eq!(connected.process.as_deref(), Some("server"));
}

#[test]
fn ports_protocol_filter_selects_tcp() {
    let tcp = ports(mixed(), &call("all", "tcp"), &ctx()).unwrap();
    assert!(tcp.iter().all(|p| p.protocol == Protocol::Tcp));
    assert_eq!(tcp.len(), 2);
}

#[test]
fn ports_invalid_state_is_invalid_value() {
    let error = ports(mixed(), &call("closed", "any"), &ctx()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn ports_unattributed_sockets_noted_once() {
    let mut source = mixed();
    source.unattributed = 3;
    let ctx = ctx();
    ports(source, &call("all", "any"), &ctx).unwrap();
    let notes = ctx.take_notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].code, Notable::SourceUnavailable);
    assert_eq!(notes[0].count, Some(3));
}

/// Row of `MIB_TCPROW_OWNER_PID`: state, local address and port, remote
/// address and port, process.
fn tcp4_row(
    state: u32,
    local: [u8; 4],
    port: u16,
    remote: [u8; 4],
    rport: u16,
    pid: u32,
) -> Vec<u8> {
    let mut row = state.to_le_bytes().to_vec();
    row.extend_from_slice(&local);
    row.extend_from_slice(&[port.to_be_bytes()[0], port.to_be_bytes()[1], 0, 0]);
    row.extend_from_slice(&remote);
    row.extend_from_slice(&[rport.to_be_bytes()[0], rport.to_be_bytes()[1], 0, 0]);
    row.extend_from_slice(&pid.to_le_bytes());
    assert_eq!(row.len(), TCP4_ROW_SIZE);
    row
}

#[test]
fn ports_decode_win32_tcp_table_buffer() {
    let mut buffer = 2_u32.to_le_bytes().to_vec();
    buffer.extend(tcp4_row(2, [127, 0, 0, 1], 8080, [0; 4], 0, 4321));
    buffer.extend(tcp4_row(5, [10, 0, 0, 2], 443, [10, 0, 0, 9], 50123, 99));
    let entries = decode_table(TableKind::Tcp4, &buffer).unwrap();
    assert_eq!(
        entries[0].local,
        "127.0.0.1:8080".parse::<SocketAddr>().unwrap()
    );
    assert_eq!(entries[0].state, PortState::Listen);
    assert_eq!(entries[0].remote, None);
    assert_eq!(entries[0].pid, Some(4321));
    assert_eq!(entries[1].state, PortState::Established);
    assert_eq!(entries[1].remote, Some("10.0.0.9:50123".parse().unwrap()));
    assert_eq!(
        decode_table(TableKind::Tcp4, &buffer[..30]),
        None,
        "a cut row is rejected"
    );

    let mut v6 = 1_u32.to_le_bytes().to_vec();
    let mut row = Ipv6Addr::LOCALHOST.octets().to_vec();
    row.extend_from_slice(&0_u32.to_le_bytes());
    row.extend_from_slice(&[0x1F, 0x90, 0, 0]);
    row.extend_from_slice(&[0; 16]);
    row.extend_from_slice(&0_u32.to_le_bytes());
    row.extend_from_slice(&0_u32.to_le_bytes());
    row.extend_from_slice(&2_u32.to_le_bytes());
    row.extend_from_slice(&7_u32.to_le_bytes());
    assert_eq!(row.len(), TCP6_ROW_SIZE);
    v6.extend(row);
    let entries = decode_table(TableKind::Tcp6, &v6).unwrap();
    assert_eq!(
        entries[0].local,
        "[::1]:8080".parse::<SocketAddr>().unwrap()
    );
    assert_eq!(entries[0].pid, Some(7));
}

#[test]
fn ports_decode_win32_udp_table_buffers() {
    let mut udp = 1_u32.to_le_bytes().to_vec();
    udp.extend_from_slice(&[0, 0, 0, 0, 0, 53, 0, 0]);
    udp.extend_from_slice(&5_u32.to_le_bytes());
    assert_eq!(udp.len(), 4 + UDP4_ROW_SIZE);
    let entries = decode_table(TableKind::Udp4, &udp).unwrap();
    assert_eq!(entries[0].protocol, Protocol::Udp);
    assert_eq!(entries[0].state, PortState::None);
    assert_eq!(entries[0].local.port(), 53);

    let mut udp6 = 1_u32.to_le_bytes().to_vec();
    udp6.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
    udp6.extend_from_slice(&0_u32.to_le_bytes());
    udp6.extend_from_slice(&[0x14, 0xE9, 0, 0]);
    udp6.extend_from_slice(&11_u32.to_le_bytes());
    assert_eq!(udp6.len(), 4 + UDP6_ROW_SIZE);
    let entries = decode_table(TableKind::Udp6, &udp6).unwrap();
    assert_eq!(
        entries[0].local,
        "[::1]:5353".parse::<SocketAddr>().unwrap()
    );
    assert_eq!(entries[0].state, PortState::None);
    assert_eq!(entries[0].pid, Some(11));
}

fn put_int(buffer: &mut [u8], offset: usize, value: i32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

/// Port as stored by the kernel: network order in the lower 16 bits.
fn kernel_port(port: u16) -> i32 {
    i32::from(u16::from_ne_bytes(port.to_be_bytes()))
}

/// Value the checked layout file asserts for `label`: the sum after `==` on
/// the line that names the label.
fn asserted_layout(label: &str) -> usize {
    let source = include_str!("../../tests/fixtures/macos/socket_layout.c");
    let line = source
        .lines()
        .find(|line| line.contains("_Static_assert") && line.contains(&format!("\"{label}\"")))
        .unwrap_or_else(|| panic!("the layout file does not assert {label}"));
    let expression = line
        .split("==")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .unwrap_or_else(|| panic!("no comparison in {line}"));
    expression
        .split('+')
        .map(|term| term.trim().parse::<usize>().unwrap())
        .sum()
}

#[test]
fn macos_socket_layout_constants_equal_the_layout_checked_against_the_sdk() {
    // tests/fixtures/macos/socket_layout.c compiles against the headers of
    // the macOS SDK only when these numbers are the layout of the SDK.
    assert_eq!(SOCKET_FDINFO_SIZE, asserted_layout("SOCKET_FDINFO_SIZE"));
    assert_eq!(SOI_PROTOCOL, asserted_layout("SOI_PROTOCOL"));
    assert_eq!(SOI_KIND, asserted_layout("SOI_KIND"));
    assert_eq!(INSI_FPORT, asserted_layout("INSI_FPORT"));
    assert_eq!(INSI_LPORT, asserted_layout("INSI_LPORT"));
    assert_eq!(INSI_VFLAG, asserted_layout("INSI_VFLAG"));
    assert_eq!(INSI_FADDR, asserted_layout("INSI_FADDR"));
    assert_eq!(INSI_LADDR, asserted_layout("INSI_LADDR"));
    assert_eq!(TCPSI_STATE, asserted_layout("TCPSI_STATE"));
}

#[test]
fn ports_decode_libproc_socket_info() {
    let mut tcp = vec![0_u8; SOCKET_FDINFO_SIZE];
    put_int(&mut tcp, SOI_KIND, SOCKINFO_TCP);
    tcp[INSI_VFLAG] = INI_IPV4;
    tcp[INSI_LADDR + 12..INSI_LADDR + 16].copy_from_slice(&[127, 0, 0, 1]);
    put_int(&mut tcp, INSI_LPORT, kernel_port(5432));
    put_int(&mut tcp, TCPSI_STATE, 1);
    let decoded = libproc::decode(&tcp).unwrap();
    assert_eq!(decoded.protocol, Protocol::Tcp);
    assert_eq!(
        decoded.local,
        "127.0.0.1:5432".parse::<SocketAddr>().unwrap()
    );
    assert_eq!(decoded.state, PortState::Listen);
    assert_eq!(decoded.remote, None);

    let mut udp = vec![0_u8; SOCKET_FDINFO_SIZE];
    put_int(&mut udp, SOI_KIND, SOCKINFO_IN);
    put_int(&mut udp, SOI_PROTOCOL, 17);
    udp[INSI_LADDR..INSI_LADDR + 16].copy_from_slice(&Ipv6Addr::LOCALHOST.octets());
    udp[INSI_FADDR..INSI_FADDR + 16].copy_from_slice(&Ipv6Addr::LOCALHOST.octets());
    put_int(&mut udp, INSI_LPORT, kernel_port(5353));
    put_int(&mut udp, INSI_FPORT, kernel_port(53));
    let decoded = libproc::decode(&udp).unwrap();
    assert_eq!(decoded.protocol, Protocol::Udp);
    assert_eq!(decoded.local, "[::1]:5353".parse::<SocketAddr>().unwrap());
    assert_eq!(decoded.remote, Some("[::1]:53".parse().unwrap()));

    let mut unix = vec![0_u8; SOCKET_FDINFO_SIZE];
    put_int(&mut unix, SOI_KIND, 3);
    assert_eq!(
        libproc::decode(&unix),
        None,
        "other socket kinds are skipped"
    );
    assert_eq!(libproc::decode(&tcp[..100]), None);
}

#[cfg(unix)]
mod procnet_tree {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;
    use crate::common::SysRoot;
    use crate::ports::procnet::{ProcNet, parse_endpoint, parse_table, tcp_state};
    use crate::processes::testing::write;

    const HEADER: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when \
                          retrnsmt   uid  timeout inode\n";

    /// Address of `ip` as printed by the kernel of this host.
    fn hex(ip: IpAddr) -> String {
        match ip {
            IpAddr::V4(v4) => format!("{:08X}", u32::from_ne_bytes(v4.octets())),
            IpAddr::V6(v6) => v6
                .octets()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|word| format!("{:08X}", u32::from_ne_bytes(*word)))
                .collect(),
        }
    }

    fn row(index: usize, local: SocketAddr, remote: SocketAddr, state: u8, inode: u64) -> String {
        format!(
            "{index:4}: {}:{:04X} {}:{:04X} {state:02X} 00000000:00000000 00:00000000 00000000 \
             1000 0 {inode} 1 0000000000000000 100 0 0 10 0\n",
            hex(local.ip()),
            local.port(),
            hex(remote.ip()),
            remote.port()
        )
    }

    fn any4() -> SocketAddr {
        SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)
    }

    #[test]
    fn ports_parse_proc_net_tcp_ipv4() {
        let local: SocketAddr = "127.0.0.1:53".parse().unwrap();
        let peer: SocketAddr = "192.168.1.20:40000".parse().unwrap();
        let text = format!(
            "{HEADER}{}{}   2: malformed\n",
            row(0, local, any4(), 0x0A, 1234),
            row(1, local, peer, 0x01, 0)
        );
        let rows = parse_table(&text);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].local, local);
        assert_eq!(tcp_state(rows[0].state), PortState::Listen);
        assert_eq!(rows[0].inode, 1234);
        assert_eq!(rows[1].remote, peer);
        assert_eq!(tcp_state(rows[1].state), PortState::Established);
        assert_eq!(tcp_state(0x06), PortState::TimeWait);
        assert_eq!(tcp_state(0x04), PortState::FinWait);
        assert_eq!(tcp_state(0x07), PortState::Closed);
    }

    #[test]
    fn ports_parse_proc_net_tcp6_word_order() {
        let address: Ipv6Addr = "2001:db8::1:2".parse().unwrap();
        let endpoint = format!("{}:1F90", hex(IpAddr::V6(address)));
        assert_eq!(
            parse_endpoint(&endpoint),
            Some(SocketAddr::new(address.into(), 8080))
        );
        assert_eq!(
            parse_endpoint("0000:0050"),
            None,
            "an address of 16 bits is malformed"
        );
    }

    #[test]
    fn ports_inode_to_pid_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let listener: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        let tcp6: SocketAddr = "[::]:22".parse().unwrap();
        let any6 = SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0);
        write(
            root,
            "/proc/net/tcp",
            format!("{HEADER}{}", row(0, listener, any4(), 0x0A, 555)),
        );
        write(
            root,
            "/proc/net/tcp6",
            format!("{HEADER}{}", row(0, tcp6, any6, 0x0A, 777)),
        );
        write(root, "/proc/net/udp", HEADER);
        write(root, "/proc/31/comm", "nginx\n");
        write(root, "/proc/31/fd/.keep", "");
        std::fs::remove_file(root.join("proc/31/fd/.keep")).unwrap();
        std::os::unix::fs::symlink("socket:[555]", root.join("proc/31/fd/3")).unwrap();
        std::os::unix::fs::symlink("/dev/null", root.join("proc/31/fd/0")).unwrap();
        let stage = Ports::with_source(Arc::new(ProcNet::new(SysRoot::new(root))));
        let records: Vec<Port> = run(&stage, &BoundArgs::new(COMMAND), &ctx())
            .unwrap()
            .into_iter()
            .map(|record| Port::try_from(record).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        let web = records.iter().find(|p| p.port == 8080).unwrap();
        assert_eq!(web.pid, Some(31));
        assert_eq!(web.process.as_deref(), Some("nginx"));
        assert_eq!(web.local, "0.0.0.0:8080");
        let ssh = records.iter().find(|p| p.port == 22).unwrap();
        assert_eq!(ssh.pid, None, "no descriptor links to inode 777");
        assert_eq!(ssh.local, "[::]:22");
    }

    #[test]
    fn ports_process_name_that_is_not_utf8_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let listener: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        write(
            root,
            "/proc/net/tcp",
            format!("{HEADER}{}", row(0, listener, any4(), 0x0A, 555)),
        );
        std::fs::create_dir_all(root.join("proc/31/fd")).unwrap();
        std::fs::write(root.join("proc/31/comm"), b"ng\xFFinx\n".as_slice()).unwrap();
        std::os::unix::fs::symlink("socket:[555]", root.join("proc/31/fd/3")).unwrap();
        let stage = Ports::with_source(Arc::new(ProcNet::new(SysRoot::new(root))));
        let records: Vec<Port> = run(&stage, &BoundArgs::new(COMMAND), &ctx())
            .unwrap()
            .into_iter()
            .map(|record| Port::try_from(record).unwrap())
            .collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].pid, Some(31));
        assert_eq!(records[0].process.as_deref(), Some("ng\u{FFFD}inx"));
    }
}

#[test]
#[ignore = "requires sockets on the loopback interface of the running system"]
fn it_ports_finds_own_loopback_listener() {
    let v4 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let v6 = std::net::TcpListener::bind("[::1]:0").ok();
    let ctx = context(SystemClock::new(), Demand::default(), CancelToken::new());
    let records: Vec<Port> = run(&Ports::new(), &call("listen", "tcp"), &ctx)
        .unwrap()
        .into_iter()
        .map(|record| Port::try_from(record).unwrap())
        .collect();
    let own = Some(u64::from(std::process::id()));
    let mut expected = vec![v4.local_addr().unwrap()];
    expected.extend(v6.iter().map(|listener| listener.local_addr().unwrap()));
    for address in expected {
        let found = records
            .iter()
            .find(|p| p.local == address.to_string())
            .unwrap_or_else(|| panic!("{address} is not listed"));
        assert_eq!(found.pid, own);
        assert_eq!(found.state, PortState::Listen);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
