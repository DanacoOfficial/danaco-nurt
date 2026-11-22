<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Stage interface

This document specifies how a command of the Nurt language is implemented
and executed. The Rust form of the contract is the module `stage` of the crate
`nurt-contracts`; clock and host facts are in the module `env`. An
implementation of the language in another programming language follows the
same steps and the same rules.

## 1. Execution steps

A pipeline is executed in fixed steps. Steps 0 to 4 have no effect in the
world. The step number appears in error positions as text.

| Step | Name | What happens | Effect in the world |
|---|---|---|---|
| `0` | parse | text is corrected and parsed into the surface tree | none |
| `1` | bind | the surface tree is bound to the catalog; static checks of every stage | none |
| `2` | read | sources and transforms produce records; file records carry `etag` | reads only |
| `3` | check | checks before the effect (`assert`) | none; failure changes nothing |
| `4` | plan | the effect stage computes its targets and the planned effect; `preview` ends here | reads only |
| `5` | apply | the effect stage changes the world through a journal transaction | change |
| `5a` | after | stages after the effect read the records of the effect | none |
| `6` | verify | the `verify` pipeline runs; failure rolls the effect back | rollback |
| `7` | report | the effect report is built | none |

## 2. Dispatch by phase

The binder assigns every bound stage a phase. The phase selects the interface
the executor calls:

| Phase | Step | Interface |
|---|---|---|
| `source`, `transform` | 2 | `ReadStage::execute` |
| `check` | 3 | `ReadStage::execute` |
| `mutate`, `run` | 4 | `EffectStage::plan` |
| `mutate`, `run` | 5 | `EffectStage::apply` |
| `after` | 5a | `ReadStage::execute` |
| `verify` | 6 | executor |

A command implementation (`StageImpl`) provides the read interface, the
effect interface or both. A command whose effect depends on an argument (one
action reads, another changes) provides both; the binder assigns the phase
from the arguments.

`StageImpl::check` runs in step 1, after binding, with access to the catalog
specification, the host facts and the configuration only. It may reject
arguments that the catalog types cannot express and reports static facts
(`StaticInfo`): whether the stage waits, network endpoints it will contact,
external programs it will start, paths it names, and command-specific
details. These facts are returned by the check of a program without executing
it, so that an embedding application can apply its own rules before
execution.

## 3. Registry

One command may have several implementations, for example `remove` for file
entries, for document items and for jobs, supplied by different crates. Each
implementation describes itself with a `StageDescriptor`: the command name,
whether it can start a pipeline, and the record kinds it accepts. The registry
selects the implementation by command name and by the record kind declared by
the catalog for the output of the previous stage. When no implementation
accepts that kind, binding fails with `E_TYPE` and the list of accepted kinds.
A stream whose records have kinds handled by different implementations fails
with `E_TYPE` when the first record of another kind reaches the stage; the
program is split by the caller.

## 4. Records and streams

Records flow between stages in a `RecordStream`, a lazy iterator of
`Result<Record>`:

* A stage pulls records from its input only as needed. A pipeline therefore
  holds the whole input in memory only when a stage must (for example `sort`,
  `last`, `group`); such a stage collects with a bound, and exceeding
  `limits.max_records` fails with `E_LIMIT`. Records are never dropped
  silently.
* An error item ends the program. A stream yields nothing after its first
  error.
* Items skipped without an error (an unreadable entry during a listing, an
  unavailable log channel) are recorded as notable entries of the report
  through the context; they do not end the program.
* The output of a stage is records, a single value (for example the result of
  `count`) or nothing. It may carry a header record describing the whole
  stream (for example `test_run` in front of `test_result` records), the order
  of the stream known to its producer (so that a following `sort` by the same
  keys does not collect the stream) and a presentation hint for the text
  renderer (`tree`, `sizes`). Pass-through stages keep the header, order and
  hint.

## 5. Arguments

A stage receives its bound arguments in signature order. Every parameter of
the signature is present; defaults inserted by the binder are marked.
Literal values, defaults and references to named results are evaluated before
the stage runs. Per-record expressions are evaluated by the stage for each
input record through the evaluator of the context. Pipeline arguments (the
check of `verify`, `write from:`) are run through the pipeline runner of the
context.

## 6. Effect stages

An effect stage (a change, an external action or a launched program) has two
steps.

### Plan (step 4)

The stage consumes its input, records the entity tag of
every target as read, and returns an effect plan: the planned changes with
differences, notable effects and irreversible effects known in advance,
records describing an effect without file targets (for example the command
line and directory of a program), whether the operation is bulk, and private
state of the implementation. The plan step may read the world, including
read-only network requests needed to compute the plan; it creates, changes,
sends and starts nothing. A preview reports the plan and stops. The same plan
is used by the apply step, so a preview shows exactly the targets that an
execution changes. The private state holds what is needed to produce new
content, not the content itself.

### Apply (step 5)

The executor admits the operation and opens a journal
transaction. The stage receives the plan and the transaction; every change of
user data goes through the transaction: all targets are declared and their
previous state is preserved before the first change, then the changes are
applied and the journal checks entity tags (`E_STALE` on mismatch). Effects
outside the file system are marked irreversible on the transaction. The stage
never commits; the executor commits on success and rolls back on failure or
cancellation. The stage returns the records of the effect (`effect`, `exec`,
`job`, …), the changes as applied and the notable and irreversible effects
observed. The returned records do not borrow the transaction.

When a launched program continues in the background after its `wait`, the
outcome is marked detached and holds a `job` record; the stages after the
effect are not run.

## 7. Stages after the effect

The stages allowed after the effect (`assert`, `filter`, `select`, `first`,
`last`, `skip`, `count`, `sort`, `group`) are read stages executed in step 5a
on the records of the effect. They never change the world. A failed `assert`
in step 5a reports `E_ASSERT` with step `5a` next to the full effect report;
the effect is not rolled back. Only `verify` (step 6) rolls back.

## 8. Execution context

A stage reaches the world only through its execution context:

| Facility | Purpose |
|---|---|
| file system | read-only access to files; writes go through the transaction only |
| journal store | the journal commands (`history`, `undo`, `redo`, `journal.*`) |
| clock | wall time, monotonic time, waiting |
| host | host facts and the environment of the engine process |
| configuration, request options | effective settings and options of the call |
| cancellation, deadline | cooperative cancellation and the time limit of the program |
| limits | records held in memory, size of text files, threads |
| state directory | spool, jobs and caches of the engine |
| evaluator, pipeline runner | per-record expressions and pipeline arguments |
| services | registry of the services of other command groups (documents, source code, programs, file listing, repository queries, text codecs, file holders, review rules); a service is absent when its group is not loaded |
| extensions | further shared objects registered by type |
| frame | index, command and step of the running stage, and the demand of the rest of the pipeline |
| events | progress and output of launched programs |
| notes | notable entries of the report |

### Demand

The context tells a stage what the rest of the pipeline needs:
the fields used later (expensive fields that are not used may be left out),
whether records of files must carry `etag` (an effect stage or an entity-tag
check follows), and how many records are used later (the stage may stop
early). The output must be correct for any demand.

### Services

When a service is absent, the stage degrades as its command
documentation describes (for example a plain view instead of an outline) and
records a notable entry; it does not fail.

### Environment

Stages read environment variables only through the host of
the context, never from the process directly, so that an embedding
application can give each execution its own environment and tests can
substitute it.

## 9. Cancellation and limits

Cancellation is cooperative. The context holds a cancellation token shared by
all stages of one execution; child tokens are cancelled with their parent and
can also be cancelled on their own (for example one background job). The first
reason wins:

| Reason | Error |
|---|---|
| requested by the caller | `E_CANCELLED` |
| time limit of the program | `E_TIMEOUT` |
| resource limit | `E_LIMIT` |
| engine shutdown | `E_CANCELLED` |

A stage calls the checkpoint of the context in long loops and before blocking
work; the checkpoint fails when the token is cancelled or the deadline has
passed (and then cancels the token with the time-limit reason). Wrapping a
stream with the checked adapter performs the checkpoint before every pull.
Waiting uses the clock of the context and never extends past the deadline. A
virtual clock completes waits immediately, which makes time limits testable
without waiting.

Cancellation during step 5 rolls the transaction back. Cancellation before
step 5 has no effect in the world.

## 10. Events

Events are delivered while a program executes. Every event has its type in
the member `t`:

| `t` | Content |
|---|---|
| `progress` | `command`, `phase`, `done`, `total`, `bytes`, `message` |
| `job.started` | `id`, `label`, `command`, `cwd`, `pid`, `started` |
| `job.output` | `id`, `stream` (`stdout`, `stderr`), `cursor`, `lines`, `bytes` |
| `job.progress` | `id`, `percent`, `line` |
| `job.finished` | `id`, `state` (`exited`, `failed`, `stopped`, `timed_out`), `exit_code`, `signal`, `elapsed`, `output_file`, `changes` |
| `log` | `level`, `message`, `source` |

Every launched program has a job identifier from its start, so the same
events describe a program that finishes in the foreground and one that
continues in the background. The engine adds a sequence number, the time and
the request identifier when it delivers an event to the embedding
application. Emitting an event never blocks the execution for long; event
sinks buffer and deliver events in order.

## 11. Clock and host

The clock provides wall time (nanoseconds since the Unix epoch, UTC), a
monotonic time that includes suspend and never decreases, the readings of all
clocks at one moment (wall time, boot identifier, monotonic time with and
without suspend, used by the journal to measure the age of entries) and
cancellable waiting.

The host provides stable facts (operating system, version, architecture, host
name, account, home and temporary directories, path notation, local time
rules, locale, number of processors) and the environment variables of the
engine process. Values that change while the program runs are read by the
commands that report them.

## 12. Concurrency

The interface is synchronous. Every interface is safe to share between
threads; record streams can be moved between threads. A stage may use up to
`limits.threads` threads for parallel work and must produce its records in a
deterministic order regardless of scheduling.
