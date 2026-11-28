<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/errors.md -->
<!-- Responsibility:   Documents Error codes. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Kody błędów {#error-codes}

Każdy błąd języka ma stabilny kod zapisany wielkimi literami z prefiksem `E_`, komunikat oraz, gdy są znane, etap, pozycję w programie i gotowe zamienniki tekstu programu. Błędy etapów `parse` i `bind` nie mają skutku w świecie: opisują pomyłkę w programie i sposób jej poprawienia, nigdy odmowę. Kody są tylko dodawane w obrębie wersji głównej języka; znaczenie kodu nigdy się nie zmienia.

Poniższa lista kodów, wyliczenie `ErrorCode` skrzynki `nurt-contracts` i definicja `ErrorCode` schematu `nurt-value-1.schema.json` są tożsame; test skrzynki porównuje je ze sobą, a drugi test sprawdza, czy każdy kod wspomniany w specyfikacji i w dokumentacji znajduje się na liście.

## Obiekt błędu {#error-object}

Błąd jest składnikiem `error` raportu skutku. Składniki inne niż `code` i `message` są obecne tylko wtedy, gdy mają wartość.

| Składnik | Typ | Znaczenie |
|---|---|---|
| `code` | tekst | Stabilny kod z poniższej tabeli. |
| `message` | tekst | Komunikat w języku angielskim, jedno lub dwa zdania. |
| `at` | obiekt | Pozycja błędu: `stage`, etap, w którym wystąpił błąd (`0` do `7` lub `5a`), oraz miejsce w programie, `line`, `column` i `length`, liczone w skalarnych wartościach Unicode od 1. Błędy zgłoszone podczas działania etapu zawierają `stage` bez miejsca w programie. |
| `location` | obiekt | Pozycja w pliku danych: `path`, `line`, `column`, `end_line`, `end_column`, `byte`. |
| `found` | tekst | Znaleziony fragment, na przykład nazwa nieobsługiwanej konstrukcji. |
| `problem` | tekst | Krótka nazwa konkretnego problemu, na przykład `not_merged`. |
| `fix` | lista tekstów | Gotowe zamienniki tekstu programu, najlepszy pierwszy. |
| `help` | tekst | Temat pomocy z obowiązującymi regułami. |
| `expected` | dowolny | Co było oczekiwane: nazwy, typy, liczby lub skróty; do ośmiu najbliższych nazw dla nieznanych nazw. |
| `actual` | dowolny | Co znaleziono zamiast tego. |
| `sample` | lista tekstów | Próbka dotyczących rekordów lub ścieżek. |
| `candidates` | lista obiektów | Uszeregowane gotowe programy: `rank`, `ref` (identyfikator dla `accept`), `program`, `why`. |
| `next_steps` | lista tekstów | Dalsze wywołania kończące zadanie, na przykład części podzielonego programu. |
| `effect_applied` | bool | Skutek programu jest w świecie mimo błędu, jak po nieudanej asercji w etapie `5a` lub scaleniu z konfliktami; nieobecne, gdy fałsz. |
| `details` | obiekt | Dane właściwe dla kodu, na przykład `path`, `os_error`, `holders` używanego pliku, `partial_read` nieudanego etapu, `released` elementy częściowego wpisu dziennika. |
| `cause` | obiekt | Błąd, który spowodował ten błąd, w tej samej postaci. |

## Etapy {#stages}

| Etap | Nazwa | Skutek w świecie |
|---|---|---|
| `0` | `parse` | brak |
| `1` | `bind` | brak |
| `2` | `read` | tylko odczyty |
| `3` | `check` | brak; nieudana asercja niczego nie zmienia |
| `4` | `plan` | brak; podgląd kończy się tutaj |
| `5` | `apply` | zmiana, zapisana w dzienniku |
| `5a` | `after` | brak; etapy na rekordach skutku, skutek pozostaje |
| `6` | `verify` | nieudana weryfikacja wycofuje skutek |
| `7` | `report` | brak |

## Status interfejsu C {#status-of-the-c-interface}

Funkcje interfejsu C zwracające dokument odpowiedzi (`nurt_run_json`, `nurt_check`, `nurt_parse`, `nurt_correct`, `nurt_exec_wait`, `nurt_page`, `nurt_history`, `nurt_space`) zwracają `NURT_OK` dla każdego błędu języka i przenoszą błąd w dokumencie. Funkcje bez dokumentu odpowiedzi, takie jak `nurt_engine_open`, `nurt_print` i `nurt_ext_load`, zwracają status z kolumny `Status C` i podają obiekt błędu przez `nurt_last_error`. `NURT_E_ABI` jest zarezerwowany dla niezgodności wersji interfejsu; żaden błąd języka nie jest na niego odwzorowywany. Kody ze statusem `NURT_OK` występują tylko w wywołaniach zwracających dokument odpowiedzi.

## Kody {#codes}

| Kod | Etapy | Status C | Znaczenie |
|---|---|---|---|
| `E_LEXICAL` | 0 | `NURT_E_ARG` | Token jest nieprawidłowy: nieznany znak, niezakończony tekst albo źle zbudowana liczba, rozmiar lub czas trwania. |
| `E_ESCAPE` | 0 | `NURT_E_ARG` | Sekwencja ucieczki w tekście jest nieprawidłowa; pomoc pokazuje przyjmowane sekwencje ucieczki i postać tekstu surowego. |
| `E_SYNTAX` | 0 | `NURT_E_ARG` | Program nie jest zgodny z gramatyką. |
| `E_AMBIGUOUS` | 0, 1 | `NURT_E_ARG` | Poprawka jest możliwa, ale dotyka celu zmiany; `candidates` wymienia uszeregowane gotowe programy. |
| `E_UNCORRECTABLE` | 0, 1 | `NURT_E_ARG` | Program nie może zostać poprawiony regułami poprawiania; `fix` i `help` pokazują dalszą drogę. |
| `E_INTERNAL_CORRECTION` | 0, 1 | `NURT_E_INTERNAL` | Poprawka naruszyła niezmiennik; nic nie jest wykonywane. |
| `E_EXPIRED_CANDIDATE` | 0, 1 | `NURT_E_ARG` | `accept` wskazuje kandydata poprawki, który wygasł lub jest nieznany; program musi zostać wysłany ponownie. |
| `E_AST_VERSION` | 0 | `NURT_E_ARG` | Wersja drzewa składni jest nowsza, niż obsługuje instancja; nieznane węzły są wymienione. |
| `E_AST_SCHEMA` | 0 | `NURT_E_ARG` | Drzewo składni nie jest zgodne ze swoim schematem. |
| `E_UNKNOWN_COMMAND` | 1 | `NURT_E_ARG` | Nieznane polecenie; `expected` wymienia do ośmiu najbliższych nazw. |
| `E_UNKNOWN_PARAM` | 1 | `NURT_E_ARG` | Nieznany parametr polecenia; `expected` wymienia do ośmiu najbliższych nazw. |
| `E_UNKNOWN_NAME` | 1 | `NURT_E_ARG` | Nieznana nazwa `@name` wyniku `let`. |
| `E_UNKNOWN_FIELD` | 1, 2 | `NURT_E_ARG` | Nieznane pole rekordu; `expected` wymienia pola. |
| `E_UNKNOWN_VALUE` | 1 | `NURT_E_ARG` | Nieznana wartość wyliczenia; `expected` wymienia wartości. |
| `E_MISSING_PARAM` | 1 | `NURT_E_ARG` | Brakuje wymaganego parametru. |
| `E_PARAM_CONFLICT` | 1 | `NURT_E_ARG` | Podano razem parametry, które się wykluczają; `expected` wymienia dopuszczalne kombinacje. |
| `E_TYPE` | 1, 2 | `NURT_E_ARG` | Wartość lub strumień rekordów ma typ, którego miejsce nie przyjmuje; `expected` wymienia przyjmowane typy. |
| `E_ARITY` | 1 | `NURT_E_ARG` | Nieprawidłowa liczba argumentów. |
| `E_INVALID_VALUE` | 1 | `NURT_E_ARG` | Wartość ma właściwy typ, ale nie jest prawidłowa dla miejsca, na przykład wyrażenie cron, strefa czasowa lub zbyt krótki interwał. |
| `E_CONFIG` | 0, 1 | `NURT_E_ARG` | Konfiguracja jest nieprawidłowa: nieznany klucz, wartość nieprawidłowego typu lub poza zakresem albo plik, który się nie parsuje; `location` wskazuje miejsce w pliku. |
| `E_PATH_RELATIVE` | 1 | `NURT_E_ARG` | Podano ścieżkę względną bez parametru wywołania `base`. |
| `E_PATH_FOREIGN` | 1 | `NURT_E_ARG` | Ścieżka jest zapisana w postaci obcej dla systemu, na przykład litera dysku w systemie Linux lub nazwa, której system Windows nie dopuszcza. |
| `E_ONE_EFFECT` | 1 | `NURT_E_ARG` | Więcej niż jeden etap zmieniający lub uruchamiający albo etap po skutku, który nie jest tam dozwolony; `next_steps` dzieli program. |
| `E_OS_ONLY` | 1 | `NURT_E_ARG` | Polecenie istnieje tylko w innym systemie operacyjnym. |
| `E_UNSUPPORTED` | 1, 5 | `NURT_E_ARG` | Parametr lub tryb nie jest dostępny w tym systemie, na przykład limit pamięci bez obsługi w jądrze. |
| `E_REGEX` | 1 | `NURT_E_ARG` | Wyrażenie regularne lub szablon zamiany jest nieprawidłowy. |
| `E_KEY_SYNTAX` | 1 | `NURT_E_ARG` | Ścieżka kluczy w dokumencie jest nieprawidłowa. |
| `E_PATTERN_SYNTAX` | 1 | `NURT_E_ARG` | Wzorzec składni nie parsuje się w języku docelowym. |
| `E_QUERY` | 1, 2, 5 | `NURT_E_ARG` | Zapytanie do dokumentu nie kompiluje się lub nie daje się uruchomić. |
| `E_SQL` | 1, 2, 5 | `NURT_E_ARG` | Instrukcji SQL nie można przygotować ani wykonać albo zmienia dane przy `write: false`. |
| `E_ARG` | 1, 5 | `NURT_E_ARG` | Argumentu wywołania nie można przekazać bez zmiany jego znaczenia, na przykład znak NUL w argumencie procesu. |
| `E_CONTINUATION_EXPIRED` | 2 | `NURT_E_ARG` | Kontynuacja odczytywana przez `next` wygasła lub została usunięta przez limity `continuation.max_age`, `continuation.max_count` lub `continuation.max_size`, albo żadna nie jest przechowywana; `next_steps` zawiera wywołanie, które ją wytworzyło. |
| `E_EXTERNAL_MISSING` | 1, 5 | `NURT_OK` | Program wymagany przez polecenie nie jest zainstalowany; `fix` pokazuje, jak go zainstalować. |
| `E_EXTERNAL_FAILED` | 2, 5 | `NURT_OK` | Program zewnętrzny używany przez polecenie, taki jak serwer języka, zawiódł lub złamał swój protokół. |
| `E_PROGRAM_NOT_FOUND` | 1, 5 | `NURT_OK` | Program do uruchomienia nie został znaleziony w ścieżce wyszukiwania; `candidates` wymienia podobne nazwy. |
| `E_SPAWN` | 5 | `NURT_OK` | Nie udało się uruchomić programu: brak uprawnienia do wykonania, nieznany format, brakujący interpreter lub katalog roboczy. |
| `E_ASSERT` | 3, 4, 5a | `NURT_OK` | Asercja nie jest spełniona. Przed skutkiem nic się nie zmieniło; po skutku skutek pozostaje, a raport jest kompletny. |
| `E_BINARY` | 2, 5 | `NURT_OK` | Zawartość binarna tam, gdzie wymagany jest tekst. |
| `E_DOC_PARSE` | 2, 5 | `NURT_E_ARG` | Plik nie jest prawidłowym dokumentem swojego formatu; `location` pokazuje miejsce. |
| `E_FORMAT` | 2, 4, 5 | `NURT_OK` | Format jest uszkodzony lub nieobsługiwany, na przykład archiwum lub notatnik. |
| `E_ENCRYPTED` | 2 | `NURT_OK` | Archiwum lub dokument jest zaszyfrowany. |
| `E_UNSUPPORTED_CONSTRUCT` | 4, 5 | `NURT_OK` | Konstrukcji dokumentu nie można zmienić bez utraty jego układu; konstrukcja jest nazwana w `found`; nic się nie zmieniło. |
| `E_KEY_NOT_FOUND` | 2, 5 | `NURT_OK` | Ścieżka kluczy nie istnieje w dokumencie. |
| `E_KEY_CONFLICT` | 5 | `NURT_OK` | Ścieżka kluczy przechodzi przez wartość innego rodzaju, na przykład indeks w mapie. |
| `E_UNSUPPORTED_LANGUAGE` | 2, 5 | `NURT_OK` | Brak gramatyki dla języka pliku. |
| `E_SYMBOL_NOT_FOUND` | 2, 5 | `NURT_OK` | Symbol nie ma definicji. |
| `E_SYMBOL_AMBIGUOUS` | 2, 5 | `NURT_OK` | Symbol ma kilka definicji; `candidates` wymienia gotowe programy. |
| `E_RENAME_CONFLICT` | 4, 5 | `NURT_OK` | Nowa nazwa koliduje z istniejącym symbolem w swoim zakresie. |
| `E_NO_MATCH` | 4 | `NURT_OK` | Zamiana lub edycja nie znalazła dopasowania, choć wywołanie go wymagało. |
| `E_PATCH_REJECTED` | 4 | `NURT_OK` | Fragment łatki nie daje się zastosować. |
| `E_OUT_OF_RANGE` | 2, 4 | `NURT_OK` | Numer wiersza, strona lub pozycja leży poza zawartością, na przykład wiersz za końcem pliku. |
| `E_STALE` | 5 | `NURT_OK` | Plik zmienił się po jego odczytaniu; cała operacja została wycofana. |
| `E_ENCODING` | 5 | `NURT_OK` | Znaku nie można przedstawić w kodowaniu pliku. |
| `E_IO` | 2, 5 | `NURT_E_IO` | Błąd wejścia lub wyjścia systemu plików. |
| `E_PERMISSION` | 2, 5 | `NURT_E_IO` | Dostęp odmówiony przez uprawnienia. |
| `E_EXISTS` | 2, 5 | `NURT_E_IO` | Cel już istnieje. |
| `E_NOT_FOUND` | 2, 5 | `NURT_E_IO` | Cel nie istnieje. |
| `E_IN_USE` | 2, 5 | `NURT_E_IO` | Cel jest używany; procesy, które go trzymają, są wymienione. |
| `E_PRIVILEGE` | 2, 5 | `NURT_E_IO` | Operacja wymaga uprawnień, które nie są dostępne: żaden backend podniesienia uprawnień jej nie wykonał albo system odmówił podniesienia uprawnień kontu. Szczegóły: `backend` (wypróbowane backendy i wynik każdego: `not_installed`, `password_required`, `denied`, `not_administrator`, `cannot_switch_user`, `failed`), `reason` (`disabled_by_configuration`, gdy `privilege.allowed` ma wartość `false`); `next_steps` wskazuje dalszą drogę. |
| `E_NO_SPACE` | 2, 5 | `NURT_E_IO` | Za mało miejsca na woluminie. |
| `E_TIMEOUT` | 2, 5 | `NURT_E_TIMEOUT` | Osiągnięto limit czasu. |
| `E_CANCELLED` | 2, 5 | `NURT_E_CANCELLED` | Wykonanie zostało anulowane. |
| `E_LIMIT` | 2, 5 | `NURT_OK` | Osiągnięto limit zasobów. |
| `E_STAGE_FAILED` | 2, 5, 6 | `NURT_OK` | Etap zakończył się niepowodzeniem; `partial_read` w `details` może zawierać to, co odczytano. |
| `E_ROLLED_BACK` | 2, 5, 6 | `NURT_OK` | Weryfikacja się nie powiodła, a skutek został wycofany. |
| `E_ROLLBACK_FAILED` | 2, 5, 6 | `NURT_OK` | Wycofanie nie mogło przywrócić wszystkiego; pliki są wymienione wraz z instrukcjami. |
| `E_JOURNAL_NOOP` | 4 | `NURT_OK` | Brak takiej operacji w dzienniku albo nic do cofnięcia lub ponowienia. |
| `E_JOURNAL_EXPIRED` | 4 | `NURT_OK` | Wpis dziennika wygasł; nic się nie zmieniło. |
| `E_JOURNAL_CONFLICT` | 4 | `NURT_OK` | Bieżący stan różni się od stanu po operacji; różnice i możliwości wyboru są wymienione; nic się nie zmieniło. |
| `E_JOURNAL_PARTIAL` | 4 | `NURT_OK` | Wpis jest częściowy: niektóre elementy zostały zwolnione; wywołanie żądało całego wpisu; nic się nie zmieniło. |
| `E_JOURNAL_CORRUPT` | 4, 5 | `NURT_E_IO` | Wpis dziennika nie przeszedł kontroli integralności; jego dane są zachowane do naprawy. |
| `E_UNKNOWN_JOB` | 2 | `NURT_OK` | Nieznane zadanie w tle. |
| `E_JOB_STATE` | 5 | `NURT_OK` | Działanie nie jest dozwolone w bieżącym stanie zadania, na przykład wznowienie zadania, które nie jest wstrzymane. |
| `E_TEST_NO_FRAMEWORK` | 2 | `NURT_OK` | Nie rozpoznano frameworka testowego projektu. |
| `E_UNKNOWN_RUN` | 2 | `NURT_OK` | Nieznane uruchomienie testów. |
| `E_UNKNOWN_ANALYSIS` | 2 | `NURT_OK` | Nieznana analiza. |
| `E_OUTPUT_PARSE` | 5 | `NURT_OK` | Nie udało się odczytać wyjścia frameworka testowego lub analizatora; dołączony jest surowy wynik wykonania. |
| `E_NETWORK` | 2, 5 | `NURT_OK` | Błąd sieci: rozpoznawanie nazw, połączenie, TLS, serwer pośredniczący, przekierowania lub protokół. |
| `E_HASH_MISMATCH` | 5 | `NURT_OK` | Pobrany plik nie ma oczekiwanego skrótu; nie został zachowany. |
| `E_SERVICE` | 5 | `NURT_OK` | Menedżer usług zgłosił błąd. |
| `E_SERVICE_NOT_RUNNING` | 2, 5 | `NURT_OK` | Usługa w tle instancji nie działa. |
| `E_CATALOG` | 2 | `NURT_OK` | Jawnie skonfigurowanego katalogu programów nie można odczytać. |
| `E_GIT_NOT_REPOSITORY` | 2 | `NURT_OK` | Ścieżka nie leży wewnątrz repozytorium; `candidates` wymienia pobliskie repozytoria. |
| `E_GIT_REVISION` | 2 | `NURT_OK` | Rewizja jest nieznana, niejednoznaczna lub źle zbudowana. |
| `E_GIT_REF` | 2, 5 | `NURT_OK` | Nazwa referencji jest nieprawidłowa, istnieje, nie istnieje, nie jest scalona lub jest wyewidencjonowana gdzie indziej. |
| `E_GIT_DIRTY` | 4 | `NURT_OK` | Zmiany lokalne zostałyby nadpisane; nic się nie zmieniło. |
| `E_GIT_CONFLICT` | 5 | `NURT_OK` | Scalanie zakończyło się konfliktami; skutek jest zastosowany, a konflikty są wymienione. |
| `E_GIT_IN_PROGRESS` | 4 | `NURT_OK` | Trwa scalanie lub zmiana bazy albo żadne nie trwa, gdy jest wymagane. |
| `E_GIT_LOCKED` | 4, 5 | `NURT_OK` | Istnieje plik blokady repozytorium. |
| `E_GIT_NON_FAST_FORWARD` | 4 | `NURT_OK` | Historie się rozeszły tam, gdzie dozwolone jest tylko przewinięcie. |
| `E_GIT_NOTHING_TO_COMMIT` | 4 | `NURT_OK` | Nie ma nic do zatwierdzenia. |
| `E_GIT_HOOK` | 5 | `NURT_OK` | Hak zakończył się niezerowym kodem wyjścia; nie utworzono zatwierdzenia. |
| `E_GIT_SIGN` | 5 | `NURT_OK` | Nie udało się podpisać zatwierdzenia lub znacznika. |
| `E_GIT_AUTH` | 5 | `NURT_OK` | Uwierzytelnienie jest wymagane lub zostało odrzucone przez zdalne repozytorium. |
| `E_GIT_HOST_KEY` | 5 | `NURT_OK` | Klucz hosta zdalnego repozytorium jest nieznany lub się zmienił. |
| `E_GIT_PUSH_REJECTED` | 5 | `NURT_OK` | Zdalne repozytorium odrzuciło aktualizację co najmniej jednej referencji. |
| `E_GIT_PROTOCOL` | 5 | `NURT_OK` | Zdalne repozytorium złamało protokół albo brakuje mu wymaganej możliwości. |
| `E_GIT_UNSUPPORTED` | 2, 4 | `NURT_OK` | Funkcja repozytorium nie jest obsługiwana. |
| `E_GIT_HUNK` | 4, 5 | `NURT_OK` | Identyfikator fragmentu jest źle zbudowany lub nieznany, fragment zmienił się od chwili wymienienia albo wybranych wierszy nie można zastosować. |
| `E_GIT_CHECKPOINT` | 4, 6 | `NURT_OK` | Punkt kontrolny nie ma zapisanego indeksu albo jego zawartość nie zgadza się po przywróceniu; ścieżki są wymienione. |
| `E_GIT_WORKTREE` | 4 | `NURT_OK` | Drzewo robocze jest nieznane, jest głównym drzewem roboczym, jest zablokowane lub zawiera katalog roboczy. |
| `E_GIT_CONFIG` | 2, 4, 5 | `NURT_OK` | Konfiguracja repozytorium jest nieprawidłowa: wartości nie można sparsować lub brakuje wymaganego klucza. |
| `E_INTERNAL` | 0, 1, 2, 3, 4, 5, 5a, 6, 7 | `NURT_E_INTERNAL` | Błąd wewnętrzny; raport jest kompletny do tego miejsca. |

## Błędy poprawiania {#errors-of-the-correction}

Błędnie zapisana nazwa polecenia, które zmienia system plików lub stan zdalny albo uruchamia program (`remvoe "/srv/a.txt"`), nigdy nie jest poprawiana samoczynnie. Odpowiedzią jest `E_AMBIGUOUS` w etapie 1; nic nie jest uruchamiane, a pierwszy z `candidates` jest programem z poprawnie zapisaną nazwą, gotowym do ponownego wysłania lub zaakceptowania przez `accept`. Błędnie zapisana nazwa polecenia, które tylko czyta, jest poprawiana, a program jest uruchamiany, podobnie jak nazwy zapisane inną wielkością liter lub synonimem.

Tekst z sekwencją ucieczki, której język nie ma (`"\d+"`, `"C:\proj\data"`), jest odczytywany według swojego miejsca. W miejscu wyrażenia regularnego jest tym wyrażeniem (`search "\d+" "/srv"` działa jako `search /\d+/ "/srv"`), a tekst zapisany jako ścieżka systemu Windows (`"C:\temp\new"`) w miejscu ścieżki jest tą ścieżką z odwrotnymi ukośnikami jako separatorami. Te odczytania są prawdopodobne: w programie, który czyta, są stosowane i pokazywane, a w programie, który coś zmienia, odpowiedzią jest `E_AMBIGUOUS` z poprawionym programem. W każdym innym miejscu nieznana sekwencja ucieczki to `E_ESCAPE` w etapie 0, a `fix` wymienia cały program z tekstem zapisanym jako tekst surowy (`r"\d"`) oraz, dla ścieżki systemu Windows, z `/`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
