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
