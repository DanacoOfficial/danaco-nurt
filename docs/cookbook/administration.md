---
category: administration
title: Administration, packages and schedules
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Administration, packages and schedules

## List the services

List the services whose name matches a pattern.

```nurt example
services /nginx/
```

```text expected
nginx
```

## Restart a service

Restart a service.

```nurt example
services name: "nginx" |> restart
```

```text expected
nginx
```

```bash
systemctl restart nginx
```

```powershell
Restart-Service nginx
```

## Enable a service at boot

Enable a service so that it starts at boot.

```nurt example
services name: "nginx" |> enable
```

```text expected
nginx
```

## Query WMI classes

Read a CIM class on Windows.

```nurt example
windows.cim "Win32_OperatingSystem" properties: ["Caption", "Version"]
```

```text expected
Caption
```

```powershell
Get-CimInstance -ClassName Win32_OperatingSystem
```

## Read a registry value

Read a value of the Windows registry as a path.

```nurt example
read "registry:/HKCU/Software/Example/Mode"
```

```text expected
Mode
```

## List the installed packages

List the packages that have a newer version.

```nurt example
package.list outdated: true
```

```text expected
package
```

## Search a package

Search the package manager for a program.

```nurt example
package.search "ripgrep"
```

```text expected
ripgrep
```

## Install a package

Install a package with the package manager of the host.

```nurt example
package.install ["ripgrep"]
```

```text expected
ripgrep
```

```bash
apt install ripgrep
```

## Upgrade the packages

Upgrade all packages of the host.

```nurt example
package.upgrade
```

```text expected
package
```

## Schedule a daily job

Run a backup program every day at 03:00.

```nurt example
schedule.create "backup" cron: "0 3 * * *" action: (run ["backup"] cwd: "/home/user")
```

```text expected
backup
```

```bash
(crontab -l; echo '0 3 * * * cd /home/user && backup') | crontab -
```

## List the schedules

List the active schedules.

```nurt example
schedule.list state: active
```

```text expected
backup
```

## Pause a schedule

Pause a schedule without removing it.

```nurt example
schedule.pause "s4"
```

```text expected
s4
```

## Run a schedule now

Run the action of a schedule once, outside its time.

```nurt example
schedule.run "s4"
```

```text expected
s4
```

## Show the history of a schedule

Show the last runs of a schedule.

```nurt example
schedule.history "s4" limit: 10
```

```text expected
s4
```

## Find a program for a task

Search the programs and packages for a task.

```nurt example
program.search "ocr pdf" limit: 3
```

```text expected
ocr
```

## Stop a service

Stop a service and wait for it to exit.

```nurt example
services name: "nginx" |> stop grace: 10s
```

```text expected
nginx
```

```bash
systemctl stop nginx
```

```powershell
Stop-Service nginx
```

## Disable a service at boot

Prevent a service from starting at boot.

```nurt example
services name: "cups" |> disable
```

```text expected
cups
```

```bash
systemctl disable cups
```

## Start a service and wait for it

Start a service and wait up to 30 seconds for it to run.

```nurt example
services name: "postgresql" |> start wait: 30s
```

```text expected
postgresql
```

## List the services of the current user

List the services of the user manager instead of the system manager.

```nurt example
services scope: user
```

```text expected
service
```

```bash
systemctl --user list-units --type=service
```

## Query the running Windows services

Read two properties of the running services through CIM.

```nurt example
windows.cim "Win32_Service" where: "State = 'Running'" properties: ["Name", "State"]
```

```text expected
Name
```

```powershell
Get-CimInstance -ClassName Win32_Service -Filter "State = 'Running'"
```

## Install a package in a given version

Install an exact version of a package with a chosen package manager.

```nurt example
package.install ["nodejs"] version: "20.11.0" manager: apt
```

```text expected
nodejs
```

## Remove a package

Remove a package with the package manager of the host.

```nurt example
package.remove ["ripgrep"]
```

```text expected
ripgrep
```

```bash
apt remove ripgrep
```

## Upgrade selected packages

Upgrade two packages and leave the others unchanged.

```nurt example
package.upgrade ["openssl", "curl"]
```

```text expected
openssl
```

## Schedule a job at an interval

Run a cleanup program every six hours.

```nurt example
schedule.create "cleanup" every: 6h action: (run ["cleanup"] cwd: "/home/user")
```

```text expected
cleanup
```

## Schedule a weekly job with a time zone

Run a report every Monday at 08:00 in a given time zone, at most twelve times.

```nurt example
schedule.create "report" cron: "0 8 * * 1" timezone: "Europe/Warsaw" max_runs: 12 action: (run ["report"] cwd: "/home/user")
```

```text expected
report
```

## Resume a paused schedule

Resume a schedule that was paused.

```nurt example
schedule.resume "s4"
```

```text expected
s4
```

## Remove a schedule

Remove a schedule that is no longer needed.

```nurt example
schedule.remove ["s4"]
```

```text expected
s4
```

## Locate a program

Show where a program is installed and its version.

```nurt example
program.find ["git"] versions: true
```

```text expected
git
```

```bash
command -v git && git --version
```
