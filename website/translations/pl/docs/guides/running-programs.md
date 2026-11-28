---
name: running-programs
description: Uruchamianie programów zewnętrznych, oczekiwanie na nie, odczyt ich wyjścia i rejestrowanie ich zmian.
commands: [run, jobs, stop, retry, assert, sleep]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/running-programs.md -->
<!-- Responsibility:   Documents running-programs. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# uruchamianie programów {#running-programs}

## Uruchomienie programu {#starting-a-program}

`run` uruchamia program bez powłoki. Pierwszy argument jest listą: program, po którym następują jego argumenty, więc nie obowiązują żadne reguły cytowania powłoki. `cwd` ustawia katalog roboczy, `env` dodaje zmienne, a `inherit_env: none` uruchamia program z pustym środowiskiem. Program, który nie istnieje, kończy się błędem `E_PROGRAM_NOT_FOUND`.

```nurt
run ["go", "test", "./..."] cwd: "/srv/app" |> assert exit_code: 0
```

## Oczekiwanie, limity i zadania w tle {#waiting-limits-and-background-jobs}

Wartością domyślną jest, że `run` czeka na program. `timeout` zatrzymuje program, który działa dłużej, a `idle_timeout` zatrzymuje program, który przez podany czas niczego nie zapisuje. Z `background: true` wywołanie od razu zwraca zadanie, a `jobs` odczytuje później jego stan, wyjście lub wynik. `stop` kończy zadanie, a `retry` uruchamia je ponownie z tym samym katalogiem i środowiskiem.

```nurt
run ["make", "all"] cwd: "/srv/app" background: true
jobs state: running
```

## Wyjście {#output}

`show` wybiera, co zawiera wynik: `stdout`, `stderr`, `both`, `errors`, `summary` lub `none`. `head` i `tail` zachowują pierwsze lub ostatnie wiersze, `filter` zachowuje wiersze pasujące do wzorca, a `dedupe` (domyślnie włączone) zwija powtarzające się wiersze, także wiersze różniące się tylko liczbami. Tekst wszystkich strumieni razem nigdy nie przekracza `max_bytes` (domyślnie 4096, `0` oznacza brak limitu), a wiersz dłuższy niż 400 znaków jest obcinany. Po kodzie zakończenia 0 wynik zawiera ostatnie 5 wierszy i pole `digest`; po niepowodzeniu `cargo` lub `pytest` pole `diagnostics` wymienia pierwsze błędy wraz z plikiem i wierszem, a odpowiedź, która pomija wiersze, kończy się wskazówką "more: next": `next` zwraca pominięte wiersze z pliku wyjścia bez ponownego uruchamiania programu. Rekord `exec` zawiera `exit_code`, `signal` i `timed_out`.

```nurt
run ["npm", "run", "build"] cwd: "/srv/app" show: errors tail: 40
```

## Kody zakończenia {#exit-codes}

Program, który kończy się kodem niezerowym, nie jest błędem `run`: kod jest polem wyniku. `assert exit_code: 0` zamienia nieoczekiwany kod w `E_ASSERT`, a `E_TIMEOUT` zgłasza program zatrzymany przez swój limit.

## Wykrywanie zmian {#detecting-changes}

Pliki, które program zmienia, są wykrywane tylko w ścieżkach wskazanych przez `watch`; wartością domyślną jest katalog roboczy programu. `track_changes` wybiera `scan`, `events`, `auto` lub `none`. Zmiana dokonana przez program jest rejestrowana w dzienniku, a gdy jej poprzedniego stanu nie udało się zachować, jest oznaczana jako nieodwracalna.

```nurt
run ["./build.sh"] cwd: "/srv/app" watch: ["/srv/app/dist"] track_changes: scan
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
