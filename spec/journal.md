<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/journal.md -->
<!-- Responsibility:   Documents Journal and file system contract. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Journal and file system contract

This document specifies the contract of the operation journal (module
`nurt_contracts::journal`) and of the file system interface (module
`nurt_contracts::fs`). The Rust documentation of both modules is normative for
signatures; this document is normative for the protocol, the guarantees and the
reference semantics of test implementations.

## 1. Principles

1. Coverage. Every change of files, provider values or repository state
   made by a command goes through a mutation transaction and is recorded in
   the journal. The only exceptions are listed in the coverage table
   (`tables/journal-coverage.toml`, section 13) and are always reported with an
   explicit irreversible marker.
2. Order. The previous content of every target is preserved before
   anything is changed. At every instant of a crash the previous content exists
   in at least one place.
3. Space priority. Free space for the user has priority over the journal.
   The journal never causes a user operation to fail for lack of space,
   eligibility or limits: when admission declines to keep the previous
   content, the operation proceeds unpreserved and the target is marked
   irreversible. A failure of the journal itself while it moves a removed
   target into the journal (any error other than a cross-device rename, which
   falls back to a copy, and a lack of space for that copy) fails the
   operation with the target still in place; a removal never deletes data
   after the journal failed to keep it.
4. Explicitness. Missing coverage is always visible in the result. Results
   never state until when an operation can be undone.

## 2. Concepts

The term change is defined in the [glossary](glossary.md#change).

| Term | Meaning |
|---|---|
| Operation | One effect stage of one program; one number (`OpId`), one entry per account, one entry part per volume. |
| Target | One address changed by the operation: a file system entry or a provider key (`TargetPath`). A directory tree removed or created as a whole is one target. |
| Preserve mode | How the previous state of a target is kept (`PreserveMode`, section 5). |
| Entry state | Persistent state of an entry (`EntryState`, section 6). |
| Operation state | State shown to the language (`OpState`): `available`, `partial`, `expired`, `undone`. |

Operation numbers form one sequence per account, starting at 1. Reads and
previews do not consume numbers. Numbers are unique but not necessarily dense.

The journal directory of a volume is shared by every account that works on the
volume. An entry and its tombstone record the account that made it (the state
directory of the account). `history`, `undo` and `redo` see only the entries of
the calling account; entries written before the account was recorded have no
such record and are seen by every account. Recovery completes an entry of
another account but leaves its index row, and its marker, to that account.

Accounts with separate state directories number their operations locally. The
exclusive creation of the marker `pending/<N>` decides who owns a number. A
transaction that loses that claim (the marker or an entry of the number
already exists) takes a new number and tries again, at most eight times, while
it has no part open yet; only then is the journal given up for the operation
and the target marked irreversible with `journal_error`.

## 3. Transaction protocol

The executor opens a transaction with `JournalStore::begin`, passes it to the
effect stage as `&mut dyn MutationTx`, and commits or rolls it back after the
stage returns. The stage never commits.

1. Declare and preserve. The stage declares every target with
   `MutationTx::preserve`, in one call or in batches. For each target the
   journal chooses a preserve mode, consulting `Admission`; it assigns the
   operation number at the first call, writes the entry in the `prepared`
   state and preserves the previous state of all declared targets before
   returning. `preserve` is not allowed after the first change. In a call that
   declares at least 64 targets, the previous content of a small file (at most
   4096 bytes, a single name, no extended attributes, ACL or streams) is kept in
   the manifest of the entry instead of a block per file (store `copy`, section
   5); the manifest is written again, durably, before `preserve` returns.
2. Apply. The stage changes targets with `MutationTx::apply`. New content
   is written to a staged file in the directory of the target, either filled
   by the journal (`Mutation::WriteBytes`) or by the stage through the handle
   returned by `MutationTx::stage` (`Mutation::CommitStaged`), and switched in
   atomically. The metadata of the previous file is transferred by the journal.
   In a transaction with at least 64 targets the journal does not flush each
   new file and directory right after the change; it flushes all of them with
   one `FsWrite::sync_paths` call before any other kind of change and before
   the commit. This is safe because the previous state of every target is
   durable before its change and the entry is `prepared` until the commit: a
   crash before the flush is rolled back by recovery, which restores every
   target that does not hold the recorded previous state, whatever part of the
   new content reached the disk (section 4). Nothing is committed before the
   flush returned; a failed flush rolls the transaction back.
3. Expected state. `TargetSpec::expect` (`unchecked`, `absent`, an entity
   tag or a content hash) is checked atomically with the change. A mismatch
   fails with `E_STALE` and rolls the whole transaction back. An entity tag
   is `size-modified-volume-low-high` in hexadecimal, optionally followed by
   `-sha256:<digest>`; tags without the hash are accepted. The tag is opaque:
   its form depends on how it was made. File modification times have a
   granularity of up to 2 s (FAT) and, on ZFS and ext4 without multigrain
   timestamps, a few milliseconds, so a same-size rewrite within that
   granularity leaves size and time unchanged. A tag with a hash is therefore
   also compared with the hash of the current content. A read stage that read
   the content (`read`, `lines`, `edit`, `convert`, `compress`, notebooks) puts
   the hash of the bytes it returned into the tag, so an edit, convert or
   compress after such a read refuses a foreign same-size rewrite with
   `E_STALE`. Tags made from metadata only (`list`, `find`, `info`, the git
   apply, `write`/`copy`/`remove` of paths they did not read) are checked by
   metadata, as before; a same-size rewrite within the timestamp granularity is
   not detected for them. Clocks: the check relies on the local clock only in
   change tracking (see below); NFS attribute caches and a file system clock
   that differs from the local clock by more than 3 s are not covered.
4. External writers. Targets written by a library (embedded database
   engine, repository index writer) or by a launched program are declared with
   the action `external_write`; the journal preserves their full previous
   content by clone or copy. After the writer finishes, the stage reports the
   changes with `MutationTx::observe_external`. Declared external targets not
   reported are dropped from the entry at commit and their copies released.
   Changes detected by scanning without a declared target are recorded as
   irreversible (`previous_content_not_kept`) unless a copy exists.
5. Effects outside the file system. Network requests, stopped processes,
   changed services and installed packages are recorded with
   `MutationTx::mark_irreversible`. A transaction may consist of markers only.
6. Exempt writes. Writes that are outside the journal by design and never
   undone (content-addressed objects added to a repository object store) are
   recorded with `MutationTx::record_exempt`.
7. Commit or roll back. `MutationTx::commit` makes the changes durable
   and the entry `committed`, and returns an `OpHandle` for the report. A
   transaction that changed nothing and marked nothing returns a handle with
   `op: null`, and no entry exists. `MutationTx::rollback` restores every
   target changed so far, in reverse order. Dropping a transaction without
   commit rolls it back.
8. Checks after the change. A stage may check the result before the
   executor commits (validation, diagnostics); on failure the executor rolls
   back with reason `validate_failed` and the result has status `rolled_back`.
   A check after commit (`verify`) uses `JournalStore::rollback_committed`,
   which creates an entry of kind `undo` and marks the source entry undone
   with cause `verify_failed`. Failures detected before commit create no new
   entry.

After a method of `MutationTx` fails, the transaction is rolled back and every
further call fails with `E_ROLLED_BACK`. Misuse of the protocol (preserve after
a change, an undeclared target, a mutation that does not match the declared
action) is a defect of the caller: test builds panic, production builds fail
with `E_STAGE_FAILED` and roll back.

### 3.1 Common operations

| Operation | Declaration (`TargetAction`, `Expect`) | Mutation |
|---|---|---|
| Replace the content of a file | `modify`, entity tag of the read stage | `write_bytes`, or `stage` and `commit_staged` for streamed content |
| Create a file | `create`, `absent` | `write_bytes` or `commit_staged` |
| Append | `append` | `append_bytes` |
| Write at an offset, change the length | `modify` / `truncate` | `write_at` / `set_length` |
| Remove a file or a directory tree | `delete` | `remove` |
| Rename or move, also across volumes | `rename { to, replace }` | `rename` |
| Replace an entry by one of another kind | `replace` | `remove`, then a creating mutation |
| Create a directory or link | `create` / `link` | `create_dir`, `create_link`, `create_hard_link` |
| Change metadata | `attributes` | `set_meta` |
| Set or remove a provider value | `modify` / `delete` on a provider target | `provider_set` / `provider_remove` |
| Copy a provider key with its subtree, or a value | `create`, `absent`, on the destination | `provider_copy` from the source key |
| Undo a change of a provider target | `modify` on the changed target | `provider_restore` with the kept snapshot |
| Library writes the file itself | `external_write` | `observe_external` after the write |
| New tree populated by the operation | `create_tree` | writes inside are not declared |

Several files changed by one command are one transaction and one operation,
reported as a bulk operation and undone as a whole.

### 3.2 Command-level operations

`FileOp` pairs the declaration and the mutation of each row of the table
above (`write`, `create`, `append`, `write_at`, `set_length`, `remove`,
`rename`, `copy`, `copy_over`, `create_dir`, `create_link`,
`create_hard_link`, `set_meta`), with `expect` and `size_hint` as optional
settings. `apply_ops(tx, ops)` performs one protocol pass: it declares and
preserves every target with a single `preserve` call and then applies the
mutations in order. A command therefore never chooses a preserve mode, the
order of the protocol or the strategy across volumes; the journal does.

Streamed content and library writes keep the explicit form: declare all
targets with `preserve`, then `stage` and `commit_staged` per file, or
`observe_external` after the library wrote the file. Applying a mutation to
an undeclared target is a protocol violation (section 3).

`AfterState.meta`, when present, holds the metadata the journal read after
the change, so that callers recording file status (a repository index) do
not repeat the `stat`.

## 4. Crash guarantees and recovery

| Moment of the crash | State after recovery |
|---|---|
| Before the entry is `prepared` | Nothing changed; no entry. |
| After `prepared`, during preservation or apply | The entry is rolled back: targets whose current hash equals the hash before are left alone; targets with the hash after, or unknown, are restored from the preserved data; staged files are removed. The entry becomes `undone` with cause `crash`. |
| During rollback (`rolling_back`) | The rollback is completed; it is idempotent. |
| After `committed` | The entry is complete and can be undone. |
| During reclaim (`expiring`) | The removal of the entry data is completed. |
| Manifest fails its integrity check | The entry becomes `corrupt`; its data is kept for manual salvage. |
| The rollback cannot be made: the previous content of a target cannot be located, or the directory that held a target no longer exists | Nothing is changed; the entry becomes `corrupt`, and its data is kept for manual salvage. The entry is not tried again. |

The manifest of an operation with at least 256 targets has format version 2:
the target records are packed (`targets_z`: the JSON array compressed with
Zstandard, Base64), and the digest covers the unpacked manifest. The items of
an undo or redo entry are packed in the same way (`items_z`), so that the entry
of an undo of many files stays proportional to their number with a small
constant. A journal written before this format is read unchanged (version 1); a reader of version 1
rejects version 2 and changes nothing.

`JournalStore::recover` runs before the first transaction of a process and
before the first operation after a reboot; it also expires overdue entries.
File systems without durable metadata after a flush have the journal disabled
by default.

## 5. Preserve modes

| Mode | Used for | Undo |
|---|---|---|
| `moved` | deletion; overwritten destination of a move | moves the entry back |
| `exchange` | overwrite by atomic exchange of names | moves the old content back |
| `replace_backup` | overwrite on Windows, backup name of the replace call | as `exchange` |
| `hardlink` | overwrite, hard link to the old content before replacement | as `exchange` |
| `clone` | block-sharing clone of the old content | writes the clone back |
| `copy` | byte copy of the old content, in a block of the entry or, for small files of an operation with many targets, in the manifest (`inline`, hex) | writes the copy back |
| `none` | previous content not kept; always with an irreversible marker | not possible |
| `created` | the target did not exist | moves the new entry into the undo entry |
| `meta` | metadata change only | restores the recorded metadata |
| `ref` | repository references and index, kept as files | restores the recorded files |
| `value` | provider value or key subtree | restores the snapshot, including absence |
| `append` | append, reference log; previous length and prefix hash | truncates |
| `rename` | rename within a volume | renames back |
| `external` | effect outside the file system; always with an irreversible marker | not possible |

One entry may combine several modes for its targets (for example a hard
reset: `ref`, `append`, overwrite modes, `moved` and `created`). The undo check
covers every target of the entry, whatever its mode.

The default overwrite strategy is `exchange` with fallback to `clone`, `copy`
and `none`. A file with more than one hard link is written in place so that all
names see the new content; its previous content is kept by clone or copy and
the report shows `hardlink_set_preserved`.

## 6. Entry states

| Entry state | Operation state | Meaning |
|---|---|---|
| `prepared` | not listed | Being preserved or applied; rolled back after a crash. |
| `committed` | `available` | Can be undone. |
| `rolling_back` | not listed | Rollback in progress. |
| `partial` | `partial` | Some items released for space or undone individually; the rest can be undone. |
| `undone` | `undone` | Undone completely; cause `undo`, `rolled_back`, `crash`, `verify_failed` or `validate_failed`. |
| `expiring` | `expired` | Being reclaimed. |
| `expired` | `expired` | Only a tombstone remains for `history`. |
| `corrupt` | `expired` | Failed its integrity check, or could not be rolled back after a crash. |

## 7. Admission and space priority

Before preserving a target the journal asks `Admission`. Admission keeps the
previous content only when all hold: the volume is eligible
(`Admission::eligibility`), the space service is alive, free space after the
write stays at or above reserve plus margin, and the size budget holds.
Otherwise it releases the oldest entries within the synchronous time budget;
when that is not enough it returns `skip` with the reason reported to the user
(`journal_inactive`, `no_space_for_copy`, `too_large_for_journal`,
`network_volume`, `removable_volume`). Admission never fails.

| Volume state | Condition | Behaviour |
|---|---|---|
| `normal` | free space at or above the soft threshold | data accepted; expiry by limits only |
| `soft` | between reserve and soft threshold | data accepted only when free space after the write stays at or above reserve plus margin; background reclaim |
| `hard` | below the reserve | no data accepted; immediate reclaim |
| `emergency` | below half the reserve | as `hard`, reclaim on all cores, callers do not wait |

The journal is inactive by default on network, removable and memory-backed
volumes and on volumes without durable metadata; it is active on overlay
volumes, where entries of lower layers are preserved by copy.

## 8. Undo and redo

| Selector | Semantics | Entry |
|---|---|---|
| `last` | newest available entry of the session that is not an undo | `undo`, scope `op` |
| `op` | one entry, completely | scope `op` |
| `to { after, all_sessions }` | all available entries newer than `after`, newest first, as one transaction; other sessions skipped and reported unless `all_sessions` | scope `to` |
| `file { path, since }` | newest entries that touched one file, for that file only; source entries become `partial` | scope `file` |
| `paths { op, paths }` | selected items of one entry; the entry becomes `partial` until all items are undone | scope `paths` |

The conflict check compares the current hash of every item with the hash
recorded after the operation (for preserved data, the hash before) for the
whole selection before the first write. With `on_conflict: report` (default)
differences fail with `E_JOURNAL_CONFLICT` and nothing changes; `force` undoes
anyway and keeps the current content in the undo entry; `include_later` also
undoes later entries touching the same items; `partial` undoes the remaining
items of a partial entry; `abort` changes nothing. The undo is itself an entry
with its own number; it keeps the current state of every restored item, so it
can be redone. A later ordinary operation on the same items invalidates `redo`
through the conflict check.

### 8.1 Elevated entries

An operation that runs with `privilege: elevated` is executed by a separate
process with the identity of the administrator. Its entries are in the journal
of the account that asked for it, like every other entry, and the account has
access to their copies. There is one journal and one counter: an elevated
entry takes the next number of the account, and `history`, `journal.status`,
`undo`, `redo` and `pin` treat it like any other entry. The entry names the
backend in its `action` (`elevated:<backend>`), and the report of the
operation states `elevated: true` and the `backend`.

The elevated process writes the entry, with the previous content, owner,
group, mode, access control list, extended attributes and the security context
of every target (sections 3 to 7), with the rights of the account, not of the
administrator: on Linux it can switch the file system user and group of the
calling thread (`fsuid`, `fsgid`) to those of the account for every call that
concerns the state directory or a journal directory of the account, so the
kernel checks each component of the path, a link included, with the
permissions of the account, and every file and directory is created owned by
the account. On macOS and the other Unix systems, which have no `fsuid`, the
process opens the journal directory without following links (it must be a
directory owned by the account), creates every file with `openat(O_CREAT |
O_EXCL | O_NOFOLLOW)` and every directory with `mkdirat` relative to that
descriptor, and gives each new entry to the account with `fchown` (`fchownat`
with `AT_SYMLINK_NOFOLLOW` for a link) and `fchmod` before anything is
written; on Windows the backend `self` runs as the same account and needs
none of this. In every case the process never follows a link with more rights than the account has,
and it never leaves a file owned by the administrator below the state
directory. A copy of a target into the journal is read with the rights of the
administrator and written with those of the account, block by block; a hard
link, a clone or a rename of a target into the journal is not used (the journal
copies instead). The account sends the user, the group and the state directory
in the request (`nurt-elevated/1`); the elevated process refuses a state
directory that is not an existing directory owned by that user, and a user that
differs from the one the backend recorded (`SUDO_UID`). The request of an
operation with `as_user` carries no owner: the entry is in the journal of that
account.

`undo` and `redo` of an entry that an elevated operation wrote restore the
targets with the rights of the administrator, through the same backend with the
same access rule, and report `elevation_reason: elevated_entry`; without a
backend they end with `E_PRIVILEGE` and `next_steps`. `undo` and `redo` without
a number first find the entry they would restore, as the journal does (the
newest available entry that is not an undo, or the newest available undo, of
the session); when an elevated operation wrote that entry, they end with
`E_PRIVILEGE` and the next step `undo N privilege: elevated` (`redo N …` for
`redo`), with the number of the entry. `undo to:` runs with the rights of the
account.

An `enable` or `disable` is undone only when the recorded start type is
`disabled` or `automatic` (release 0.1.1). A service whose recorded start type
is `manual`, `static` or `masked`, or a delayed start on Windows, makes the
entry irreversible: the report keeps the marker `service_changed` and its
`detail` gives the reason. A package that the repository no longer offers in
the recorded version is irreversible per package; the other packages of an
`upgrade` stay undoable.

Operations outside the file system are undone by inverse operations:

| Operation | Recorded state | Undo |
|---|---|---|
| `package.install` | whether the package was installed before and its version | `package.remove` when it was not installed before; reinstall of the recorded version when the install changed the version |
| `package.remove` | installed version | `package.install` of that version when the repository still offers it; otherwise irreversible at the time of the change |
| `package.upgrade` | versions before the upgrade | install of the recorded versions when the repository still offers them; otherwise irreversible per package |
| `start`, `stop` (services) | previous state | the opposite command |
| `enable`, `disable` | previous start type | the opposite command with the recorded start type |
| `restart`, `stop` of a process, `run` | none | irreversible; the report states `irreversible` before the change |
| `firewall.allow`, `firewall.deny` | target `firewall:<rule>`, `absent` to `present` | `firewall.remove` of the rule, in the backend and table that were used |
| `firewall.remove` | target `firewall:<rule>`, `present` to `absent`, one change per removed rule | `firewall.allow` or `firewall.deny` of every removed rule |
| `network.route_add`, `network.route_remove` | target `route:<destination>` with `via=<gateway> dev=<interface> metric=<n>` | the opposite command with the recorded route |
| `network.dns_set` | target `dns:<interface>` with `servers=<list>;search=<list>` | `network.dns_set` with the previous servers and domains; `/etc/resolv.conf` is a file target restored by the journal |
| `clock.timezone_set`, `clock.ntp` | targets `clock:timezone` and `clock:ntp` | the same command with the previous value |
| `clock.set` | target `clock:time` | irreversible; the report keeps the marker `external_effect` and its `detail` gives the reason |
| `windows.cim` with `invoke` | target `cim:<class>.<method>` with the namespace and the condition | the documented opposite method on the same instances; otherwise irreversible |
| `accounts.create` | absent before | `accounts.remove` with `keep_home: true` |
| `accounts.modify` | shell, home, groups, comment and lock state before and after | `accounts.modify` with the recorded values of the fields that changed |
| `accounts.remove` | the whole state of the account; the home directory goes through the journal | `accounts.create` with the recorded identifier, shell, home, groups and comment, and `accounts.modify` for a locked account; the journal restores the home directory |
| `accounts.password` | none | irreversible |
| `groups.create`, `groups.remove` | absent before; identifier and members | pairwise; a removed group returns with `groups.add_member` of its members |
| `groups.add_member`, `groups.remove_member` | members before and after | the opposite command |
| `mount`, `unmount` | absent before; device, type and options | pairwise; an edit of `/etc/fstab` is a file of the journal |
| `sysctl.set` | previous value | `sysctl.set` with that value; the file of `persistent: true` is a file of the journal |
| `modules.load`, `modules.unload` | loaded or not | the opposite command |
| `power.restart`, `power.shutdown` | schedule | `power.cancel` for a scheduled action; irreversible when the action runs at once |
| `power.cancel`, `disks.partition`, `disks.format` | none | irreversible; the preview of a disk change lists the data that is lost |

Files changed by a program started with `run` and `privilege: elevated` are not
preserved; the entry is irreversible and the report says so.

## 9. Errors

All methods of `MutationTx` and `JournalStore` return the language error. The
details of journal errors carry the types of the journal module:

| Code | When | Details |
|---|---|---|
| `E_STALE` | expected state does not match | `path`, `expected`, `actual` |
| `E_NOT_FOUND`, `E_EXISTS`, `E_PERMISSION`, `E_PRIVILEGE`, `E_IN_USE`, `E_NO_SPACE`, `E_IO` | failure of a target | `path`, `kind` (file system error kind), `os_error`, `holders` |
| `E_CANCELLED` | the operation was cancelled; rolled back | none |
| `E_ROLLED_BACK` | call on a transaction that was rolled back; failed check after the change | `reason` |
| `E_ROLLBACK_FAILED` | rollback or undo could not restore every item | `failed`: list of `TargetPath` |
| `E_STAGE_FAILED` | protocol misuse in production builds | `problem` |
| `E_JOURNAL_NOOP` | no such entry, nothing to undo or redo | `op`, `state` |
| `E_JOURNAL_EXPIRED` | an entry of the selection expired; nothing changed | `expired`: list of `OpId` |
| `E_JOURNAL_CONFLICT` | the undo check found differences; nothing changed | `conflicts`: list of `Conflict` |
| `E_JOURNAL_PARTIAL` | a partial entry and the call asked for the whole entry | `op`, `available`, `unavailable` |
| `E_JOURNAL_CORRUPT` | the entry failed its integrity check | `op` |

Lack of space for the journal, an inactive journal and journal input/output
errors are never errors of a user operation.

## 10. Retention

An entry expires when any of three limits is exceeded (steps, age, size) or
when the user needs the space. Limits are evaluated against the current
configuration: lowering a limit expires entries immediately, raising it never
restores expired entries. A bulk operation expires as a whole; only reclaim for
space may release part of an entry, which then becomes `partial`.

Retention state (number of entries, bytes held, oldest entry, effective limits
and their sources) is reported only by `journal.status`
(`JournalStore::status`). `OpHandle`, `OperationRecord`, `UndoOutcome` and
every other result carry no expiry time and no quantity from which one can be
derived.

## 11. File system interface

`Fs` is the read-only interface given to every command. `FsWrite` performs
mutations; each method requires a `TxScope`, which only journal and space
implementations create, so commands change data only through `MutationTx`.
`Provider` and `ProviderWrite` serve path-like resources such as `registry:`
and `env:`; writable providers support snapshots and exact restoration.

Paths are absolute and lexically normalised (`AbsPath`); their serialized form
is the language form with `/` separators. A path with bytes that are not valid
Unicode is serialized as the object `{"path": <language form with U+FFFD>,
"raw_path": <native bytes, base64>}`, and is read back from `raw_path`, so the
manifest of an entry leads back to the entry that was changed; the directory
listing of a preserved tree gives such a name as `raw_path` next to `path`.
Errors are `FsError` values with a
kind that maps to a language code:

| Kind | Code |
|---|---|
| `not_found` | `E_NOT_FOUND` |
| `permission_denied`, `read_only` | `E_PERMISSION` |
| `privilege` | `E_PRIVILEGE` |
| `exists` | `E_EXISTS` |
| `in_use` | `E_IN_USE` |
| `no_space`, `quota_exceeded` | `E_NO_SPACE` |
| `stale` | `E_STALE` |
| `cross_device`, `unsupported`, `invalid_path`, `not_a_directory`, `is_a_directory`, `not_empty`, `name_too_long`, `loop`, `interrupted`, `io` | `E_IO`, with the kind in the details |

## 12. Reference semantics for test implementations

Test implementations must behave exactly as follows, so that tests written
against them hold for the real implementations.

### 12.1 Pass-through journal (`NoopJournal`)

1. `begin` returns a transaction that applies mutations directly to the
   underlying `FsWrite` with a scope of purpose `operation`.
2. `preserve` validates the declarations as the real journal does (existence
   for `create`, `modify`, `delete`; the expected state) and returns, for every
   target, mode `none` with reason `journal_inactive`.
3. Operation numbers are assigned from 1 upwards per instance, at the first
   `preserve` or, for marker-only transactions, at commit.
4. `apply` checks the expected state immediately before the change and fails
   with `E_STALE` on mismatch; after any failure every further call fails with
   `E_ROLLED_BACK`.
5. `commit` returns a handle with all targets marked `journal_inactive`;
   `rollback` restores nothing and lists every changed target in `failed`.
6. `history` returns an empty list; `details`, `undo`, `redo` and
   `rollback_committed` fail with `E_JOURNAL_NOOP`; `pin` reports every entry
   as not available; `purge`, `status` and `space` return empty results;
   `limits` returns `applied: false`; `recover` and `sweep` return empty
   reports.
7. A recording variant additionally keeps the list of calls in order, for
   assertions that every mutation went through a transaction.

### 12.2 Admission

The test admission returns `eligible`, `keep`, state `normal` and `alive`,
unless configured otherwise; a configured decision applies to every request.

### 12.3 Virtual file system

1. Paths are compared byte-wise on case-sensitive volumes; on volumes
   configured as case-insensitive, after NFC normalisation and case folding.
2. Each failure condition yields one error kind: missing entry or parent
   `not_found`; a file used as a directory component `not_a_directory`; an
   existing destination with `no_replace` or `create_new` `exists`; removal of
   a non-empty directory `not_empty`; a rename between volumes `cross_device`;
   `exchange`, `replace_with_backup` and `clone_file` on a volume without the
   feature `unsupported`; a configured sharing violation `in_use` with the
   configured holders.
3. `read_dir` returns entries in byte order of their names.
4. Timestamps are taken from the injected `Clock`; every content change sets
   the modification time to the current wall time and increments nothing else.
5. File identities are assigned from 1 upwards per volume; a rename keeps the
   identity, a new file gets a new one.
6. The entity tag is `(size, modified_ns, file_id)`; `content_hash` is
   SHA-256 of the content.
7. `holders` is empty unless configured; `can_write_dir` follows the mode bits
   of the directory for the configured account.
8. Faults are injected per path and operation (`permission_denied`, `io`,
   `no_space`, `in_use`, `loop`, short reads) and apply to the next matching
   call only, unless configured as persistent.

## 13. Coverage table

`tables/journal-coverage.toml` (format `nurt-journal-coverage/1`) is the source
of the coverage table. It has one row per command, or per command and action,
of the categories change and run, and of journal commands with an effect.

| Field | Meaning |
|---|---|
| `command` | catalog command name |
| `action` | optional action, mode or target variant; a row without it covers the whole command |
| `group` | catalog group |
| `changes` | what the operation changes |
| `preserve` | preserve modes (section 5) |
| `coverage` | `full`, `partial`, `none` or `no_effect` |
| `irreversible` | reasons reported for the row, in addition to `generic_irreversible` |
| `exception` | description of the exception; required for `partial` and `none` |
| `note` | optional clarification |

Rules of the coverage values: `full` rows list only modes that keep data and no
reasons; `partial` rows list at least one mode that keeps data and an
exception; `none` rows list `none` or `external` (possibly with modes for local
side effects) and an exception; `no_effect` rows list no modes.

The reasons listed in `generic_irreversible` may degrade any row (inactive
journal, network or removable volume, no space for the copy, size limits, a
failure of the journal while recording). The
test suite checks that the table is well formed, that every catalog command and
every catalog action with an effect has a row, that every row names a catalog
command, and that irreversible catalog entries are not marked `full` and
reversible ones are not marked `none`.

The coverage test of the journal implementation runs every row: it performs
the operation, compares the full tree state (hashes and metadata) before the
operation and after undo, checks the preserve mode against the table, checks
the irreversible reasons of `partial` and `none` rows, and detects mutations
outside a transaction.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
