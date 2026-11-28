<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           CHANGELOG.md -->
<!-- Responsibility:   Documents Changelog. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Changelog

All notable changes to Danaco Nurt are documented in this file.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
and the project adheres to [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).
The language, the syntax tree, the command catalog and the C interface carry
their own version numbers, described in the specification.

## [Unreleased]

## [0.1.2] - 2026-11-28

### Fixed

- Search and replace with a regular expression no longer take time quadratic in
  the number of matches on one line: the bounds of the line of a match are
  derived from the match instead of being searched again from the start of the
  line. 20 000 matches on a line of 60 000 bytes are replaced in about 0.1 s
  instead of 2.5 s.
- macOS: the journal is active for the paths of the data volume. The mount point
  of the data volume (`/System/Volumes/Data`) does not prefix the paths of the
  firmlinked directories, so no directory of the volume qualified as a journal
  anchor and every operation ran without a journal; the volume now reports the
  mount point that holds its paths.
- macOS: `max_memory` no longer makes every start of a program fail with
  `EINVAL`. The limit of the address space (`RLIMIT_AS`) is applied on Linux
  only; on macOS the resident memory is measured, as documented.
- macOS: the member list of a group is read without assuming the alignment of
  `gr_mem`, which removes undefined behavior when the groups of an account are
  listed.
- macOS: suspending, resuming and stopping a program whose process group has
  ended no longer fail with `EPERM` ("cannot signal the program"); macOS
  answers `EPERM` instead of `ESRCH` for such a group.
- Windows targets can be built with a clang outside `/usr/bin`:
  `NURT_MSVC_CLANG` names the clang of the MSVC wrappers of the release build.
- Variant C: a program with a sub-pipeline in the condition of `when` or
  `while`, in the source of `each` or in the options of a loop is printed with
  the names of its arguments; before, printing failed with "variant C has no
  positional arguments".
- Variant C: a `bool` parameter written as a bare word (`sysctl.set "key" "1"
  persistent`, `remove preview`) is printed as `persistent=true`; before, the
  word was counted as a positional argument and printing failed. This applies
  to the format request of the engine and to the examples of the help.
- The guide "Administration" showed a `steps` example that the language
  rejects (`assert` at the start of a pipeline); the example now pipes `run`
  into `assert exit_code: 0`.
- `nurt check` (and the `check` request of the engine) no longer rejects a
  valid program whose argument is a duration, a size, a regular expression, an
  instant with a zone or a map of literals, for example `sleep 500ms`,
  `parse /(?<key>\w+)/` or `journal.limits age: 5m`. The static check passed
  such literals to the command as expressions, so the command reported a
  missing or mistyped value that the run accepted.
- `nurt check` no longer rejects a command whose argument is computed before
  the stage runs: a parameter of a function (`define rotate(keep: int) { find
  "/var/log" |> skip @keep }`), a named result or a call such as
  `undo file: "/home/user/a.go" since: ago(1h)`. The static check of such a
  stage leaves the verdict on its arguments to the run.
- Documentation: 24 examples that the language rejects are corrected, among
  them `git.log` (`since: ago(30d)`), `test.failed` and `test.show` (the run
  identifier is a text), the enum lists of `analyze.check`, `git.review`,
  `code.profile` and `find` (values without quotes), `run` without `cwd`,
  `write` without `mode`, `undo paths:` without the operation, and `sort` with
  `order: desc`.
- The fields that `program.find`, `program.describe`, `program.search` and
  `program.catalog` add to their records (`version_source`, `alias_of`,
  `guide_sections`, `guide_etag`, `requested`, `alias_kind`, `query` and the
  others of the command pages) are declared as optional fields by the record
  kinds `program_card`, `program_location` and `program_category`, so `select`,
  `filter`, `sort` and `evaluate` accept them; the examples of the command
  pages no longer fail with `E_UNKNOWN_FIELD`. The same holds for the fields
  `baseline`, `regressions`, `fixed` and `slowest` of `test_run`, `similarity`
  of `change` and `return_value` of `effect`, which `test.run`, `git.diff` and
  `windows.cim` set on their records. The values and the order of the fields in
  the result are unchanged.
- A start of `nurt` no longer ends with `E_IN_USE` ("cannot assemble the
  journal") while another process keeps the lock of a journal entry that is
  due to expire, for example while it reclaims the same entry. The expiry pass
  of the start leaves such an entry to its holder or to the next pass.
  `journal.purge`, `journal.limits` and the reclaim for space still report
  `E_IN_USE`.

### Changed

- The archive `nurt-lib` of the musl targets is about four times smaller: the
  static library `libnurt.a` no longer holds the debug information and the LLVM
  bitcode that rustc embeds for link-time optimization. The library shrinks from
  811 MB to 298 MB and the archive of `x86_64-unknown-linux-musl` from 235 MB to
  63 MB. The exported functions are unchanged and a C program still links with
  the library.
- Unix: the window in which the account could swap a directory of its own
  journal for a link is closed in the descriptor mode of the journal of an
  account that an elevated process writes (macOS, `doas` and `sudo` there).
  Reading, listing, moving, linking, locking and removing now name the entry
  relative to the descriptor of its checked directory (`openat` with
  `O_NOFOLLOW`, `fstatat`, `renameat`, `linkat`, `unlinkat`) instead of
  walking the path again after the check, as creating files already did. The
  change of metadata (`set_meta`, `restore_meta`: owner, mode, times, extended
  attributes), `truncate`, `create_node` and in-place writes use the same
  descriptor, so an elevated write, `undo` and `redo` of a file below the state
  directory of the account keep its mode and extended attributes. A call
  without such a form (`replace_with_backup`, `clone_file` between two paths of
  the account, alternate streams) is refused instead of falling back to a path;
  the journal copies the bytes when a clone is refused and never asks for
  `replace_with_backup` on Unix.

### Known issues

- macOS: tests of the change tracking of `nurt-proc` (`track::tests`, for
  example `events::changes_below_ignored_names_are_counted` and
  `bounds::a_scope_within_the_cap_is_tracked_and_the_note_names_the_directories`)
  and the protocol test `client_compat_d2_journal_inactive_rules` of
  `nurt-service` fail intermittently on the macOS runners of the continuous
  integration: FSEvents reports fewer changes than were made, and the service
  test exceeds its time limit. The cause is not yet established. No macOS
  archive is published for this release.

## [0.1.1] - 2026-11-27

### Changed

- The preview of `sql` lists the effects of `load_extension` and of the pragmas
  `temp_store_directory` and `data_store_directory` as irreversible entries.

### Added

- `create` makes named pipes and device nodes on Linux and macOS: `type: fifo
  mode: "0660"` and `type: device kind: char|block major: <int> minor: <int>`
  (a device node needs `privilege: elevated`). On Windows both fail with
  `E_OS_ONLY`. Both are journaled as created files, and `undo` removes them.
  `info` and `list` report the types `fifo` and `device`; a device node has
  the fields `major` and `minor`.
- Administration of the firewall, the network and the clock (group `administration`,
  language brief section 10): `firewall.rules`, `firewall.allow`,
  `firewall.deny` and `firewall.remove` on nftables, firewalld and ufw (Linux),
  pf (macOS) and the Windows Defender Firewall; `network.route_add`,
  `network.route_remove` and `network.dns_set` (systemd-resolved,
  `/etc/resolv.conf`, `networksetup`, the Windows interface DNS);
  `clock.timezone_set`, `clock.ntp` and `clock.set`. Each takes
  `privilege: elevated` and reports one `state` change of the target
  `firewall:<rule>`, `route:<destination>`, `dns:<interface>`, `clock:timezone`
  or `clock:ntp` with the value before and after; `undo` runs the inverse
  derived from those values (`firewall.remove` for `firewall.allow`, the
  previous zone, the previous servers, and so on). `clock.set` is irreversible.
  A change that the system already holds is reported as `unchanged`.
- `windows.cim` calls methods: `invoke: "<method>"` with `where` (the selected
  instances) or without it (a static method of the class) and `arguments`
  (text, int and bool values). A method with a documented opposite
  (`StartService` and `StopService`, `Enable` and `Disable`, `Pause` and
  `Resume`) is undone by the opposite call; any other call is irreversible.
- Administration commands for accounts, groups, mounts, the kernel, power and
  disks: `accounts.create`, `accounts.modify`, `accounts.remove` (`keep_home`),
  `accounts.password` (the value is a secret and goes to standard input),
  `groups.create`, `groups.remove`, `groups.add_member`,
  `groups.remove_member`, `mount` and `unmount` (`persistent` edits
  `/etc/fstab` through the journal), `sysctl.set` (`persistent` writes
  `/etc/sysctl.d/90-nurt.conf`), `modules.load`, `modules.unload`,
  `power.restart`, `power.shutdown` (`at`, `delay`, `message`),
  `power.cancel`, `disks.partition` and `disks.format`. Every command reads
  the state it changes, reports `unchanged` when nothing is to do, records the
  state before and after, and accepts `privilege`. `undo` runs the inverse
  command; a password, an immediate restart or shutdown, a cancelled schedule,
  a partition change and a format are irreversible, and the preview of a disk
  change lists the partitions, file systems, mount points and the size of the
  data that is lost.

- Translation of bash into the constructs of the language (language brief,
  section 11): `if`, `elif`, `else`, `case`, `[ ]`, `[[ ]]` and `test` become
  `when`; `for`, `while` and `until` become `each`, `while` and `repeat until`;
  functions become `define` and their calls calls; `$1`, `$2`, `$@`, `shift`
  and `getopts` of a script become `params`; `A && B && C` with changes becomes
  `steps` (in a single call, `E_ONE_EFFECT` carries the `steps` program as its
  first candidate, also for PowerShell); `test ... || CMD` becomes `when not`.
  The translation of the administration programs `mkfifo`, `mknod`, `useradd`,
  `usermod`, `userdel`, `groupadd`, `groupdel`, `gpasswd`, `passwd`, `mount`,
  `umount`, `sysctl -w`, `modprobe`, `rmmod`, `ufw`, `firewall-cmd`, `nft`,
  `iptables`, `ip route`, `resolvectl dns`, `timedatectl`, `date -s`,
  `shutdown`, `reboot`, `poweroff`, `parted`, `mkfs.*` and `sqlite3` writes the
  native commands (rules `B41` to `B44`). `nurt migrate` translates these
  structures, declares the `params` block first and remembers the functions of
  a script. Only `eval`, subshells, groups, the arithmetic `for`, `select`,
  interactive input and here-documents other than `cat > file` stay without a
  translation (`docs/guides/bash-migration.md`).

### Changed

- `sql` has no refusals. A statement that changes anything opens the database
  for writing and journals the file and its `-wal`, `-shm` and `-journal` files
  first; `write` is optional, and `write: false` keeps the call read-only.
  `BEGIN`, `COMMIT` and `END` merge with the transaction of the call,
  `ROLLBACK` ends it with `E_ROLLED_BACK`, `ATTACH` and `DETACH` are allowed
  with every attached file journaled before the first change, `VACUUM INTO` is
  journaled as a created file, `VACUUM` runs after the other statements,
  `load_extension` is allowed (its effects are reported as unknown and
  irreversible) and the pragmas `temp_store_directory` and
  `data_store_directory` are allowed (reported as effects on the whole Nurt
  process, which `undo` does not reset). The file of `ATTACH` and `VACUUM INTO`
  is resolved like every path argument (`E_PATH_RELATIVE` without `base`),
  `file:` URIs are read to their path, the call turns the defensive mode of
  SQLite off when it writes, a preview lists the files of `ATTACH` and
  `VACUUM INTO`, the other database files appear as changes of the report, and
  the kind of each statement is read from its own text.
- Control constructs (language brief, sections 1 to 6 and 8): `when` with
  `otherwise when` and `otherwise`; the loops `each`, `repeat until` and
  `while` with `break`, `continue`, `parallel`, `atomic: false`, `limit` and
  the configuration keys `loop.max_iterations` and `loop.max_time`; `steps`
  for several changes in one call; functions with `define` and the key
  `function.max_depth`; script parameters with `params`, the command
  `nurt run <file> name: value` with `--help`, and the member `params` of the
  request. Each construct is one journal entry, all or nothing, previewed
  without a change and checked before any effect; the report lists its
  `parts`. A parameter of type `bool` written without a value is `true`.
- Syntax tree version 1.1 (`nurt-ast/1` and `nurt-bound/1`): the statements
  `when`, `each`, `repeat`, `while`, `break`, `continue`, `steps`, `define`
  and `params` and the pipeline head `call`. Documents of version 1.0 are read
  unchanged. The grammar is in `spec/grammar.md`.
- Changes of the API of `nurt-syntax`: `lex::Checkpoint` is no longer `Copy`
  (it keeps the open blocks of the lexer); `MutationTx` gains the methods
  `allow_incremental`, `savepoint`, `release_savepoint` and
  `rollback_to_savepoint`, with defaults that keep the old behaviour; `Request`
  gains the member `params`; `Report` gains `parts` and `parts_total`.
- `MutationTx::allow_incremental`: a transaction of the journal may preserve
  further targets after its first change, so that a construct is one entry.

### Added

- Privilege elevation: the call parameters `privilege` (`account`, `elevated`,
  `auto`) and `as_user`, written on a stage, given in the request or on the
  command line (`--privilege`, `--as-user`); the elevation backends `self`,
  `sudo`, `doas` and `run0` (the backend `broker` is reported as
  `not_installed` until the service `nurt-elevation`); the configuration
  section `[privilege]`; elevated entries in the journal of the account, with
  the same numbering, whose copies the account can access, written by the
  elevated process with the rights of the account on Linux and macOS;
  `undo N` and `redo N` of an elevated entry through the same backend, while
  `undo` and `redo` without a number that would restore one end with
  `E_PRIVILEGE` and the next step `undo N privilege: elevated`; and undo of
  elevated changes and of changes to packages and services by inverse
  operations.

### Changed

- `sudo`, `doas` and `run0` in bash lines are translated into `privilege:
  elevated` or `as_user` instead of being rejected; `sudo -i` and `su` end with
  `E_AMBIGUOUS`.
- `E_PRIVILEGE` means that no elevation backend ran the operation or that the
  system denied the elevation; it carries `backend` and `next_steps`.

### Security

- The Git library `gix` and its sub-crates are updated to the 0.91 generation,
  which no longer depends on `heapless` 0.8 (nine advisories about unsound code,
  RUSTSEC-2026-0574 to RUSTSEC-2026-0582).
- The unused `netstat2` dependency and the AVIF encoder of the `image` crate
  (`ravif`, `rav1e`) are removed, which removes the unmaintained `paste` macro
  crate (RUSTSEC-2024-0436). Reading of AVIF files was never enabled.
- OpenPGP signatures of commits and tags are made and checked by GnuPG instead
  of the `pgp` crate, which removes `rsa` 0.9 (RUSTSEC-2023-0071) and
  `crypto-bigint` 0.5 (RUSTSEC-2026-0387) from the dependency graph. The
  program named by `gpg.openpgp.program` or `gpg.program` (`gpg` by default)
  runs through the program service; `git.signing.key_file` is imported into a
  temporary GnuPG home directory. A missing or failing program refuses the
  signature with `E_GIT_SIGN` (problems `program_unavailable` and
  `program_failed`); verification without the program reports the status
  `unsupported`. SSH signatures are unchanged and still made in the process.
- The text extraction of `pdf-extract` 0.12.1 is a crate of the workspace
  (`crates/nurt-pdf-extract`, MIT license) built on `lopdf` 0.47, which no
  longer needs the unmaintained `ttf-parser` crate (RUSTSEC-2026-0192).
- The `chrono` feature is off for `croner`, `lopdf`, `rmcp` (updated to 3.12)
  and, through it, `schemars`. `chrono` stays in the dependency graph only
  through `macos-unifiedlogs` and `russh-util` (RUSTSEC-2026-0543), for which
  `deny.toml` records the reason.
- The lock files of the fuzz targets are regenerated.

### Added

- The command `next` and the field `continuation` of every shortened response:
  a response that leaves lines, records or the end of long
  lines out carries `continuation` (`id`, `unit`, `shown`, `remaining`,
  `cut_lines`) and its text ends with `more: next`; `next`, `next c3`,
  `next 200`, `next c3 200`, `next all` and `next full` return the next part
  from the output file of `run`, from the kept result of a reading command or
  from the cursor of `logs`, without running the program again. The keys
  `continuation.max_age` (`1h`), `continuation.max_count` (`50`) and
  `continuation.max_size` (`256mib`) bound the kept results; an expired
  continuation ends with the new error `E_CONTINUATION_EXPIRED`. The record
  `exec` no longer has the field `read_hint`; `continuation` replaces it.
  The default sizes of the responses are unchanged. A continuation belongs to
  the session that made it and is allocated under a lock file, so that several
  processes can share one state directory. When one program has several stages
  that cut their output, the first one in program order is continued.

### Fixed

- The bindings for Python, Go, Node, Java and .NET read the answers of the
  engine in the shape it returns: `describe` returns its text from the field
  `text` of the description, `catalog` sends the chosen group or command as
  `what`, `correct` returns the program that ran, and a language error of
  `describe`, `help`, `print` or `schema` ends in a language error of the
  binding, not in an empty text or a protocol error. Each binding has tests
  that run against the engine when `NURT_TEST_EXECUTABLE` names it.
- The request types of the Go, .NET and Java bindings carry `params`,
  `privilege` and `as_user`, and the test of the Python binding expects the
  tree format 1.1 that the engine reports.
- The tests of the journal that undo operations create their directories on
  the disk of the build and no longer end with `E_JOURNAL_NOOP` when
  `TMPDIR` is a tmpfs, where the journal of a volume is inactive.
- The code review of 0.1.0 corrected the defects found in every crate of the
  workspace, the bindings, the packaging and the documentation; the main ones:
  - requests of `nurt serve` resolve relative paths against `options.base`;
  - the journal of a volume shared by several accounts gives each operation
    a number of its own, and history shows the entries of the account only;
  - `check` reports the types of fields, `in`, arithmetic and the arguments of
    functions;
  - a corrected name of a changing command is offered as a candidate
    (`E_AMBIGUOUS`) and is not applied by itself;
  - paths that are not valid Unicode keep their native bytes beside their
    display form in reports and journal records (the records `git_status` and
    `git_conflict` and the hunks of `git.show` included, and `query` reads an
    entry of such a name), and report values stay on one line in the text
    views;
  - `registry:` paths are accepted by the file commands on Windows;
  - `nurt fmt` keeps the comments of the program;
  - `nurt schema tool` prints the tool definition for the platforms of the
    integrators, as documented;
  - `service install` writes the user unit where the service manager reads it.
- Defects found by running the whole test suite on Windows and on real Linux
  volumes:
  - a file of the lower layer of an overlay belongs to the volume of the
    overlay, so that the journal preserves it on the right volume;
  - an entry preserved by copy keeps its permissions and times;
  - `clone` on Windows writes the configuration as Git for Windows does (line
    ends, `core.symlinks` from the system configuration) and stores a local
    URL with forward slashes;
  - credential helpers, `gpg.ssh.allowedSignersFile` and GnuPG of Git for
    Windows (an MSYS program, which gets `/c/…` paths) work on Windows;
  - `push` from a second worktree on Windows finds the common directory of the
    repository;
  - the change report of a command that writes through the journal names the
    kind of every changed entry on Windows as well.
  - a cancelled `sql` change stops also when the cancellation comes between
    two statements of its transaction; before, such a cancellation was lost and
    a statement without end kept running.
  - concurrent refreshes of one symbol index on Windows keep the rule "the
    last rename wins"; before, a refresh could fail to store the index while
    another process replaced the same file.
  - change detection by scan finds a file rewritten with the same size within
    the timestamp granularity of the file system (ZFS, ext4 without multigrain
    timestamps, FAT 2 s, HFS+ 1 s) and restores it on undo: a file modified
    less than 3 s before the start of the first scan gets its content hash
    computed in both scans, as the racy-entry rule of the Git index does. Only
    files up to `run.preimage_copy_max` (64 MiB when unlimited) are hashed for
    this; a larger file is still compared by metadata, so a same-size rewrite
    of it within the timestamp granularity stays undetected.
  - an edit, convert or compress after a read, and an `if_match` with the etag
    of a read, refuse with `E_STALE` a file that another program rewrote with
    the same size within the timestamp granularity of the file system: the
    etag (`etag`) of a file whose content was read carries the SHA-256 of the
    bytes returned, and the journal compares it with the current content. Etags
    made from metadata only (`list`, `find`, `info`, git apply) are checked by
    metadata as before. The etag is opaque, and its form depends on the
    operation that made it (with or without the hash, 72 characters longer).
    Etags without the hash are still accepted.

### Changed

- The full-scale undo test of one million removed, moved, replaced or
  extracted files allows an undo of up to six times the duration of the
  operation (three times before); the undo of one million removed files takes
  5.3 times the removal.

- `run.spool_max` defaults to `1gib` instead of `unlimited`: a program that
  writes without end no longer fills the disk; output beyond the limit is
  counted, hashed and reported with the `spool_limit` note.
- Journal: an operation with many small files (a `replace` over thousands of
  files and its `undo`) is several times faster and its entry is proportional to
  the changed content. In a `preserve` call with at least 64 targets the
  previous content of small files (up to 4096 bytes) is kept in the manifest
  instead of a block and a metadata file per file; a transaction with at least
  64 targets flushes its new files and directories with one call before the
  commit instead of one flush per file; the volume of a directory is described
  once per directory. Manifests of operations with at least 256 targets have
  format version 2 (packed target records, same digest rule); journals written
  by 0.1.0 are read unchanged, and a reader of version 1 rejects version 2.
  The items of an undo or redo entry are packed with the targets, so that the
  entry of an undo of thousands of files is about seven times smaller.
- `run`: the response is short by default. The text of all streams together
  fits in `max_bytes` (new parameter, key `output.max_bytes`, 4096 bytes, `0`
  is no limit), filled with the errors found, the end of standard error, the
  end of standard output and the beginning; a line longer than 400 characters
  is cut and ends with `… [+N characters]`. `dedupe` is on by default and also
  collapses lines that differ only in numbers, versions or the names after
  `Compiling` and similar words. After exit code 0, `show: auto` returns the
  last 5 lines and the field `digest` instead of the first 20 and last 40
  lines. After a failure with more than 20 lines of output, the output of
  `cargo`/`rustc` and `pytest` is recognised: the field `diagnostics` holds
  the numbers of errors and warnings and the first 10 distinct errors with
  file, line, column, code and message, and the text keeps the last 10 lines.
  A cut response carries `read_hint`, a sentence with `read` and `search`
  commands that read the rest from the output file. Pass `dedupe: false`,
  `max_bytes: 0` or `head` and `tail` to get the earlier behaviour.
- Linux: the executable and the library link liblzma statically; the only
  shared libraries are `libc.so.6`, `libm.so.6`, `libgcc_s.so.1` and the
  dynamic loader, so the program runs on a system without `liblzma.so.5`. The
  `.deb` package no longer depends on `liblzma5`.
- `nurt-service` compiles for the target `x86_64-unknown-linux-musl`
  (`sched_param` and `pthread_t` differ from glibc), and the release procedure
  builds the static musl archive `nurt-<version>-x86_64-unknown-linux-musl`.
- The start banner of an interactive session shows the mark of Danaco Nurt in
  half blocks (ASCII on terminals without UTF-8) in its two colours, instead of
  the name in letters.
- The colours of the mark in the terminal follow the production identity:
  silver `#DDE1E6` (palette 254) and gold `#C9A15A` (palette 179).
- The Windows executable and the documentation site use the new icon and
  favicon.

### Known limitations

- No macOS archive is built; the code for macOS passes `cargo check` for
  `x86_64-apple-darwin` and `aarch64-apple-darwin`, but it is not run.

## [0.1.0] - 2026-11-22

First release.

### Added

- Cargo workspace with the crates of the engine, the command line program, the
  C interface and the test support libraries.
- Contract crate `nurt-contracts` with the shared interfaces and data formats.
- Command catalog and specification documents.
- Language: lexer, parser with three syntax variants, correction of programs,
  binding against the command catalog, printing, help and descriptions in
  several layers, migration of Bash and PowerShell programs and a language
  server.
- Engine: planning, checking and execution of programs, structured reports of
  effects in text, JSON and Markdown form, pagination, sessions, candidates for
  ambiguous programs and cancellation.
- Undo journal: every change to files, provider values and repository state
  that the program can preserve is recorded and can be reviewed, undone and
  redone; recovery after a crash; retention limits; history queries. Effects
  that cannot be preserved are recorded as irreversible and reported
  separately.
- Command groups: files, text, archives, notebooks, structured documents,
  source code analysis with a language server client, processes, tests and
  analysis, version control implemented in Rust, system, network, scheduling,
  services and programs.
- Command line program with the subcommands `exec`, `check`, `script`, `repl`,
  `serve`, `journal`, `mcp`, `lsp`, `migrate`, `service` and `ext`; the Model
  Context Protocol server; the C interface (`libnurt`, `nurt.h`); bindings for
  several languages; a conformance suite and benchmarks.
- Packaging for Linux, macOS and Windows, release automation and integrations
  for editors and containers.
- License, notice, contribution guide, security policy and code of conduct.
- OpenPGP signature of `checksums.txt` and of the release tag, the Authenticode
  signature of `nurt.exe`, and the public key and certificate in
  `packaging/release`; see `SECURITY.md` for the verification.
- Dependency policy for `cargo-deny`, continuous integration on Linux, macOS
  and Windows, and a repository hygiene check.
- The banner of an interactive session draws the name of the product with
  characters in silver and gold; the prompt is gold with a silver indicator and
  the cursor is a blinking underscore while a line is typed. The highlighting of
  a typed line uses gold for commands, silver for the pipe and cyan for strings.
  A terminal that reports 24-bit colour shows exactly the colours of the mark,
  any other terminal the nearest colours of the 256-colour palette.
- The visual identity in `branding`: the mark of two chevrons turned by 180
  degrees, the application icons, the favicon of the documentation site, the
  lockups with the name drawn in letters of its own, a render of the mark and the identity guidelines.
- `nurt.exe` carries version information and a manifest with the execution
  level `asInvoker`, long path support and the UTF-8 code page, and the icon
  of the product (`crates/nurt-cli/windows/nurt.ico`).
- `install.ps1` installs for the current user by default and for all users into
  `%ProgramFiles%\Nurt` with `NURT_SCOPE=machine` from a terminal that runs as
  administrator; it installs the documentation next to the executable.
- The archives hold the documentation site, the completion scripts for five
  shells and, on Linux, the manual pages; `packaging/release/pack.py` assembles
  them reproducibly.
- The shared library `libnurt.so.1` records the SONAME `libnurt.so.1`.
- Every commit of the `main` branch is signed with a dedicated OpenPGP key; see
  `SECURITY.md`.

### Known limitations

- The journal is active on volumes that keep a journal; changes on volumes held
  in memory, such as `tmpfs`, are reported as irreversible.
- This release has archives for Linux (glibc, x86-64) and Windows (MSVC,
  x86-64). The Windows archive and `install.ps1` were run on Windows 11 only,
  and the registry provider is available only on Windows. The code for macOS
  exists, but no macOS archive is built. `install.sh` takes the glibc archive of
  Linux because the release has no musl archive.
- Native extensions are not available; extensions are WebAssembly modules.
- The subcommand `journal` offers only `salvage`; the other journal operations
  are commands of the language.
- The recovery time of 5 seconds for a journal of one million entries is not
  confirmed: the last measurement was 6.2 seconds, before later corrections.
- The certificate that signs `nurt.exe` is issued by Danaco Holding Group
  Sp. z o.o. itself, not by a public certification authority. Windows reports
  the publisher as unverified until a certificate of a public authority replaces
  it.

[Unreleased]: https://github.com/DanacoOfficial/danaco-nurt/compare/v0.1.2...main
[0.1.2]: https://github.com/DanacoOfficial/danaco-nurt/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/DanacoOfficial/danaco-nurt/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/DanacoOfficial/danaco-nurt/releases/tag/v0.1.0

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
