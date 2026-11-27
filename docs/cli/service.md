---
command: service
group: schedule
summary: Run, install, remove and inspect the background service.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# service

## Synopsis

```text
nurt service run
nurt service install [--mode user|system]
nurt service uninstall [--mode user|system]
nurt service status
```

## Description

The privileged elevation service (`nurt service install --elevation`, `uninstall --elevation`, and its line in `nurt service status`) is not part of release 0.1.1; until it is released the elevation backend `broker` is reported as `not_installed` and elevated calls use `sudo`, `doas` or `run0`.

`service` is a subcommand of the `nurt` program, not a command of the
pipeline language. The background service hosts the free space module of the
journal and the schedule module. The journal keeps the previous content of
changed files only while the service runs, and schedules fire only while the
service runs; without it a schedule reports `active: false` with `reason:
service_not_running`.

`run` starts the service in the foreground. It takes the instance lock
`service.lock` in the state directory, removes a control socket left by a
previous service, opens the control channel, starts every module on its own
thread and writes the heartbeat `service.hb` every `space.heartbeat_ms`
milliseconds, at least once per second. A module that fails is restarted
after 1 s, 2 s, 4 s and so on up to 60 s; after five restarts within ten
minutes it stays in state `failed` until the configuration is reloaded. A
failure of one module does not stop the other. `SIGTERM` and `SIGINT` stop
the service, `SIGHUP` reloads the configuration. On Windows the service
control manager stops the service with `Stop` or `Shutdown`.

`install` registers the service with the service manager of the operating
system, starts it and writes the installation manifest
`service.install.json` to the state directory. `uninstall` stops the
service, removes the registration and deletes exactly the files listed in
the manifest.

| System | Mode `user` | Mode `system` |
|---|---|---|
| Linux | systemd user unit `nurt-service.service` in the unit directory of the user manager | systemd unit `/etc/systemd/system/nurt-service.service` with `User=` and `Group=` of the service account |
| macOS | launchd agent `nurt.service` in `$HOME/Library/LaunchAgents` | launchd daemon `/Library/LaunchDaemons/nurt.service.plist` with `UserName` |
| Windows | task `Nurt Service` of the Task Scheduler, started at logon | service `NurtService` of the service control manager, started at boot |

In mode `user` on Linux the unit file is written to the directory that the systemd user manager itself reads: the first directory `user` inside a directory `systemd` of the property `UnitPath` of the manager (read over D-Bus) outside `/etc`, `/run`, `/usr` and `/lib`, the roots of the directories of the system, of the runtime and of the vendors. The manager may run with another `HOME` than the account that runs `install`, and a unit written under the `HOME` of the installing session would be invisible to it (`NoSuchUnit`). Only when the manager cannot be reached or reports no such directory, the unit is written to `$XDG_CONFIG_HOME/systemd/user` (by default `$HOME/.config/systemd/user`). The unit carries `Environment="HOME=<home>"`, and `Environment="XDG_STATE_HOME=<dir>"` when the installing session sets it, so that the service uses the state directory its clients use, whatever `HOME` the manager has.

`ExecStart` names the program by the path it was started with (`nurt` found in `PATH`, a relative path made absolute, an absolute path), with links kept, as long as that path leads to the running program; a link such as `current` or `bin/nurt` thus keeps pointing at the installed version after an update. Only when the path cannot be determined or leads to another file, the resolved file of the running process is written. The path stays in the manifest as `executable`.

Without `--mode` the mode is chosen from the host. An account without
administrator rights gets mode `user`. With administrator rights, a host
without a graphical session whose default systemd target is
`multi-user.target`, an account whose login shell is `nologin` or `false`,
and a Windows Server edition get mode `system`; other hosts get mode
`user`. The result names the reason of the choice.

### Lingering and login sessions

In mode `user` on Linux the systemd user manager runs only while the account
has a login session, unless lingering is enabled for the account. `install`
enables lingering through `org.freedesktop.login1.Manager.SetUserLinger` and
reports the result in `linger`. When the system refuses the request, for
example because the polkit policy requires an administrator, the
installation still succeeds with `linger: false` and a note: the service
starts when the account logs in, and until then the journal keeps no
previous content and schedules report `service_not_running`. Mode `system`
does not depend on login sessions.

When the systemd user manager is not reachable at all (no
`XDG_RUNTIME_DIR`, a session without `pam_systemd`, a container), `install`
writes the unit file and fails with `E_SERVICE`. Log in with a session that
runs the user manager and repeat the command, or install in mode `system`.

On macOS without a graphical session the agent is loaded into the
background domain `user/<uid>` and starts again at the next login.

### Liveness

`status` and every process that depends on the service apply the same
rule. The service runs when `service.hb` exists, its `beat` is at most
`space.heartbeat_max_age_ms` old (2000 ms by default), the process `pid` of
the heartbeat exists, and `ping` on the control channel answers within
50 ms.

### Control channel

The control channel is the Unix socket `service.sock` in the state
directory (mode `0600`, directory `0700`) or, on Windows, the named pipe
`\\.\pipe\nurt-service-<SID>` that admits only the account and `SYSTEM`.
Every message is one JSON object on one line of at most 65536 bytes.

| Message | Form |
|---|---|
| request | `{"v":1,"id":<id>,"op":<op>,"body":{...}}`; `id` and `body` are optional |
| success | `{"v":1,"id":<id>,"ok":true,"result":{...}}` |
| failure | `{"v":1,"id":<id>,"ok":false,"error":{"code":...,"message":...}}` |
| event | `{"v":1,"event":<name>,"data":{...}}` |

| Operation | Result |
|---|---|
| `ping` | `{"pid"}`; answered without waiting for any module |
| `status` | `{"pid","started","version","protocol","modules","subscribers"}` |
| `reload` | `{"modules":{<name>:"reloaded"\|"restarted"\|{"error"}}}` |
| `subscribe` | `{"subscribed":true}`; the connection receives events addressed to all |
| `<module>.<op>`, `<module>/<op>` | reply of the module, for example `schedule.run` or `schedule/ack` |

Protocol errors are `bad_request`, `unknown_op` (with `ops`),
`unsupported_version` (with `supported`), `line_too_long` (the connection is
closed) and `module_unavailable`. Errors of a module carry `E_` codes. A
connection that sends nothing for 30 seconds is closed unless it subscribed
or registered a remote action kind.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `--mode` | `user` \| `system` | `install`: chosen from the host; `uninstall`: the installed mode | Installation mode. Mode `system` requires administrator rights. |

Configuration keys read by `run`: `service.mode`, `space.heartbeat_ms`
(default `500`), `space.heartbeat_max_age_ms` (default `2000`), `state.dir`.
Keys `service.*` in the `.nurt.toml` file of a project are ignored.

## Input and output

The subcommand reads no records. `run` prints nothing while the service
runs. The other forms print one object.

| Form | Output |
|---|---|
| `install` | `{mode, mode_reason, backend, files, started, linger, notes}` |
| `uninstall` | `{mode, backend, removed, notes}` |
| `status` | `{installed, mode, running, reason, pid, started, version, linger, heartbeat_age_ms, modules}` |

`mode_reason` is `requested`, `no_privilege`, `server` or `desktop`.
`backend` is `systemd_user`, `systemd_system`, `launch_agent`,
`launch_daemon`, `task_scheduler` or `windows_service`. In `status`,
`reason` is `no_heartbeat`, `stale_heartbeat`, `process_gone` or
`no_response` when the service does not run; `modules` maps `space` and
`schedule` to `starting`, `running`, `degraded`, `restarting`, `failed` or
`stopped`.

## Effect

`run` writes only `service.lock`, `service.hb`, `service.sock` and, through
the schedule module, `schedules/state.json` in the state directory. These
files do not pass through the journal. Schedule definitions in
`schedules/definitions/` are changed only by the console.

`install` and `uninstall` change the configuration of the service manager.
On Linux they use D-Bus (`org.freedesktop.systemd1`,
`org.freedesktop.login1`), on Windows the service control manager and the
Task Scheduler. On macOS they run the external program `launchctl`
(`bootstrap` and `bootout`) with a time limit of ten seconds.

## Examples

Show the state of the service installed in mode `user` on Linux:

```nurt example
services name: "nurt-service" scope: user
```

List the active schedules that do not fire because the service does not
run:

```nurt example
schedule.list state: active |> filter reason == "service_not_running" |> select label reason
```

Show the errors logged by the service unit:

```nurt example
logs unit: "nurt-service" level: error
```

Install the service for the current account from the command line:

```sh
nurt service install --mode user
```

```text
mode: user  mode_reason: requested  backend: systemd_user  started: true  linger: true
```

Show whether the service is installed and running:

```sh
nurt service status
```

```text
installed: true  mode: user  running: true  linger: true  modules: {schedule: running, space: running}
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_EXISTS` | `run`: the service already runs for this state directory; `details.pid` names it | stop the running service first |
| `E_PRIVILEGE` | mode `system` without administrator rights; `details.required` is `administrator` | run the command as an administrator or use mode `user` |
| `E_SERVICE` | the service manager refused the request or is not reachable | follow the step named in the message, for example log in with a session or use mode `system` |
| `E_EXTERNAL_MISSING` | macOS: `launchctl` is not available; `details.program` is `launchctl` | restore the system program |
| `E_NOT_FOUND` | `uninstall`: there is no installation manifest | the service is not installed |
| `E_INVALID_VALUE` | `uninstall --mode` names another mode than the installed one, or two modules have the same name | use the installed mode |
| `E_IO` | a file of the state directory or of the service manager cannot be written | check the permissions of the directory in `details.path` |

## Related

Configuration keys `service.mode`, `space.heartbeat_ms`,
`space.heartbeat_max_age_ms` and `state.dir`; the commands of the `schedule`
group; `services`, `logs`.
