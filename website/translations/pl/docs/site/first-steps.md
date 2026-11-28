---
title: Pierwsze kroki
order: 3
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/first-steps.md -->
<!-- Responsibility:   Documents First steps. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Pierwsze kroki {#first-steps}

## Uruchamianie programu {#running-a-program}

`nurt` przyjmuje program jako argument. Program to polecenie z argumentami, po którym opcjonalnie następują kolejne polecenia połączone przez `|>`.

```nurt example
find "/home/user/app" glob: "**/*.go" |> filter size > 1mb |> sort size order: desc |> first 5
```

Polecenie `find` tworzy rekordy `entry`, po jednym dla każdego pliku. `filter`, `sort` i `first` pobierają i tworzą rekordy tego samego rodzaju. Tekst wyniku wymienia pozostałe rekordy.

## Ścieżki i argumenty {#paths-and-arguments}

Ścieżka to tekst w cudzysłowie: `"/home/user/app"` lub `"C:/Users/user/app"`. Wyrażenie regularne zapisuje się między ukośnikami: `/oldName\(/`. Nazwa z dwukropkiem wprowadza argument nazwany: `glob: "**/*.go"`. Argumenty pozycyjne występują w kolejności parametrów podanej na stronie polecenia.

## Sprawdzanie przed uruchomieniem {#checking-before-running}

`nurt check` analizuje program składniowo, stosuje korekty języka i wiąże go z katalogiem. Wypisuje postać kanoniczną i skutek, niczego nie zmieniając.

```text
nurt check -e 'remove "/home/user/app/build"'
```

Parametr wywołania `preview: true` uruchamia program zmieniający bez wprowadzania żadnych zmian i zgłasza cele.

## Czytanie raportu {#reading-the-report}

Program zmieniający pliki zwraca raport skutków: status, numer operacji w dzienniku, cele, zmiany i ewentualne nietypowe skutki. [Raport skutków](effect-report.md) wymienia pola.

```nurt example
replace /oldName\(/ "newName(" in: ["/home/user/app/src"] glob: "**/*.go"
```

## Cofanie zmiany {#undoing-a-change}

Każda operacja zmieniająca pliki jest zapisywana w dzienniku. `history` wypisuje operacje, a `undo` cofa jedną z nich.

```nurt example
undo 18
```

Konflikty, retencję i operacje, których nie można odwrócić, opisuje [Dziennik i cofanie](journal.md).

## Uzyskiwanie pomocy {#getting-help}

`help` bez argumentów wypisuje polecenia. `help <command>` pokazuje składnię, parametry, rodzaje rekordów, skutek i przykłady.

```nurt example
help "replace"
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
