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
