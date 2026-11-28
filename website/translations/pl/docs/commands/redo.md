---
command: redo
group: core
summary: Stosuje ponownie cofniętą operację.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# redo

## Składnia {#synopsis}

```text
redo [<id>]
```

## Opis {#description}

`redo` stosuje ponownie zmiany odwrócone przez cofnięcie (undo). `id` to numer operacji cofnięcia, taki jak na liście `history` (zapis `op: N` lub `operation: N` jest poprawiany na `id: N`, tak jak w `undo`); bez niego wybierane jest najnowsze dostępne cofnięcie sesji (konta bez sesji).

Operacja cofnięcia jest odwracana jako całość. Dla cofnięcia jednego pliku (`undo file:`) lub wybranych elementów (`undo N paths:`) ponownie stosowane są tylko te elementy, które cofnięcie przywróciło.

Zanim cokolwiek zostanie zapisane, każdy element jest porównywany ze stanem zapisanym przez cofnięcie, tym samym sprawdzeniem co w `undo`. Późniejsza operacja na tych samych plikach powoduje niepowodzenie porównania, więc ponowienie nigdy nie nadpisuje nowszej pracy: wywołanie kończy się błędem `E_JOURNAL_CONFLICT` i nic się nie zmienia. `redo` nie ma parametru `on_conflict`; aby mimo to zastosować zmiany, cofnij operację cofnięcia poleceniem `undo N on_conflict: force`.

Operacja cofnięcia otrzymuje stan `undone`. Operacje, które cofnęła, pozostają `undone`; ich zmiany są teraz przechowywane przez operację ponowienia, którą cofa się poleceniem `undo` jak każdą inną operację.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `int` | najnowsza cofnięta operacja | Cofnięta operacja do ponownego zastosowania. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `journal` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Ponowne zastosowanie najnowszego cofnięcia sesji:

```nurt example
redo
```

Ponowne zastosowanie operacji cofnięcia 42:

```nurt example
redo 42
```

Odwrócenie ponowienia przez jego cofnięcie:

```nurt example
undo 43
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_JOURNAL_NOOP` | nie ma cofnięcia do ponowienia, operacja nie jest cofnięciem albo została już ponowiona | wyświetl operacje poleceniem `history` |
| `E_JOURNAL_EXPIRED` | operacja cofnięcia wygasła; nic się nie zmieniło | brak; zmian nie można już zastosować z dziennika |
| `E_JOURNAL_CONFLICT` | pliki zmieniły się po cofnięciu; nic się nie zmieniło | sprawdź różnice w `conflicts` |
| `E_JOURNAL_PARTIAL` | elementy operacji cofnięcia zostały zwolnione w celu odzyskania miejsca; nic się nie zmieniło | `undo N on_conflict: partial` z operacją cofnięcia |
| `E_JOURNAL_CORRUPT` | operacja cofnięcia nie przeszła kontroli integralności | `nurt journal salvage` |
| `E_UNSUPPORTED` | dziennik nie może przywrócić elementu (zmiana nazwy między woluminami lub nad istniejącym wpisem); nic się nie zmieniło | zastosuj zmianę ponownie oryginalnym poleceniem |
| `E_STALE` | element zmienił się w trakcie ponowienia; ponowienie zostało wycofane | powtórz ponowienie |

## Powiązane {#related}

`undo`, `history`, `help skill: "journal"`.
