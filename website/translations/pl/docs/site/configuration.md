---
title: Konfiguracja
order: 8
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/configuration.md -->
<!-- Responsibility:   Documents Configuration. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Konfiguracja {#configuration}

Format to `nurt-config/1`. Pełna lista kluczy z typami i wartościami domyślnymi znajduje się w [specyfikacji](../../spec/configuration.md); schemat wypisuje `nurt schema config`.

## Pliki {#files}

| System | Plik systemowy | Plik użytkownika | Katalog stanu |
|---|---|---|---|
| Linux | `/etc/nurt/nurt.toml` | `$XDG_CONFIG_HOME/nurt/nurt.toml` (wartość domyślna `~/.config/nurt/nurt.toml`) | `$XDG_STATE_HOME/nurt/` |
| macOS | `/Library/Application Support/Nurt/nurt.toml` | `~/Library/Application Support/Nurt/nurt.toml` | `~/Library/Application Support/Nurt/` |
| Windows | `%PROGRAMDATA%\Nurt\nurt.toml` | `%APPDATA%\Nurt\nurt.toml` | `%LOCALAPPDATA%\Nurt\` |

Projekt może zawierać `.nurt.toml`.

## Pierwszeństwo {#precedence}

Późniejsze poziomy nadpisują wcześniejsze:

1. wbudowane wartości domyślne
2. plik systemowy
3. plik użytkownika
4. `.nurt.toml` projektu
5. plik wskazany przez `NURT_CONFIG`
6. zmienne `NURT_*`
7. parametry wywołania

## Pliki projektu {#project-files}

`.nurt.toml` może ustawiać `scope.base`, `scope.project_roots`, `report.*` i `run.*`, a `journal.*` wyłącznie dla zaufanych projektów. Klucze `scope.trusted_projects`, `scope.trust*` i `service.*` w pliku projektu są ignorowane i zgłaszane jako `trust_config_ignored`.

## Klucze używane w dokumentacji {#keys-used-in-the-documentation}

| Klucz | Zmienna | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `syntax.variant` | `NURT_SYNTAX` | `a` | wariant składni tekstowej |
| `syntax.tolerant` | `NURT_SYNTAX_TOLERANT` | `true` | akceptowanie składni pozostałych wariantów |
| `correction.enabled` | `NURT_CORRECTION` | `true` | korekty formy |
| `journal.enabled` | `NURT_JOURNAL` | `true` | dziennik zmian |
| `journal.max_ops` | `NURT_JOURNAL_MAX_OPS` | `30` | liczba przechowywanych operacji |
| `journal.max_age` | `NURT_JOURNAL_MAX_AGE` | `"30m"` | wiek przechowywanych operacji |
| `run.timeout` | `NURT_RUN_TIMEOUT` | `"120s"` | limit czasu `run` |
| `output.language` | `NURT_LANGUAGE` | `"en"` | język `help`, `describe`, komunikatów błędów i uwag w wyniku (`en` lub `pl`); kody i programy nigdy nie są tłumaczone |
| `output.color` | `NURT_COLOR` | `"auto"` | kolory dla człowieka |
| `session.id` | `NURT_SESSION` | brak | identyfikator sesji w dzienniku |

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
