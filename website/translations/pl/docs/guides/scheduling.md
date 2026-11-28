---
name: scheduling
description: Uruchamianie potoków według harmonogramu z wyrażeniami cron, interwałami i godzinami.
commands: [schedule.create, schedule.list, schedule.run, schedule.pause, schedule.resume, schedule.remove, schedule.history, run]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/scheduling.md -->
<!-- Responsibility:   Documents scheduling. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# harmonogramy {#scheduling}

## Tworzenie harmonogramu {#creating-a-schedule}

`schedule.create` przyjmuje etykietę i akcję, która jest potokiem, oraz jeden z terminów `cron`, `every` lub `at`. `timezone` wybiera strefę, w której terminy są odczytywane, `max_runs` i `expires` kończą harmonogram, a `enabled: false` tworzy go wstrzymanego. `run_now: true` uruchamia pierwsze wykonanie od razu.

```nurt
schedule.create "backup" cron: "0 3 * * *" action: (run ["backup"] cwd: "/srv")
schedule.create "health" every: 5m action: (http.request "https://example.com/health" |> assert count: 1)
```

## Pominięte uruchomienia i nakładanie się {#missed-runs-and-overlap}

`on_missed` decyduje, co dzieje się z uruchomieniami, które były należne, gdy usługa była zatrzymana: `skip` je odrzuca, a `run_once` wykonuje akcję raz przy starcie. `overlap` decyduje, co dzieje się, gdy uruchomienie startuje, a poprzednie nadal trwa: `skip` odrzuca nowe uruchomienie, `queue` czeka na poprzednie, a `parallel` uruchamia oba.

```nurt
schedule.create "sync" every: 1h action: (run ["sync"] cwd: "/srv") on_missed: run_once overlap: skip
```

## Usługa {#the-service}

Harmonogramy działają tylko wtedy, gdy działa `nurt service`. Gdy nie działa, harmonogram jest przechowywany, a `schedule.list` zgłasza dla niego `active: false`. Następne uruchomienie i stan każdego harmonogramu są polami rekordu `schedule`.

```nurt
schedule.list state: active
```

## Sterowanie harmonogramami {#controlling-schedules}

`schedule.pause` i `schedule.resume` zatrzymują i wznawiają harmonogram bez jego usuwania, `schedule.remove` go usuwa, a `schedule.run` wykonuje akcję teraz, poza jej terminami i limitem uruchomień. Pierwszym argumentem jest `id` lub `all: true` dla każdego harmonogramu.

```nurt
schedule.pause "backup"
schedule.run "backup"
```

## Historia {#history}

`schedule.history` wyświetla minione uruchomienia wraz z ich wynikiem i operacją dziennika dla dokonanych przez nie zmian. Zmianę dokonaną przez zaplanowane uruchomienie można cofnąć jak każdą inną zmianę.

```nurt
schedule.history "backup" limit: 10
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
