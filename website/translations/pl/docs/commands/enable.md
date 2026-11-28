---
command: enable
group: administration
summary: Włącza automatyczne uruchamianie usług z wejścia przy starcie systemu lub logowaniu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# enable

## Składnia {#synopsis}

```text
service |> enable
```

## Opis {#description}

`enable` włącza automatyczne uruchamianie każdej usługi z wejścia: przy starcie
systemu dla usług systemowych i przy logowaniu dla usług bieżącego konta.
W systemie Linux plik jednostki zostaje włączony, a menedżer przeładowuje swoją
konfigurację. Pola `before` i `after` zawierają typ uruchamiania; jednostka bez
sekcji instalacji pozostaje `static`. Usługa nie jest uruchamiana; uruchamia ją
`start`.

Każda usługa z wejścia jest odczytywana przed wykonaniem działania. Usługa,
której typ uruchamiania to już `automatic`, nie jest przekazywana menedżerowi;
jej rekord skutku ma `before` równe `after` oraz kod uwagi `unchanged`. Dla
każdej innej usługi operacja jest oznaczana jako nieodwracalna przez
`service_changed`, zanim menedżer zostanie poproszony o działanie. Gdy zmiana
się zakończy, silnik zachowuje operację odwrotną i usuwa to oznaczenie z
raportu. Usługi są przetwarzane w kolejności wejścia; błąd zatrzymuje
przetwarzanie pozostałych usług.

**Uprawnienia.** Jednostki systemowe wymagają administratora: zapisz `privilege: elevated` w etapie. `undo` wpisu ponownie wyłącza usługi, jeśli ich typ uruchamiania przed zmianą to `disabled`.

```nurt
services name: "cron" |> enable privilege: elevated
```

## Parametry {#parameters}

Polecenie nie ma parametrów.

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `service` | `effect` |

Każdy rekord `effect` zawiera `kind: "enable"`, `target` (`service:<name>`),
`action`, `before` i `after`. Podgląd pokazuje te same rekordy z
`status: planned` i oczekiwaną wartością `after`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `builtin` |
| Warunki odwracalności | Cofnięcie uruchamia operację odwrotną: wyłączenie dla usługi, której typ uruchamiania przed zmianą był wyłączony; każdy inny typ uruchamiania przed zmianą pozostaje nieodwracalny. |
| External use | `system_program`: `launchctl`; W systemie macOS usługi są włączane przez składnik launchctl systemu operacyjnego. |

## Przykłady {#examples}

Włączenie jednej usługi:

```nurt example
services name: "cron" |> enable
```

Włączenie usługi bieżącego konta:

```nurt example
services name: "syncthing" scope: user |> enable
```

Włączenie usług, których nazwa zaczyna się od `backup`:

```nurt example
services name: /^backup/ |> enable
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Usługa już nie istnieje. | Odczytaj usługi ponownie poleceniem `services`. |
| `E_PRIVILEGE` | Menedżer odmówił wykonania działania; `required` wskazuje wymagane uprawnienie: `root` w systemie Linux i `administrator` w systemie Windows. Interaktywna autoryzacja nigdy nie jest żądana. | Uruchom polecenie na koncie z tym uprawnieniem. |
| `E_SERVICE` | Menedżer zgłosił błąd, na przykład dla zamaskowanej jednostki; `os_code` i `os_message` zawierają błąd menedżera. | Usuń przyczynę wskazaną w `os_message`. |
| `E_UNSUPPORTED` | Rekord pochodzi z menedżera innego systemu. | Odczytaj usługi w tym systemie poleceniem `services`. |
| `E_CANCELLED` | Wykonanie zostało anulowane. | Brak; anulowanie zażądał wywołujący. |

## Powiązane {#related}

`services` tworzy rekordy wejściowe. `disable` wyłącza automatyczne
uruchamianie usługi, a `start` uruchamia ją od razu. Strona grupy
`administration` zawiera listę pozostałych poleceń tej grupy.
