---
command: power.cancel
group: administration
summary: Anuluje zaplanowane ponowne uruchomienie lub wyłączenie.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# power.cancel

## Składnia {#synopsis}

```text
power.cancel
```

## Opis {#description}

`power.cancel` anuluje zaplanowane ponowne uruchomienie lub wyłączenie za pomocą `shutdown -c` w systemie Linux, `shutdown /a` w systemie Windows oraz przez zatrzymanie `shutdown` w systemie macOS. W systemie Linux harmonogram jest odczytywany z `/run/systemd/shutdown/scheduled`; bez niego polecenie zgłasza `unchanged`. Anulowany harmonogram nie jest przywracany przez `undo`.

Uprawnienia: polecenie przyjmuje `privilege`; wpisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je po podniesieniu uprawnień. Wpis z podniesionymi uprawnieniami jest cofany przez ten sam proces z podniesionymi uprawnieniami.

Cofnięcie (`undo`) zgłasza, że anulowany harmonogram nie jest przywracany.

## Parametry {#parameters}

Polecenie nie ma parametrów.

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
| Implementacja | `system_program` |
| Warunki odwracalności | Anulowany harmonogram nie jest przywracany. |
| External use | `system_program`: `shutdown`, `pkill`; Harmonogram jest anulowany za pomocą shutdown -c w systemie Linux, shutdown /a w systemie Windows oraz przez zatrzymanie shutdown w systemie macOS. |

## Przykłady {#examples}

Anulowanie harmonogramu.

```nurt example
power.cancel privilege: elevated
```

Uruchomienie anulowania z podniesionymi uprawnieniami; dodanie parametru wywołania `preview: true` najpierw pokazuje skutek.

```nurt example
power.cancel privilege: elevated
```

Anulowanie i pokazanie skutku.

```nurt example
power.cancel privilege: elevated |> select target
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_STAGE_FAILED` | Nic nie jest zaplanowane (Windows). | Nie jest potrzebne żadne działanie. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wymieniony w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`power.restart`, `power.shutdown`.
