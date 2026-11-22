---
category: running-programs
title: Running programs and background jobs
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Running programs and background jobs

## Run a program in a directory

Run a build in a project directory.

```nurt example
run ["make", "build"] cwd: "/home/user/app"
```

```text expected
exit_code
```

```bash
cd /home/user/app && make build
```

## Fail when the exit code is not zero

Run a program and make the program fail unless it exits with code 0.

```nurt example
run ["go", "test", "./..."] cwd: "/home/user/app" |> assert exit_code: 0
```

```text expected
exit_code
```

## Pass environment variables

Run a program with an additional environment variable.

```nurt example
run ["npm", "test"] cwd: "/home/user/app" env: {CI: "1"}
```

```text expected
exit_code
```

```bash
CI=1 npm test
```

## Limit the run time

Stop a program that runs longer than one minute.

```nurt example
run ["make", "test"] cwd: "/home/user/app" timeout: 60s
```

```text expected
timed_out
```

## Show only the errors of a long output

Show the lines that look like errors, the first 5 lines and the last 10 lines.

```nurt example
run ["npm", "test"] cwd: "/home/user/app" show: errors head: 5 tail: 10
```

```text expected
exit_code
```

```bash
npm test 2>&1 | tail -n 50
```

## Start a program in the background

Start a long build and return at once.

```nurt example
run ["npm", "run", "build"] cwd: "/home/user/app" background: true
```

```text expected
job
```

```bash
npm run build > out.log 2>&1 &
```

## Read the output of a background job

Read the last 20 lines of the output of a job.

```nurt example
jobs "j1" tail: 20
```

```text expected
j1
```

## Wait for a job

Wait up to 30 seconds for a job to finish.

```nurt example
jobs "j1" wait: 30s
```

```text expected
j1
```

## List the running jobs

List the jobs that are still running.

```nurt example
jobs state: running
```

```text expected
running
```

## Stop a job

Stop a running job.

```nurt example
jobs "j1" |> stop
```

```text expected
j1
```

## Retry a failed job

Run a failed job again with the same arguments.

```nurt example
jobs "j1" |> retry
```

```text expected
j1
```

## List the files a program changed

Run a formatter and list the files it changed.

```nurt example
run ["gofmt", "-w", "."] cwd: "/home/user/app" watch: ["/home/user/app"]
```

```text expected
exit_code
```

## Pause for a number of seconds

Wait five seconds before the next call.

```nurt example
sleep 5s
```

```text expected
5s
```

## Run the tests and read the failures

Run the tests of a project and show at most 10 failures with 5 lines of message each.

```nurt example
test.run "/home/user/app" failures: 10 message_lines: 5
```

```text expected
test_run
```

## Feed a file to a program

Run a program with the content of a file as its standard input.

```nurt example
run ["sort", "-u"] stdin: "/home/user/app/names.txt"
```

```text expected
exit_code
```

```bash
sort -u < /home/user/app/names.txt
```

## Stop a program that produces no output

Stop an installation that prints nothing for two minutes.

```nurt example
run ["npm", "install"] cwd: "/home/user/app" idle_timeout: 2m
```

```text expected
exit_code
```

## Show only warnings and errors with context

Merge the error stream into the output and keep the lines that match a pattern with two lines of context.

```nurt example
run ["make"] cwd: "/home/user/app" merge_stderr: true filter: /warning|error/ context: 2
```

```text expected
exit_code
```

```bash
make 2>&1 | grep -E -C 2 'warning|error'
```

## Run with a clean environment

Run a program without the environment of the session and with one variable.

```nurt example
run ["env"] inherit_env: none env: {LANG: "C"}
```

```text expected
LANG
```

```bash
env -i LANG=C env
```

## Limit the resources of a program

Run a build with a low priority and a limit of memory.

```nurt example
run ["make", "-j4"] cwd: "/home/user/app" max_memory: 2gb priority: low
```

```text expected
exit_code
```

```bash
nice -n 19 make -j4
```

## Run a program for each file

Run an image optimizer on every PNG file, four at a time, 8 files per call.

```nurt example
find "/home/user/app/img" glob: "**/*.png" |> run ["optipng", "-o2"] parallel: 4 batch: 8
```

```text expected
exit_code
```

```bash
find /home/user/app/img -name '*.png' -print0 | xargs -0 -n 8 -P 4 optipng -o2
```

## Name a background job

Start a release build as a labelled job.

```nurt example
run ["make", "release"] cwd: "/home/user/app" background: true label: "release-build" description: "Release build of the application"
```

```text expected
release-build
```

## Report progress and a summary of a run

Report the percentage lines of a run and keep the line that sums up the result.

```nurt example
run ["cargo", "test"] cwd: "/home/user/app" progress: /\d+%/ summary: [/test result/]
```

```text expected
summary_lines
```

## Read the errors of a job

Read the lines of the output of a job that match a pattern, with two lines of context.

```nurt example
jobs "j1" view: output filter: /error/ context: 2
```

```text expected
j1
```

## Read the result of a finished job

Read the exit code and the duration of a finished job.

```nurt example
jobs "j1" view: result
```

```text expected
exit_code
```

## Stop a job gently

Interrupt a job and give it ten seconds to finish before it is killed.

```nurt example
jobs "j1" |> stop signal: interrupt grace: 10s
```

```text expected
j1
```
