---
name: bash-migration
description: Habits from bash and how Nurt corrects or translates them, including conditions, loops, functions, script parameters and administration commands.
commands: [remove, move, copy, create, read, find, search, replace, write, run, list, lines, attributes]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# bash-migration

## What is corrected

A line that starts with a bash command is read with the rules of the POSIX shell and translated to a Nurt program. The result shows `corrected` and `ran`, the program that was executed. A translation that rests on an assumption and touches the target of a change is not applied: the answer is E_AMBIGUOUS with ready programs.

A line is a bash command when its first word is a bash command and it shows notation that only the shell has: a flag such as `-rf`, an operator such as `&&` or `>`, a quoted `'text'`, `$NAME`, an assignment before the program (`CI=1 npm test`) or an unquoted pattern. A first word that Nurt does not know at all (`touch`, `gzip`, `sha256sum`) needs none of these, and a word that Nurt reads as a synonym (`cat`, `ps`, `which`) is a bash command when its argument is a bare word that Nurt cannot take (`ps aux`, `which git`). A line with Nurt notation, a pipe `|>` or a named argument such as `glob:`, is always a Nurt program.

```text
rm -rf /srv/app/build
```

The line is corrected to `remove "/srv/app/build"`. The form to write directly uses a pattern, and the console lists the targets it computed:

```nurt
find "/srv/app" glob: "**/*.tmp" |> remove
```

## Commands and flags

`mv A B` becomes `move "A" to: "B"`, `cp -r A B` becomes `copy "A" to: "B"`, `mkdir -p D` becomes `create "D" type: directory`, `touch F` becomes `create "F" type: file on_exists: touch`, `cat F` becomes `read "F"`, and `ls D` becomes `list "D"`. Recursion and the creation of missing parents are the default, so `-r` and `-p` need no counterpart.

A long flag or a short flag becomes the named parameter of the command with the same meaning: `grep -i` becomes `case_sensitive: false`, `grep -v` becomes `invert: true`, `grep -l` becomes `output: files`. A flag without a counterpart is an error that lists the parameters of the command.

## Privilege elevation

`sudo CMD`, `doas CMD` and `run0 CMD` become the translation of `CMD` with the call parameter `privilege: elevated`, and `sudo -u NAME CMD` (also `--user NAME`, `--user=NAME`) becomes the translation of `CMD` with `as_user: "NAME"`. The options `-n`, `-H`, `-E`, `-k`, `-b` and `--` are dropped. `sudo systemctl restart nginx` becomes `services name: "nginx" |> restart` with `privilege: elevated`; `sudo PROGRAM ARGS` where `PROGRAM` has no counterpart becomes `run ["PROGRAM", ARGS...] cwd: "..."` with `privilege: elevated`. `sudo -i`, `sudo -s`, `su` and `su -` open an elevated session, which is not a command: they end with `E_AMBIGUOUS` and a next step that gives the command with `privilege: elevated`. A group (`sudo -g`) and other options without a counterpart are reported with the option. The correction `run ["sudo", ...]` written in Nurt is replaced by the same `run` without `sudo` and with `privilege: elevated`.

## Redirection and variables

`echo "x" > F` becomes `write "F" "x\n" mode: overwrite` and `>>` becomes `mode: append`. A here-document written to a file becomes `write` with a raw text.

`$NAME` becomes `@NAME` when the program has a matching `let`. Inside a text the substitution is not translated: write `@name.path` as an argument instead.

```nurt
let files = find "/srv/app" glob: "**/*.go"
run ["wc", "-l", ...@files.path] cwd: "/srv/app"
```

## Pipes, lists and sequences

A pipe `A | B` becomes `A |> B` when both sides translate. `find D -name P -delete` becomes `find "D" glob: "**/P" |> remove`. `xargs` and `find -exec` become a changing stage after `find`, or a `run` that receives `...@files.path`.

A list `A && B && C` or `A ; B` with several changing commands becomes one `steps` block: the steps run in order, the block is one journal entry, and a failure in any step rolls back the earlier ones (a `run` step is listed as not reversible). In a single call, the answer is `E_ONE_EFFECT`; `candidates` has the `steps` block as the first program and `next_steps` has the separate calls.

```text
mkdir -p /srv/out && cp -a /srv/in/data.csv /srv/out/
```

```nurt
steps {
    create "/srv/out" type: directory
    copy "/srv/in/data.csv" into: "/srv/out"
}
```

A list with one changing command stays that command. `cd D && make && make install` becomes a `steps` block of two `run` steps with `cwd: "D"`.

## Conditions

`if`, `elif` and `else` become `when`, `otherwise when` and `otherwise`. The tests `[ … ]`, `[[ … ]]` and `test` become a `bool` expression: `-e`, `-f`, `-d`, `-L`, `-s` (file tests, checked with `exists` and `info`), `-z`, `-n`, `=`, `==`, `!=`, `=~` (a regular expression), `-eq`, `-ne`, `-gt`, `-ge`, `-lt`, `-le`, with `!`, `-a`, `-o`, `&&` and `||` as `not`, `and` and `or`. A command used as a condition (`if systemctl is-active x`) has a translation only when it is a `run`; its exit code is `r.exit_code == 0` after `let r = run …`.

```text
if [ -f /srv/a ]; then rm /srv/a; fi
```

```nurt
when exists("/srv/a") and ((info "/srv/a" |> first 1)).type == "file" {
    remove "/srv/a" on_missing: error
}
```

`case` becomes the same chain: every pattern is `==` for a literal and `~` for a pattern with `*`, `?` or `[…]`, `a|b)` is `or`, `*)` is `otherwise`.

`test -f F || CMD` and `[ … ] || CMD` become `when not (…) { CMD }`. `CMD || true` is `CMD` with the assumption that its failure is no longer ignored. Any other `A || B` has no translation: the error lists `A` and `B` as separate programs. `[ … ] && CMD` is `when … { CMD }`.

## Loops

`for x in PATTERN; do … done` becomes `each (find … ) as x { … }`: the variable is a record and `$x` becomes `@x.path`. `for x in $(CMD)` is `each (CMD) as x` when `CMD` reads records. `for x in a b c` is `each ["a", "b", "c"] as x`. `for x in "$@"` is `each @args as x` with the script parameter `args`. `while [ … ]; do … done` becomes `while … { … }`, and `until [ … ]; do …; sleep N; done` becomes `repeat until: … every: Ns { … }` (the body runs once before the first test, which the translation reports as an assumption). `break` and `continue` stay as they are.

Not translated: the arithmetic `for ((…))`, `select`, a loop body that uses `$x` inside a longer text (write `@x.path`), and a pipe into `while read`.

## Functions and script parameters

`f() { … }` and `function f { … }` become `define f(…) { … }`. A parameter is named after the variable that takes `$1`, `$2` (`dir="$1"`, `days=${2:-30}`), and is `arg1`, `arg2` otherwise, with a note in the report. A call `f x y` becomes `f("x", "y")`.

In a script, `$1`, `$2` and `$@` outside functions become the `params` block that starts the program, the type is taken from the use (`path` for the operand of `rm`, `cp` and the file tests, `int` for `-eq`, `-gt` and the other numeric tests, `text` otherwise), a default `${2:-30}` is the default of the parameter, `shift` moves the numbering, `$@` is the list `args`, and `getopts` declares one parameter per option: `bool` for a flag, `text` for an option with an argument. The program is called with `nurt run script.nurt dir: "/srv" days: 7`.

```text
dir="$1"
keep=${2:-7}
rm -rf "$dir"
```

```nurt
params {
    dir: path
    keep: text = "7"
}
remove @dir
```

## Administration of accounts, mounts and the kernel

| Bash | Nurt |
|---|---|
| `mkfifo [-m MODE] P` | `create "P" type: fifo mode: "MODE"` |
| `mknod P p`, `mknod P c|b MAJOR MINOR` | `create "P" type: fifo`, `create "P" type: device kind: char|block major: MAJOR minor: MINOR` |
| `useradd [-m] [-s SHELL] [-d HOME] [-G G1,G2] [-c TEXT] [-u UID] [-r] NAME` | `accounts.create name: "NAME" …` |
| `usermod [-s] [-d] [-G] [-c] [-L or -U] NAME` | `accounts.modify name: "NAME" …` (`-L` is `lock: true`, `-U` is `lock: false`) |
| `usermod -aG GROUP NAME` | `groups.add_member name: "GROUP" member: "NAME"` |
| `userdel [-r] NAME` | `accounts.remove name: "NAME" keep_home: true`; with `-r`, `keep_home: false` |
| `groupadd [-g GID] [-r] NAME`, `groupdel NAME` | `groups.create name: "NAME" …`, `groups.remove name: "NAME"` |
| `gpasswd -a USER GROUP`, `gpasswd -d USER GROUP` | `groups.add_member`, `groups.remove_member` |
| `passwd NAME` | `accounts.password name: "NAME"`; the password comes from a secret reference of the engine, never from the program |
| `mount [-t T] [-o O1,O2] DEVICE TARGET`, `umount TARGET` | `mount device: "DEVICE" target: "TARGET" type: "T" options: ["O1", "O2"]`, `unmount target: "TARGET"` |
| `sysctl -w KEY=VALUE` (one key per call) | `sysctl.set key: "KEY" value: "VALUE"` |

## Modules and firewall

| Bash | Nurt |
|---|---|
| `modprobe MODULE [PARAM…]`, `modprobe -r MODULE`, `rmmod MODULE` | `modules.load name: "MODULE" params: […]`, `modules.unload name: "MODULE"` |
| `ufw allow or deny PORT[/PROTO]`, `ufw allow from SOURCE to any port PORT proto PROTO` | `firewall.allow` or `firewall.deny` with `port: PORT protocol: "PROTO" source: "SOURCE"` |
| `firewall-cmd [--zone=Z] --add-port=PORT/PROTO`, `--remove-port=…` | `firewall.allow port: PORT protocol: "PROTO" zone: "Z"`, `firewall.remove …` |
| `nft add rule … input or output tcp or udp dport PORT accept or drop`, `iptables -A INPUT or OUTPUT -p tcp or udp --dport PORT -j ACCEPT or DROP [-s SOURCE]` | `firewall.allow` or `firewall.deny` with `port: PORT protocol: "tcp" direction: "in"` or `"out"` |

## Routes, name servers, clock, power and disks

| Bash | Nurt |
|---|---|
| `ip route add DEST via GW dev IF metric N`, `ip route del DEST` | `network.route_add destination: "DEST" gateway: "GW" interface: "IF" metric: N`, `network.route_remove destination: "DEST"` |
| `resolvectl dns IF SERVER…` | `network.dns_set interface: "IF" servers: ["SERVER", …]` |
| `timedatectl set-timezone ZONE`, `date -s "TIME"` | `clock.timezone_set "ZONE"`, `clock.set "TIME"` |
| `shutdown -r or -h [now or +MINUTES] [MESSAGE]`, `shutdown -c`, `reboot`, `poweroff`, `halt` | `power.restart` or `power.shutdown` with `delay: Nm` and `message: "…"`, `power.cancel`, `power.restart`, `power.shutdown` |
| `parted -s DEVICE rm N`, `parted -s DEVICE mkpart … START END` | `disks.partition device: "DEVICE" action: "delete" number: N`, `action: "create" start: "START" end: "END"` |
| `mkfs.FS [-L LABEL] DEVICE`, `mkfs -t FS DEVICE` | `disks.format device: "DEVICE" fs: "FS" label: "LABEL"` |
| `sqlite3 DB "SQL"` | `sql "DB" "SQL"` |

A rule beyond a port rule (`iptables -t nat …`, a rule with a match module) is run with `run ["iptables", …] cwd: "/"`, and with `privilege: elevated` when written with `sudo`. A form with an option that has no counterpart is an error that names the option. `sudo` before any of these commands adds `privilege: elevated`, as for every command.

## What is rejected

Only forms that depend on the shell itself have no translation: `eval`, subshells `( … )` and groups `{ …; }`, the arithmetic `for ((…))`, `select`, interactive input (`read`) and here-documents other than `cat > file`. `cd D && CMD` is translated only for an absolute path in the same line, as the `cwd` of `run` or as paths resolved against `D`; other uses of `cd` are rejected with a note on the call parameter `base`. A script that needs one of them runs unchanged with `run ["bash", "script.sh"] cwd: "/srv/app"`.

## Scripts

An existing shell script is run unchanged through `run ["bash", "script.sh"] cwd: "/srv/app"`. The command `nurt migrate` translates a script to a Nurt program with conditions, loops, functions and `params`, and reports what it could not translate. A function defined in the script is a `define`, and the later calls of it are calls.
