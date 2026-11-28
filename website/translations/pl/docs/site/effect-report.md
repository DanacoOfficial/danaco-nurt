---
title: Raport skutków
order: 6
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/effect-report.md -->
<!-- Responsibility:   Documents Effect report. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Raport skutków {#effect-report}

Każde uruchomienie zwraca opis skutku, a nie tylko informację, że coś się wykonało. Format to `nurt-report/1`; schemat i pełna lista pól znajdują się w [specyfikacji](../../spec/report.md).

## Status {#status}

| Status | Znaczenie |
|---|---|
| `ok` | program się wykonał i skutek został zastosowany |
| `planned` | podgląd obliczył cele i skutek; nic się nie zmieniło |
| `partial` | skutek zastosowano do części celów; `irreversible` lub `error` wymienia pozostałe |
| `failed` | program zakończył się niepowodzeniem; opisuje je element `error` |

## Pola {#fields}

| Pole | Zawartość |
|---|---|
| `op` | numer wpisu dziennika operacji; nieobecne dla programów odczytujących i podglądów |
| `targets` | liczba plików, katalogów i bajtów, których dotknął program |
| `changes` | zmienione cele na bieżącej stronie |
| `notable` | nietypowe skutki, takie jak `mass_delete`, `size_shrunk` lub `nothing_matched` |
| `irreversible` | skutki, których cofnięcie (undo) nie może odwrócić |
| `records` | wynik programu odczytującego: typ rekordu, liczba dopasowań i wierszy |
| `error` | kod, komunikat, etap, pozycja i korekty programu zakończonego niepowodzeniem |

## Szczegółowość i stronicowanie {#verbosity-and-paging}

Parametr wywołania `verbosity` wybiera `quiet`, `normal` lub `detailed`. `limit` ustawia liczbę rekordów lub zmian na stronie, a `cursor` odczytuje następną stronę; ponowny odczyt strony nigdy nie uruchamia ponownie programu zmieniającego. Odpowiedź, która pomija wiersze, rekordy lub końcówki długich wierszy, zawiera pole `continuation`, a jej tekst kończy się `more: next`; polecenie `next` zwraca następną część (`next 200`, `next all`, `next full`). `format` wybiera `text`, `json`, `markdown` lub `compact`.

## Operacje na wielu celach {#operations-on-many-targets}

Operacja z 200 lub większą liczbą celów jest operacją masową. Opis pokazuje rozszerzenia, pierwszy i ostatni cel oraz nietypowe skutki, a pełna lista jest dostępna przez kursor.

## Błędy {#errors}

Błąd ma stały kod. [Kody błędów](../../spec/errors.md) wymieniają kody, etap zgłaszający każdy z nich oraz status interfejsu C.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
