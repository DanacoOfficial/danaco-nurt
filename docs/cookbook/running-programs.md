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
