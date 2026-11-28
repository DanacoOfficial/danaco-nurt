<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/configuration.md -->
<!-- Responsibility:   Documents Configuration `nurt-config/1`. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Konfiguracja `nurt-config/1` {#configuration-nurt-config1}

Danaco Nurt odczytuje swoją konfigurację z plików TOML, zmiennych środowiskowych
i ustawień czasu wykonania na siedmiu poziomach pierwszeństwa. Postacią
maszynowo czytelną formatu jest schemat JSON `spec/schemas/nurt-config-1.schema.json`,
generowany z typu Rust `nurt_contracts::config::Config`. Przykładowe pliki
znajdują się w `spec/golden/config/`;
`spec/golden/config/defaults.toml` wymienia każdy klucz wraz z jego wartością
domyślną.

## 1. Pliki {#1-files}

Plik konfiguracji jest dokumentem TOML. Tabele odpowiadają poniższym sekcjom, a
każdy klucz jest opcjonalny: pusty plik jest konfiguracją wbudowaną.

| System | Plik systemowy | Plik użytkownika | Katalog stanu |
|---|---|---|---|
| Linux | `/etc/nurt/nurt.toml` | `$XDG_CONFIG_HOME/nurt/nurt.toml` (domyślnie `~/.config/nurt/nurt.toml`) | `$XDG_STATE_HOME/nurt/` |
| macOS | `/Library/Application Support/Nurt/nurt.toml` | `~/Library/Application Support/Nurt/nurt.toml` | `~/Library/Application Support/Nurt/` |
| Windows | `%PROGRAMDATA%\Nurt\nurt.toml` | `%APPDATA%\Nurt\nurt.toml` | `%LOCALAPPDATA%\Nurt\` |

Plik projektu nazywa się `.nurt.toml` i leży w katalogu projektu. Zmienna
`NURT_CONFIG` wskazuje jeszcze jeden plik. Aplikacja osadzająca bibliotekę może
przekazać konfigurację w JSON przy otwieraniu silnika (`nurt_engine_open`); jest
ona stosowana na poziomie pliku `NURT_CONFIG`, po tym pliku.

## 2. Poziomy i pierwszeństwo {#2-levels-and-precedence}

| Poziom | Identyfikator | Źródło |
|---|---|---|
| 1 | `builtin` | wbudowane wartości domyślne |
| 2 | `system` | plik systemowy |
| 3 | `user` | plik użytkownika |
| 4 | `project` | `.nurt.toml` projektu |
| 5 | `config_file` | plik wskazany przez `NURT_CONFIG`; konfiguracja aplikacji osadzającej |
| 6 | `environment` | zmienne `NURT_*`, odczytywane przy uruchomieniu procesu |
| 7 | `runtime` | parametry wywołania, opcje żądania, `journal.limits` |

Wyższy poziom nadpisuje niższy, klucz po kluczu:

- tabele są scalane: klucz ustawiony na wyższym poziomie zastępuje tylko ten
  klucz;
- skalary i tablice są zastępowane w całości (do tablicy nigdy się nie
  dopisuje);
- warstwy tego samego poziomu są stosowane w kolejności ich odczytu.

Poziom, który ustawił każdy klucz, jest zapisywany. Raportuje go na przykład
`journal.status` dla obowiązujących limitów dziennika.

Klucz, którego format nie definiuje, jest ignorowany i zgłaszany
wywołującemu; wartość niezgodna z typem swojego klucza jest błędem, który
wskazuje klucz, poziom i plik.

## 3. Pliki projektu i zaufanie {#3-project-files-and-trust}

Plik projektu pochodzi z repozytorium i nie jest zaufany. Może ustawiać
wyłącznie:

| Klucze | Warunek |
|---|---|
| `scope.base`, `scope.project_roots` | zawsze |
| `report.*`, `run.*` | zawsze |
| `journal.*` | tylko dla projektu zaufanego |

Projekt jest zaufany, gdy jego katalog jest wymieniony w
`scope.trusted_projects`. Listę można ustawić w pliku systemowym, w pliku
użytkownika, w pliku `NURT_CONFIG` (lub w konfiguracji aplikacji osadzającej)
oraz w zmiennej `NURT_TRUSTED_PROJECTS`. Nie można jej ustawić w pliku projektu
ani w czasie wykonania, tak aby ani repozytorium, ani żądanie nie mogły nadać
zaufania.

Klucze, które nie są stosowane, są zgłaszane za pomocą tych kodów `notable`:

| Klucz | Poziom | Kod `notable` |
|---|---|---|
| `scope.trust*` | project, runtime | `trust_config_ignored` |
| `service.*` | project | `trust_config_ignored` |
| `privilege.*` | project | `trust_config_ignored` |
| `journal.*` | projekt, który nie jest zaufany | `journal_config_ignored` |
| każdy inny klucz spoza powyższej listy | project | `project_config_ignored` |

Reguła jest dostępna dla implementacji jako czysta funkcja
`nurt_contracts::config::allowed_at(level, key)`.

## 4. Progi ochronne {#4-protection-floors}

`journal.floor.max_ops` i `journal.floor.max_age` są najniższymi limitami
kroków i wieku dziennika. Niższa wartość efektywna, z dowolnego poziomu, jest
podnoszona do progu, a zmiana jest odnotowywana w `journal.status`; `unlimited`
jest powyżej każdego progu. Wartości pochodzące z pliku projektu są podnoszone
do co najmniej 10 kroków i 5 minut, nawet gdy żaden próg nie jest ustawiony.
Progi ustawia zwykle administrator (plik systemowy) albo aplikacja osadzająca
przy otwieraniu silnika; próg może jedynie podnieść ochronę: gdy kilka poziomów
ustawia ten sam próg, wygrywa najwyższa wartość, niezależnie od pierwszeństwa
poziomów, tak aby plik użytkownika nie mógł obniżyć progu ustawionego w pliku
systemowym.

## 5. Wartości {#5-values}

| Typ | Składnia | Przykłady |
|---|---|---|
| czas trwania | cyfry, po których następuje `ms`, `s`, `m`, `h`, `d` lub `w`, także łączone; bez gołych liczb | `"1500ms"`, `"30m"`, `"1h30m"` |
| rozmiar | liczba z `b`, `kb`, `mb`, `gb`, `tb` (potęgi 1000) lub `kib`, `mib`, `gib`, `tib` (potęgi 1024); dozwolony jest ułamek; liczba całkowita oznacza bajty | `"256mib"`, `"1.5tb"`, `4096` |
| procent | liczba z co najwyżej dwoma miejscami po przecinku, po której następuje `%` | `"15%"`, `"12.5%"` |
| limit kroków | liczba całkowita od `1` do `100000` albo `"unlimited"`; `0` nie jest akceptowane | `30`, `"unlimited"` |
| limit wieku | czas trwania od `10s` do `365d` albo `"unlimited"` | `"30m"` |
| budżet rozmiaru | rozmiar co najmniej `1mib`, procent pojemności woluminu (`1%` do `90%`), procent wolnego miejsca łącznie z danymi dziennika (`1%free` do `100%free`) albo `"unlimited"` | `"20gib"`, `"10%"`, `"50%free"` |
| limit bajtów | rozmiar albo `"unlimited"` | `"unlimited"` |
| rozmiar lub procent | rozmiar albo procent pojemności woluminu | `"5gib"`, `"15%"` |

Jednostki zapisuje się małymi literami. Gdy konfiguracja jest zapisywana
z powrotem (na przykład przez `nurt_engine_config_get`), czasy trwania i
rozmiary używają największej jednostki, która dokładnie reprezentuje wartość:
`"120s"` jest zapisywane jako `"2m"`, `"24h"` jako `"1d"`.

## 6. Zmienne środowiskowe {#6-environment-variables}

| Zmienna | Klucz | Wartość |
|---|---|---|
| `NURT_CONFIG` | — | ścieżka dodatkowego pliku konfiguracji |
| `NURT_SYNTAX` | `syntax.variant` | tekst |
| `NURT_SYNTAX_TOLERANT` | `syntax.tolerant` | wartość logiczna |
| `NURT_CORRECTION` | `correction.enabled` | wartość logiczna |
| `NURT_BASE` | `scope.base` | tekst |
| `NURT_PROJECT_ROOTS` | `scope.project_roots` | lista |
| `NURT_TRUSTED_PROJECTS` | `scope.trusted_projects` | lista |
| `NURT_JOURNAL` | `journal.enabled` | wartość logiczna |
| `NURT_JOURNAL_MAX_OPS` | `journal.max_ops` | tekst |
| `NURT_JOURNAL_MAX_AGE` | `journal.max_age` | tekst |
| `NURT_JOURNAL_MAX_SIZE` | `journal.max_size` | tekst |
| `NURT_JOURNAL_MAX_OP_SIZE` | `journal.max_op_size` | tekst |
| `NURT_JOURNAL_OVERSIZE` | `journal.oversize` | tekst |
| `NURT_JOURNAL_OVERWRITE` | `journal.overwrite` | tekst |
| `NURT_JOURNAL_NETWORK` | `journal.volumes.network` | wartość logiczna |
| `NURT_JOURNAL_REMOVABLE` | `journal.volumes.removable` | wartość logiczna |
| `NURT_SPACE_RESERVE` | `space.reserve` | tekst |
| `NURT_SPACE_SOFT` | `space.soft` | tekst |
| `NURT_SPACE_POLL_MS` | `space.poll_ms` | liczba całkowita |
| `NURT_RUN_TIMEOUT` | `run.timeout` | tekst |
| `NURT_RUN_WAIT` | `run.wait` | tekst |
| `NURT_LANGUAGE` | `output.language` | tekst |
| `NURT_COLOR` | `output.color` | tekst |
| `NURT_RESULTS_DIR` | `output.results_dir` | tekst |
| `NURT_MAX_RECORDS` | `limits.max_records` | liczba całkowita |
| `NURT_THREADS` | `limits.threads` | liczba całkowita |
| `NURT_PROGRAMS_CATALOGS` | `programs.catalogs` | lista |
| `NURT_HTTP_PROXY` | `http.proxy` | tekst |
| `NURT_EXT_DIR` | `ext.dir` | tekst |
| `NURT_STATE_DIR` | `state.dir` | tekst |
| `NURT_LOG` | `log.level` | tekst |
| `NURT_SESSION` | `session.id` | tekst |

Wartości logiczne to `1`, `true`, `yes`, `on`, `0`, `false`, `no` lub `off` (w
dowolnej wielkości liter). Listy są rozdzielane znakiem `:` (`;` w systemie
Windows); puste elementy są pomijane. Tekst jest konwertowany według typu
klucza, więc `NURT_JOURNAL_MAX_OPS=unlimited` i `NURT_JOURNAL_MAX_OPS=30` są
obie poprawne. Klient HTTP respektuje także standardowe zmienne `HTTP_PROXY`,
`HTTPS_PROXY` i `NO_PROXY`. Pozostałe zmienne są ignorowane.

## 7. Klucze {#7-keys}

„Brak” w kolumnie wartości domyślnej oznacza, że klucz nie jest ustawiony i
obowiązuje zachowanie opisane w kolumnie znaczenia.

### 7.1 `[syntax]` {#71-syntax}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `syntax.variant` | `a`, `b`, `c` | `a` | wariant notacji postaci tekstowej |
| `syntax.tolerant` | wartość logiczna | `true` | akceptowanie także notacji pozostałych wariantów |

### 7.2 `[correction]` {#72-correction}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `correction.enabled` | wartość logiczna | `true` | poprawianie zapisanej postaci programów |
| `correction.max_read` | liczba całkowita | `8` | budżet poprawek programów, które tylko odczytują |
| `correction.max_effect` | liczba całkowita | `4` | budżet poprawek programów, które coś zmieniają |
| `correction.require_first_char` | wartość logiczna | `true` | rozmyte poprawianie nazw zachowuje pierwszy znak |
| `correction.candidates_ttl` | czas trwania | `10m` | czas życia kandydatów, które można zaakceptować po programie niejednoznacznym |

### 7.3 `[scope]` {#73-scope}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `scope.base` | ścieżka | brak | katalog bezwzględny dla ścieżek względnych |
| `scope.project_roots` | lista ścieżek | `[]` | katalogi projektów; zmiany poza nimi są raportowane jako `outside_project` |
| `scope.trusted_projects` | lista ścieżek | `[]` | projekty, których `.nurt.toml` może zmieniać `journal.*` (sekcja 3) |

### 7.4 `[journal]` {#74-journal}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `journal.enabled` | wartość logiczna | `true` | zapisywanie zmian w dzienniku |
| `journal.max_ops` | limit kroków | `30` | liczba ostatnich operacji zmieniających, które można cofnąć |
| `journal.max_age` | limit wieku | `30m` | jak długo poprzednia zawartość jest przechowywana |
| `journal.max_size` | budżet rozmiaru | `unlimited` | budżet rozmiaru na wolumin; rezerwa miejsca zawsze obowiązuje |
| `journal.max_size_min` | rozmiar | `2gib` | dolne ograniczenie budżetów procentowych |
| `journal.max_size_max` | rozmiar | `50gib` | górne ograniczenie budżetów procentowych |
| `journal.max_size_total` | limit bajtów | `unlimited` | budżet sumowany dla wszystkich woluminów |
| `journal.max_op_size` | limit bajtów | `unlimited` | limit pojedynczej operacji |
| `journal.oversize` | `skip`, `keep` | `skip` | operacja większa niż budżet: wykonana bez zachowania poprzedniej zawartości (raportowana jako nieodwracalna) albo zachowana ponad budżet |
| `journal.age_clock` | `wall`, `active` | `wall` | wpisy starzeją się według czasu zegarowego albo tylko wtedy, gdy komputer pracuje |
| `journal.floor.max_ops` | limit kroków | brak | najniższy limit kroków (sekcja 4) |
| `journal.floor.max_age` | limit wieku | brak | najniższy limit wieku (sekcja 4) |
| `journal.pin_max_age` | czas trwania | `24h` | domyślny czas trwania `journal.pin` |
| `journal.overwrite` | `exchange`, `clone_or_copy` | `exchange` | zachowywanie nadpisywanych plików |
| `journal.count_undo_as_op` | wartość logiczna | `true` | cofnięcie liczy się jako krok |
| `journal.history_keep` | liczba całkowita | `1000` | wygasłe wpisy, których metryki są zachowywane dla `history` |
| `journal.dir_name` | tekst | `.nurt-journal` | nazwa katalogu dziennika na każdym woluminie |
| `journal.volumes.network` | wartość logiczna | `false` | prowadzenie dziennika na woluminach sieciowych |
| `journal.volumes.removable` | wartość logiczna | `false` | prowadzenie dziennika na woluminach wymiennych |
| `journal.volume` | tabela według punktu montowania | pusta | ustawienia jednego woluminu: `enabled`, `reserve`, `soft`, `max_ops`, `max_age`, `max_size`; nieobecne klucze są dziedziczone |

### 7.5 `[space]` {#75-space}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `space.reserve` | rozmiar lub procent | `15%` | wolne miejsce, którego dziennik nigdy nie zajmuje |
| `space.reserve_min` | rozmiar | `5gib` | dolne ograniczenie rezerwy procentowej |
| `space.reserve_max` | rozmiar | `50gib` | górne ograniczenie rezerwy procentowej |
| `space.soft` | rozmiar lub procent | brak: rezerwa plus większa z wartości 5 % i 2 GiB, nie więcej niż rezerwa plus 20 GiB | próg, przy którym usługa zaczyna zwalniać dane dziennika |
| `space.margin` | rozmiar lub procent | brak: mniejsza z wartości 1 % i 1 GiB | margines powyżej rezerwy |
| `space.poll_ms` | liczba całkowita | `250` | okres pomiaru wolnego miejsca, w milisekundach |
| `space.poll_fast_ms` | liczba całkowita | `50` | okres pomiaru w pobliżu progu, w milisekundach |
| `space.heartbeat_ms` | liczba całkowita | `500` | okres sygnału życia usługi, w milisekundach |
| `space.heartbeat_max_age_ms` | liczba całkowita | `2000` | wiek, po którym sygnał życia jest nieaktualny, w milisekundach |
| `space.sync_budget_ms` | liczba całkowita | `1500` | budżet synchronicznego zwalniania przed zapisem, w milisekundach |
| `space.settle_ms` | liczba całkowita | `1000` | oczekiwanie na wzrost wolnego miejsca po zwolnieniu, w milisekundach |
| `space.quota_aware` | wartość logiczna | `true` | uwzględnianie limitów (quot) konta |

### 7.6 `[run]` {#76-run}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `run.timeout` | czas trwania | `120s` | limit czasu programu |
| `run.wait` | czas trwania | `3s` | czas oczekiwania na pierwszym planie, zanim program będzie kontynuowany w tle |
| `run.idle_timeout` | czas trwania | `60s` | czas bez wyjścia i aktywności procesora, po którym program jest raportowany jako zablokowany |
| `run.kill_grace` | czas trwania | `5s` | czas między żądaniem zakończenia a zabiciem grupy procesów |
| `run.env_defaults` | tabela tekstów | `PYTHONUTF8="1"`, `PYTHONIOENCODING="utf-8"`, `NO_COLOR="1"`, `TERM="dumb"`, `GIT_TERMINAL_PROMPT="0"`, `PAGER="cat"`, `GIT_PAGER="cat"`, `DEBIAN_FRONTEND="noninteractive"` | zmienne dodawane do środowiska każdego programu |
| `run.track_changes` | `auto`, `scan`, `events`, `none` | `auto` | wykrywanie zmian plików dokonanych przez program |
| `run.track_ignore` | lista nazw | `[".git"]` | nazwy wyłączone z wykrywania zmian |
| `run.preimage` | `auto`, `off` | `auto` | zachowywanie poprzedniej zawartości zadeklarowanych i obserwowanych plików przed uruchomieniem programu |
| `run.preimage_budget` | czas trwania | `1500ms` | budżet czasu zachowywania poprzedniej zawartości |
| `run.preimage_max_entries` | liczba całkowita | `20000` | największa liczba zachowywanych plików |
| `run.preimage_copy_max` | rozmiar | `256mib` | największa ilość kopiowana, gdy klonowanie jest niedostępne |
| `run.preimage_exclude` | lista nazw | `node_modules`, `.git`, `target`, `build`, `dist`, `.venv`, `__pycache__` | nazwy nigdy niezachowywane |
| `run.irreversible_patterns` | lista wzorców | `[]` | wzorce argumentów dodawane do wbudowanej tabeli programów ze skutkami poza komputerem; tokeny są rozdzielane spacjami, `*` pasuje do dowolnego tokenu, `a\|b` pasuje do alternatyw; wbudowanej tabeli nie można usunąć |
| `run.parallel` | liczba całkowita | `1` | domyślna liczba partii uruchamianych jednocześnie |
| `run.spool_max` | limit bajtów | `1gib` | największe wyjście zachowywane na program: jego pierwsza część oraz, do jednej czwartej limitu (najwyżej 64 MiB), jego ostatnia część; bajty między nimi są pomijane, oznaczane wierszem `[... N bytes omitted ...]` w pliku buforowym i raportowane z uwagą `spool_limit`; liczniki i skrót obejmują całe wyjście; `unlimited` zachowuje wszystko i może być ustawione na dowolnym poziomie, łącznie z poziomem czasu wykonania dla jednego przebiegu |
| `run.supervisor` | `auto`, `none` | `auto` | nadzór nad programami, które kontynuują w tle po zakończeniu wiersza poleceń; `none` dopuszcza programy w tle wyłącznie wewnątrz długo żyjącego procesu |
| `run.waits_threshold` | czas trwania | `3s` | czas oczekiwania, powyżej którego program jest wymieniany wśród oczekiwań sprawdzanego programu |

### 7.7 `[output]` {#77-output}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `output.language` | `en`, `pl` | `en` | język komunikatów i pomocy |
| `output.color` | `auto`, `always`, `never` | `auto` | kolory dla ludzi |
| `output.head` | liczba całkowita | `20` | pierwsze wiersze wyjścia programu pokazywane w odpowiedzi |
| `output.tail` | liczba całkowita | `40` | ostatnie wiersze wyjścia programu pokazywane w odpowiedzi |
| `output.max_bytes` | liczba całkowita | `4096` | bajty tekstu jednego strumienia wyjścia programu pokazywane w odpowiedzi; `0` oznacza brak limitu |
| `output.spool_ttl` | czas trwania | `1h` | czas życia plików z pełnym wyjściem programu |
| `output.results_dir` | ścieżka | brak: `results` w katalogu stanu | katalog pełnych wyników zapisanych do plików |

### 7.8 `[report]` {#78-report}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `report.mass_threshold` | liczba całkowita | `200` | liczba celów, od której operacja jest raportowana jako zbiorcza |
| `report.diff_max_hunks` | liczba całkowita | `20` | fragmenty w próbce różnic |
| `report.diff_max_lines` | liczba całkowita | `200` | wiersze w próbce różnic |
| `report.notable.mass_files` | liczba całkowita | `100` | usunięte pliki, od których raportowane jest `mass_delete` |
| `report.notable.mass_bytes` | rozmiar | `1gib` | usunięte bajty, od których raportowane jest `mass_delete` |
| `report.notable.mass_ratio` | liczba | `0.8` | udział usuniętego katalogu, od którego raportowane jest `mass_delete` |
| `report.notable.shrink_ratio` | liczba | `0.5` | udział rozmiaru, o który kurczy się plik o rozmiarze co najmniej 1 KiB, od którego raportowane jest `size_shrunk` |
| `report.notable.per_file_repl` | liczba całkowita | `100` | zamiany w jednym pliku, od których raportowane jest `many_per_file` |
| `report.notable.targets_high` | liczba całkowita | `500` | cele, od których raportowane jest `target_count_high` |

### 7.8a `[continuation]` {#78a-continuation}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `continuation.max_age` | czas trwania | `1h` | kontynuacja starsza niż ta wartość wygasa |
| `continuation.max_count` | liczba całkowita | `50` | największa liczba zachowywanych kontynuacji; najstarsza jest usuwana jako pierwsza |
| `continuation.max_size` | rozmiar | `256mib` | największa ilość miejsca wszystkich zachowywanych wyników; najstarszy jest usuwany jako pierwszy |

Zachowywane wyniki znajdują się w obszarze `results` katalogu stanu, poza
dziennikiem, i są usuwane na końcu sesji konsoli lub przez te limity.

### 7.9 `[text]`, `[limits]`, `[fs]` {#79-text-limits-fs}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `text.legacy_encodings` | lista | `windows-1250`, `ibm852`, `iso-8859-2`, `windows-1252` | starsze kodowania brane pod uwagę, w kolejności, gdy plik nie jest poprawnym UTF-8 |
| `text.new_file_line_ending` | `lf`, `crlf` | `lf` | koniec wiersza nowych plików |
| `limits.max_records` | liczba całkowita | `5000000` | największa liczba rekordów przechowywanych w pamięci przez jeden etap; więcej to błąd (`E_LIMIT`), nigdy ciche obcięcie |
| `limits.max_file_bytes_text` | rozmiar | `1gib` | największy plik odczytywany jako tekst |
| `limits.threads` | liczba całkowita | brak: liczba rdzeni procesora | wątki robocze |
| `fs.sharing_retry_ms` | liczba całkowita | `2000` | czas ponowień po naruszeniu współdzielenia (Windows), w milisekundach |

### 7.9a `[loop]`, `[function]` {#79a-loop-function}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `loop.max_iterations` | liczba całkowita | `10000` | największa liczba iteracji `repeat` lub `while` bez `limit` (jawny `limit` zastępuje oba klucze pętli); osiągnięcie jej kończy `repeat` błędem `E_LIMIT`, a `while` normalnie, i raport to podaje |
| `loop.max_time` | czas trwania | `10m` | najdłuższy czas `repeat` lub `while` bez `limit`, z tym samym wynikiem |
| `function.max_depth` | liczba całkowita, od 1 do 1024 | `64` | najgłębsze zagnieżdżenie wywołań funkcji zdefiniowanych przez `define`; głębsza rekurencja kończy się błędem `E_LIMIT` |

### 7.10 `[code]` {#710-code}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `code.servers` | tabela według języka | `python`: `pyright-langserver --stdio`; `typescript`, `javascript`: `typescript-language-server --stdio`; `go`: `gopls`; `rust`: `rust-analyzer`; `c`, `cpp`: `clangd` | serwery języków; każdy wpis ma `command` (program i argumenty, wyszukiwany w `PATH`) oraz opcjonalne `init` (opcje inicjalizacji) |
| `code.servers_allow` | `all`, `off` lub lista języków | `all` | serwery, które mogą zostać uruchomione |
| `code.server_idle` | czas trwania | `10m` | czas, po którym nieużywany serwer się zatrzymuje |
| `code.server_retry` | czas trwania | `5m` | czas przed ponownym uruchomieniem serwera, który zawiódł |
| `code.max_servers` | liczba całkowita | `4` | serwery działające jednocześnie |
| `code.timeout` | czas trwania | `20s` | limit czasu żądania do serwera |
| `code.timeout_max` | czas trwania | `2m` | największy limit czasu, jaki może zażądać polecenie |
| `code.index_max_files` | liczba całkowita | `20000` | największa liczba plików indeksowanych do analizy składniowej |
| `code.file_bytes_max` | rozmiar | `1mib` | największy analizowany plik |
| `code.diagnostics_quiet` | czas trwania | `1500ms` | czas bez nowych diagnostyk, po którym diagnostyki są kompletne (co najmniej `50ms`) |

### 7.11 `[documents]`, `[sql]` {#711-documents-sql}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `documents.max_bytes` | rozmiar | `256mib` | największy odczytywany dokument ustrukturyzowany |
| `documents.yaml_version` | `1.1`, `1.2` | `1.2` | wersja YAML dokumentów bez dyrektywy |
| `documents.xml_external_entities` | wartość logiczna | `false` | rozwiązywanie encji zewnętrznych dokumentów XML |
| `sql.busy_timeout` | czas trwania | `5s` | oczekiwanie na zablokowaną bazę SQLite |

### 7.12 `[programs]`, `[http]` {#712-programs-http}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `programs.catalogs` | lista ścieżek lub tabel `{ path, priority, enabled }` | `[]` | zewnętrzne katalogi programów w formacie `nurt-program-catalog/1`; bez nich polecenia opisują zainstalowane programy |
| `programs.index_dir` | ścieżka | brak: katalog pamięci podręcznej użytkownika | katalog indeksu programów |
| `programs.help_timeout` | czas trwania | `3s` | limit czasu odczytu pomocy programu na żądanie |
| `programs.sources_disabled` | lista wartości `path`, `package`, `app`, `man`, `catalog` | `[]` | źródła informacji o programach, które nie są używane |
| `http.proxy` | URL | brak: ustawienie systemowe | serwer pośredniczący żądań HTTP i pobrań Git |

### 7.13 `[git]` {#713-git}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `git.trash_refs` | wartość logiczna | `false` | zachowywanie usuniętych gałęzi w `refs/nurt/trash/` |
| `git.hooks.timeout` | czas trwania | `10m` | limit czasu haka repozytorium |
| `git.ssh.known_hosts` | `strict`, `accept_new` | `strict` | weryfikacja kluczy hostów SSH |
| `git.ssh.identity_files` | lista ścieżek | `[]`: standardowe pliki tożsamości | pliki kluczy prywatnych |
| `git.signing.key_file` | ścieżka | brak | plik tajnego klucza OpenPGP, importowany do tymczasowego katalogu domowego GnuPG w celu podpisywania zatwierdzeń, gdy konfiguracja repozytorium nie wskazuje klucza przechowywanego przez `gpg-agent` |
| `git.signing.openpgp_source` | `gpg_agent`, `key_file` | brak: gpg-agent, a następnie plik klucza | jedyny posiadacz tajnego klucza używanego do podpisów OpenPGP; podpis w obu przypadkach tworzy GnuPG (`gpg.openpgp.program` lub `gpg.program`) |
| `git.checkpoint.keep` | liczba całkowita | `50` | punkty kontrolne zachowywane na repozytorium |
| `git.status.limit` | liczba całkowita | `200` | wpisy wymieniane przez `git.status` przed stronicowaniem |
| `git.network.timeout` | czas trwania | `5m` | limit czasu operacji sieciowej |

Konfiguracja repozytorium Git (na przykład `user.signingKey`,
`commit.gpgSign`, `http.proxy`, `core.*`) obowiązuje tak jak w Git; powyższe
klucze obejmują tylko to, czego Git nie definiuje.

### 7.14 `[verify]`, `[ext]`, `[service]`, `[state]`, `[log]`, `[session]` {#714-verify-ext-service-state-log-session}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `verify.history_max` | liczba całkowita | `50` | przebiegi testów zachowywane do porównania |
| `ext.dir` | ścieżka | brak: `ext` w katalogu konfiguracji | katalog rozszerzeń |
| `ext.native` | wartość logiczna | `false` | zezwolenie na natywne biblioteki rozszerzeń obok WebAssembly |
| `service.mode` | `user`, `system` | `user` | tryb instalacji usługi działającej w tle; `user` działa dla konta (w systemie Linux z włączonym lingering), `system` jest zalecany dla serwerów |
| `state.dir` | ścieżka | brak: katalog stanu systemu operacyjnego | katalog stanu |
| `log.level` | `error`, `warn`, `info`, `debug`, `trace` | `warn` | poziom logów diagnostycznych |
| `session.id` | tekst | brak | identyfikator sesji zapisywany w dzienniku |
| `session.ttl` | czas trwania | `24h` | czas bez wywołań, po którym wygasają ustawienia sesji (na przykład limity ustawione przez `journal.limits` dla sesji) |

### 7.15 `[privilege]` {#715-privilege}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `privilege.default` | `account`, `elevated`, `auto` | `account` | wartość parametru wywołania `privilege`, gdy wywołanie go nie podaje; może ją ustawić silnik osadzający |
| `privilege.allowed` | wartość logiczna | `true` | gdy `false`, `elevated` i `auto` kończą się błędem `E_PRIVILEGE` i przyczyną `disabled_by_configuration`; przełącznik, którym silnik osadzający stosuje swoją politykę |
| `privilege.backends` | lista wartości `self`, `broker`, `sudo`, `doas`, `run0` | brak: lista systemowa | dozwolone backendy w kolejności prób; lista systemowa to `["self", "broker", "sudo", "doas", "run0"]` w systemie Linux, `["self", "broker", "sudo"]` w systemie macOS i `["self", "broker"]` w systemie Windows; `broker` jest raportowany jako `not_installed`, dopóki usługa `nurt-elevation` nie istnieje |
| `privilege.group` | tekst | brak: `sudo` lub `wheel` (Linux), `admin` (macOS) | grupa uprawniona do łączenia się z usługą `nurt-elevation` |

Plik projektu nie może ustawiać `privilege.*`.

## 8. Interfejs dla implementacji {#8-interface-for-implementations}

Moduł `nurt_contracts::config` udostępnia:

| Element | Przeznaczenie |
|---|---|
| `Config` i jego sekcje | typowana konfiguracja z wbudowanymi wartościami domyślnymi (`Default`) oraz deserializacją TOML lub JSON |
| `ConfigLevel` | siedem poziomów, w rosnącym pierwszeństwie |
| `allowed_at(level, key)` | reguła z sekcji 3 jako czysta funkcja |
| `ConfigLayer`, `resolve(layers, project_trusted)` | scalanie poziomów z dopuszczaniem, uwagami i poziomem każdego klucza |
| `trusted_projects(layers)` | projekty zaufane z poziomów, które mogą deklarować zaufanie |
| `env_layer(vars, separator)` | poziom środowiska zbudowany ze zmiennych procesu |
| `known_keys()`, `ENV_VARS` | każdy klucz i każda zmienna formatu |
| `PROJECT_FLOOR_MAX_OPS`, `PROJECT_FLOOR_MAX_AGE` | progi wartości z plików projektu |

Testy zgodności w `crates/nurt-contracts/tests/config_*.rs` sprawdzają wartości
domyślne względem `spec/golden/config/defaults.toml`, wygenerowany schemat
względem zatwierdzonego pliku, każdy plik wzorcowy względem schematu,
pierwszeństwo siedmiu poziomów dla trzech limitów retencji, reguły zaufania
oraz poziom środowiska.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
