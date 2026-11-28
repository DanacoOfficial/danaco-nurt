<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/conventions/naming.md -->
<!-- Responsibility:   Documents Naming conventions. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Konwencje nazewnictwa {#naming-conventions}

Ten dokument określa nazwy produktu, jego artefaktów i
elementów języka oraz reguły wyboru nowych nazw.

## Nazwa produktu {#product-name}

| Element | Nazwa |
|---|---|
| Produkt | Danaco Nurt (w tekście ciągłym: Nurt) |
| Wiersz opisu | Powłoka poleceń i język, w którym każdą zmianę można cofnąć (`nurt_contracts::product::TAGLINE`) |
| Plik wykonywalny | `nurt` (`nurt.exe` w systemie Windows); nie są instalowane żadne inne nazwy ani aliasy |
| Wymowa | NOORT |

Nazwy „Danaco" i „Danaco Nurt" są nazwami Danaco Holding Group
Sp. z o.o. Apache License 2.0 nie udziela pozwolenia na ich używanie
(sekcja 6 Licencji), z wyjątkiem zwyczajowego opisu pochodzenia
produktu i powielania pliku `NOTICE`. Opisowe
stwierdzenia, takie jak „współpracuje z Danaco Nurt", są użyciem zwyczajowym. Zmodyfikowana
wersja programu nie może być dystrybuowana pod nazwą Danaco Nurt
bez zgody Danaco Holding Group Sp. z o.o.

## Identyfikatory pochodne od nazwy produktu {#identifiers-derived-from-the-product-name}

| Element | Identyfikator |
|---|---|
| Pliki programów | rozszerzenie `.nurt`, UTF-8; opcjonalny pierwszy wiersz `#!/usr/bin/env nurt` |
| Drzewo składni w JSON | rozszerzenie `.nurt.json` |
| Typy mediów | `application/vnd.nurt.ast+json`, `application/vnd.nurt.ast+cbor`, `application/vnd.nurt.report+json`, `application/vnd.nurt.request+json` |
| Biblioteka C | `libnurt` (`libnurt.so`, `libnurt.dylib`, `nurt.dll`); statyczna `libnurt.a`, `nurt.lib`; SONAME `libnurt.so.<ABI major>` |
| Nagłówek C | `nurt.h` |
| Funkcje i typy C | przedrostek `nurt_`, w postaci `nurt_<verb>_<noun>` (na przykład `nurt_run_json`) |
| Makra C | przedrostek `NURT_` |
| Zmienne środowiskowe | przedrostek `NURT_`, wielkie litery (na przykład `NURT_CONFIG`) |
| Plik konfiguracyjny | `nurt.toml`; konfiguracja projektu `.nurt.toml` |
| Katalog dziennika | `.nurt-journal` (`.nurt-journal.noindex` w macOS) |
| Katalog stanu użytkownika | Linux `$XDG_STATE_HOME/nurt/`, macOS `~/Library/Application Support/Nurt/`, Windows `%LOCALAPPDATA%\Nurt\` |
| Usługa | `nurt service run`; jednostka systemd `nurt-service.service`; usługa Windows `NurtService` (nazwa wyświetlana „Danaco Nurt Service"); etykieta launchd `nurt.service` |
| Nazwa narzędzia w integracjach i w serwerze MCP | `nurt` |
| Crate'y Rust | `nurt-<area>` (na przykład `nurt-journal`, `nurt-git-ops`) |

## Nazwy w języku {#names-in-the-language}

Słownictwo języka używa standardowych terminów informatycznych i zwykłych
słów angielskich. Nazwa jest akceptowana, gdy:

1. jest powszechnym terminem informatycznym lub zwykłym słowem angielskim;
2. nie jest skrótem kojarzonym z jednym programem (na przykład
   `grep`, `sed`, `awk`, `rm`, `ls`, `mv`, `cp`, `cat`); czasowniki będące zarówno
   zwykłym słowem, jak i nazwą programu (`find`, `sort`, `diff`) są dozwolone;
3. nie jest terminem wymyślonym ani metaforą;
4. jest jednoznaczna w katalogu: żadne dwie nazwy poleceń nie mają odległości
   edycyjnej równej 1, z wyjątkiem udokumentowanych par;
5. jest znana z powszechnego użycia w systemach operacyjnych, językach programowania,
   SQL i edytorach.

Nazwy innych produktów nie są używane jako nazwy poleceń ani parametrów.
Grupy poleceń mogą być nazwane od technologii, na której operują (na przykład
grupa `git.` dla systemu kontroli wersji).

| Element | Postać | Przykłady |
|---|---|---|
| Polecenia | małe litery, `snake_case`; grupy rozdzielone kropką | `find`, `replace`, `git.status` |
| Parametry | małe litery, `snake_case` | `path`, `glob`, `depth`, `order` |
| Wartości wyliczenia | małe litery | `asc`, `desc`, `directory`, `overwrite` |
| Typy rekordów | małe litery | `entry`, `line`, `match`, `job` |
| Kody błędów | `E_` i następujące po nim słowa pisane wielkimi literami | `E_NOT_FOUND`, `E_ASSERT` |
| Klucze konfiguracji | `section.key`, małe litery `snake_case` | `journal.max_ops` |

Pełne słowa są preferowane wobec skrótów (`directory`, a nie `dir`; `instant`,
a nie `timestamp`). Skróty są używane wyłącznie wtedy, gdy są terminem
standardowym (`lf`, `crlf`, `json`, `regex`).

## Nazwy w kodzie {#names-in-the-code}

Kod Rust stosuje Rust API Guidelines: `UpperCamelCase` dla typów i
cech, `snake_case` dla funkcji, metod, modułów i zmiennych,
`SCREAMING_SNAKE_CASE` dla stałych i statyków. Typy crate'a kontraktu,
które odpowiadają elementom języka, używają nazwy elementu (na przykład
typ rekordu `entry` to typ `Entry`). Nazwy serializowanych pól to
`snake_case`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
