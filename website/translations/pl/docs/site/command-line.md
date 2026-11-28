---
title: Wiersz poleceń
order: 5
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/command-line.md -->
<!-- Responsibility:   Documents Command line. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Wiersz poleceń {#command-line}

## Składnia {#synopsis}

```text
nurt -V | --version
nurt -h | --help
nurt <program>
nurt exec [--variant a|b|c] [--preview] [--json | --format F] [--limit N] [--cursor C] [--verbosity V] [--privilege P] [--as-user NAME] <program>
nurt script <file.nurt> [--preview]
nurt check <file.nurt | -e '<program>'>
nurt fmt <file.nurt> [--variant a|b|c]
nurt ast [--from a|b|c|json] [--to json|cbor|a|b|c] <input>
nurt schema ast|bound|report|request|config|tool
nurt schema tool [--format <format>] [--language en|pl] [--groups core|all|<group>,<group>...]
nurt catalog [command | group]
nurt help [topic]
nurt history|undo|redo [arguments]
nurt serve --stdio
nurt mcp [--stdio | --http <address>]
nurt lsp
nurt migrate <file.sh | file.ps1>
nurt service install|uninstall|start|stop|status|run
nurt journal salvage <volume>
nurt ext install|remove|list
nurt doctor
nurt version
```

## Opis {#description}

`nurt` to powłoka i język, w którym każdą zmianę można cofnąć.

`nurt` uruchamia programy w języku Danaco Nurt i udostępnia silnik innym programom. Pierwszy argument, który nie jest podpoleceniem, jest programem: `nurt run ["go", "test"] cwd: "/home/user/app"`. `nurt exec` jest zawsze jednoznaczne.

Opcje z `--` należą do programu `nurt`. Język nie ma opcji z myślnikiem; korekty tłumaczą takie nawyki.

## Opcje {#options}

| Opcja | Funkcja |
|---|---|
| `-V` | wypisuje `nurt <version>` |
| `--version` | wypisuje `nurt <version> (<commit> <date>)`, wiersz z prawami autorskimi i licencję |
| `-h`, `--help` | wypisuje pomoc; pierwszy wiersz to opis programu |

Wynik `--version` ma trzy wiersze. Część w nawiasach jest pomijana, gdy plik wykonywalny zbudowano bez danych repozytorium.

```text
nurt <version> (<commit> <date>)
Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.
License: Apache-2.0
```

## Podpolecenia {#subcommands}

| Podpolecenie | Funkcja |
|---|---|
| `exec` | uruchamia program z argumentu |
| `script` | uruchamia kolejno potoki z pliku; każdy potok jest osobną operacją |
| `repl` | sesja interaktywna z historią, uzupełnianiem nazw i parametrów oraz sterowaniem zadaniami |
| `check` | analizuje składniowo, koryguje i wiąże program, wypisuje postać kanoniczną, niczego nie zmienia |
| `fmt` | wypisuje plik w postaci kanonicznej |
| `ast` | konwertuje program między wariantami tekstowymi a formatami drzewa składni |
| `schema` | wypisuje schemat JSON albo definicję narzędzia dla platformy AI |
| `catalog` | wypisuje katalog, grupę albo jedno polecenie |
| `help` | pomoc wbudowana |
| `history`, `undo`, `redo` | skróty poleceń dziennika |
| `serve` | serwer JSON-RPC `nurt-rpc/1` dla programu hosta |
| `mcp` | serwer Model Context Protocol udostępniający narzędzie `nurt` |
| `lsp` | serwer języka dla plików `.nurt` |
| `migrate` | tłumaczy skrypt powłoki na plik `.nurt` |
| `service` | zarządza usługą dziennika działającą w tle |
| `journal salvage` | naprawia wpisy oznaczone jako `corrupt` |
| `ext` | zarządza rozszerzeniami |
| `doctor` | raportuje właściwości systemu plików, uprawnienia, usługę, wersje i serwery języka |
| `version` | wypisuje wersję |

## Środowisko {#environment}

Zmienne z przedrostkiem `NURT_` ustawiają klucze konfiguracji; zobacz [Konfiguracja](configuration.md). `NURT_CONFIG` wskazuje dodatkowy plik konfiguracji.

## Pliki {#files}

Systemowe, użytkownika i projektu pliki konfiguracji oraz katalog stanu wymieniono w [Konfiguracja](configuration.md).

## Zobacz także {#see-also}

`nurt-commands(7)`, `nurt-<command>(1)`

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
