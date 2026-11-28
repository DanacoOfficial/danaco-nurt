<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/integrators/tool-definition.md -->
<!-- Responsibility:   Documents Tool definition. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Definicja narzędzia {#tool-definition}

## Składnia {#syntax}

```
nurt schema tool [--format <format>] [--language en|pl] [--groups core|all|<group>,<group>...]
```

`nurt schema tool` bez opcji wypisuje opublikowany schemat `nurt-tool/1` (schemat formatu `json-schema` opisanego niżej), tak jak każda inna nazwa w `nurt schema`. Z co najmniej jedną opcją wypisuje definicję narzędzia; opcja, która nie została podana, przyjmuje wartość domyślną. Teksty opisów i wersja katalogu pochodzą z zainstalowanego silnika.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `--format` | wyliczenie | `json-schema` | format wyjścia, zobacz niżej |
| `--language` | `en`, `pl` | `en` | język opisu; także wartość domyślna argumentu `language` |
| `--groups` | tekst | `core` | grupy poleceń opisane dodatkowo oprócz grupy podstawowej |

Wartość następuje po opcji jako osobny argument lub po `=`. Nieznany format, język lub nieznana grupa to błąd `E_UNKNOWN_VALUE` z listą akceptowanych wartości; nieznana opcja lub brakująca wartość to błąd użycia (kod wyjścia 2).

Skrzynka Rust `nurt-tooldef` tworzy to samo wyjście: `render(ToolFormat, &RenderOptions)`. Wyjście jest deterministyczne dla tych samych danych wejściowych.

## Wynik {#result}

Jeden obiekt JSON: element listy narzędzi danej platformy.

| Format | Platforma | Obiekt |
|---|---|---|
| `anthropic` | Anthropic Messages API | `{name, description, input_schema}` |
| `openai` | OpenAI Chat Completions | `{type: "function", function: {name, description, parameters, strict: false}}` |
| `openai-strict` | OpenAI Chat Completions, structured outputs | jak `openai`, `strict: true` |
| `openai-responses` | OpenAI Responses API | `{type: "function", name, description, parameters, strict: false}` |
| `openai-responses-strict` | OpenAI Responses API, structured outputs | jak `openai-responses`, `strict: true` |
| `gemini` | Gemini API | element `functionDeclarations`: `{name, description, parameters}` |
| `mcp` | Model Context Protocol | element `tools/list`: `{name, description, inputSchema, _meta}` |
| `json-schema` | dowolna | `{name, description, input_schema, catalog_version}`; schemat wejścia to JSON Schema 2020-12 |

Nazwa narzędzia to `nurt` w każdym formacie. Argumentami są dane wejściowe i opcje wywołania z `nurt-request/1`:

| Argument | Typ | Znaczenie |
|---|---|---|
| `program` | tekst | tekst programu |
| `ast` | obiekt | drzewo składni `nurt-ast/1`, równoważne `program` |
| `cursor` | tekst | kursor dalszej strony wcześniejszego wyniku |
| `accept` | tekst | identyfikator kandydata z wcześniejszego błędu `E_AMBIGUOUS` |
| `preview`, `base`, `limit`, `fields`, `format`, `verbosity`, `diff`, `diff_context`, `max_chars`, `save_to`, `timeout`, `session`, `language`, `script` | jak w `nurt-request/1` | opcje wywołania |

Podawane jest dokładnie jedno z `program`, `ast`, `cursor` i `accept`. Pola koperty `nurt` (wersja formatu) i `variant` (wariant notacji, wykrywany przez konsolę) nie są argumentami.

## Reguły dla poszczególnych platform {#rules-per-platform}

Definicja jest generowana z jednego źródła. Konstrukcje, których platforma nie akceptuje, są przepisywane tak, aby każdy format akceptował te same wywołania; ograniczenie przeniesione do opisu jest nadal sprawdzane przez konsolę względem `nurt-request/1`.

| Reguła | Formaty |
|---|---|
| wyłączny wybór spośród `program`, `ast`, `cursor` i `accept` (`oneOf` w korzeniu schematu żądania) jest podany w opisie `description` korzenia | wszystkie |
| `additionalProperties: false` jest usuwane | `gemini` |
| `default` jest zapisywane w opisie argumentu | `openai-strict`, `openai-responses-strict`, `gemini` |
| `minLength` jest zapisywane w opisie | `openai-strict`, `openai-responses-strict` |
| każdy argument jest wymagany i ma `null` w swoim typie; `null` oznacza brak argumentu | `openai-strict`, `openai-responses-strict` |
| `ast` jest przekazywane jako tekst JSON | `openai-strict`, `openai-responses-strict`, `gemini` |
| wyliczenia mają `type: string`; nazwy typów to `STRING`, `INTEGER`, `OBJECT`, ... z `format: enum` | `gemini` (oba), formaty strict (tylko pierwsze) |

Host, który otrzymuje argumenty w formacie strict lub w `gemini`, usuwa wartości `null` i parsuje tekst `ast` przed zbudowaniem żądania. Robią to wiązania językowe oraz `nurt_tooldef::request_from_arguments`.

## Opis i grupy poleceń {#description-and-command-groups}

Opis zawiera podstawowy opis języka, a następnie opis każdej wybranej grupy. Grupa podstawowa jest zawsze obecna. Pozostałe grupy są ładowane na żądanie:

1. Zacznij od `--groups core`.
2. Gdy model potrzebuje grupy, na przykład po `help group: "git"` lub po `E_UNKNOWN_COMMAND` dla polecenia grupy opcjonalnej, wyrenderuj definicję ponownie z dodaną grupą (`--groups git`) albo dołącz do kontekstu tekst `describe` dla tej grupy.
3. Serwery MCP wysyłają `notifications/tools/list_changed` po zmianie zestawu grup; klienci odczytują listę narzędzi ponownie.

`catalog_version` (`json-schema`) i `_meta["pl.danaco-group/nurt-catalog"]` (`mcp`) przenoszą wersję katalogu poleceń, z którego powstał opis. Host, który buforuje definicje, używa jako klucza pamięci podręcznej formatu, języka, grup i tej wersji.

## Przykłady {#examples}

```sh
nurt schema tool --format anthropic
nurt schema tool --format openai-responses-strict --groups core,git
nurt schema tool --format mcp --language pl --groups all
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
