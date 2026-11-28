<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/report.md -->
<!-- Responsibility:   Documents Effect report and request envelope. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Raport skutków i koperta żądania {#effect-report-and-request-envelope}

| Format | Schemat | Typy Rust |
|---|---|---|
| `nurt-report/1`, raport skutków jednego programu | `schemas/nurt-report-1.schema.json` | `nurt_contracts::report` |
| `nurt-request/1`, koperta żądania | `schemas/nurt-request-1.schema.json` | `nurt_contracts::envelope::Request` |
| `nurt-response/1`, odpowiedź na żądanie | `$defs/response` schematu żądania | `nurt_contracts::envelope::Response` |

Wszystkie trzy są dokumentami JSON w UTF-8; CBOR jest kodowaniem równoważnym.
Każdy niesie wersję `"nurt": "MAJOR.MINOR"`, obecnie `"1.0"`. W obrębie wersji
głównej 1 formaty zyskują wyłącznie pola opcjonalne i nowe wartości
wyliczeń kodów; czytnik ignoruje pola raportu, których nie zna. Nazwa produktu
nigdy nie pojawia się w wyjściu programu.

## 1. Zasady raportu {#1-principles-of-the-report}

1. Raport pokazuje skutek, a nie sam fakt, że coś się wykonało: cele, zmienione
   i usunięte pliki wraz z ich liczbą i łącznym rozmiarem, rozmiary i skróty
   przed i po, próbkę różnic, podsumowanie operacji zbiorczych oraz
   nietypowe skutki.
2. Raport niesie numer operacji `op` i nic więcej o dzienniku. Żadne pole,
   flaga ani zdanie raportu nie podaje, czy, jak ani do kiedy operację można
   cofnąć; nie ma stanu dziennika ani wygaśnięcia. Stan wpisów dziennika
   pokazuje wyłącznie jawne zapytanie `history`. Raport `undo` i `redo` jest
   zwykłym raportem skutków.
3. `irreversible` jest obecne tylko wtedy, gdy skutek jest nieodwracalny. Jego
   brak nie niesie żadnej informacji o cofnięciu.
4. `notable` opisuje nietypowe skutki. Nigdy niczego nie blokuje; polityka
   należy do wywołującego.
5. Listy są pomijane, gdy są puste, a pola opcjonalne, gdy ich brak, tak aby
   raport zwykłego skutku pozostawał krótki.

## 2. Pola raportu {#2-report-fields}

| Pole | Typ | Znaczenie |
|---|---|---|
| `nurt` | tekst, wymagane | wersja formatu `1.<minor>` |
| `status` | kod, wymagane | wynik, sekcja 3 |
| `kind` | tekst, wymagane | polecenie zmieniające albo polecenie źródłowe programu odczytującego (`replace`, `remove`, `run`, `find`); `request` dla błędów koperty wykrytych, zanim znany jest jakikolwiek program |
| `op` | liczba całkowita od 1 | wpis dziennika operacji; nieobecne dla programów odczytujących, podglądów i programów zmieniających bez celów |
| `ran` | tekst | kanoniczny tekst programu, który się wykonał; obecne tylko wtedy, gdy zastosowano poprawki |
| `elevated` | `true` | obecne tylko wtedy, gdy operacja wykonała się z podwyższonymi uprawnieniami |
| `backend` | `self`, `broker`, `sudo`, `doas`, `run0` | backend podwyższania uprawnień, który wykonał operację; obecne tylko z `elevated` |
| `as_user` | tekst | konto, jako które wykonała się operacja, gdy wywołanie podało `as_user` |
| `elevation_reason` | `permission_denied`, `elevated_entry` | dlaczego operacja została wykonana z podwyższonymi uprawnieniami bez takiego żądania: `privilege: auto` powtórzyło etap po odmowie albo `undo` lub `redo` wpisu z podwyższonymi uprawnieniami zostało wysłane do backendu |
| `ast_hash` | `sha256:<64 hex>` | skrót drzewa składni, patrz `syntax-tree.md` |
| `result_file` | ścieżka | plik zawierający pełny wynik (UTF-8), gdy wynik został zapisany do pliku (`save_to` lub `output.results_dir`) |
| `corrections` | lista | poprawki zastosowane do tekstu programu: `id`, `rule` (`L`, `T`, `V`, `B` lub `P` i dwie cyfry), `class` (`C`, `S`, `F`, `P`), `at` (`line`, `column`, `length`), `from`, `to`, `kind`, `alternatives` |
| `targets` | obiekt | `count` (wymagane), `files`, `directories`, `bytes`, `listed` (liczba wpisów `changes` na tej stronie) |
| `summary` | obiekt | `files_created`, `files_modified`, `files_deleted`, `files_renamed`, `directories_created`, `directories_deleted`, `replacements`, `lines_added`, `lines_removed`, `bytes_before`, `bytes_after`, `bytes_freed` |
| `changes` | lista | zmienione cele na tej stronie, sekcja 5 |
| `diff` | lista | fragmenty różnic w formacie unified: `path`, `raw_path`, `old_start`, `old_len`, `new_start`, `new_len`, `lines` (prefiks `+`, `-` lub spacja albo wiersz znacznika `\ No newline at end of file`), `header` |
| `bulk` | obiekt | podsumowanie operacji zbiorczej: `by_directory` i `by_extension` (od największych, z `count` i `bytes`; katalog, który nie jest poprawnym Unicode, ma także `raw_directory`), `sample_head`, `sample_tail` (oraz `raw_sample_head`, `raw_sample_tail`, gdy próbkowany cel nie jest poprawnym Unicode) |
| `exec` | rekord `exec` | program, który się wykonał |
| `job` | rekord `job` | program, który kontynuuje w tle; obecne, gdy status to `running` |
| `test` | rekord `test_run` | przebieg testów |
| `analysis` | rekord `analysis` | werdykt analizy |
| `git` | rekord `git_effect` | skutek dla repozytorium |
| `records` | obiekt | wynik programu odczytującego: `type` (typ rekordu), `matched`, `header` (rekord nagłówka strumienia), `rows` |
| `notable` | lista | nietypowe skutki w malejącej kolejności ważności, sekcja 6 |
| `irreversible` | niepusta lista | skutki nieodwracalne, sekcja 7 |
| `error` | obiekt | błąd języka, sekcja 8 |
| `next_cursor` | tekst | kursor następnej strony; nieobecny na ostatniej stronie |
| `continuation` | obiekt | co pomija skrócona odpowiedź i jak odczytuje to `next`, sekcja 2.1 |
| `timing` | obiekt | `parse_ms`, `read_ms`, `plan_ms`, `apply_ms` (szczegółowość `detailed`) |
| `parts` | lista | części konstrukcji sterującej w kolejności, sekcja 2.1 |
| `parts_total` | liczba całkowita | liczba części, gdy `parts` jest skrócone do pierwszych 200 |

### 2.1 Części konstrukcji sterującej {#21-parts-of-a-control-construct}

Program zawierający konstrukcję sterującą (`when`, `each`, `repeat`, `while`,
`steps`, wywołanie funkcji) jest jedną operacją i jednym wpisem dziennika; `op`
jest numerem wpisu, a `kind` słowem konstrukcji. Raport wymienia części po
kolei w `parts`, każdą z:

| Człon | Znaczenie |
|---|---|
| `at` | miejsce części: `when`, `when.branch[2]`, `each.iteration[3]`, `steps.step[1]`, `call rotate.step[2]` |
| `kind` | `branch` (wybrana gałąź), `iteration`, `step`, `loop` (sumy pętli) |
| `condition` | wartość warunku wybranej gałęzi; `false`, gdy żadna gałąź się nie wykonała |
| `branch` | numer wybranej gałęzi od 1; gałąź `otherwise` jest o jeden za ostatnim warunkiem |
| `command` | polecenie, którego skutek raportuje krok |
| `targets` | liczba celów, które część zmieniła |
| `ok` | `false` dla części, która zawiodła |
| `rolled_back` | skutki części zostały wycofane (części, które zawiodły lub zostały cofnięte przez niepowodzenie późniejszej części) |
| `irreversible` | skutków części nie można wycofać; są wymienione w `irreversible` |
| `error` | kod błędu, który zakończył część |
| `iterations` | liczba iteracji części `loop` |
| `op` | wpis dziennika iteracji pętli z `atomic: false` |
| `note` | uwaga: limit zakończył pętlę, podgląd planuje jedną iterację |

Gdy część zawodzi, cała konstrukcja jest wycofywana: status to `rolled_back`,
`error.details.restored` wymienia to, co przywrócono, a
`error.details.not_rolled_back` wymienia części, które katalog oznacza jako
nieodwracalne. `parallel: N` w `each` odczytuje każdą iterację dwa razy, raz
aby znaleźć jej cele i raz aby ją wykonać, oraz porównuje ścieżki po
składnikach, z obydwoma separatorami i bez rozróżniania wielkości liter; w tym
samym czasie działa tylko odczyt i planowanie iteracji, a ich zmiany są
stosowane pojedynczo wewnątrz jednej transakcji. Iteracje uruchamia
jednocześnie tylko wtedy, gdy ich cele są rozłączne, co ustala przez
zaplanowanie każdej iteracji przed startem; w przeciwnym razie iteracje
wykonują się po kolei, a część `loop` mówi o tym w `note`. Z `preview: true`
`repeat` lub `while` planuje jedną iterację (liczba iteracji jest znana dopiero
po wykonaniu, `note` to podaje), a `each` planuje każdą iterację; status to
`planned`, `changes` wymienia skutki każdej części i nic nie jest zmieniane.
Pętla z `atomic: false` zachowuje iteracje ukończone przed niepowodzeniem,
każdą w osobnym wpisie (`op` części); raport wskazuje iterację, która
zawiodła.

Rekordy (`exec`, `job`, `test`, `analysis`, `git`, `records.header` i
`records.rows`) pojawiają się w swojej postaci JSON zdefiniowanej przez formaty
wartości i rekordów; schemat raportu nie ogranicza ich pól. Wyniki domenowe,
które nie mieszczą się we wspólnych polach, są niesione przez rekord w nazwanym
polu opcjonalnym; nowa domena dodaje nowe pole opcjonalne tego samego rodzaju.

Pola oznaczone w schemacie `x-nurt-volatile` (`op`, `result_file`,
`next_cursor`, `timing`) różnią się między przebiegami tego samego programu i
są ignorowane przy porównywaniu raportów.

### 2.1 Kontynuacja skróconej odpowiedzi {#21-continuation-of-a-shortened-response}

Każda odpowiedź, która pomija wiersze, rekordy, bajty lub końce długich
wierszy, niesie pole `continuation`:

| Pole | Typ | Znaczenie |
|---|---|---|
| `id` | tekst | krótki identyfikator, `c1`, `c2`, … w obrębie sesji konsoli |
| `unit` | `lines`, `records`, `bytes` | jednostka części |
| `shown` | liczba całkowita | jednostki w dotąd udzielonych odpowiedziach |
| `remaining` | liczba całkowita | jednostki jeszcze niepodane; `-1`, gdy nie jest to znane z góry (zapytanie kontynuowane po kursorze, takie jak `logs`) |
| `cut_lines` | liczba całkowita | wiersze pokazanej części, które zostały skrócone przez `max_line` lub `max_chars` |

Jednowierszowy tekst odpowiedzi kończy się `more: next`, gdy kontynuacja jest
najnowsza, a w przeciwnym razie `more: next c3`. Polecenie `next` zwraca
następną część i nigdy nie oblicza wyniku ponownie. Powiadomienia
`truncated_lines` i `limit_reached` pozostają i zyskują opcjonalne pole
`continuation`: identyfikator (`c3`) kontynuacji odpowiedzi, nieobecny, gdy nic
nie jest kontynuowane. Zdanie `read_hint` rekordu `exec` już nie istnieje:
plik buforowy przebiegu pozostaje, jego ścieżka pozostaje w rekordzie
(`output_file`, `stdout_file`, `stderr_file`), a kontynuacja odczytuje z tego
pliku pominięte wiersze. Wynik będący wyborem wierszy (`show: errors`) jest
kontynuowany od pierwszego wiersza pliku. Zachowane wyniki podlegają kluczom
`continuation.max_age`, `continuation.max_count` i `continuation.max_size`;
kontynuacja, która wygasła, kończy się błędem `E_CONTINUATION_EXPIRED`.

## 3. Status {#3-status}

| Status | Znaczenie | Reguły |
|---|---|---|
| `ok` | program zakończył się; program zmieniający zastosował swój skutek | może nieść `error` dla sprawdzenia, które zawiodło po zastosowaniu skutku; skutek pozostaje |
| `planned` | podgląd (`preview: true`): cele i skutek obliczone, nic nie zmieniono | brak `op` |
| `running` | program uruchomiony w tle nadal działa | wymagane `job` |
| `partial` | skutek zastosowano do części celów, na przykład cofnięcie wpisu zbiorczego, którego zachowana zawartość została częściowo zwolniona dla miejsca na dysku | wymagane `op`; `irreversible` lub `error` wymienia resztę |
| `rolled_back` | skutek zastosowano i cofnięto, ponieważ późniejsze sprawdzenie zawiodło | wymagane `error` (`E_ROLLED_BACK` z `cause`) |
| `failed` | program zatrzymał się z błędem | wymagane `error` |

Program zatrzymany przez swój limit czasu, limit zasobów lub anulowanie kończy
się `failed` i błędem `E_TIMEOUT`, `E_LIMIT` lub `E_CANCELLED`.

## 4. Szczegółowość {#4-verbosity}

| Poziom | Zawartość |
|---|---|
| `quiet` | `status`, `op`, `targets`, `summary`, `notable`, `irreversible`; bez zmian i bez różnic |
| `normal` (domyślny) | `summary`, zmiany do `limit` (domyślnie 50), próbka różnic do `report.diff_max_hunks` (20) fragmentów i `report.diff_max_lines` (200) wierszy, `bulk` dla operacji zbiorczych |
| `detailed` | wszystkie zmiany i różnice, stronicowane przez kursor, oraz `timing` |

Operacja jest operacją zbiorczą od `report.mass_threshold` (200) celów. Na
poziomie `normal` pokazuje sumy, dziesięć największych katalogów i
rozszerzeń, pięć celów od początku i dwa od końca oraz `notable`. Pełna lista
celów jest zawsze dostępna przez kursor; nic nie jest ukrywane.

## 5. Zmiany {#5-changes}

| `action` | Znaczenie |
|---|---|
| `create` | cel utworzony |
| `modify` | zawartość zmieniona |
| `delete` | cel usunięty |
| `rename` | cel przemianowany lub przeniesiony; `path` to ścieżka po zmianie, `from` to poprzednia ścieżka (wymagane), a `to` powtarza `path` |
| `attributes` | zmieniono tryb, własność lub kontrolę dostępu |
| `link` | dowiązanie utworzone lub zmienione |
| `truncate` | cel obcięty |
| `state` | zmieniono stan celu poza systemem plików; `path` to `<provider>:<name>` (na przykład `service:nginx`), `before` i `after` zawierają stany |

`flags` niesie dodatkowe znaczniki zmiany, na przykład `crlf` lub
`keys_created`.

Ścieżka, która nie jest poprawnym Unicode, jest zapisywana z U+FFFD w `path`,
`from` i `to`; zmiana ma wtedy także natywne bajty w `raw_path`, `raw_from` i
`raw_to` (standardowy base64), jak opisano dla ścieżek w `records.md`.

## 6. Nietypowe skutki (`notable`) {#6-unusual-effects-notable}

Każdy wpis ma postać `{code, severity, path?, raw_path?, count?, detail?}`;
`raw_path` zawiera natywne bajty `path`, gdy nie jest ono poprawnym Unicode;
wpisy są sortowane według ważności, `alert` jako pierwszy. Ważności w
kolejności rosnącej: `info`, `notice`, `warning`, `alert`. Tabela podaje
domyślną ważność; progi konfiguruje się w `report.notable.*`. Kody ze źródłem
„report” są wyprowadzane przez konstruktor raportu ze zmian; wszystkie
pozostałe kody są emitowane przez polecenie, które zaobserwowało warunek.

| Kod | Domyślnie | Źródło | Warunek |
|---|---|---|---|
| `outside_project` | alert | report | cel poza każdym katalogiem `scope.project_roots` (gdy ustawione) |
| `system_path` | alert | report | cel w katalogu systemowym |
| `home_config` | warning | report | konfiguracja w katalogu domowym (`~/.ssh`, `~/.config`, profile powłoki) |
| `mass_delete` | warning, alert przy dziesięciokrotności progu | report | usunięte pliki od `mass_files` (100), bajty od `mass_bytes` (1 GiB) lub udział od `mass_ratio` (80%) |
| `delete_tree` | notice | report | rekurencyjne usunięcie niepustego katalogu |
| `overwrote_existing` | warning | report | istniejące pliki zastąpione przez zapis, przeniesienie lub kopię |
| `file_emptied` | warning | report | plik z zawartością stał się pusty |
| `size_shrunk` | warning | report | rozmiar zmalał o `shrink_ratio` (50%, pliki od 1 KiB) lub więcej |
| `many_per_file` | notice | report | `per_file_repl` (100) lub więcej zamian w jednym pliku |
| `target_count_high` | notice | report | `targets_high` (500) lub więcej celów |
| `nothing_matched` | notice | report | etap zmieniający nie otrzymał celów; nigdy dla `run`, które nie ma celów do wyboru |
| `unchanged` | info | command | zapis dał identyczną zawartość |
| `binary_target` | warning | command | zmieniono plik binarny |
| `binary_skipped` | notice | command | pominięto pliki binarne (`count`) |
| `special_file` | warning | command | zapisano urządzenie, potok, gniazdo lub plik pseudo |
| `symlink_followed` | warning | command | zmiana przeszła przez dowiązanie symboliczne do celu poza jego katalogiem |
| `hardlink_broken` | notice | command | plik z wieloma twardymi dowiązaniami został zastąpiony, co rozdzieliło dowiązania |
| `hardlink_set_preserved` | info | command | plik z wieloma twardymi dowiązaniami został zapisany w miejscu, z zachowaniem dowiązań |
| `attributes_changed` | notice | command | zmieniono tryb, własność lub kontrolę dostępu |
| `encoding_preserved`, `line_ending_preserved` | info | command | plik zachował kodowanie inne niż UTF-8 lub końce wierszy inne niż `lf` |
| `line_ending_mixed` | notice | command | mieszane końce wierszy |
| `encoding_changed`, `line_ending_changed`, `bom_changed` | notice | command | kodowanie, końce wierszy lub znacznik kolejności bajtów zostały przekonwertowane |
| `truncated_lines` | notice | command | długie wiersze zostały skrócone w wyjściu |
| `regex_backtracking` | warning | command | wyrażenie regularne osiągnęło limit nawrotów |
| `decompressed` | info | command | skompresowana zawartość została rozpakowana w sposób przezroczysty |
| `cross_device` | notice | command | przeniesienie między woluminami, wykonane jako kopiowanie i usunięcie |
| `to_is_dir` | notice | command | miejsce docelowe przeniesienia lub kopii jest istniejącym katalogiem |
| `case_collision`, `case_alias`, `nfd_name` | notice | command | nazwy różniące się wielkością liter kolidują; nazwa dopasowana w innej wielkości liter lub formie normalizacji |
| `portability` | notice | command | nazwa nieprzenośna między systemami |
| `external_volume` | notice | command | cel na woluminie sieciowym lub wymiennym |
| `archive_path_escape` | warning | command | wpisy archiwum poza miejscem docelowym zostały pominięte |
| `permission_denied_skipped` | warning | command | pliki bez uprawnienia do odczytu zostały pominięte |
| `link_loop_skipped` | notice | command | dowiązanie symboliczne prowadzące z powrotem do jednego ze swoich katalogów nadrzędnych nie zostało prześledzone |
| `ignored` | info | command | zmiany w ścieżkach wyłączonych ze śledzenia (`run.track_ignore`) |
| `alias_shared` | warning | command | zmieniono węzeł, do którego odwołują się aliasy |
| `keys_created` | info | command | utworzono klucze pośrednie |
| `duplicate_key` | warning | command | powtórzony klucz; zmieniono ostatnie wystąpienie |
| `semantic_fallback` | notice | command | operacja na kodzie przeszła na analizę składniową |
| `references_left` | notice | command | tekstowe odwołania do starej nazwy pozostają |
| `rename_incomplete` | warning | command | zmiana nazwy nie dotarła do każdego odwołania |
| `detection_scope` | info | command | po `run`: zmiany poza `watch` nie są wykrywane; przy `track_changes: auto` uwaga wymienia też katalogi innych systemów plików, do których nie wchodzono, albo, gdy wykrywanie pominięto (korzeń systemu plików, ponad 200 000 wpisów, skan dłuższy niż 3 s), przyczynę i co zrobić |
| `exit_nonzero` | notice | command | program zakończył się kodem innym niż zero; `count` zawiera kod |
| `post_stages_deferred` | info | command | etapy po skutku zostały pominięte, ponieważ program przeszedł do tła |
| `rollback_partial` | warning | command | wycofanie przywróciło tylko część zmian |
| `output_binary` | notice | command | program zapisał wyjście binarne |
| `spool_limit` | warning | command | wyjście programu przekroczyło limit bufora |
| `stdin_closed_early` | info | command | program zamknął swoje wejście wcześniej |
| `job_lost` | warning | command | zadanie w tle utraciło swój nadzorca |
| `tool_missing` | notice | command | żądane narzędzie analizy nie jest zainstalowane |
| `tests_none` | notice | command | nie znaleziono ani nie uruchomiono żadnych testów |
| `trust_config_ignored` | warning | command | ustawienia zaufania w pliku konfiguracji projektu zostały zignorowane; projekt nie może sam uczynić się zaufanym |
| `project_config_ignored` | notice | command | klucze pliku konfiguracji projektu spoza kluczy dozwolonych dla projektów zostały zignorowane |
| `journal_config_ignored` | notice | command | ustawienia dziennika niezaufanego projektu zostały zignorowane |
| `journal_oversize` | warning | command | operacja przekracza limit rozmiaru dziennika; pokazywane przez podgląd przed zmianą |
| `source_unavailable` | warning | command | źródło danych systemu operacyjnego było niedostępne; wyniki są niekompletne |
| `settings_broadcast` | notice | command | zmiana ustawień została rozesłana do działających programów |
| `hooks_skipped` | notice | command | haki repozytorium nie zostały uruchomione |
| `history_rewritten` | warning | command | historia repozytorium została przepisana |
| `force_push` | alert | command | zdalna gałąź została nadpisana |
| `detached_head` | notice | command | stan odłączonej głowy |
| `branch_deleted_unmerged` | warning | command | usunięto niescaloną gałąź |
| `fast_forward` | info | command | gałąź została zaktualizowana przez fast-forward |
| `conflicts` | warning | command | operacja zatrzymała się z konfliktami (`count`) |
| `untracked_removed` | warning | command | usunięto pliki nieśledzone |
| `local_changes_carried` | info | command | niezatwierdzone zmiany zostały przeniesione na inną gałąź |
| `filter_not_applied` | warning | command | filtr zawartości repozytorium nie został zastosowany |
| `credential_helper_ignored`, `ssh_command_ignored`, `fsmonitor_ignored`, `signing_program_ignored` | notice | command | program wskazany przez konfigurację repozytorium nie został uruchomiony |
| `submodules_not_updated` | notice | command | podmoduły nie zostały zaktualizowane |
| `sparse_checkout`, `shallow_repository` | info | command | repozytorium używa częściowego wyewidencjonowania lub płytkiej historii |
| `line_endings_converted` | info | command | końce wierszy przekonwertowane według ustawień repozytorium |
| `upstream_set` | info | command | ustawiono gałąź nadrzędną |
| `stash_kept` | notice | command | wpis schowka został zachowany po konflikcie |
| `signed` | info | command | zatwierdzenie lub znacznik został podpisany |
| `amend_published` | alert | command | poprawiane zatwierdzenie jest osiągalne z gałęzi śledzącej zdalnej |
| `commit_detached_head` | warning | command | zatwierdzenie utworzono na odłączonej `HEAD` |
| `operation_in_progress` | warning | command | zatwierdzenie utworzono w trakcie rebase, cherry-pick, revert lub bisekcji |
| `excluded` | info | command | pliki dopasowane przez `exclude` zostały pominięte (`count`) |
| `review_skipped` | notice | command | pliki nie zostały przejrzane: pliki nieśledzone ponad limit rozmiaru, pliki binarne lub pliki ponad limit liczby |
| `checkpoint_without_index` | notice | command | punkt kontrolny utworzono bez indeksu, ponieważ indeks ma konflikty |
| `tls_verification_disabled` | warning | command | weryfikacja certyfikatów TLS jest wyłączona przez konfigurację repozytorium |
| `http_option_ignored` | notice | command | nieobsługiwane ustawienie HTTP konfiguracji repozytorium zostało zignorowane |
| `pack_without_deltas` | info | command | wypychanie wysłało obiekty bez nowej kompresji delta |
| `push_status_unknown` | warning | command | zdalne repozytorium nie zgłosiło wyniku aktualizacji referencji |
| `host_key_unverified` | warning | command | klucz hosta zdalnego repozytorium został zaakceptowany bez weryfikacji |
| `match_exec_ignored` | notice | command | warunek `Match exec` konfiguracji SSH nie został uruchomiony |
| `credentials_in_url` | warning | command | użyto poświadczeń zapisanych w adresie URL zdalnego repozytorium |
| `limit_reached` | info | command | wyjście zatrzymało się na liczbie wpisów podanej przez `limit` (`count` zawiera liczbę pominiętych wpisów) |
| `interactive_program` | notice | command | uruchomiono program sklasyfikowany jako interaktywny (edytor, pager, monitor terminala, interpreter bez skryptu); może czekać na wejście z terminala |
| `opaque_script` | notice | command | program uruchomił skrypt podany w swoich argumentach (`bash -c`, `python -c`); skutki skryptu nie są klasyfikowane |

## 7. Skutki nieodwracalne (`irreversible`) {#7-irreversible-effects-irreversible}

Każdy wpis ma postać `{reason, path?, raw_path?, count?, detail?}`; `raw_path`
zawiera natywne bajty `path`, gdy nie jest ono poprawnym Unicode.

| Przyczyna | Kiedy |
|---|---|
| `external_effect` | uruchomiono program ze skutkiem poza lokalnym systemem plików (publikowanie, wdrożenie, zdalne powłoki) |
| `remote_ref_updated` | referencje zdalnego repozytorium zostały zaktualizowane lub pobrane |
| `network` | wysłano żądanie sieciowe zmieniające stan zdalny |
| `database` | baza danych została zmieniona bez zachowanej kopii |
| `process_stopped` | proces lub zadanie zostało zatrzymane |
| `service_changed` | usługa systemowa została uruchomiona, zrestartowana, włączona, wyłączona lub zatrzymana |
| `package_changed` | pakiety systemowe zostały zainstalowane, usunięte lub zaktualizowane |
| `previous_content_not_kept` | program zmienił plik, którego poprzednia zawartość nie została zachowana; `detail` podaje przyczynę, na przykład budżet czasu lub system plików bez współdzielenia bloków |
| `no_space_for_copy` | brak miejsca na kopię poprzedniej zawartości; zmianę wykonano bezpośrednio |
| `journal_inactive` | dziennik jest wyłączony lub jego usługa miejsca nie działa |
| `network_volume`, `removable_volume` | cel znajduje się na takim woluminie, który domyślnie nie ma dziennika |
| `purged` | wpisy dziennika zostały wyczyszczone |
| `special_file` | zapisano urządzenie, potok, gniazdo lub plik pseudo |
| `too_large_for_journal` | operacja jest większa niż limit dla jednego wpisu dziennika |
| `released_for_space` | zachowana zawartość części operacji zbiorczej została zwolniona dla miejsca na dysku |
| `job_output_removed` | zachowane wyjście zadania w tle zostało usunięte wraz z zadaniem |
| `process_state_changed` | zmieniono stan działającego procesu: wstrzymany, wznowiony, zasygnalizowany lub z innym priorytetem |
| `journal_entries_expired` | wpisy dziennika wcześniejszych operacji wygasły z powodu tej operacji, na przykład gdy obniżono limity dziennika |
| `journal_error` | dziennik zawiódł podczas zapisu operacji; zmianę wykonano bez zachowanej poprzedniej zawartości |

## 8. Błędy {#8-errors}

`error` ma postać `{code, message, at?, location?, found?, problem?, fix?, help?,
expected?, actual?, sample?, candidates?, next_steps?, effect_applied?,
details?, cause?}`. `at` ma postać `{line?, column?, length?, stage?}`: `stage`
to etap wykonania jako tekst (`"0"` do `"7"`, `"5a"`), a `line`, `column` i
`length` podają miejsce w tekście programu, gdy błąd je ma. Lista kodów jest
częścią schematu (`$defs/error_code`); każdy kod zdefiniowany przez kontrakt
błędów jest tam wymieniony, a test utrzymuje oba w zgodności. Sama struktura
błędu jest określona razem z kodami błędów; schemat raportu dopuszcza
dodatkowe pola błędu.

Błędy przed jakimkolwiek skutkiem pozostawiają świat bez zmian i przychodzą z
gotową poprawką. Sprawdzenie, które zawodzi po zastosowaniu skutku, na
przykład `assert exit_code: 0` po `run`, jest raportowane ze statusem `ok` i
błędem; skutek pozostaje.

## 9. Koperta żądania (`nurt-request/1`) {#9-request-envelope-nurt-request1}

```json
{
  "nurt": "1.0",
  "program": "find \"/srv/app/src\" glob: \"**/*.go\"",
  "variant": "a",
  "options": { "limit": 20, "verbosity": "normal" }
}
```

Podawane jest dokładnie jedno wejście:

| Wejście | Znaczenie |
|---|---|
| `program`, z opcjonalnym `variant` | tekst programu w notacji `a`, `b` lub `c`; bez `variant` obowiązuje skonfigurowany wariant |
| `ast` | powierzchniowe drzewo składni `nurt-ast/1` |
| `cursor` | kursor dalszej strony wcześniejszego wyniku; odczyt strony nigdy nie uruchamia ponownie programu zmieniającego |
| `accept` | identyfikator kandydata z wcześniejszego błędu `E_AMBIGUOUS` |

Człon `params` towarzyszy `program` lub `ast`: obiekt z wartościami parametrów
zadeklarowanych przez program w `params { … }`. Wartość jest typowanym JSON
albo, tak jak przekazuje ją wiersz poleceń, tekstem, który odczytuje
zadeklarowany typ (`"7"` dla `int`, `"10mb"` dla `size`). Nazwa, która nie jest
zadeklarowana, to `E_UNKNOWN_PARAM` z najbliższymi nazwami, brakujący wymagany
parametr to `E_MISSING_PARAM`, a wartość nieprawidłowego typu to `E_TYPE`;
każdy błąd wymienia deklarację i nic się nie wykonało.

Warunek konstrukcji jest sprawdzany przy wiązaniu względem typów zadeklarowanych
przez rekord wyjściowy polecenia źródłowego: użycie pola, którego nie spełnia
żadna wartość jego typu, to `E_TYPE` z `effect_applied: false`. Pola typu `any`
lub `json` są jedynym wyjątkiem i są sprawdzane, gdy warunek jest obliczany.
Etap, który może zmienić świat (`may_affect`) i bierze swoją akcję z wartości,
która nie jest literałem, to `E_TYPE` przy wiązaniu w warunku.

Opcje (`options`) są wspólne dla każdego polecenia. Nieobecna opcja przyjmuje
wartość z konfiguracji, a bez skonfigurowanej wartości z poniższej wartości
domyślnej. Opcje wywołania mają pierwszeństwo przed konfiguracją.

| Opcja | Typ i wartość domyślna | Znaczenie |
|---|---|---|
| `preview` | bool, `false` | tylko obliczenie celów i skutku; nic nie zmienia |
| `base` | ścieżka bezwzględna | katalog, względem którego rozwiązywane są ścieżki względne; nie jest to katalog bieżący |
| `limit` | od 1 do 5000, `50` | rekordy lub zmiany na stronę |
| `fields` | lista tekstów | pola wyniku do zachowania |
| `format` | `text`, `json`, `markdown` lub `compact`, `text` | postać wyniku |
| `verbosity` | `quiet`, `normal` lub `detailed`, `normal` | poziom szczegółowości raportu |
| `diff` | `none`, `stat` lub `unified`, `unified` | postać różnic |
| `diff_context` | int, `3` | wiersze kontekstu różnic |
| `max_chars` | int, `0` | największa liczba znaków jednego pola tekstowego; `0` oznacza brak limitu; skrócone pola są oznaczane `truncated: true` |
| `save_to` | ścieżka bezwzględna | plik, który otrzymuje pełny wynik; raport podaje `result_file` |
| `timeout` | czas trwania, `120s` | limit czasu całego programu, na przykład `250ms`, `30s`, `5m`, `1h30m` lub `7d` |
| `session` | tekst | identyfikator sesji zapisywany w dzienniku |
| `language` | `en` lub `pl`, `en` | język komunikatów i pomocy; kody, pola i słowa kluczowe są zawsze angielskie |
| `script` | bool, `false` | kilka potoków, każdy jako osobna operacja |

Nieznane pola są odrzucane w kopercie i w `options`: błędnie napisana opcja
nigdy nie jest ignorowana. Ścieżki są bezwzględne w notacji kanonicznej:
`/srv/app`, `C:/proj`, `//server/share` lub ścieżka dostawcy, taka jak
`registry:/HKCU`.

Koperta, która nie jest poprawnym JSON lub nie zgadza się ze schematem, jest
błędem wywołania interfejsu (`NURT_E_ARG` interfejsu C). Koperta, która zgadza
się ze schematem, ale łamie regułę koperty, na przykład dwa wejścia naraz,
otrzymuje odpowiedź w postaci raportu ze statusem `failed` i błędem `E_ARG`.

## 10. Odpowiedź (`nurt-response/1`) {#10-response-nurt-response1}

```json
{
  "nurt": "1.0",
  "report": { "nurt": "1.0", "status": "ok", "kind": "remove", "op": 21 },
  "rendered": { "format": "text", "text": "op 21  remove  ok\n" }
}
```

Raport jest zawsze obecny. `rendered` zawiera tekst w żądanym formacie dla
`text`, `markdown` i `compact`; jest nieobecne dla `json`, gdzie wynikiem jest
raport. Dalsza strona istnieje, gdy obecne jest `report.next_cursor`.
`nurt_run_json`, metody `run` i `page` programu `nurt serve` oraz narzędzie
`nurt` integracji zwracają ten dokument.

### 10.1 Skrypt z kilkoma potokami {#101-script-with-several-pipelines}

Skrypt (opcja `script`), którego wykonanie przeprowadziło więcej niż jedną
operację, zachowuje wynik każdego potoku. Odpowiedź zawiera wtedy `reports`,
tablicę z jednym raportem na wykonany potok, w kolejności programu:

```json
{
  "nurt": "1.0",
  "report": { "nurt": "1.0", "status": "ok", "kind": "remove", "op": 21 },
  "reports": [
    { "nurt": "1.0", "status": "ok", "kind": "find",
      "records": { "type": "row", "rows": [ { "value": 3 } ] } },
    { "nurt": "1.0", "status": "ok", "kind": "remove", "op": 21,
      "changes": [] }
  ]
}
```

- Każdy raport z `reports` niesie `kind` własnego potoku (ostatnie polecenie
  skutkujące potoku, w przeciwnym razie jego polecenie źródłowe), status i błąd
  własnej operacji oraz własne zmiany, skutki nietypowe i nieodwracalne, `op` i
  rekordy.
- `report` jest raportem całego programu. Jego `kind` to ostatnie polecenie
  skutkujące programu, w przeciwnym razie polecenie źródłowe pierwszego
  potoku. Niesie zmiany, skutki nietypowe i nieodwracalne oraz błąd każdej
  operacji, ale nie niesie rekordów, ponieważ rekordy należą do raportu ich
  potoku.
- Skrypt zatrzymuje się na pierwszej operacji, która kończy się błędem;
  `reports` kończy się wtedy raportem tej operacji.
- `rendered` zawiera tekst `report`, a po nim tekst każdego raportu z
  `reports`, każdy zaczynający się w nowym wierszu.
- Raporty z `reports` nie są stronicowane i nie niosą `next_cursor`;
  stronicowanie i plik wyniku (`save_to`) dotyczą `report`.
- Program z jedną operacją nie ma `reports`; jego `report` niesie rekordy jak
  dotąd.

## 11. Zgodność {#11-conformance}

Katalog `golden/report/` zawiera raporty dla każdego statusu, dla każdej grupy
kodów błędów, dla operacji zbiorczej oraz dla raportów ze skutkami nietypowymi
i nieodwracalnymi; `golden/request/` zawiera koperty dla każdego wejścia, a
`golden/request/response/` zawiera odpowiedzi. Każdy plik wzorcowy przechodzi
walidację względem opublikowanego schematu i względem schematu wygenerowanego z
typów Rust oraz przetrwa bez zmian przejścia w obie strony przez JSON i CBOR.
Wygenerowane schematy są przechowywane jako zatwierdzone migawki w
`golden/report/schema/` i `golden/request/schema/`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
