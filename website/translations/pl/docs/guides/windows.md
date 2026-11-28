---
name: windows
description: Ścieżki, rejestr, dowiązania, usługi i kodowania w systemie Windows.
commands: [list, read, write, create, convert, services, run, package.install, windows.cim, info]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/windows.md -->
<!-- Responsibility:   Documents windows. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# windows {#windows}

## Ścieżki {#paths}

Ścieżka jest cytowanym tekstem bezwzględnym z `/` jako separatorem w każdym systemie: `"C:/proj/app"`. Ukośnik odwrotny nie jest separatorem w ścieżce Nurt. Ścieżki względne są dozwolone tylko z parametrem wywołania `base`. Ścieżka, która jest poprawna w innym systemie, ale nie w bieżącym, kończy się błędem `E_PATH_FOREIGN`.

```nurt
list "C:/Users/dev" depth: 1
info "C:/proj/app/readme.txt"
```

## Nazwy zarezerwowane i pliki w użyciu {#reserved-names-and-files-in-use}

Nazwy takie jak `CON`, `NUL` i `COM1` są zarezerwowane przez system i nie można ich utworzyć. Pliku, który inny proces trzyma otwarty, nie można zastąpić ani usunąć: polecenie kończy się błędem `E_IN_USE`, a `open_files` wskazuje proces, który trzyma plik.

## Dowiązania i rejestr {#links-and-the-registry}

`create` z `type: junction` i `target` tworzy punkt połączenia (junction), a `type: link` tworzy dowiązanie symboliczne. Rejestr jest adresowany jako ścieżka z prefiksem `registry:` i działa w `list`, `find`, `read`, `write`, `remove`, `copy` i `move`.

```nurt
create "C:/proj/current" type: junction target: "C:/proj/release-12"
read "registry:/HKCU/Software/Example"
```

## Usługi i pakiety {#services-and-packages}

`services` wyświetla usługi wraz z ich stanem, a `start`, `restart`, `enable` i `disable` je zmieniają. `package.install` z `manager: winget`, `choco` lub `scoop` instaluje pakiety za pośrednictwem menedżera pakietów; instalacja jest rejestrowana jako nieodwracalna. `windows.cim` odpytuje klasy CIM.

```nurt
services name: "Spooler" |> restart
package.install ["jq"] manager: winget
windows.cim "Win32_LogicalDisk"
```

## Kodowania i interpretery poleceń {#encodings-and-command-interpreters}

Pliki w stronach kodowych takich jak 852 lub 1250 są odczytywane z `encoding` i konwertowane przez `convert`. Programy interpretera poleceń lub PowerShell uruchamia się przez `run` z listą argumentów, na przykład `run ["powershell", "-NoProfile", "-File", "C:/ops/task.ps1"]`.

```nurt
convert "C:/legacy/readme.txt" encoding: "utf-8" line_ending: crlf
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
