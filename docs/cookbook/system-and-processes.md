---
category: system-and-processes
title: System and processes
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# System and processes

## List the processes by memory

List the five processes that use the most memory.

```nurt example
processes |> sort memory order: desc |> first 5
```

```text expected
pid
```

```bash
ps aux --sort=-%mem | head -6
```

## Find a process by name

Find the processes whose name matches a pattern.

```nurt example
processes /node/
```

```text expected
node
```

## Show who listens on a port

Show the process that listens on port 8080.

```nurt example
ports state: listen |> filter port == 8080
```

```text expected
8080
```

```bash
ss -ltnp | grep :8080
```

## Stop a process

Stop the process with a given identifier.

```nurt example
processes pid: 4132 |> stop
```

```text expected
4132
```

## Kill a process that ignores termination

Kill a process that does not react to the termination signal.

```nurt example
processes pid: 4132 |> stop signal: kill
```

```text expected
4132
```

## Show the disks

Show the mounted volumes and their free space.

```nurt example
disks
```

```text expected
free
```

```bash
df -h
```

## Show the host

Show the operating system, the user and the programs that are installed.

```nurt example
host programs: ["git", "python3", "node"] versions: true
```

```text expected
git
```

## Show the environment variables

Show the environment variables of the process whose name contains `PATH`.

```nurt example
environment /PATH/
```

```text expected
PATH
```

## Find the program in the search path

Find where a program is installed.

```nurt example
program.find "git"
```

```text expected
git
```

## Find the processes that hold a file open

Find the processes that keep a file open.

```nurt example
open_files "/home/user/app/app.log"
```

```text expected
pid
```

## Read the system log

Read the 50 latest error events of the system log.

```nurt example
logs level: error limit: 50
```

```text expected
error
```

## Read the log of one unit

Read the log of one service unit since one hour ago.

```nurt example
logs unit: "nginx" since: ago(1h)
```

```text expected
nginx
```

## Show the accounts

List the user accounts of the host.

```nurt example
accounts kind: user
```

```text expected
user
```

## Suspend a process

Suspend a process until it is resumed.

```nurt example
processes pid: 4132 |> suspend
```

```text expected
4132
```

## Resume a process

Resume a suspended process.

```nurt example
processes pid: 4132 |> resume
```

```text expected
4132
```

## List the processes of an account

List the processes that run for one account.

```nurt example
processes user: "deploy"
```

```text expected
deploy
```

```bash
ps -u deploy
```

## Show the working directory of processes

Show the working directories of the processes whose name matches a pattern.

```nurt example
processes /node/ cwd: true
```

```text expected
node
```

## Show the UDP ports

Show all ports that use the UDP protocol, not only the listening ones.

```nurt example
ports state: all protocol: udp
```

```text expected
udp
```

```bash
ss -uanp
```

## Find the disk with the least free space

Show the volume with the least free space.

```nurt example
disks |> sort free order: asc |> first 1
```

```text expected
free
```

```bash
df -h --output=source,avail | sort -h -k2 | head -2
```

## Show the variables of the account

Show the environment variable `PATH` as it is set for the account, not for the session.

```nurt example
environment "PATH" scope: user
```

```text expected
PATH
```

## Search the warnings of the system log

Show the warnings of the last day that mention a timeout.

```nurt example
logs level: warning message: /timeout/ since: ago(24h)
```

```text expected
timeout
```

```bash
journalctl -p warning --since '-1 day' | grep -i timeout
```

## Read a window of the log of a unit

Read the log of a unit between two points in time.

```nurt example
logs unit: "sshd" since: ago(2h) until: ago(1h)
```

```text expected
sshd
```

## Find an installed program by a keyword

Search the path, the packages and the applications for programs that handle JSON.

```nurt example
program.search "json" source: [path, package] limit: 10
```

```text expected
json
```

## Read the description of a program

Read the card of a program with its usage guide.

```nurt example
program.describe "jq" include_guide: true
```

```text expected
jq
```

## Show the versions of installed programs

Show the locations and versions of several programs.

```nurt example
program.find ["git", "make"] versions: true
```

```text expected
git
```

## Stop a process after a grace period

Terminate a process and kill it when it does not exit within five seconds.

```nurt example
processes /^runaway$/ |> stop grace: 5s
```

```text expected
runaway
```
