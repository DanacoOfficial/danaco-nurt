---
command: start
group: administration
summary: Uruchamia usługi wejściowe.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# start

## Składnia {#synopsis}

```text
service |> start [name: value]...
```

## Opis {#description}

`start` prosi menedżera usług o uruchomienie każdej usługi wejściowej i czeka
do `wait`, aż usługa się ustabilizuje. Stan jest odczytywany początkowo co
100 ms; odstęp rośnie o 100 ms do 500 ms, a oczekiwanie kończy się, gdy stan
nie jest ani `starting`, ani `stopping`. Usługa, która działa, zgłasza
`after: "running"`; usługa, która wykonała się do końca, taka jak jednostka
systemd `oneshot`, zgłasza `after: "stopped"`. Oczekiwanie uwzględnia
anulowanie i limit czasu programu. Przy `wait: 0s` polecenie wraca
natychmiast, a `after` zawiera stan odczytany tuż po żądaniu, zwykle
`starting`.

Każda usługa wejściowa jest odczytywana przed akcją. Usługa, która już działa,
nie jest przekazywana menedżerowi; jej rekord skutku ma `before` równe `after`
i godny uwagi kod `unchanged`. Dla każdej innej usługi operacja jest oznaczana
jako nieodwracalna znacznikiem `service_changed`, zanim menedżer zostanie
poproszony o działanie, więc skutek jest zgłaszany nawet wtedy, gdy
oczekiwanie zawiedzie. Gdy zmiana się zakończy, silnik zachowuje operację
odwrotną i usuwa znacznik z raportu. Usługi są przetwarzane w kolejności
wejścia; błąd zatrzymuje przetwarzanie pozostałych usług.

**Uprawnienie.** Jednostki systemowe wymagają administratora: zapisz `privilege: elevated` na etapie. `undo` wpisu zatrzymuje usługi, które uruchomiło polecenie.

```nurt
services name: "nginx" |> start privilege: elevated
```

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `wait` | `duration` | `10s` | Czas oczekiwania, aż każda usługa osiągnie stan uruchomienia. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `service` | `effect` |

Każdy rekord `effect` zawiera `kind: "start"`, `target` (`service:<name>`),
`action`, `before` i `after`. Podgląd pokazuje te same rekordy z
`status: planned` i oczekiwanym `after`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `external` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `builtin` |
| Warunki odwracalności | Cofnięcie uruchamia operację odwrotną: zatrzymanie dla usługi, która wcześniej była zatrzymana; usługa, której stanu sprzed zmiany nie przywraca żadne polecenie, pozostaje nieodwracalna. |
| External use | `system_program`: `launchctl`; w systemie macOS usługi są uruchamiane przez składnik launchctl systemu operacyjnego. |

## Przykłady {#examples}

Uruchomienie jednej usługi:

```nurt example
services name: "cron" |> start
```

Uruchomienie zatrzymanych usług bieżącego konta bez oczekiwania:

```nurt example
services scope: user state: "stopped" |> start wait: 0s
```

Uruchomienie usługi i danie jej minuty na start:

```nurt example
services name: "postgresql" |> start wait: 1m
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_TIMEOUT` | Usługa nie ustabilizowała się w czasie `wait` ani w limicie czasu programu. `expected` zawiera `running`, `actual` bieżący stan; skutek został już zastosowany. | Podaj dłuższy `wait`. |
| `E_SERVICE` | Usługa przeszła w stan `failed`; `os_code` i `result` zawierają przyczynę podaną przez menedżera, a `next_steps` zawiera `logs unit: "<name>"`. Menedżer może też zgłosić inny błąd, na przykład zamaskowaną jednostkę, w `os_code` i `os_message`. | Odczytaj dziennik usługi i popraw jej konfigurację. |
| `E_NOT_FOUND` | Usługa już nie istnieje. | Odczytaj usługi ponownie za pomocą `services`. |
| `E_PRIVILEGE` | Menedżer odrzucił akcję; `required` wskazuje uprawnienie, `root` w systemie Linux i `administrator` w systemie Windows. Interaktywna autoryzacja nigdy nie jest żądana. | Uruchom jako konto z tym uprawnieniem. |
| `E_UNSUPPORTED` | Rekord pochodzi z menedżera innego systemu. | Odczytaj usługi tego systemu za pomocą `services`. |
| `E_INVALID_VALUE` | `wait` jest ujemne. | Podaj nieujemny czas trwania, taki jak `10s`. |
| `E_CANCELLED` | Wykonanie zostało anulowane. | Brak; anulowanie zażądał wywołujący. |

## Powiązane {#related}

`services` produkuje rekordy wejściowe. `restart` uruchamia usługi ponownie, a
`stop` je zatrzymuje. `logs unit:` pokazuje, dlaczego usługa zawiodła. Strona
grupy `administration` wylicza pozostałe polecenia grupy.
