<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/services.md -->
<!-- Responsibility:   Documents Shared services. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Usługi współdzielone {#shared-services}

Skrzynki poleceń zależą wyłącznie od `nurt-contracts`. Gdy polecenie jednej grupy
potrzebuje możliwości zaimplementowanej przez inną grupę, wywołuje cechę usługi
z `nurt_contracts::services`. Silnik rejestruje implementacje jednokrotnie, w
rejestrze `Services` niesionym przez kontekst wykonania. Silnik skonfigurowany
dla działającego procesu (`EngineConfig::system`) rejestruje usługi każdej grupy
kompilacji, których aplikacja osadzająca nie dostarczyła sama: dokumenty, kod
źródłowy, host procesów, listowanie plików (gdy silnik ma system plików),
zapytania o repozytorium, kodek tekstu i posiadaczy plików. Dostawca reguł
przeglądu nie ma własnej implementacji.

Brakująca usługa nigdy nie jest sama w sobie błędem. Oznacza, że grupa, która
ją dostarcza, nie jest załadowana; wywołujące polecenie stosuje rozwiązanie
zastępcze wymienione poniżej i, gdy wpływa to na wynik, zgłasza to w raporcie
efektu.

## Usługi {#services}

| Cecha | Dostarczana przez | Używana przez | Bez usługi |
|---|---|---|---|
| `DocService` | grupa dokumentów | `read key:`, `write format:`, `validate` na dokumentach, polecenia testów i analizy odczytujące pliki konfiguracji | `read key:` i `write format:` kończą się błędem `E_EXTERNAL_MISSING`; `validate` jest pomijane z uwagą |
| `CodeService` | grupa kodu | `read symbol:`, `read view: outline \| symbols`, `search mode: syntax`, `search scope:`, `search enclosing:`, `validate` na kodzie źródłowym, kontrole odwołań w `remove`, `rename` i `replace`, `info details: symbols`, `git.diff symbol:`, `git.log symbol:`, `analyze.check checks: complexity, duplication` | `view:` wraca do pełnej zawartości z uwagą; `scope:` inne niż `any`, `symbol:` i `mode: syntax` kończą się błędem `E_EXTERNAL_MISSING`; kontrole odwołań są tekstowe; polecenia git stosują reguły kontekstu funkcji git |
| `ProcessHost` | grupa uruchamiania | klient serwera języka, haki git, menedżery usług i pakietów | klient serwera języka zgłasza `source: syntax`; haki i menedżery kończą się błędem `E_EXTERNAL_MISSING` |
| `FileListService` | polecenia plikowe | polecenia kodu obejmujące całe repozytorium (`code.symbols`, `code.references`, `code.profile`) | grupa kodu samodzielnie przechodzi drzewo z tymi samymi wykluczeniami |
| `GitQueryService` | grupa git | gorące miejsca `code.profile`, `code.diagnostics changed:`, `info details: git`, `analyze.* changed_only` | pola zależne są puste; `changed: true` kończy się błędem `E_EXTERNAL_MISSING` |
| `TextCodecService` | grupa tekstu | polecenia dokumentów i kodu, które dekodują lub kodują pliki | tylko UTF-8 i UTF-16 ze znacznikiem kolejności bajtów |
| `FileHolderService` | grupa systemu | warstwa plików (`E_IN_USE` wymienia posiadaczy), `open_files` | `E_IN_USE` bez posiadaczy |
| `ReviewRuleProvider` | silnik lub rozszerzenie | `git.review checks: [secrets]` | wyłącznie reguły wbudowane |

## Reguły wspólne {#common-rules}

1. Każda cecha jest bezpieczna dla obiektów (object safe), `Send` i `Sync`, i jest
   używana jako `&dyn Trait`. Uchwyty działających procesów (`ServiceChild`) są
   wyłącznie `Send`: mają jednego użytkownika naraz.
2. Metody są synchroniczne. Długą pracę ogranicza wywołujący: host procesów
   przyjmuje token anulowania etapu; pozostałe usługi pracują na bajtach, które
   są już w pamięci.
3. Usługi dokumentów i kodu nigdy nie czytają plików. Wywołujący odczytuje plik
   przez warstwę plików i przekazuje bajty, dzięki czemu te same metody sprawdzają
   zawartość jeszcze niezapisaną (`validate`) i działają na wirtualnych systemach
   plików w testach.
4. Błędy używają typu błędu kontraktu. Usługa nigdy nie zwraca błędu dla zwykłej
   odpowiedzi negatywnej: ścieżka kluczy, która niczego nie wybiera, to pusta
   lista, a program kończący się kodem niezerowym to wynik.
5. Pozycje są podawane podwójnie: jako zakresy bajtów (liczone od zera, koniec
   wyłączony) na potrzeby edycji oraz jako wiersze i kolumny liczone od jedności
   w skalarnych wartościach Unicode dla rekordów.

## Usługa dokumentów {#document-service}

| Metoda | Przeznaczenie |
|---|---|
| `formats()` | formaty wraz z rozszerzeniami nazw plików i informacją, czy klucze można zmieniać z zachowaniem układu |
| `detect_format(path, bytes)` | format na podstawie nazwy pliku i zawartości; `None` dla zawartości nieustrukturyzowanej |
| `read_keys(bytes, format, key, depth)` | wartości wybrane ścieżką kluczy, jako `DocItem` (kanoniczny konkretny klucz, wartość, obszar, komentarz, indeks dokumentu, kotwica YAML, alias i znacznik) |
| `check_syntax(bytes, format)` | problemy składni; pusta lista oznacza, że zawartość da się sparsować |
| `serialize_records(records, format, options)` | rekordy zapisane jako dokument (`write format: json \| yaml \| toml \| csv \| tsv`) |

Identyfikatory formatów są zapisywane małymi literami (`json`, `jsonc`, `yaml`,
`toml`, `xml`, `ini`, `csv`, `tsv`, `properties`, `env`, `plist`); rozszerzenia
mogą dodać kolejne. Zmiana dokumentów nie należy do usługi: `set` oraz `remove`
lub `replace` na rekordach `item` należą do grupy dokumentów i są kierowane
według typu rekordu.

## Usługa kodu {#code-service}

| Metoda | Przeznaczenie |
|---|---|
| `languages()` | języki z gramatyką |
| `language_of(path, bytes)` | język na podstawie nazwy pliku i zawartości |
| `outline(bytes, language, depth)` | symbole w kolejności źródła z głębokością zagnieżdżenia |
| `find_symbol(bytes, language, name)` | definicje nazwy; `Container.name` zawęża kontener; kilka wyników oznacza, że nazwa jest niejednoznaczna |
| `enclosing(bytes, language, offset)` | najgłębszy symbol zawierający dany bajt |
| `classify_spans(bytes, language)` | obszary komentarzy i ciągów znaków; reszta to kod |
| `check_syntax(bytes, language)` | błędy składni |
| `compile_pattern(pattern, language)` | wzorzec strukturalny ze zmiennymi meta `$X` i `$$$X`, jako nieprzezroczysty `SyntaxPattern` |
| `syntax_search(bytes, pattern)` | dopasowania z przechwyceniami |
| `render_template(match, template)` | tekst zamiany z podstawionymi przechwyceniami |
| `references(bytes, language, name)` | wystąpienia identyfikatora z rolą i klasą leksykalną |
| `function_metrics(bytes, language)` | wiersze, parametry, złożoność cyklomatyczna i zagnieżdżenie dla każdej funkcji |
| `tokens(bytes, language)` | tokeny do wykrywania duplikatów |

Język bez gramatyki kończy się błędem `E_UNSUPPORTED_LANGUAGE`; wzorzec, który
nie parsuje się w danym języku, kończy się błędem `E_PATTERN_SYNTAX`. Wyszukiwania
w całym repozytorium łączą `FileListService` z wywołaniami dla poszczególnych
plików.

## Host procesów {#process-host}

| Metoda | Przeznaczenie |
|---|---|
| `resolve_program(program, cwd)` | plik programu tak, jak znalazłoby go `run` |
| `run_program(spec, cancel)` | uruchomienie do końca z przechwyconym wyjściem, limitem czasu, anulowaniem i opcjonalnym wykrywaniem zmienionych plików |
| `spawn_service(spec)` | długo działający pomocnik z potokami standardowych strumieni we własnej grupie procesów lub obiekcie zadania |

Programy podlegają regułom `run`: argumenty jako lista bez powłoki, wyszukiwanie
w `PATH` (i `PATHEXT` w systemie Windows), pliki wsadowe uruchamiane przez
interpreter poleceń z każdym argumentem w cudzysłowie, bez terminala, a całe
drzewo procesów jest zatrzymywane przy przekroczeniu czasu, anulowaniu lub
`kill_tree`.

## Listowanie plików {#file-listing}

`walk(spec)` wymienia wpisy pod katalogiem głównym z parametrami i wartościami
domyślnymi `find`: wzorce glob (domyślnie `**`), rodzaj wpisu (domyślnie pliki),
głębokość, wpisy ukryte (domyślnie uwzględnione), dowiązania symboliczne
(domyślnie nieodwiedzane), pliki ignorowania (domyślnie wyłączone), wykluczenia
w składni gitignore oraz nazwane grupy wykluczeń `vcs`, `deps`, `build`,
`cache`. `WalkSpec::source_tree` to żądanie używane przez analizę kodu całego
repozytorium: pliki ignorowania włączone, a `vcs`, `deps`, `cache` wykluczone.

## Rejestr {#registry}

`Services` jest budowany za pomocą `with_doc`, `with_code`, `with_process_host`,
`with_file_list`, `with_git_query`, `with_text_codec`, `with_file_holders` i
`with_review_rules`. Akcesory o tych samych nazwach (`doc()`, `code()`, …)
zwracają `Option<&dyn Trait>`; `available()` wymienia zarejestrowane usługi
według ich stałych nazw (`doc`, `code`, `process_host`, `file_list`, `git_query`,
`text_codec`, `file_holders`, `review_rules`).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
