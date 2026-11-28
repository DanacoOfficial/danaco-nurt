---
command: schedule.create
group: schedule
summary: Tworzy harmonogram uruchamiający potok według wyrażenia cron, interwału lub czasu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# schedule.create

## Składnia {#synopsis}

```text
schedule.create <label> action: <pipeline> (cron: <text> | every: <duration> | at: <instant>) [name: value]...
```

## Opis {#description}

`schedule.create` zapisuje harmonogram i zwraca jego rekord `schedule`. Harmonogram uruchamia swoją akcję w terminach uruchomienia dokładnie jednego wyzwalacza:

- `cron` przyjmuje pięć pól: minutę (0-59), godzinę (0-23), dzień miesiąca (1-31), miesiąc (1-12 lub `jan`-`dec`) i dzień tygodnia (0-7 lub `sun`-`sat`, gdzie 0 i 7 oznaczają niedzielę). Pole zawiera `*`, liczby, nazwy, zakresy `a-b`, listy `a,b` i kroki: `*/n` od dolnej granicy, `a-b/n` w obrębie zakresu i `a/n` od `a` do końca zakresu. W nazwach wielkość liter nie ma znaczenia. Gdy ani dzień miesiąca, ani dzień tygodnia nie zaczyna się od `*`, dzień pasuje, gdy pasuje którekolwiek z pól; w przeciwnym razie muszą pasować oba. Akceptowane są skróty `@yearly`, `@annually`, `@monthly`, `@weekly`, `@daily` i `@hourly`. Wyrażenie bez terminu uruchomienia w ciągu pięciu lat jest odrzucane.
- `every` uruchamia w stałym interwale. Pierwszy termin uruchomienia to czas utworzenia powiększony o interwał. Interwał mieści się między `schedule.min_interval` (wartość domyślna `30s`) a `schedule.max_interval` (wartość domyślna `366d`).
- `at` uruchamia jednokrotnie o podanym czasie, który musi leżeć w przyszłości.

Każdy termin uruchomienia jest ściśle późniejszy niż czas, od którego jest obliczany: harmonogram utworzony o 14:40:00 z `cron: "*/10 * * * *"` uruchamia się po raz pierwszy o 14:50.

`timezone` dotyczy pól wyrażenia cron i czasów wyświetlanych przez `schedule.list`. Wartość `local` jest zastępowana nazwą IANA strefy czasowej urządzenia w chwili utworzenia, więc późniejsza zmiana strefy czasowej urządzenia nie przesuwa terminów uruchomienia. Czas zegarowy pominięty przez przejście na czas letni jest uruchamiany w tej samej odległości po zmianie (02:30 staje się 03:30); czas zegarowy powtórzony przez powrót do czasu zimowego jest uruchamiany jednokrotnie, przy pierwszym wystąpieniu.

Akcja jest potokiem języka. Jest wiązana i sprawdzana podczas tworzenia harmonogramu oraz zapisywana wraz ze skrótem integralności. Każde uruchomienie wykonuje potok jako operację sesji `schedule:<id>` z katalogiem bazowym i katalogami projektu wywołania tworzącego, więc każde uruchomienie ma własny wpis w dzienniku operacji. Środowiskiem programów akcji jest środowisko usługi w tle wraz z `run.env_defaults`; zmienne sesji wywołującej są przekazywane jawnie parametrem `env` polecenia `run`. `timeout` ogranicza całe uruchomienie i jest limitem czasu etapów `run` akcji, które nie ustawiają żadnego. Definicja, której akcja nie zgadza się już ze skrótem integralności, nie jest uruchamiana: uruchomienie jest zapisywane z `state: rejected` i `reason: definition_changed`.

Harmonogramy działają w usłudze w tle konta. Gdy usługa nie działa, harmonogram jest zapisywany, a rekord zawiera `active: false` i `reason: service_not_running`; usługa stosuje harmonogram po uruchomieniu. Z `run_now: true` i działającą usługą akcja jest dodatkowo uruchamiana jednokrotnie przy tworzeniu, poza terminami uruchomienia i limitem uruchomień; tworzenie nie czeka na to uruchomienie, a jego rekord pojawia się w `schedule.history`.

Termin uruchomienia osiągnięty później niż `misfire` po jego nastaniu nie jest nadrabiany pojedynczo: `on_missed: skip` zapisuje jedno pominięte uruchomienie (`reason: misfire`), a `on_missed: run_once` wykonuje dokładnie jedno uruchomienie (`trigger: missed`); w obu przypadkach następny termin uruchomienia jest liczony od chwili obecnej. Pojedynczy czas (`at`) pominięty w ten sposób powoduje, że harmonogram staje się `expired`. Termin uruchomienia osiągnięty, gdy trwa uruchomienie harmonogramu, jest pomijany (`overlap: skip`), czeka w kolejce o jednym miejscu (`overlap: queue`) albo również jest uruchamiany (`overlap: parallel`, w granicach `schedule.max_parallel`).

Identyfikatory to `s1`, `s2` i tak dalej. Liczba harmonogramów, które nie są zakończone ani wygasłe, wraz z wstrzymanymi, jest ograniczona przez `schedule.max_active` (wartość domyślna 50).

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `label` | `text` | wymagany | Nazwa harmonogramu. |
| `action` | `pipeline` | wymagany | Potok do uruchomienia. |
| `cron` | `text` | jeden z `trigger` | Pięciopolowe wyrażenie cron lub skrót, taki jak @daily. |
| `every` | `duration` | jeden z `trigger` | Interwał między uruchomieniami; co najmniej skonfigurowane minimum. |
| `at` | `instant` | jeden z `trigger` | Pojedyncze uruchomienie o tym czasie. |
| `timezone` | `text` | `"local"` | Strefa czasowa wyrażenia cron i czasów. |
| `on_missed` | `enum(skip\|run_once)` | `skip` | Zachowanie dla uruchomień pominiętych, gdy usługa nie działała. |
| `misfire` | `duration` | `2m` | Maksymalne opóźnienie, po którym pominięte uruchomienie nie jest nadrabiane. |
| `overlap` | `enum(skip\|queue\|parallel)` | `skip` | Zachowanie, gdy uruchomienie jest należne, a poprzednie nadal trwa. |
| `timeout` | `duration` | `1h` | Limit czasu każdego uruchomienia. |
| `max_runs` | `int` | none | Liczba uruchomień, po której harmonogram się kończy. |
| `expires` | `instant \| duration` | none | Czas lub czas od teraz, po którym harmonogram wygasa. |
| `run_now` | `bool` | `false` | Dodatkowo uruchamia jednokrotnie natychmiast. |
| `enabled` | `bool` | `true` | Tworzy harmonogram jako aktywny, a nie wstrzymany. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `schedule` |

Rekord zawiera `id`, `label`, `trigger` (`kind`, `expression`, `timezone`), `next_run`, `state`, `active`, `reason`, `action`, `action_kind`, `runs`, `max_runs`, `expires`, `on_missed`, `misfire`, `overlap`, `timeout`, `created` i `session`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

Plik definicji jest zapisywany przez dziennik operacji. Stan uruchomień (`<state>/schedules/state.json`: następne terminy uruchomienia, liczniki i historia) jest zapisywany wyłącznie przez usługę w tle i nie jest częścią dziennika.

## Przykłady {#examples}

Uruchamianie kopii zapasowej codziennie o 03:00 w strefie czasowej urządzenia:

```nurt example
schedule.create "backup" cron: "0 3 * * *" action: (run ["tar", "-czf", "/srv/backup.tgz", "data"] cwd: "/srv")
```

Sprawdzanie katalogu co dziesięć minut z co najwyżej jednym oczekującym uruchomieniem:

```nurt example
schedule.create "inbox" every: 10m overlap: queue action: (run ["./process-inbox.sh"] cwd: "/srv/inbox")
```

Jednokrotne uruchomienie o podanym czasie i nadrobienie uruchomienia, jeśli urządzenie w tym czasie śpi:

```nurt example
schedule.create "reminder" at: "2026-12-24T18:00:00+01:00" on_missed: run_once action: (run ["notify-send", "Stand-up"] cwd: "/tmp")
```

Uruchamianie w dni robocze między 09:00 a 17:45 co kwadrans, najwyżej dwadzieścia razy:

```nurt example
schedule.create "probe" cron: "*/15 9-17 * * mon-fri" timezone: "Europe/Warsaw" max_runs: 20 action: (run ["./probe"] cwd: "/opt/probe")
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Brakuje `label` albo nie podano żadnego z `cron`, `every` i `at`. | Podaj etykietę i dokładnie jeden wyzwalacz. |
| `E_PARAM_CONFLICT` | Podano więcej niż jedno z `cron`, `every` i `at`. `expected` wyświetla parametry grupy. | Zachowaj jeden wyzwalacz. |
| `E_INVALID_VALUE` | Wyrażenie cron jest niepoprawne lub nie ma terminu uruchomienia w ciągu pięciu lat, interwał leży poza skonfigurowanymi granicami, `at` lub `expires` minęło, strefa czasowa jest nieznana, `timeout` przekracza `schedule.max_timeout` albo `max_runs` jest mniejsze niż 1. | Popraw wartość wskazaną w komunikacie; `fix` zawiera poprawną postać, jeśli istnieje. |
| `E_UNKNOWN_VALUE` | `on_missed` lub `overlap` ma nieznaną wartość. | Użyj jednej z wymienionych wartości. |
| `E_TYPE` | Parametr ma niewłaściwy typ, na przykład liczbę dla `every`. | Podaj wartość wymienionego typu, taką jak `10m`. |
| `E_LIMIT` | Istnieje `schedule.max_active` harmonogramów. | Usuń harmonogramy poleceniem `schedule.remove` lub podnieś limit. |
| `E_IO` | Magazynu harmonogramów nie można odczytać ani zapisać. | Sprawdź uprawnienia katalogu stanu. |

## Powiązane {#related}

`schedule.list` wyświetla harmonogramy i ich następne terminy uruchomienia, `schedule.run` uruchamia akcję poza jej terminami uruchomienia, `schedule.pause` i `schedule.resume` wstrzymują i wznawiają harmonogramy, `schedule.remove` je usuwa, a `schedule.history` wyświetla minione uruchomienia.
