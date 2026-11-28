<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cli/nurt.md -->
<!-- Responsibility:   Documents nurt. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# nurt {#nurt}

Plik wykonywalny `nurt` uruchamia programy języka, oferuje sesję interaktywną i hostuje serwery protokołów.

## Wywołanie {#invocation}

```
nurt [OPTIONS] <program>
nurt <subcommand> [arguments]
nurt <subcommand> -h | --help
nurt -V | --version | -h | --help
```

Pierwszy argument wybiera podpolecenie tylko wtedy, gdy jest dokładnie równy nazwie podpolecenia. Każdy inny pierwszy argument rozpoczyna program języka, a wszystkie argumenty są łączone pojedynczymi spacjami w tekst programu. `nurt exec` jest zawsze jednoznaczne. Nazwy rozróżniają wielkość liter; argument w cudzysłowie lub z otaczającym białym znakiem jest programem.

Bez argumentów plik wykonywalny uruchamia sesję interaktywną.

| Opcja | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `-V` | flaga | | Wypisz `nurt <version>`. |
| `--version` | flaga | | Wypisz wersję wraz z danymi kompilacji, prawami autorskimi i licencją. |
| `-h`, `--help` | flaga | | Wypisz sposób użycia. Jako pierwszy argument po nazwie podpolecenia wypisuje sposób użycia tego podpolecenia i kończy działanie z kodem wyjścia 0, bez jego uruchamiania. |

Opcje programu (`--variant`, `--preview`, `--json`, `--format`, `--limit`, `--cursor`, `--verbosity`, `--privilege`, `--as-user`) są przyjmowane przed pierwszym słowem programu, więc `nurt --preview remove ...` jest równoważne `nurt exec --preview remove ...`. Słowa po pierwszym słowie programu należą do programu. Język nie ma opcji z myślnikiem.

## Wyjście i kolor {#output-and-colour}

Wynik jest zapisywany na standardowe wyjście w żądanym formacie. Błędy wiersza poleceń są zapisywane na standardowe wyjście błędów. Błąd języka w `check`, `fmt`, `ast` i `schema` jest zapisywany na standardowe wyjście błędów jako tekst, z wyjątkiem jawnego `--format json` lub `--json`: wtedy jest to dokument JSON na standardowym wyjściu (`{"error": {...}}` oraz `{"diagnostics": [], "error": {...}}` dla `check`, jak w `docs/reference/c-abi.md`), standardowe wyjście błędów pozostaje puste, a kod wyjścia jest taki, jaki błąd ma w postaci tekstowej. Błędy niebędące błędami języka, takie jak nieprawidłowa opcja lub nieczytelny plik, są zawsze zapisywane na standardowe wyjście błędów jako tekst. Kolory podlegają `output.color` (`NURT_COLOR`): przy `auto` kolorowany jest tylko terminal, a zmienna `NO_COLOR` wyłącza kolory. Baner sesji interaktywnej jest zapisywany tylko na terminal interaktywny.

## Funkcje kompilacji {#build-features}

Podpolecenia `mcp`, `lsp`, `migrate`, `service` i `journal` są implementowane przez opcjonalne komponenty. Każdy komponent jest dołączany przez funkcję Cargo skrzynki `nurt-cli`; kompilacja domyślna dołącza je wszystkie (funkcje domyślne `nurt-cli`); kompilacja z `--no-default-features` nie dołącza żadnego. Podpolecenie, którego komponent nie wchodzi w skład kompilacji, zapisuje na standardowe wyjście błędów `nurt: <subcommand> is not available in this build` i kończy działanie z kodem wyjścia 3. `nurt doctor` wymienia te podpolecenia wraz z funkcją, której każde z nich potrzebuje.

| Funkcja | Dołącza | Podpolecenie |
|---|---|---|
| `mcp` | `nurt-mcp` | `mcp --stdio` |
| `mcp-http` | `nurt-mcp` z transportem HTTP | `mcp --http` |
| `lsp` | `nurt-langserver` | `lsp` |
| `migrate` | `nurt-migrate` | `migrate` |
| `service` | `nurt-service` | `service` |
| `journal` | `nurt-journal` | `journal salvage` |
| `stub` | `nurt-ffi` z zarejestrowanymi odpowiedziami | `serve` (tylko do testów) |

## `exec` {#exec}

Uruchamia program podany jako argumenty.

Składnia:

```
nurt exec [--variant a|b|c] [--preview] [--json | --format F] [--limit N]
          [--cursor C] [--verbosity V] [--privilege P] [--as-user NAME] <program>
```

| Parametr | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `--variant` | `a`, `b`, `c` | skonfigurowany wariant | Notacja tekstu programu. |
| `--preview` | flaga | wyłączone | Oblicz skutek bez jego zastosowania. |
| `--json` | flaga | wyłączone | To samo co `--format json`. |
| `--format` | `text`, `json`, `markdown`, `compact` | `text` | Format wyniku. |
| `--limit` | liczba całkowita od 1 do 5000 | 50 | Rekordy na stronę. |
| `--cursor` | tekst | brak | Odczytaj kolejną stronę wcześniejszego wyniku; nie podaje się programu. |
| `--verbosity` | `quiet`, `normal`, `detailed` | `normal` | Poziom szczegółowości. |
| `--privilege` | `account`, `elevated`, `auto` | `privilege.default` | Uprawnienia operacji. |
| `--as-user` | nazwa konta | brak | Konto wykonujące operację (Linux i macOS). |

Wynik: wyrenderowany wynik programu; z `--json` dokument odpowiedzi. Kod wyjścia: zob. `exit-codes.md`.

Błędy: błąd języka jest częścią wyniku i ustawia kod wyjścia; nieprawidłowa opcja jest błędem użycia (2).

Przykład:

```
nurt exec --preview remove "/tmp/build/old.log"
```

## Pliki programów: `nurt run` {#program-files-nurt-run}

Uruchamia plik programu z wartościami jego parametrów. Plik jest jednym programem: dozwolonych jest kilka instrukcji, `when`, pętle, `steps` i funkcje; kilka potoków, które zmieniają, zapisuje się za pomocą `steps`. Pierwszy wiersz zaczynający się od `#!` jest ignorowany.

```text
nurt run <file> [name: value | flag ...] [--variant a|b|c] [--preview] [--json | --format F] [--limit N] [--verbosity V]
nurt run <file> --help
```

Plik deklaruje swoje parametry za pomocą `params { … }` jako pierwszą instrukcję. `name: value` nadaje wartość (dwa argumenty albo `name:value` w jednym); sama nazwa jest flagą typu `bool` i oznacza `true`, a `name: false` oznacza `false`. Wiersz poleceń przekazuje każdą wartość jako tekst, a zadeklarowany typ ją odczytuje (`7` dla `int`, `10mb` dla `size`, `/srv/app` dla `path`). Zanim cokolwiek zostanie uruchomione, silnik sprawdza, czy każdy wymagany parametr ma wartość (`E_MISSING_PARAM`), czy każda nazwa jest zadeklarowana (`E_UNKNOWN_PARAM`, z najbliższą nazwą) i czy każda wartość pasuje do swojego typu (`E_TYPE`); błąd wymienia deklarację. Plik bez `params` nie przyjmuje wartości.

`nurt run <file> --help` wypisuje deklarację z typami i wartościami domyślnymi oraz wiersz użycia:

```text
$ nurt run cleanup.nurt --help
Usage: nurt run cleanup.nurt dir: <path> [days: <int>] [dry: <bool>]

Parameters of cleanup.nurt:
  dir: path
  days: int = 30
  dry: bool = false
```

Kody wyjścia są takie same jak w `exec`. `nurt run` z następującą po nim nazwą kończącą się na `.nurt` uruchamia plik; każde inne `nurt run …` jest programem polecenia `run`.

Przykłady:

```text
nurt run cleanup.nurt dir: /srv/app days: 7 dry
nurt run cleanup.nurt dir: /srv/app --preview
```

## `script` {#script}

Uruchamia plik skryptu. Każdy potok pliku jest osobną operacją. Pierwszy wiersz zaczynający się od `#!` jest ignorowany, więc plik może zaczynać się od `#!/usr/bin/env nurt`.

Składnia:

```
nurt script <file | -> [--preview] [--json | --format F] [--limit N] [--verbosity V] [--privilege P] [--as-user NAME]
```

Plik `-` jest standardowym wejściem. Opcje są takie same jak w `exec`.

Błędy: nieczytelny plik kończy się kodem wyjścia 1.

Przykład:

```
nurt script clean.nurt --preview
```

## `repl` {#repl}

Uruchamia sesję interaktywną.

Składnia:

```
nurt repl
```

Na terminalu sesja oferuje historię przechowywaną w katalogu stanu, uzupełnianie nazw poleceń i parametrów z katalogu poleceń, podświetlanie oraz baner: znak produktu, srebrny szewron i złoty szewron, narysowane półblokami (znakami 7-bitowego ASCII na terminalu bez UTF-8), z nazwą, zdaniem kanonicznym, wersją, wskazówką i wierszem praw autorskich obok. Na terminalu zbyt wąskim dla obu tekst znajduje się pod rysunkiem, a na terminalu węższym niż rysunek baner jest samym tekstem. Kursor jest migającym podkreślnikiem podczas wpisywania wiersza. Baner, znak zachęty i podświetlanie wiersza używają kolorów produktu: srebra i złota znaku oraz cyjanu jego błękitu. Terminal zgłaszający kolor 24-bitowy (`COLORTERM=truecolor`, Windows Terminal) pokazuje dokładnie te kolory, każdy inny terminal najbliższe kolory palety 256 kolorów; `NO_COLOR` i `output.color` je wyłączają. Wejście niebędące terminalem jest czytane wiersz po wierszu bez banera. Wiersz jest programem albo jednym z tych słów sesji:

| Słowo | Znaczenie |
|---|---|
| `help [topic]` | Pokaż wprowadzenie lub temat. |
| `bg <program>` | Uruchom program w tle jako numerowane zadanie. |
| `jobs` | Wymień zadania. |
| `fg [id]` | Poczekaj na zadanie i pokaż jego wynik; najnowsze zadanie bez `id`. |
| `kill [id]` | Anuluj zadanie; najnowsze zadanie bez `id`. |
| `exit`, `quit` | Zakończ sesję. |

Ctrl-C w trakcie działania programu anuluje go i wraca do znaku zachęty; program zgłasza `E_CANCELLED`, a sesja trwa dalej. Ctrl-C przy znaku zachęty odrzuca wiersz; Ctrl-D kończy sesję. Kodem wyjścia jest kod ostatnio uruchomionego programu.

Przykład:

```
printf 'find "/srv"\nexit\n' | nurt repl
```

## `check` {#check}

Parsuje, poprawia i wiąże program oraz zgłasza, czego by dotknął, bez jego uruchamiania.

Składnia:

```
nurt check <file | -e program> [--variant a|b|c] [--json | --format F]
```

| Parametr | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `file` | ścieżka lub `-` | brak | Plik programu; `-` jest standardowym wejściem. |
| `-e` | tekst | brak | Tekst programu podany w wierszu poleceń. |

Wynik: `ok`, liczba poprawek i podsumowanie kontroli; z `--json` dokument kontroli. Błędy: błąd języka jest zapisywany na standardowe wyjście błędów; z `--json` lub `--format json` jest to dokument `{"diagnostics": [], "error": {...}}` na standardowym wyjściu, z tym samym kodem wyjścia. Kontrola zgłasza błędy programu, które uruchomienie zgłosiłoby przy wiązaniu jego etapów, w tym `E_INVALID_VALUE` i `E_TYPE` dla literału porównywanego z polem znanego typu (`filter size > "abc"`), na pozycji literału.

Przykład:

```
nurt check -e 'remove "/tmp/build/old.log"'
```

## `fmt` {#fmt}

Wypisuje program w postaci kanonicznej.

Składnia:

```
nurt fmt <file | -> [--variant a|b|c] [--json | --format F]
```

Wynik: tekst programu w podanym wariancie, domyślnie w skonfigurowanym wariancie. Błędy: błąd języka jest zapisywany tak, jak opisano w sekcji „Wyjście i kolor”; udany wynik jest tekstem programu w każdym formacie. Komentarze są zachowywane: komentarz w osobnym wierszu pozostaje przed instrukcją lub etapem, który po nim następuje, a komentarz na końcu wiersza pozostaje na końcu wiersza etapu, który tam się skończył; puste wiersze są usuwane.

Przykład:

```
nurt fmt clean.nurt --variant b
```

## `ast` {#ast}

Konwertuje drzewo składni między notacjami.

Składnia:

```
nurt ast [--from a|b|c|json] [--to json|cbor|a|b|c] [--json | --format F] <input | ->
```

| Parametr | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `--from` | `a`, `b`, `c`, `json` | `json` | Notacja wejścia. |
| `--to` | `json`, `cbor`, `a`, `b`, `c` | `json` | Notacja wyjścia. |
| `--json`, `--format` | flaga, `text`, `json`, `markdown`, `compact` | `text` | Z `json` błąd języka jest dokumentem JSON. |
| `input` | tekst lub `-` | brak | Tekst wejściowy; `-` jest standardowym wejściem. |

Wynik: drzewo w żądanej notacji; `cbor` jest binarny. Format wybiera tylko postać błędu języka, jak opisano w sekcji „Wyjście i kolor”; notację udanego wyniku wybiera `--to`.

Przykład:

```
nurt ast --from a --to json 'find "/srv"'
```

## `schema` {#schema}

Wypisuje opublikowany schemat JSON albo definicję narzędzia `nurt` dla platformy AI.

Składnia:

```
nurt schema ast|bound|report|request|config|tool [--json | --format json|text]
nurt schema tool [--format <format>] [--language en|pl] [--groups core|all|<group>,<group>...]
```

`nurt schema tool` bez opcji wypisuje opublikowany schemat `nurt-tool/1`, schemat neutralnej definicji. Z co najmniej jedną opcją inną niż `--json`, `--format json` lub `--format text` wypisuje definicję narzędzia, wyrenderowaną z tekstów opisów i rewizji katalogu poleceń zainstalowanego silnika: `--format` wybiera platformę (`anthropic`, `openai`, `openai-strict`, `openai-responses`, `openai-responses-strict`, `gemini`, `mcp` lub `json-schema`; domyślnie `json-schema`), `--language` język opisu (domyślnie `en`), a `--groups` grupy poleceń opisane oprócz grupy rdzenia (domyślnie `core`). Formaty, argumenty narzędzia i reguły dla poszczególnych platform znajdują się w [Definicji narzędzia](../integrators/tool-definition.md).

Błędy: nieznana nazwa, format, język lub grupa to `E_UNKNOWN_VALUE` ze znanymi wartościami; nieznana opcja lub brakująca wartość jest błędem użycia. Z `--json` lub `--format json` nieznana nazwa jest zapisywana jako dokument JSON na standardowym wyjściu, jak opisano w sekcji „Wyjście i kolor”.

Przykłady:

```
nurt schema report
nurt schema tool --format anthropic
```

## `catalog` {#catalog}

Opisuje katalog poleceń, grupę lub polecenie.

Składnia:

```
nurt catalog [command | group | check-distance]
```

Bez argumentu polecenie wypisuje podsumowanie katalogu. `check-distance` jest dostępne, gdy komponenty poprawiania wchodzą w skład kompilacji.

Przykład:

```
nurt catalog remove
```

## `help` {#help}

Pokazuje wbudowaną pomoc.

Składnia:

```
nurt help [topic]
```

Bez tematu polecenie wypisuje wprowadzenie; pierwszy wiersz nazywa produkt i jego wersję. Tematem jest polecenie, grupa lub `core`.

Przykład:

```
nurt help remove
```

## `history` {#history}

Wymienia zapisane operacje. Polecenie jest skrótem programu `history` z podanymi argumentami.

Składnia:

```
nurt history [arguments of the history command]
```

Opcje `exec` są przyjmowane przed argumentami.

Przykład:

```
nurt history limit: 5
```

## `undo` {#undo}

Cofa zapisane operacje. Polecenie jest skrótem programu `undo` z podanymi argumentami.

Składnia:

```
nurt undo [arguments of the undo command]
```

Przykład:

```
nurt undo
```

## `redo` {#redo}

Ponawia cofnięte operacje. Polecenie jest skrótem programu `redo` z podanymi argumentami.

Składnia:

```
nurt redo [arguments of the redo command]
```

Przykład:

```
nurt redo
```

## `serve` {#serve}

Obsługuje protokół żądań dla procesu klienta.

Składnia:

```
nurt serve --stdio | --socket <path> [--framing ndjson|content-length] [--stub <directory>]
```

| Parametr | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `--stdio` | flaga | brak | Obsługuje jednego klienta na standardowym wejściu i wyjściu. |
| `--socket <path>` | ścieżka | brak | Zarezerwowane dla transportu przez gniazdo lokalne. Opcja jest rozpoznawana i w wydaniu 0.1.2 kończy działanie z kodem wyjścia 3; należy łączyć się przez `--stdio`. |
| `--framing` | `ndjson` lub `content-length` | `ndjson` | Ramkowanie komunikatów. |
| `--stub <directory>` | ścieżka | brak | Odpowiada na żądania z zarejestrowanych przypadków zamiast z silnika. Wymaga funkcji `stub`; bez niej polecenie kończy działanie z kodem wyjścia 3. |

Wymagane jest dokładnie jedno przyłączenie. Na standardowe wyjście nie jest zapisywane nic poza komunikatami protokołu i nie jest wypisywany żaden baner. Kod wyjścia to 0 po `shutdown` lub na końcu wejścia oraz 1, gdy nie można odczytać wejścia lub zapisać wyjścia. Nieprawidłowe argumenty kończą się kodem wyjścia 2.

Przykład:

```
nurt serve --stdio
```

## `mcp` {#mcp}

Obsługuje interfejs narzędzi protokołu Model Context Protocol.

Składnia:

```
nurt mcp [--stdio | --http <address>]
```

Bez komponentu w kompilacji polecenie kończy działanie z kodem wyjścia 3.

Przykład:

```
nurt mcp --stdio
```

## `lsp` {#lsp}

Obsługuje protokół języka dla plików `.nurt`.

Składnia:

```
nurt lsp
```

Bez komponentu w kompilacji polecenie kończy działanie z kodem wyjścia 3.

Przykład:

```
nurt lsp
```

## `migrate` {#migrate}

Tłumaczy skrypt powłoki na skrypt Nurt.

Składnia:

```
nurt migrate <file.sh | file.ps1>
```

Bez komponentu w kompilacji polecenie kończy działanie z kodem wyjścia 3.

Przykład:

```
nurt migrate deploy.sh
```

## `service` {#service}

Zarządza usługą działającą w tle.

Składnia:

```
nurt service install | uninstall | status | run
```

Bez komponentu w kompilacji polecenie kończy działanie z kodem wyjścia 3.

Przykład:

```
nurt service status
```

## `journal` {#journal}

Utrzymuje dziennik.

Składnia:

```
nurt journal salvage <volume>
```

`salvage` naprawia wpisy oznaczone jako uszkodzone na woluminie. Bez komponentu w kompilacji polecenie kończy działanie z kodem wyjścia 3.

Przykład:

```
nurt journal salvage /
```

## `ext` {#ext}

Zarządza rozszerzeniami.

Składnia:

```
nurt ext install | remove | list
```

Bez komponentu w kompilacji polecenie kończy działanie z kodem wyjścia 3.

Przykład:

```
nurt ext list
```

## `doctor` {#doctor}

Sprawdza instalację i wypisuje tabelę kontroli: wersja, platforma, formaty, funkcje, katalog stanu (utworzony i usunięty plik próbny), pliki konfiguracji, usługi (usługi współdzielone zarejestrowane przez silnik, według nazwy), dziennik, komponenty języka oraz podpolecenia, które nie są dostępne. Część, której kompilacja nie może zbadać, jest wymieniana jako `unavailable`. Kod wyjścia to 1, gdy kontrola się nie powiodła, w przeciwnym razie 0.

Składnia:

```
nurt doctor
```

Przykład:

```
nurt doctor
```

## `version` {#version}

Wypisuje pełną wersję, ten sam tekst co `nurt --version`.

Składnia:

```
nurt version
```

Wynik: trzy wiersze. Pierwszy to `nurt <version> (<commit> <date>)`; kompilacja bez danych repozytorium pomija nawias. Drugi to wiersz praw autorskich, trzeci `License: Apache-2.0`. Kod wyjścia 0.

Przykład:

```
nurt version
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
