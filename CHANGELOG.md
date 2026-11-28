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

### Added

- Windows setup wizard: the release publishes `nurt-<version>-<target>-setup.exe`
  for `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`, one file that
  carries the program archive. The window is drawn by the program (English,
  without a web runtime) and guides through the requirements, the license, the
  scope (current user or all users) and the integration (`PATH`, shortcuts,
  Windows Terminal profile, PowerShell completion, first `nurt.toml`). Every
  change is written to the journal `setup-journal.jsonl`; Repair, Update and
  Undo (`uninstall.exe`, "Installed apps") reverse it, and `/S` with
  `--scope`, `--dir` and further switches installs without a window.
- `tools/release/build-release.sh` builds the setup wizard of each Windows
  target after the archive is packed and lists it in `checksums.txt`
  (`--no-setup` skips it). The option `--codesign <p12-file> <password-file>`
  signs `nurt.exe` and the setup files with Authenticode.

### Fixed

- Git: a tracked file changed in the second in which the index was written,
  without a change of its size, was no longer reported after a command of
  `nurt` rewrote the index (`git.commit`, `git.stage`, `git.reset`,
  `git.switch`, `git.merge`, `git.restore`, `git.stash`, `git.pull`,
  `git.rebase`): `git status` and `git diff` showed nothing, and `git add -A`
  and `git commit -a` left the change out. The index keeps the statistics of
  such a file from before the change, and `git` compares its content only as
  long as the index file is not younger than the entry. Before `nurt` writes
  an index it now compares these files by content and stores size 0 for an
  entry whose file differs, as `git` does, so every later reader compares the
  content. The releases 0.1.0 and 0.1.1 have the defect.
- Git: `git.commit` read the content of a file and took its statistics
  afterwards. A file that another program changed between the two steps was
  recorded in the index with the statistics of the changed file and the
  content from before the change, so `git` did not report the change. The
  statistics are now taken before the content, as `git` takes them.
- The start banner of an interactive session no longer touches the left edge of
  the terminal: as in the brand book, it stands three columns from the edge with
  one empty line above and below it.
- In the console of Windows that is not Windows Terminal (the window that opens
  from the Start menu without the profile), `nurt` turns on the processing of
  the escape sequences at the start, so the colours and the banner are drawn
  instead of being printed as `←[38;2;…m`; the console that processes them gets
  the drawing in half blocks.
- The setup wizard of Windows no longer copies the offline documentation (about
  200 HTML pages) by default: the option "Install the offline documentation" is
  off until the user switches it on. The silent mode still installs it unless
  `--no-docs` is given.
- `nurt migrate`, `script`, `run`, `check` and `fmt` read a file saved as UTF-16
  with a byte order mark (the form in which Windows PowerShell and the
  redirection of its output write a file); before, such a file ended with
  "stream did not contain valid UTF-8".
- The configuration files (`nurt.toml`, `.nurt.toml` and the file of
  `NURT_CONFIG`) are read when they start with a UTF-8 byte order mark or are
  saved as UTF-16 with a byte order mark, as Windows editors and Windows
  PowerShell 5.1 save them; before, every command failed with an error about
  the configuration file.
- When the line editor of `nurt repl` stops because of an error, the session
  reports the error on the error output and continues with plain line input,
  without editing, completion and history, instead of ending without a word.
  Before, a terminal that the line editor cannot use ended the session at its
  first prompt; started from the shortcut, the window closed at once.
- The application manifest embedded in `nurt.exe` no longer carries comments:
  the template had the footer comments after the closing element of the root,
  a form that the loader of Windows may refuse for the manifest of an
  executable (the message is "the application has failed to start because its
  side-by-side configuration is incorrect"). The footer of the template stands
  inside the root, as in the manifest of the setup wizard, and the build
  removes every comment from the embedded copy.
- A failure of the journal at the start no longer ends the program. When the
  recovery of the journal fails (an entry that cannot be recovered, a journal
  directory that cannot be read), the program starts as the documentation of
  the journal describes: programs that only read run, and a program that
  changes files runs without preserving the previous content and reports the
  change as irreversible. The entries that could not be recovered are retried
  at the next start. The interactive session writes the error of the journal
  and the advice `nurt journal salvage <volume>` below its banner, and `nurt
  doctor` reports the journal as unavailable with the same error; both list the
  causes that the journal recorded for the entries, at most five. Before, every
  command that used the engine ended with `cannot assemble the journal` at
  every start.
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
- macOS: the change tracking of `nurt-proc` waits for the delivery of the
  events of FSEvents before it reads them. The wait ended after 50 ms of
  silence even when no event had arrived, so under load the changes below
  `.git/` and new files were not reported although the result was marked as
  tracked by events; the wait now confirms the delivery with a barrier file.
- The service accepts a connection whose peer has already closed. On macOS
  setting the write timeout of such a socket fails with `EINVAL`, and the accept
  loop treated that as an accept error and waited 100 ms before the next
  connection. A client that polls every few milliseconds and gives up after
  50 ms, such as the liveness check of the journal, produced closed connections
  faster than the loop removed them, so every ping timed out. The write timeout
  is now best effort.
- The engine opens when the extension directory cannot be listed or the
  WebAssembly host cannot start; the extensions are left out instead of ending
  the start of the program.
- On Windows, the recursive removal enters the directory placeholders of cloud
  providers (OneDrive files on demand) and removes them entry by entry; links,
  junctions and network links of a distributed file system are still removed
  themselves and never entered.
- The manifest of an extension is read in UTF-8 with or without a byte order
  mark and in UTF-16 with one, which is how Windows PowerShell 5.1 and Notepad
  write a file.
- `nurt ast --to cbor` refuses to write the binary output to a terminal.
- The setup counts the space for the files that it installs: without the
  documentation, not for the whole archive.
- The setup asks for confirmation before it undoes a setup that has just
  finished.
- The shortcut that starts the program in Windows Terminal is made only when
  `wt.exe` exists, and not for an installation for all users, because the
  launcher is the alias of the account that runs the setup.
- Setup: the shortcut of the Windows Terminal profile and the start of the
  program at the end of the wizard named only the profile (`wt.exe -p "Danaco
  Nurt"`). Windows Terminal opens its default profile when it does not know
  the named one, so a Windows Terminal that was open before the setup wrote
  the profile, or one whose profile was removed, opened another shell and
  `nurt` did not start. Both now pass `nurt.exe` as the command line of the
  tab besides the profile, and the end of the wizard opens a console window
  when Windows Terminal cannot be started. An installation folder with `;`
  in its path, which Windows Terminal reads as the separator of its commands,
  gets the shortcut that starts `nurt.exe` itself.
- Setup: a `Path` of the user or of the machine that is stored as `REG_SZ`
  keeps that type when the folder of `nurt.exe` is added or removed; before,
  the value was rewritten as `REG_EXPAND_SZ`, which makes Windows expand a
  `%NAME%` that was a literal text.
- The documentation of the command line explains how to quote a text of the
  language in PowerShell.

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
- Conformance: the binding cases `conformance/bindings/api.json` agree with the
  specification and with `nurt serve`. Against the engine 12 of the 23 cases
  passed before; all 23 pass now in Python and Node, and 22 in Go (`start.cancel`
  fails because the Go binding reports the answer to a cancelled run as a call
  error). The cases state the tree version `1.1`, the member `operations` of
  `history`, the members `formats` and `features` of `info`, the member `ran` of
  `correct`, the canonical text of `print` with its final line feed, absolute
  fixtures for `check` and for the cursor of `run`, and the error `E_SYNTAX` for
  a program that ends after a pipe.
- Conformance: the fake server answers `correct` with `ast` and `ran`, and
  `info` with `version`, `formats` and `features`, as the engine does, instead
  of `program` and `name`, `language`. The binding cases pass against it in
  Python, Node, Go, .NET and Java.
- Conformance: `conformance/tools/test_suite.py` passes; 166 checks failed
  before. The suite has exec cases for the 29 commands that had none (the
  administration commands, which never change the system, and `next`), a
  journal case for every row of the coverage table (rows that no case can
  establish are listed with their reason in `PENDING_ROWS`), correction seeds of
  the new commands, and the placeholder of the identifier of a job in the exec
  and journal cases follows the record `job` (`report/job/fields/id/v`).
- Catalog: `package.install`, `package.remove`, `package.upgrade`, `start`,
  `enable` and `disable` were classified as `irreversible`, although `undo` has
  restored them by the inverse operation since 0.1.1. They are now
  `reversible_if_preserved` with a note that states the condition (a version
  that the repository no longer offers, and a service state that no command
  restores, stay irreversible); `restart` stays `irreversible`. The
  classification is what `nurt check`, the signature in `help` and the
  command pages report; the irreversible stages that a construct announces
  before it runs are unchanged because they follow the effect `external`. The groups
  `package` and `administration` of the documentation, the table of
  the journal coverage, the specification of the catalog and the golden bound
  trees (`catalog_rev`) follow.
- `nurt serve --socket <path>`: the documentation presented the option as
  working or as available in some builds. No build contains a socket
  transport; the option is recognised and ends with exit code 3. The integrator
  guide, the pages of the site and the module comment now say that it is
  reserved and that a host connects with `nurt serve --stdio`.
- Documentation: the `initialize` example of the versions guide shows the
  values the server returns (`ast` 1.1, `language` and `catalog_rev` null,
  `max_inflight` 16); the readme states the state of release 0.1.2; the
  installation page separates the systems on which the code is built from the
  release archives (Linux glibc and musl and Windows MSVC, each on x86_64 and
  aarch64; no macOS archive) and marks the NuGet and Maven channels as planned;
  the ban reasons of `deny.toml` name `saphyr` instead of `yaml-edit`; the
  readmes of the integrations name the workflow files of `.github/workflows`;
  35 examples of the command pages, the cookbook and the git group use absolute
  paths, because the language rejects a relative path (`E_PATH_RELATIVE`).
- Windows: the record of `host` took the version of the system from a file of
  Linux and reported `unknown`. The command now reads the version from the
  system, as it does on macOS.
- Windows: `undo` of a change of packages that must install an earlier version
  was always reported as irreversible, because only `apt` and `dnf` were asked
  whether the repository still offers the version. `winget` (`winget show
  --versions`) and Chocolatey (`choco search --all-versions`) are asked too.
- `package.install` with `version` did not pass the option that lets the
  manager replace an installed version with an earlier one, so `undo` of
  `package.upgrade` failed on `apt` (`apt-get` refuses a downgrade under `-y`
  without `--allow-downgrades`), on Chocolatey (`--allow-downgrade`) and on
  `zypper` (`--oldpackage`). The option is passed whenever `version` is given.
- Windows: `winget install --version` refuses a version earlier than the
  installed one ("No newer package versions are available") unless `--force`
  is given, so `undo` of `package.upgrade` through `winget` failed.
  `package.install` with `version` passes `--force` to `winget`. Run on
  Windows 11 with winget 1.29 and a portable package: version 1.8.2 was
  replaced by 1.7.1.
- Windows: the commands of the group `package` took every row of the tracking
  database of `winget` for an installed package. `winget` keeps the row of a
  removed package and one row for every version of an upgraded one, so
  `package.list` showed a removed package as installed, and `package.remove`
  reported the version before as the version after, stayed irreversible and
  kept no inverse operation. A row now counts only when the system confirms
  it: the package has a directory or a link of `winget`, or an uninstall entry
  of the registry carries the identifier of the package, a product code of
  the row or the name of the program (compared without version, architecture
  and punctuation, because an installer that updates the program outside
  `winget` writes a new product code); of several rows the one whose version
  the uninstall entry shows is taken. A row without a product code counts as
  before. The defect was present in 0.1.1.
- `undo` without a number did not run the inverse operation kept for a change
  of packages, of services or of the system configuration; it ended with
  `E_JOURNAL_NOOP` ("has nothing left to restore"), while `undo` with the
  number of the entry ran it. `undo` without a number now runs the inverse
  operation of the entry that the journal chooses. An entry that an inverse
  operation wrote is undone by its number, and `undo` without a number of an
  elevated entry ends with `E_PRIVILEGE` and the next step, as before.
- `undo` and `redo` with a journal that did not start (for example after a
  failed recovery) applied the restoration, recorded nothing and then failed
  with `E_INTERNAL`. They now refuse before the first change with `E_IO` and
  the problem `journal_unavailable`; `nurt doctor` names the cause.
- An operation that was interrupted (a program started with `run` that was
  killed, a closed window, a crash) is rolled back by the recovery of the
  journal at a later start. When the directory that held a target had been
  removed in the meantime, the rollback failed with `E_NOT_FOUND`, and the
  entry was tried again at every start, without end: the recovery reported
  "journal entries could not be recovered", and the releases 0.1.0 and 0.1.1
  then refused to start. Such an entry now ends as corrupt at the first pass,
  before anything is changed, with the content it preserved kept for
  `nurt journal salvage`; the recovery succeeds and the journal works.
- Windows: a write to a file that a program has mapped into memory, for
  example the file of a running program (system error 1224), was reported as
  `E_IO`. It is reported as `E_IN_USE` and goes through the lookup of the
  programs that use the file, like a sharing violation; the write is not
  retried.
- Windows with a display language other than English: `privilege: auto` did
  not repeat an administration command elevated, because a refusal for missing
  rights was recognised only by its English text. The error category and the
  error identifier of Windows PowerShell, which are the same in every display
  language, and the Polish text of the system error 5 are recognised too.
- Windows with a display language other than English: `accounts.modify`,
  `accounts.remove` and `accounts.password` recorded an empty state of the
  account (full name, home directory, groups, disabled flag), because the
  labels of `net user` were read in English only; `undo` then restored the
  account without them. Such a listing is now read through ADSI. The members
  of a group (`groups.*`) included the closing line of `net localgroup`.
- The error of a failed `package.install`, `package.remove` or
  `package.upgrade` quoted only the standard error of the manager, so on
  Windows it ended with an empty reason: `winget`, Chocolatey and Scoop report
  a failure on standard output. The end of the standard output is quoted when
  the standard error is empty.
- Windows: the registry back end read the class name of a key only when a
  subkey of the key carried a class name, so `undo` restored a key that
  carries a class name and has no such subkey without it. The class name of
  the key is now always read.
- Windows: a write of a registry value, a copy and `undo` created every key of
  the path again, also the keys that exist, and so asked each parent key for
  the right to create subkeys. A change below a key whose parent refuses that
  right to the account failed with `permission_denied`. Only the missing keys
  are created now.
- Windows: a `registry:` path written with the separator of the registry
  (`registry:/HKCU/Software\Example\Name`) was read as one name. A read
  failed with `not_found` or showed paths with wrong names, and a write of a
  value created a value of that name on the root key. Such a path is now
  rejected with `E_INVALID_VALUE` and the advice to write `/` between the
  keys; `\` stays valid in the name of a value below a key. Where the
  correction of the program text applies, it writes `/` and the program runs.
- Windows: a command that reads a drive without a medium (`disks` on a
  computer with an empty card reader or optical drive, a file command on such
  a drive) could stop behind the dialog of the system "There is no disk in the
  drive", which waits for a person. The program now asks the system to return
  the error instead; the programs it starts inherit the setting.
- Windows: `kill` of a process that ended at that moment answered
  `E_PERMISSION`, because the system reports the request for an ended
  process as a refused access; it now answers `E_NOT_FOUND`. The error of a
  program that cannot be started is read before the resources of the start are
  released, so its text is the one of the failed start.
- Windows: the time zone of the host was `UTC` unless the variable `TZ` was
  set, so a commit made by the program carried the offset `+0000` and a new
  schedule counted its hours in UTC. Without `TZ` the program now takes the
  zone of the system under its name of the IANA database.
- Windows: `nurt.exe` reserves 8 MiB for the stack of its main thread, the size
  of the stack on Linux. The linker reserved 1 MiB, and the program parses,
  binds and runs a program on the main thread, so a nesting that Linux accepts
  could end the process on Windows.
- Windows: when `nurt` started without arguments (the shortcut of the Start
  menu or of the desktop, a double click) cannot open the session, the window
  of the console closed together with the message of the error. The program
  now writes the message and waits for Enter when it is the only process of
  its console; a start from the prompt of a shell is unchanged. The same holds
  for a panic of the main thread: its message stays in the window until Enter
  is pressed.
- Documentation: the installation page states the earliest version of Windows:
  Windows 10 for `nurt.exe` and Windows 10 version 1809 (build 17763) for the
  setup program.
- The recovery of a journal of one million entries was measured again on the
  code of this release: 1.3 s in each of three runs of a release build on
  Linux x86_64, against the 5 s that the design assumes. The measurement of
  6.2 s reported for the releases 0.1.0 and 0.1.1 preceded later corrections.

### Known issues

- No macOS archive is published for this release. The code for macOS passes
  `cargo check` and the tests of the continuous integration on the macOS runner.
- The corrections of this release for Windows were built and run on one
  system, Windows 11 (build 26300, English display language): the start of
  `nurt.exe` in a console window and in Windows Terminal, the setup wizard, and
  the tests of the changed packages and of the wizard, a setup for all users
  with its undo, and the installation, upgrade, removal and undo of one
  portable package through `winget`. Not run: the removal of a directory with
  cloud placeholders (OneDrive), the administration commands on a system in
  another display language, the undo of a package change through Chocolatey,
  and through `winget` for a package that has an installer of its own.
- The unit tests of the setup wizard write the entry in "Installed apps",
  `App Paths` and `Path` of the account that runs them. On a machine with an
  installation of Danaco Nurt the installation loses its entry, and
  `uninstall.exe --undo` reports that Danaco Nurt is not installed until the
  setup is run again.
- On Windows, the registry back end does not report whether a key is volatile
  (the Win32 interface has no call for it), so an undone change restores a
  volatile key as an ordinary one.
- The undo of a package change that must install an earlier version is
  reported as irreversible for the managers other than `apt`, `dnf`, `winget`
  and Chocolatey (`pacman`, `zypper`, Homebrew, Scoop): they are not asked
  whether the repository still offers the version.
- No test covers the installation of an earlier version over a later one by
  `dnf`; `package.install` passes it no option for it. Through `winget` the
  installer of the package decides whether an earlier version replaces a
  later one; `--force` only makes `winget` start it.
- `undo` without a number does not step over an entry that changed only
  packages, services or the system configuration and that is already undone
  or was written by an inverse operation: it ends with `E_JOURNAL_NOOP` for
  that entry. Earlier entries are undone by their numbers.
- The administration commands recognise a refusal for missing rights by the
  text of the system program. On Windows the texts of Windows PowerShell
  (every display language), of the English system and of the Polish system
  are recognised; in another display language a refusal by `net`, `shutdown`
  or `tzutil` is reported as `E_STAGE_FAILED`, and `privilege: auto` does not
  repeat the call elevated.
- On Linux and macOS, the time zone of the host is `UTC` unless the variable
  `TZ` is set; the zone of the system is not read.
- No test covers `nurt.exe` on Windows 10: the continuous integration runs on
  the runner `windows-latest`. The setup program refuses a system earlier than
  Windows 10 version 1809 only after it has started.
- Every start of the program that opens the engine asks the system for each
  volume recorded in `volumes.json` of the state directory. A recorded volume
  that is a network drive which no longer answers delays the start by the
  time limit of the network for each such record.
- Every start of the program that opens the engine calibrates the reclaim cost
  of each recorded volume (up to about 150 ms), and the monotonic time of the
  heartbeat of the space service counts from the start of the process, so two
  programs that run at the same time can find the service stopped and keep
  no previous content for a change.

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
