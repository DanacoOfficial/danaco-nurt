---
command: disable
group: administration
summary: Wyłącza automatyczne uruchamianie usług z wejścia.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# disable

## Składnia {#synopsis}

```text
service |> disable
```

## Opis {#description}

`disable` wyłącza automatyczne uruchamianie każdej usługi z wejścia. W systemie
Linux plik jednostki zostaje wyłączony, a menedżer przeładowuje swoją
konfigurację; jednostka zamaskowana jest już uznawana za `disabled`. Pola
`before` i `after` zawierają typ uruchamiania. Działająca usługa działa dalej;
zatrzymuje ją `stop`.

Każda usługa z wejścia jest odczytywana przed wykonaniem działania. Usługa,
której typ uruchamiania to już `disabled`, nie jest przekazywana menedżerowi;
jej rekord skutku ma `before` równe `after` oraz kod uwagi `unchanged`. Dla
każdej innej usługi operacja jest oznaczana jako nieodwracalna przez
`service_changed`, zanim menedżer zostanie poproszony o działanie. Gdy zmiana
się zakończy, silnik zachowuje operację odwrotną i usuwa to oznaczenie z
raportu. Usługi są przetwarzane w kolejności wejścia; błąd zatrzymuje
przetwarzanie pozostałych usług.

**Uprawnienia.** Jednostki systemowe wymagają administratora: zapisz `privilege: elevated` w etapie. `undo` wpisu ponownie włącza usługi, jeśli ich typ uruchamiania przed zmianą to `automatic`.

```nurt
services name: "cron" |> disable privilege: elevated
```

## Parametry {#parameters}

Polecenie nie ma parametrów.

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `service` | `effect` |

Każdy rekord `effect` zawiera `kind: "disable"`, `target` (`service:<name>`),
`action`, `before` i `after`. Podgląd pokazuje te same rekordy z
`status: planned` i oczekiwaną wartością `after`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `builtin` |
| Warunki odwracalności | Cofnięcie uruchamia operację odwrotną: włączenie dla usługi, której typ uruchamiania przed zmianą był automatyczny; każdy inny typ uruchamiania przed zmianą pozostaje nieodwracalny. |
| External use | `system_program`: `launchctl`; W systemie macOS usługi są wyłączane przez składnik launchctl systemu operacyjnego. |

## Przykłady {#examples}

Wyłączenie jednej usługi:

```nurt example
services name: "cups" |> disable
```

Wyłączenie usługi bieżącego konta:

```nurt example
services name: "syncthing" scope: user |> disable
```

Wyłączenie zatrzymanych usług, których nazwa zaczyna się od `legacy`:

```nurt example
services name: /^legacy/ state: "stopped" |> disable
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Usługa już nie istnieje. | Odczytaj usługi ponownie poleceniem `services`. |
| `E_PRIVILEGE` | Menedżer odmówił wykonania działania; `required` wskazuje wymagane uprawnienie: `root` w systemie Linux i `administrator` w systemie Windows. Interaktywna autoryzacja nigdy nie jest żądana. | Uruchom polecenie na koncie z tym uprawnieniem. |
| `E_SERVICE` | Menedżer zgłosił błąd; `os_code` i `os_message` zawierają błąd menedżera. | Usuń przyczynę wskazaną w `os_message`. |
| `E_UNSUPPORTED` | Rekord pochodzi z menedżera innego systemu. | Odczytaj usługi w tym systemie poleceniem `services`. |
| `E_CANCELLED` | Wykonanie zostało anulowane. | Brak; anulowanie zażądał wywołujący. |

## Powiązane {#related}

`services` tworzy rekordy wejściowe. `enable` włącza automatyczne uruchamianie
usługi, a `stop` zatrzymuje ją od razu. Strona grupy `administration` zawiera
listę pozostałych poleceń tej grupy.
