<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/supervisor.md -->
<!-- Responsibility:   Documents Job supervisor protocol. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Job supervisor protocol

A run that continues in the background after the command line exits is
supervised by a process that outlives the command line. The command line and
the supervisor exchange files in the state directory; there is no socket and no
other channel. The Rust implementation is in `crates/nurt-proc/src/jobs`.
The protocol version is `1` (`PROTOCOL` in `jobs/table.rs`); every message file
carries it in the field `protocol`, and a file of another version is dropped.

## 1. Files of `<state>/jobs`

| File | Content |
|---|---|
| `sequence`, `sequence.lock` | last job number; identifiers `j<number>` are never reused |
| `<id>.json` | job file: public record, request without environment, result |
| `<id>.input.json` | environment and standard input; readable by the account of the engine only |
| `<id>.lock` | locked by the process that supervises the job; a job recorded as running whose lock is free has lost its supervisor |
| `<id>.control/` | control requests to the supervising process and their replies |
| `host.lock` | locked by the detached supervisor |
| `inbox/` | start requests, their replies and cancel notices |

Files are replaced atomically by renaming a complete temporary file, so a
reader never sees a partial file and takes no lock.

## 2. Start of a run

1. The client writes `inbox/<name>.request.json`, where `<name>` is
   `<process id>-<counter>`; requests are served in the order of the counters.
   The file holds `protocol`, `command_name`, the bound `program`, the `request`
   (the run with its environment and standard input) and `wait`, the time the
   run may take before it moves to the background.
2. When `host.lock` is free the client starts the supervisor with the command
   registered by the host application, and starts it again when the lock is
   still free after 3 s. A supervisor that has been started tries the lock
   up to 10 times, 10 ms apart, so that the instant in which a client probes the
   lock does not make it give up.
3. The supervisor takes a request by removing its file. Whoever removes the file
   owns the request: a client that removes it first has withdrawn the request and
   the supervisor does not run it.
4. The supervisor runs the preparation of the run (journal transaction, change
   detection, spool), starts the program, records the process in the job file and
   writes `inbox/<name>.reply.json`: `started` with the job identifier, or
   `failed` with the error. The client waits up to 30 s for the reply, removes
   it and follows the job.

## 3. Cancellation

Cancellation is a state of the call, not an event that can be missed. It is
delivered in three ways, according to how far the run has advanced:

| Moment | How the cancellation reaches the run |
|---|---|
| The supervisor has not taken the request | the client removes the request file; the call ends with `E_CANCELLED` and nothing was started |
| The supervisor took the request and the program has not started | the client writes `inbox/<name>.cancel.json` (`{"protocol": 1}`); the supervisor gives each run a cancellation token of its own and cancels it when the file appears; the preparation ends at its next check, the supervisor checks the token again immediately before it starts the program, and the request is answered with `failed` (`E_CANCELLED`); the journal entry is rolled back with the reason `cancelled` and no job is recorded |
| The program has started | the client writes the control request `cancel` into `<id>.control/`; the supervisor checks the cancellation again right after it recorded the process and, when it is set, ends the whole process tree at once |

The process tree is ended as a group: `kill(-pgid)` on Unix systems, the job
object on Windows, the same calls as at a time limit. The result of a started
program that was cancelled has `killed_by: cancel`.

Cancelling a job that has ended, or cancelling a job twice, is not an error: the
answer is the record of the job in its current state. A client removes its cancel
notice after it has read the reply; a supervisor removes every cancel notice
found at its start.

## 4. Control requests

A requester writes `<id>.control/<name>.request.json` with the action `stop`
(with `signal` and `grace`), `suspend`, `resume` or `cancel`. The job thread of
the supervising process reads the directory at each poll (every 50 ms), removes
the request file, acts and writes `<name>.reply.json` with `done` (the job record)
or `failed` (the error). `cancel` needs no reply. A requester that can no longer
remove its request waits for the reply, which the supervising process writes
after the final state of a stopped job; the waiting time is the grace time plus
60 s for `stop` and 10 s for the other actions.

## 5. Compatibility

The cancel notice is an additional file; the protocol version stays `1`.

| Client | Supervisor | Behaviour |
|---|---|---|
| older | newer | the client never writes a cancel notice; a cancellation after the supervisor took the request reaches the program as before, by the control request once the program has started, and the supervisor ends the tree at once when it finds the request after it recorded the process |
| newer | older | the older supervisor ignores `*.cancel.json` files (it reads only `*.request.json`); the client removes the notice after the reply, and the cancellation reaches the program by the control request |

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
