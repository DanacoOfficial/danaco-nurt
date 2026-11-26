---
name: running-programs
description: Running external programs, waiting for them, reading their output and recording their changes.
commands: [run, jobs, stop, retry, assert, sleep]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# running-programs

## Starting a program

`run` starts a program without a shell. The first argument is a list: the program followed by its arguments, so no quoting rules of a shell apply. `cwd` sets the working directory, `env` adds variables and `inherit_env: none` starts the program with an empty environment. A program that does not exist fails with `E_PROGRAM_NOT_FOUND`.

```nurt
run ["go", "test", "./..."] cwd: "/srv/app" |> assert exit_code: 0
```

## Waiting, limits and background jobs

By default `run` waits for the program. `timeout` stops a program that runs longer, and `idle_timeout` stops a program that writes nothing for the given time. With `background: true` the call returns a job at once, and `jobs` reads its state, output or result later. `stop` ends a job and `retry` runs it again with the same directory and environment.

```nurt
run ["make", "all"] cwd: "/srv/app" background: true
jobs state: running
```

## Output

`show` selects what the result contains: `stdout`, `stderr`, `both`, `errors`, `summary` or `none`. `head` and `tail` keep the first or last lines, `filter` keeps the lines that match a pattern and `dedupe` (on by default) collapses repeated lines, also lines that differ only in numbers. The text of all streams together never exceeds `max_bytes` (4096 by default, `0` is no limit) and a line longer than 400 characters is cut. After exit code 0 the result has the last 5 lines and the field `digest`; after a failure of `cargo` or `pytest` the field `diagnostics` lists the first errors with file and line, and `read_hint` names the `read` and `search` commands that return the rest from the output file without running the program again. The `exec` record carries `exit_code`, `signal` and `timed_out`.

```nurt
run ["npm", "run", "build"] cwd: "/srv/app" show: errors tail: 40
```

## Exit codes

A program that exits with a non-zero code is not an error of `run`: the code is a field of the result. `assert exit_code: 0` turns an unexpected code into `E_ASSERT`, and `E_TIMEOUT` reports a program stopped by its limit.

## Detecting changes

Files that a program changes are detected only in the paths named by `watch`; the default is the working directory of the program. `track_changes` selects `scan`, `events`, `auto` or `none`. A change made by a program is recorded in the journal, and when its previous state could not be kept it is marked irreversible.

```nurt
run ["./build.sh"] cwd: "/srv/app" watch: ["/srv/app/dist"] track_changes: scan
```
