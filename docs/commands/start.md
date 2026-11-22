---
command: start
group: admin
summary: Start the input services.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# start

## Synopsis

```text
services ... |> start [wait: <duration>]
```

## Description

`start` asks the service manager to start each input service and waits up to
`wait` until the service settles. The state is read every 100 ms at first;
the interval grows by 100 ms up to 500 ms, and the wait ends when the state is
neither `starting` nor `stopping`. A service that runs reports
`after: "running"`; a service that ran to completion, such as a systemd
`oneshot` unit, reports `after: "stopped"`. The wait observes cancellation and
the time limit of the program. With `wait: 0s` the command returns at once
and `after` holds the state read right after the request, usually `starting`.

Each input service is read before the action. A service that already runs is
not passed to the manager; its effect record has `before` equal to `after` and
the notable code `unchanged`. For every other service the operation is marked
irreversible with `service_changed` before the manager is asked to act, so the
effect is reported even when the wait fails. Services are processed in input
order; an error stops the processing of the remaining services.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `wait` | `duration` | `10s` | Time to wait for each service to settle; `0s` returns at once. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `services ... \|> start` | `service` | `effect` |

Each `effect` record holds `kind: "start"`, `target` (`service:<name>`),
`action`, `before` and `after`. A preview shows the same records with
`status: planned` and the expected `after`.

## Effect

| Property | Value |
|---|---|
| Category | `run` |
| Effect | `external` |
| Reversibility | `irreversible`: the journal keeps no previous state of a service; the report lists `service_changed` |
| External programs | `launchctl` on macOS only |
| Waits | yes |

## Examples

Start one service:

```nurt example
services name: "cron" |> start
```

Start the stopped services of the current account without waiting:

```nurt example
services scope: user state: "stopped" |> start wait: 0s
```

Start a service and give it a minute to come up:

```nurt example
services name: "postgresql" |> start wait: 1m
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_TIMEOUT` | The service did not settle within `wait` or the time limit of the program. `expected` holds `running`, `actual` the current state; the effect is already applied. | Give a longer `wait`. |
| `E_SERVICE` | The service entered the `failed` state; `os_code` and `result` hold the reason given by the manager and `next_steps` holds `logs unit: "<name>"`. The manager may also report another error, for example a masked unit, in `os_code` and `os_message`. | Read the log of the service and correct its configuration. |
| `E_NOT_FOUND` | The service no longer exists. | Read the services again with `services`. |
| `E_PRIVILEGE` | The manager refused the action; `required` names the privilege, `root` on Linux and `administrator` on Windows. Interactive authorization is never requested. | Run as an account with that privilege. |
| `E_UNSUPPORTED` | The record comes from the manager of another system. | Read the services on this system with `services`. |
| `E_INVALID_VALUE` | `wait` is negative. | Give a non-negative duration such as `10s`. |
| `E_CANCELLED` | The execution was cancelled. | None; the cancellation was requested by the caller. |

## Related

`services` produces the input records. `restart` restarts services and `stop`
stops them. `logs unit:` shows why a service failed. The `admin` group page
lists the other commands of the group.
