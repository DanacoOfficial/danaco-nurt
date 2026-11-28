---
command: restart
group: administration
summary: Uruchamia ponownie usługi z wejścia.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# restart

## Składnia {#synopsis}

```text
service |> restart [name: value]...
```

## Opis {#description}

`restart` prosi menedżera usług o ponowne uruchomienie każdej usługi z wejścia i czeka do `wait`, aż usługa się ustabilizuje, tak jak `start`: stan jest odczytywany początkowo co 100 ms, odstęp rośnie o 100 ms do 500 ms, a oczekiwanie kończy się, gdy stan nie jest ani `starting`, ani `stopping`. Zatrzymana usługa jest uruchamiana. Działająca usługa jest zawsze uruchamiana ponownie, więc po udanym ponownym uruchomieniu `before` i `after` mają wartość `running`. Z `wait: 0s` polecenie wraca natychmiast ze stanem odczytanym zaraz po żądaniu.

Operacja jest oznaczana jako nieodwracalna przez `service_changed` przed poproszeniem menedżera o działanie, więc skutek jest zgłaszany nawet wtedy, gdy oczekiwanie zakończy się niepowodzeniem. Usługi są przetwarzane w kolejności wejścia; błąd zatrzymuje przetwarzanie pozostałych usług.

**Uprawnienia.** Jednostki systemowe wymagają administratora: wpisz `privilege: elevated` na etapie. Ponowne uruchomienie nie ma operacji odwrotnej i jest zgłaszane jako nieodwracalne.

```nurt
services name: "nginx" |> restart privilege: elevated
```

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `wait` | `duration` | `10s` | Czas oczekiwania, aż każda usługa osiągnie stan działania. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `service` | `effect` |

Każdy rekord `effect` zawiera `kind: "restart"`, `target` (`service:<name>`), `action`, `before` i `after`. Podgląd pokazuje te same rekordy z `status: planned` i oczekiwanym `after`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `external` |
| Odwracalność | `irreversible` |
| Implementacja | `builtin` |
| External use | `system_program`: `launchctl`; W systemie macOS usługi są uruchamiane ponownie przez składnik launchctl systemu operacyjnego. |

## Przykłady {#examples}

Ponowne uruchomienie jednej usługi:

```nurt example
services name: "nginx" |> restart
```

Ponowne uruchomienie każdej działającej usługi, której nazwa zaczyna się od `php`:

```nurt example
services name: /^php/ state: "running" |> restart
```

Ponowne uruchomienie usługi z dopuszczeniem trzydziestu sekund na jej start:

```nurt example
services name: "cron" |> restart wait: 30s
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_TIMEOUT` | Usługa nie ustabilizowała się w czasie `wait` ani w limicie czasu programu. `expected` zawiera `running`, `actual` bieżący stan; skutek został już zastosowany. | Podaj dłuższe `wait`. |
| `E_SERVICE` | Usługa przeszła w stan `failed`; `os_code` i `result` zawierają przyczynę podaną przez menedżera, a `next_steps` zawiera `logs unit: "<name>"`. Menedżer może też zgłosić inny błąd w `os_code` i `os_message`. | Przeczytaj dziennik usługi i popraw jej konfigurację. |
| `E_NOT_FOUND` | Usługa już nie istnieje. | Odczytaj usługi ponownie poleceniem `services`. |
| `E_PRIVILEGE` | Menedżer odmówił wykonania działania; `required` wskazuje uprawnienie, `root` w systemie Linux i `administrator` w systemie Windows. Interaktywna autoryzacja nigdy nie jest żądana. | Uruchom jako konto z tym uprawnieniem. |
| `E_UNSUPPORTED` | Rekord pochodzi z menedżera innego systemu. | Odczytaj usługi tego systemu poleceniem `services`. |
| `E_INVALID_VALUE` | `wait` jest ujemne. | Podaj nieujemny czas trwania, taki jak `10s`. |
| `E_CANCELLED` | Wykonanie zostało anulowane. | Brak; anulowanie zażądał wywołujący. |

## Powiązane {#related}

`services` tworzy rekordy wejściowe. `start` uruchamia usługi, a `stop` je zatrzymuje. `logs unit:` pokazuje, dlaczego usługa zakończyła się niepowodzeniem. Strona grupy `administration` wyświetla pozostałe polecenia grupy.
