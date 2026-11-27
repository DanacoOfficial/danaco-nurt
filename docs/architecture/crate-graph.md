<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/architecture/crate-graph.md -->
<!-- Responsibility:   Documents Crate graph and visibility. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Crate graph and visibility

The workspace is split into crates with the prefix `nurt-`. The allowed
dependencies between them are data: `tables/crate-graph.toml`. The test
`crates/nurt-contracts/tests/workspace_graph.rs` reads every
`crates/*/Cargo.toml` and fails when a manifest declares a dependency on a
workspace crate that the table does not allow, when a crate has no entry, or
when the table breaks one of the rules below. A new dependency between
workspace crates therefore starts with a change of the table.

## Layers

Every crate belongs to one layer. A crate depends only on crates of the same
or a lower layer, and the graph is acyclic.

| Layer | Crates | Role |
|---|---|---|
| contract | `nurt-contracts` | types, traits and schemas shared by every crate; depends on no workspace crate |
| foundation | `nurt-types`, `nurt-catalog` | values of the language and the command registry |
| base | `nurt-fs`, `nurt-syntax`, `nurt-testkit` | file system layer, syntax, test support |
| library | `nurt-journal`, `nurt-space`, `nurt-report`, `nurt-plan`, `nurt-correct`, `nurt-text`, `nurt-doc`, `nurt-code`, `nurt-proc`, `nurt-git-transport`, `nurt-syslog`, `nurt-pdf-extract` | reusable services |
| component | `nurt-bind`, `nurt-help`, `nurt-langserver`, `nurt-migrate`, `nurt-exec`, `nurt-ext`, `nurt-archive`, `nurt-notebook`, `nurt-files`, `nurt-lspclient`, `nurt-verify`, `nurt-git`, `nurt-git-ops`, `nurt-sys`, `nurt-net`, `nurt-sched` | commands, binding, execution, extension points |
| host | `nurt-core`, `nurt-service` | composition of all components: the engine facade and the background service |
| interface | `nurt-ffi`, `nurt-cli`, `nurt-mcp` | entry points: C interface, command line, Model Context Protocol server |
| tool | `nurt-conformance`, `nurt-bench` | conformance suite and benchmark |

```
contract     nurt-contracts
                 |
foundation   nurt-types  nurt-catalog
                 |
base         nurt-fs  nurt-syntax  nurt-testkit
                 |
library      journal space report plan correct text doc code proc git-transport syslog
                 |
component    bind help langserver migrate exec ext archive notebook files
             lspclient verify git git-ops sys net sched
                 |
host         nurt-core  nurt-service
                 |
interface    nurt-ffi  nurt-cli  nurt-mcp
                 |
tool         nurt-conformance  nurt-bench
```

## Areas and shared crates

Every crate also belongs to an area of responsibility: `runtime`,
`language`, `storage`, `files`, `structured`, `process`, `system`, `git` or
`integration`. Crates of different areas depend on each other only through
shared crates:

| Shared crate | Why it is shared |
|---|---|
| `nurt-contracts` | the contracts themselves |
| `nurt-types` | values and records |
| `nurt-catalog` | the command registry used by binding, help and the language server |
| `nurt-fs` | the file system layer of every platform |
| `nurt-journal` | the journal of changes |
| `nurt-testkit` | test support; may also be a development dependency of any crate of the base layer or above |

Commands of one area never call another area directly. When a command needs
a service of another area (for example structured documents in a test
command), the service is a trait in `nurt-contracts`, implemented in its own
area and passed in by the host layer. Changes to files go through the
transaction interface of the journal in `nurt-contracts`, never around it.

The integrating layers (`host`, `interface`, `tool`) compose crates of all
areas and are exempt from the area rule.

## Reserved names

Entries marked `reserved = true` in the table name crates that are planned
but not yet in the workspace: `nurt-programs`, `nurt-tooldef`, `nurt-docgen`,
`nurt-crashfs` and the published facade `nurt`. The test fails when such a
crate appears in `crates/` while the mark is still present, so that its entry
is reviewed when the crate is created.

## Visibility

- Items are private by default. Items shared between modules of one crate
  are `pub(crate)`; `pub` is reserved for the public interface.
- The public interface of a crate is what `lib.rs` exposes. Internal modules
  are private (`mod`, not `pub mod`) and the public items are re-exported
  with `pub use`. The exception is `nurt-contracts`, whose public modules are
  the namespaces of the contracts (`nurt_contracts::config`,
  `nurt_contracts::journal`, and so on).
- The lint `unreachable_pub` is enabled for the workspace: a `pub` item that
  is not reachable from `lib.rs` is reported.
- `#[doc(hidden)] pub` is not used to share internals between crates; a
  need to share means the item belongs in a lower crate or in a contract.
- Every public item is documented (`missing_docs` is denied in library
  crates).
- Test helpers live in `#[cfg(test)]` modules or in `nurt-testkit`, never in
  the public interface of a production crate.
- Unsafe code is forbidden except in the crates that call operating system
  interfaces by definition (`nurt-ffi`, `nurt-fs`, `nurt-sys`); there, every
  `unsafe` block carries a `// SAFETY:` comment.
- Only `nurt-ffi` exports C symbols; their declarations are in
  `crates/nurt-ffi/include/nurt.h`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
