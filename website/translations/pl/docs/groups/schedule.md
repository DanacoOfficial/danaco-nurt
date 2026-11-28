---
group: schedule
summary: Harmonogramy potoków.
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Grupa schedule

## Zakres {#scope}

Grupa tworzy harmonogramy i zarządza nimi: programami uruchamianymi o
ustalonych porach bez wywołującego. Jedna składnia zastępuje cron, timery
systemd, zadania launchd i Harmonogram zadań systemu Windows. Akcją
harmonogramu jest program w składni Danaco Nurt, podany jako potok w
parametrze `action`.

Harmonogramy działają w usłudze w tle instancji (`nurt-service`). Uruchomienie
jest zwykłą operacją: ma wpis w dzienniku operacji, a skutki jego programu są
zgłaszane tak jak skutki programu uruchomionego przez wywołującego. Grupa tylko
definiuje harmonogramy i je odpytuje; wykonuje je usługa.

| Zadanie | Polecenie | Grupa |
|---|---|---|
| Uruchomienie lub zatrzymanie usługi w tle | `nurt service` | wiersz poleceń |
| Jednorazowe uruchomienie programu teraz | `run` | core |
| Cofnięcie skutku zaplanowanego uruchomienia | `undo` | core |
| Wylistowanie operacji dziennika | `history` | core |

## Polecenia {#commands}

| Polecenie | Kategoria | Wejście i wyjście | Odwracalność | Strona |
|---|---|---|---|---|
| `schedule.create` | change | brak do `schedule` | reversible | [schedule.create](../commands/schedule.create.md) |
| `schedule.list` | source | brak do `schedule` | read | [schedule.list](../commands/schedule.list.md) |
| `schedule.remove` | change | brak do `effect` | reversible | [schedule.remove](../commands/schedule.remove.md) |
| `schedule.pause` | change | brak do `effect` | reversible | [schedule.pause](../commands/schedule.pause.md) |
| `schedule.resume` | change | brak do `effect` | reversible | [schedule.resume](../commands/schedule.resume.md) |
| `schedule.run` | run | brak do `exec` lub `job` | reversible if preserved | [schedule.run](../commands/schedule.run.md) |
| `schedule.history` | source | brak do `schedule_run` | read | [schedule.history](../commands/schedule.history.md) |

## Rekordy {#records}

| Rekord | Wymagane pola |
|---|---|
| `schedule` | `id`, `trigger`, `next_run`, `active`, `action` |
| `schedule_run` | `schedule_id`, `started`, `exit_state`, `trigger_time` |
| `effect` | `status`, `kind` |
| `exec` | `command`, `program`, `cwd`, `pid`, `exit_code`, `started`, `elapsed` |

Pole `trigger` jest obiektem z `kind` (`cron`, `every` lub `at`),
`expression` (wyzwalacz w zapisanej postaci) i `timezone`. Pole `action`
zawiera kanoniczny tekst programu. Wartościami `exit_state` są `ok`,
`failed`, `timed_out`, `skipped_overlap`, `missed` i `cancelled`; pole
`operation` uruchomienia jest numerem jego operacji dziennika.

## Wyzwalacze i czas {#triggers-and-time}

`schedule.create` przyjmuje etykietę jako pierwszy argument i dokładnie jeden
wyzwalacz:

| Parametr | Wyzwalacz |
|---|---|
| `cron` | pięciopolowe wyrażenie cron lub `@daily` |
| `every` | interwał od `30s` do 366 dni |
| `at` | jeden punkt w czasie |

Pierwsze uruchomienie interwału następuje jeden interwał po utworzeniu.
Późniejsze uruchomienie następuje jeden interwał po poprzednim planowanym
czasie; gdy ten czas już minął, jest to jeden interwał po bieżącym czasie.
Planowany czas jest zawsze późniejszy niż czas odniesienia.

`timezone` ma wartość domyślną `local`, czyli strefę komputera w chwili
utworzenia, zapisaną jako nazwa IANA. Późniejsza zmiana strefy systemu nie
przesuwa harmonogramu. Czas, który nie istnieje przy przejściu na czas letni,
jest uruchamiany w pierwszej istniejącej chwili po przejściu. Czas, który
występuje dwukrotnie przy przejściu na czas zimowy, jest uruchamiany raz, przy
pierwszym wystąpieniu.

## Pominięte uruchomienia i nakładanie się {#missed-runs-and-overlap}

Usługa w tle działa tylko wtedy, gdy działają komputer i usługa. Planowanym
czasem, który minął, gdy nie działały, zajmuje się `on_missed`:

| Wartość | Zachowanie |
|---|---|
| `skip` | Uruchomienie jest pomijane, gdy opóźnienie przekracza `misfire` (wartość domyślna `2m`); następne uruchomienie jest planowane od bieżącego czasu. |
| `run_once` | Wykonywane jest dokładnie jedno uruchomienie, niezależnie od liczby pominiętych czasów. |

`overlap` rozstrzyga, co się dzieje, gdy planowany czas nadejdzie, a
poprzednie uruchomienie nadal trwa:

| Wartość | Zachowanie |
|---|---|
| `skip` | Uruchomienie jest pomijane i liczone jako pominięte; wartość domyślna. |
| `queue` | Jedno uruchomienie jest zachowywane na później; dalsze czasy są liczone jako pominięte. |
| `parallel` | Uruchomienie startuje natychmiast, równolegle z poprzednim. |

`max_runs` kończy harmonogram po tylu uruchomieniach, a `expires` kończy go o
określonej porze lub po upływie czasu trwania. Zakończony harmonogram ma stan
`completed` lub `expired`.

`timeout` (wartość domyślna `1h`) ogranicza cały program akcji i jest
domyślnym `timeout` jej etapów `run`, które go nie ustawiają. Bez tej reguły
domyślny limit `run` zatrzymałby akcję wcześniej, niż dopuszcza harmonogram.

`schedule.run` uruchamia akcję od razu, poza planowanymi czasami. Nie zmienia
`next_run` i nie liczy się do `max_runs`.

## Różnice między platformami {#platform-differences}

Harmonogram jest taki sam we wszystkich systemach. Różni się usługa:

| Aspekt | Linux | macOS | Windows |
|---|---|---|---|
| Menedżer usługi w tle | systemd | launchd | menedżer sterowania usługami |
| Usługa w trybie użytkownika | działa dla konta; po wylogowaniu działa nadal tylko wtedy, gdy dla konta włączono linger | działa dla konta | działa dla konta |
| Usługa w trybie systemowym | działa dla komputera | działa dla komputera | działa dla komputera |

Tryb instalacji usługi to ustawienie `service.mode`, domyślnie `user`, a dla
serwerów `system`. Bez działającej usługi harmonogram jest zapisany, ale nie
jest uruchamiany: `schedule.create` zwraca rekord z `active: false` i
`reason: service_not_running`; niepowodzenie jest zgłaszane, a nie ukrywane.

## Programy zewnętrzne {#external-programs}

Żadne polecenie grupy nie uruchamia programu. Grupa nie ma znacznika [X], [P]
ani [U]. W systemie macOS usługa jest rejestrowana w launchd przez
`nurt service`, poza tymi poleceniami.

## Przykłady {#examples}

Utworzenie harmonogramu, który uruchamia program kopii zapasowej co noc o
02:00:

```nurt example
schedule.create "backup" cron: "0 2 * * *" action: (run ["backup"] cwd: "/home/user")
```

Sprawdzanie adresu co pięć minut z uruchomieniem ograniczonym do jednej
minuty:

```nurt example
schedule.create "health" every: 5m timeout: 1m action: (http.request "https://example.com/health" |> assert all: status < 400)
```

Wylistowanie aktywnych harmonogramów z ich następnym uruchomieniem:

```nurt example
schedule.list state: active |> select id label next_run
```

Utworzenie harmonogramu, jego wstrzymanie i wznowienie:

```nurt example
schedule.create "report" every: 1h action: (run ["report"] cwd: "/home/user")
schedule.pause "s1"
schedule.resume "s1"
```

Utworzenie harmonogramu i jednorazowe natychmiastowe uruchomienie jego akcji:

```nurt example
schedule.create "pause" every: 10m action: (sleep 2s)
schedule.run "s1"
```

Pokazanie nieudanych uruchomień wszystkich harmonogramów:

```nurt example
schedule.history limit: 10 |> filter exit_state == "failed"
```

## Powiązane {#related}

[Grupa network](network.md) udostępnia żądania, które harmonogramy zwykle
uruchamiają. [Grupa system](system.md) udostępnia `sleep`.
