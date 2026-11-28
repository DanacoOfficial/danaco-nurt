---
name: unix
description: Uprawnienia, usługi, pakiety i nazwy plików w systemach Linux i macOS.
commands: [attributes, info, find, services, package.install, package.list, create, run, list]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/unix.md -->
<!-- Responsibility:   Documents unix. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# unix {#unix}

## Uprawnienia {#permissions}

`attributes` zmienia tryb, konto i grupę wpisu, czasy oraz atrybuty rozszerzone. `recursive: true` stosuje zmianę do drzewa katalogów. `info` zgłasza tryb i konto ścieżki, więc zmianę można sprawdzić przez `assert` w tym samym programie.

```nurt
attributes "/srv/app/run.sh" mode: "0755"
find "/srv/app" glob: "**/*.sh" |> filter mode != "0755"
```

## Usługi {#services}

W systemie Linux `services` odczytuje jednostki systemd, a `scope: user` wybiera jednostki bieżącego konta zamiast jednostek systemowych. W systemie macOS odczytuje zadania launchd. `start`, `restart`, `enable` i `disable` zmieniają stan usług z wejścia; dla jednostek systemowych wymagają uprawnień administratora (`privilege: elevated`). `start`, `stop`, `enable` i `disable` są cofane przez polecenie przeciwne, a `restart` jest nieodwracalne.

```nurt
services name: "nginx" |> restart
services scope: user
```

## Pakiety {#packages}

`package.install`, `package.remove` i `package.upgrade` wywołują menedżer pakietów systemu. `manager` wybiera `apt`, `dnf`, `pacman` lub `zypper` w systemie Linux oraz `brew` w systemie macOS; `auto` wybiera menedżer znaleziony w systemie. Instalowanie i usuwanie pakietów wymaga uprawnień administratora: należy zapisać `privilege: elevated` (`package.install ["jq"] privilege: elevated`); bez nich polecenie kończy się błędem `E_PRIVILEGE`. `undo` usuwa zainstalowany pakiet albo ponownie instaluje zapisaną wersję.

```nurt
package.list outdated: true manager: apt
package.install ["jq"] manager: brew
```

## Nazwy plików {#file-names}

Systemy plików Linux rozróżniają nazwy różniące się wyłącznie wielkością liter. Domyślny format woluminu systemu macOS tego nie robi i przechowuje nazwy w rozłożonej postaci Unicode. Dwie ścieżki, które wyglądają tak samo, mogą więc być różnymi wpisami w jednym systemie, a tym samym wpisem w innym. `find` z `case_sensitive: false` dopasowuje nazwy bez względu na wielkość liter.

## Dowiązania i programy {#links-and-programs}

`create` z `type: link` i `target` tworzy dowiązanie symboliczne, a `hard: true` tworzy dowiązanie twarde. Skrypt jest uruchamiany przez `run` z nazwanym na liście argumentów interpreterem, więc wynik nie zależy od powłoki.

```nurt
create "/srv/app/current" type: link target: "/srv/app/release-12"
```

## Potoki nazwane i pliki urządzeń {#named-pipes-and-device-files}

`create` z `type: fifo` tworzy potok nazwany, a `type: device` tworzy węzeł urządzenia. Oba są dostępne w systemach Linux i macOS; w systemie Windows kończą się błędem `E_OS_ONLY`, a komunikat dla `fifo` wskazuje przestrzeń nazw `\\.\pipe\` potoków nazwanych systemu Windows. `mode` jest tekstem ósemkowym (wartość domyślna `"0666"`, zmniejszana przez umask). Węzeł urządzenia przyjmuje także `kind: char` lub `kind: block`, `major` i `minor` oraz wymaga `privilege: elevated`; bez uprawnień wywołanie kończy się błędem `E_PRIVILEGE`.

```nurt
create "/srv/app/events.fifo" type: fifo mode: "0660"
create "/dev/app0" type: device kind: char major: 1 minor: 3 privilege: elevated
```

Oba są rejestrowane w dzienniku jako utworzone pliki, więc `undo` je usuwa. `info` i `list` zgłaszają `type: fifo` i `type: device`; węzeł urządzenia ma także `major` i `minor`. Parametry `kind`, `major`, `minor` i `mode` kolidują z innymi typami (`E_PARAM_CONFLICT`), urządzenie bez `kind`, `major` lub `minor` kończy się błędem `E_MISSING_PARAM`, a liczba spoza zakresu 0..4294967295 lub tryb, który nie jest ósemkowy, kończy się błędem `E_INVALID_VALUE`.

## Warunki i kilka zmian {#conditions-and-several-changes}

Skrypt dostosowujący się do systemu używa `when` i `exists`, a zmiany jednego zadania zapisuje przez `steps`, tak aby niepowodzenie w późniejszym kroku przywróciło wcześniejsze. Konstrukcje opisuje [język](../site/language.md); polecenia administrujące kontami, montowaniami, parametrami jądra, zaporą, siecią, zegarem i zasilaniem znajdują się w [administracji](administration.md).

```nurt
when exists "/etc/nginx/nginx.conf" {
    steps {
        replace /worker_processes \d+;/ "worker_processes 4;" in: "/etc/nginx/nginx.conf" expect: 1 privilege: elevated
        services name: "nginx" |> restart privilege: elevated
    }
}
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
