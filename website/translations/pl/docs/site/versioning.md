---
title: Wersjonowanie i wsparcie
order: 11
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/versioning.md -->
<!-- Responsibility:   Documents Versioning and support. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Wersjonowanie i wsparcie {#versioning-and-support}

| Element | Reguła |
|---|---|
| Program | Semantic Versioning 2.0.0. Wydania jednej wersji głównej nie łamią zgodności. Wydania podrzędne pojawiają się co 8 tygodni, a wydania poprawkowe w miarę potrzeby |
| Język | `language MAJOR.MINOR`. W obrębie wersji głównej język tylko rośnie. Synonimy i aliasy korekt są zachowywane. Wycofanie jest oznaczane w `help`, a usunięcie następuje nie wcześniej niż po 24 miesiącach, w wersji głównej |
| Drzewo składni | `nurt-ast/MAJOR`. Nowa wersja podrzędna tylko dodaje. Poprzednia wersja główna jest akceptowana przez 24 miesiące |
| Interfejs C | `NURT_ABI_MAJOR.MINOR` i nazwa biblioteki `libnurt.so.<MAJOR>`. W obrębie wersji głównej funkcje są tylko dodawane |
| Katalog | Katalog tylko rośnie w obrębie `nurt-catalog/1`: nowe polecenia, nowe opcjonalne parametry z wartością domyślną zachowującą poprzednie działanie, nowe wartości wyliczeń. Usunięcie lub zmiana nazwy polecenia albo parametru, a także zmiana typu lub wartości domyślnej, jest zmianą wersji głównej |
| Wsparcie długoterminowe | Jedno wydanie rocznie jest wydaniem wsparcia długoterminowego. Otrzymuje poprawki błędów i podatności przez 36 miesięcy i nie otrzymuje nowych funkcji |

## Obsługiwane systemy {#supported-systems}

Linux (x86_64, aarch64), macOS (x86_64, aarch64) i Windows (x86_64, aarch64).

## Wsparcie {#support}

Pytania i defekty zgłasza się na adres `support@danaco-group.pl`. Podatności podlegają zasadom z [Bezpieczeństwo](security.md).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
