---
title: Dziennik i cofanie
order: 7
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/journal.md -->
<!-- Responsibility:   Documents Journal and undo. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Dziennik i cofanie {#journal-and-undo}

Dziennik zapisuje każdą operację zmieniającą pliki wraz z poprzednią zawartością, której potrzebuje cofnięcie (undo). Kontrakt znajduje się w [specyfikacji](../../spec/journal.md).

## Polecenia {#commands}

| Polecenie | Funkcja |
|---|---|
| `history` | wypisuje operacje; filtruje według stanu, ścieżki i zakresu |
| `undo` | cofa jedną operację, operacje od wskazanego punktu albo zmiany podanych plików |
| `redo` | stosuje ponownie cofniętą operację |
| `journal.status` | pokazuje, czy dziennik jest włączony i co zawiera |
| `journal.space` | pokazuje wolne miejsce i miejsce zajmowane przez dziennik |
| `journal.limits` | ustawia retencję dla sesji lub użytkownika |
| `journal.pin` | zachowuje wpis ponad limity retencji |
| `journal.purge` | usuwa wpisy |

```nurt example
history limit: 5
```

## Stany wpisów {#entry-states}

| Stan | Znaczenie |
|---|---|
| `available` | wpis można cofnąć |
| `partial` | część zachowanej zawartości zwolniono, aby odzyskać miejsce na dysku |
| `expired` | wpis opuścił okno retencji |
| `undone` | wpis został cofnięty |

## Konflikty cofnięcia {#undo-conflicts}

Gdy plik zmienił się po operacji, `undo` zgłasza `E_JOURNAL_CONFLICT` z plikiem i skrótami. Parametr `on_conflict` wybiera `force`, `include_later`, `partial` lub `abort`; wartość domyślna `report` niczego nie zmienia.

```nurt example
undo 18 on_conflict: include_later
```

## Retencja {#retention}

Wartości domyślne zachowują ostatnie 30 operacji oraz operacje z ostatnich 30 minut. Klucze `journal.max_ops`, `journal.max_age` i `journal.max_size` zmieniają limity; zobacz [Konfiguracja](configuration.md). Operacja, której poprzednia zawartość nie została zachowana, jest zgłaszana jako `irreversible` wraz z przyczyną.

## Odporność na awarię {#crash-safety}

Operacja na wielu celach jest jednym wpisem dziennika. Po utracie zasilania krok odzyskiwania wycofuje wpis, który nie został zatwierdzony.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
