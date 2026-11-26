// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
/*
 * Layout of `struct socket_fdinfo` of <sys/proc_info.h>, as the decoder
 * crates/nurt-sys/src/ports/libproc.rs reads it. The file compiles only when
 * every offset and the size equal the constants of the decoder, so a layout
 * of the macOS SDK that differs fails the compilation of this file.
 *
 * The check needs no macOS host; it compiles for both architectures against
 * the headers of the macOS SDK:
 *
 *   zig cc -target aarch64-macos -c socket_layout.c -o /dev/null
 *   zig cc -target x86_64-macos  -c socket_layout.c -o /dev/null
 *
 * The check was run for aarch64-macos and x86_64-macos with the headers
 * shipped with zig 0.16.0; both compile.
 */
#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>
#include <sys/proc_info.h>

#define SOCKET(member) (offsetof(struct socket_fdinfo, psi) + offsetof(struct socket_info, member))
#define PROTO(member) (SOCKET(soi_proto) + offsetof(struct in_sockinfo, member))
#define TCP(member) (SOCKET(soi_proto) + offsetof(struct tcp_sockinfo, member))

_Static_assert(sizeof(struct socket_fdinfo) == 792, "SOCKET_FDINFO_SIZE");
_Static_assert(offsetof(struct socket_fdinfo, psi) == 24, "SOCKET_INFO");
_Static_assert(SOCKET(soi_protocol) == 24 + 156, "SOI_PROTOCOL");
_Static_assert(SOCKET(soi_kind) == 24 + 232, "SOI_KIND");
_Static_assert(SOCKET(soi_proto) == 24 + 240, "PROTO");
_Static_assert(PROTO(insi_fport) == 24 + 240, "INSI_FPORT");
_Static_assert(PROTO(insi_lport) == 24 + 240 + 4, "INSI_LPORT");
_Static_assert(PROTO(insi_vflag) == 24 + 240 + 24, "INSI_VFLAG");
_Static_assert(PROTO(insi_faddr) == 24 + 240 + 32, "INSI_FADDR");
_Static_assert(PROTO(insi_laddr) == 24 + 240 + 48, "INSI_LADDR");
_Static_assert(TCP(tcpsi_state) == 24 + 240 + 80, "TCPSI_STATE");
_Static_assert(offsetof(struct in4in6_addr, i46a_addr4) == 12, "IPV4_IN_IN6");
_Static_assert(sizeof(struct proc_fdinfo) == 8, "proc_fdinfo");
_Static_assert(SOCKINFO_IN == 1 && SOCKINFO_TCP == 2, "soi_kind values");
_Static_assert(INI_IPV4 == 0x1, "INI_IPV4");
_Static_assert(TSI_S_LISTEN == 1 && TSI_S_ESTABLISHED == 4 && TSI_S_TIME_WAIT == 10, "TSI_S_*");
_Static_assert(PROX_FDTYPE_SOCKET == 2, "PROX_FDTYPE_SOCKET");
