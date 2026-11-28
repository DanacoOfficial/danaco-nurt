---
command: clock.set
group: administration
summary: Ustawia datę i godzinę zegara.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# clock.set

## Składnia {#synopsis}

```text
clock.set <time>
```

## Opis {#description}

`clock.set` ustawia datę i godzinę zegara na moment w czasie UTC: `date -u -s` w systemie Linux, `date -u` w systemie macOS i `Set-Date` w systemie Windows. Ustawienie czasu jest nieodwracalne: poprzedni odczyt zostaje utracony, więc `undo` nie może go przywrócić, a raport wymienia zmianę jako nieodwracalną. System z włączonym czasem sieciowym ponownie koryguje zegar; należy go najpierw wyłączyć za pomocą `clock.ntp`.

Poprzednia wartość jest odczytywana przed zmianą i zapisywana jako cel `clock:time`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `time` | `instant` | wymagany | Nowy czas; moment w czasie UTC. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `irreversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Poprzedniego odczytu zegara nie można przywrócić. |
| External use | `system_program`: `timedatectl`, `systemsetup`, `tzutil`, `date`, `powershell`; Zegar jest odczytywany i ustawiany za pomocą timedatectl, systemsetup, tzutil, date oraz, w systemie Windows, PowerShell dla usługi czasu. |

## Przykłady {#examples}

Ustawienie zegara.

```nurt example
clock.set "2026-11-21T09:12:00Z" privilege: elevated
```

Ustawienie zegara na północ UTC.

```nurt example
clock.set "2027-01-01T00:00:00Z" privilege: elevated
```

Ustawienie zegara na ostatnią minutę roku.

```nurt example
clock.set "2026-12-31T23:59:00Z" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_TYPE` | `time` nie jest momentem. | Wpisz moment, na przykład `2026-11-21T09:12:00Z`. |
| `E_PRIVILEGE` | Program systemowy odmówił zmiany z powodu braku uprawnień. | Powtórz wywołanie z `privilege: elevated`. |

## Powiązane {#related}

`clock.ntp`, `clock.timezone_set`.
