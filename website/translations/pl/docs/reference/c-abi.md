<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/reference/c-abi.md -->
<!-- Responsibility:   Documents C interface (libnurt). -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Interfejs C (libnurt) {#c-interface-libnurt}

Biblioteka `libnurt` udostępnia silnik dla C i dla każdego języka z interfejsem
obcych funkcji C. Interfejs jest zadeklarowany w
`crates/nurt-ffi/include/nurt.h`; wyeksportowane symbole są wymienione w
`crates/nurt-ffi/nurt.symbols`. Skrzynka `nurt-ffi` buduje bibliotekę
współdzieloną (`libnurt_ffi.so`, `libnurt_ffi.dylib`, `nurt_ffi.dll`) i
bibliotekę statyczną. Wydanie publikuje bibliotekę współdzieloną we własnym
archiwum `nurt-lib-<version>-<target>`, jako `libnurt.so.1`, `libnurt.1.dylib` i
`nurt.dll`, wraz z `nurt.h` i `nurt.pc`; archiwum programu jej nie zawiera.
Pakiety instalują bibliotekę jako `libnurt.so` (także `libnurt.so.1`) i
`libnurt.dylib`. Wiązania dla .NET i Javy szukają najpierw nazwy z pakietu, a
potem nazwy z kompilacji, więc obie są znajdowane; ścieżka podana w opcjach lub
w `NURT_LIBRARY` jest używana tak, jak podano, niezależnie od jej nazwy.

Na celach musl (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`)
biblioteka wydania jest statyczna: archiwum `nurt-lib-<version>-<target>`
zawiera `libnurt.a`, `nurt.h` i `nurt.pc`, a `pkg-config --static --libs nurt`
zwraca każdą bibliotekę, której potrzebuje łączenie statyczne. W systemie musl
biblioteka jest statyczna (`libnurt.a`) i nie może być łączona w jednym
programie z inną biblioteką statyczną napisaną w Rust. Program
`examples/c/hello.c` wywołuje `nurt_abi_check` i `nurt_version`; kontrole wydania
budują go i uruchamiają względem archiwum biblioteki.

## Wersjonowanie {#versioning}

| Element | Wartość |
|---|---|
| `NURT_ABI_MAJOR` | `1` |
| `NURT_ABI_MINOR` | `0` |
| `NURT_ABI_VERSION` | `(NURT_ABI_MAJOR << 16) \| NURT_ABI_MINOR` |

W obrębie jednej wersji głównej interfejs tylko rośnie: funkcje, kody statusu i
składniki JSON są dodawane, nigdy usuwane ani zmieniane. Wywołujący przekazuje
`NURT_ABI_VERSION` swojego nagłówka do `nurt_abi_check` jednokrotnie po
załadowaniu biblioteki; wywołanie zwraca `NURT_OK`, gdy wersje główne są równe,
a wersja podrzędna biblioteki nie jest niższa, w przeciwnym razie `NURT_E_ABI`.

## Konwencje {#conventions}

- Ciągi znaków przekazywane do biblioteki są w UTF-8 i zakończone `\0`.
- Wartości złożone są dokumentami JSON: koperty żądań `nurt-request/1`,
  odpowiedzi `nurt-response/1`, drzewa składni `nurt-ast/1`, konfiguracja
  `nurt-config/1`, błędy i zdarzenia.
- Uchwyty (`nurt_engine`, `nurt_exec`) są nieprzezroczyste.
- Każda funkcja jest bezpieczna wątkowo, chyba że jej opis stanowi inaczej.
- Po `fork()` bez `exec()` silnik otwarty w procesie rodzicielskim nie nadaje się
  do użycia w procesie potomnym; jego funkcje zwracają `NURT_E_INTERNAL`.

## Kody statusu {#status-codes}

| Kod | Wartość | Znaczenie |
|---|---|---|
| `NURT_OK` | 0 | Wywołanie się powiodło. Błąd języka, jeśli wystąpił, jest w wyniku JSON. |
| `NURT_E_TIMEOUT` | 1 | `nurt_exec_wait`: wykonanie nadal trwa po upływie limitu czasu. |
| `NURT_E_CANCELLED` | 2 | `nurt_exec_wait`: wykonanie zakończyło się, ponieważ zostało anulowane. |
| `NURT_E_ARG` | 3 | Wskaźnik zerowy tam, gdzie wymagana jest wartość, tekst niebędący UTF-8 albo JSON nieprawidłowy dla argumentu. |
| `NURT_E_ABI` | 4 | Niezgodna wersja interfejsu (`nurt_abi_check`, `nurt_ext_load`). |
| `NURT_E_IO` | 5 | Nie można otworzyć lub odczytać pliku konfiguracji, dziennika, katalogu stanu albo rozszerzenia. |
| `NURT_E_INTERNAL` | 6 | Awaria wewnętrzna, w tym panika przechwycona na granicy. |

Błędy języka (składnia, wiązanie, asercje, nieaktualne cele, błędy dziennika) nie
są niepowodzeniami wywołania. Wywołanie zwraca `NURT_OK`, a odpowiedź zawiera
`report.error` ze stabilnym kodem `E_*`. Koperta żądania, która nie jest JSON lub
nie odpowiada schematowi `nurt-request/1`, to `NURT_E_ARG`; koperta, która
odpowiada schematowi, ale łamie regułę koperty (brak wejścia, dwa wejścia,
`variant` bez `program`, `limit` poza zakresem), zwraca `NURT_OK` i odpowiedź z
błędem języka `E_ARG`; względna opcja ścieżki (`base`, `save_to`) daje
`E_PATH_RELATIVE`.

### Błędy języka w innych wynikach {#language-errors-in-other-results}

| Wynik | Postać błędu języka |
|---|---|
| Odpowiedź (`nurt_run_json`, `nurt_exec_wait`, `nurt_page`) | `report.error` odpowiedzi. |
| Jeden dokument JSON (`nurt_correct`, `nurt_catalog`, `nurt_history`, `nurt_space`) | Dokument to `{"error": {...}}`. |
| Dwa dokumenty JSON (`nurt_parse`, `nurt_check`) | Pierwszy dokument to `null`; diagnostyka to `{"diagnostics": [], "error": {...}}`. |
| Tekst (`nurt_print`, `nurt_describe`, `nurt_help`) i brak wyniku (`nurt_session_end`, `nurt_ext_load`) | Wywołanie kończy się statusem kodu błędu, gdy funkcja może go zwrócić (`E_ARG`, `E_UNSUPPORTED`, `E_UNKNOWN_NAME`: `NURT_E_ARG`; `E_IO`: `NURT_E_IO` dla `nurt_session_end` i `nurt_ext_load`), w przeciwnym razie `NURT_E_INTERNAL`. `nurt_last_error` zawiera obiekt błędu pod `detail.error`. |

Obiekt błędu ma składniki błędu raportu skutku
(`spec/schemas/nurt-report-1.schema.json`): `code`, `message`, `at` (pozycja, z
`stage`, `line`, `column` i `length`), `found`, `fix`, `help`, `details`. Etapem
wykonania jest `at.stage`.

Wywołujący traktuje każdy nieznany status niezerowy jako niepowodzenie; późniejsza
wersja podrzędna może dodać kody.

### Kody statusu każdej funkcji {#status-codes-of-each-function}

| Funkcja | OK | TIMEOUT | CANCELLED | ARG | ABI | IO | INTERNAL |
|---|---|---|---|---|---|---|---|
| `nurt_abi_check` | x | | | | x | | |
| `nurt_info` | x | | | x | | | x |
| `nurt_engine_open` | x | | | x | | x | x |
| `nurt_engine_config_get` | x | | | x | | | x |
| `nurt_parse` | x | | | x | | | x |
| `nurt_print` | x | | | x | | | x |
| `nurt_correct` | x | | | x | | | x |
| `nurt_check` | x | | | x | | | x |
| `nurt_run_json` | x | | | x | | | x |
| `nurt_exec_start` | x | | | x | | | x |
| `nurt_exec_wait` | x | x | x | x | | | x |
| `nurt_exec_cancel` | x | | | x | | | x |
| `nurt_page` | x | | | x | | | x |
| `nurt_catalog` | x | | | x | | | x |
| `nurt_describe` | x | | | x | | | x |
| `nurt_help` | x | | | x | | | x |
| `nurt_schema` | x | | | x | | | x |
| `nurt_history` | x | | | x | | | x |
| `nurt_space` | x | | | x | | | x |
| `nurt_session_end` | x | | | x | | x | x |
| `nurt_ext_load` | x | | | x | x | x | x |
| `nurt_ext_list` | x | | | x | | | x |
| `nurt_last_error` | x | | | x | | | |

`nurt_abi_version`, `nurt_version`, `nurt_engine_close`, `nurt_exec_free`,
`nurt_buf_free` i `nurt_set_log_callback` nie zwracają statusu. Tabela jest
weryfikowana testami interfejsu w C, Pythonie, Node i Rust, które wytwarzają
każdy oznaczony status.

## Ostatni błąd {#last-error}

`nurt_last_error(e, out_json)` zwraca ostatnie niepowodzenie wywołującego wątku:

```json
{"status": 3, "code": "NURT_E_ARG", "message": "`program` must not be null", "detail": {"function": "nurt_parse", "argument": "program"}}
```

| Składnik | Typ | Znaczenie |
|---|---|---|
| `status` | liczba całkowita | Wartość kodu statusu. |
| `code` | ciąg znaków | Nazwa kodu statusu. |
| `message` | ciąg znaków | Co zawiodło i jak to poprawić. |
| `detail` | obiekt | `function`: wyeksportowana funkcja, która zawiodła; dalsze składniki zależą od niepowodzenia (`argument`, `path`, `expected`, `error`). |

Rekord jest przechowywany dla każdego wątku, jak `errno`, i jest zastępowany
dopiero przez następne nieudane wywołanie tego samego wątku; `NURT_OK`,
`NURT_E_TIMEOUT` i `NURT_E_CANCELLED` pozostawiają go bez zmian. `e` może być
zerowe, na przykład po nieudanym `nurt_engine_open`. Bez zapisanego niepowodzenia
bufor jest pusty.

## Pamięć {#memory}

| Reguła | Opis |
|---|---|
| Własność | Pamięć zwracana przez bibliotekę jest przechowywana w `nurt_buf` i zwalniana przez `nurt_buf_free`. Ciąg zwracany przez `nurt_version` jest statyczny. |
| Pusty bufor | `ptr` jest `NULL` dokładnie wtedy, gdy `len` wynosi 0. |
| Terminator | Bajt `\0` następuje po ostatnim bajcie i nie jest wliczany do `len`; `ptr` można używać jako ciągu C. |
| `cap` | Należy do biblioteki; wywołujący go nie odczytują. |
| Bufory wyjściowe | Bufor wyjściowy jest nadpisywany bez zwalniania: należy przekazać bufor zainicjowany zerami lub już zwolniony. |
| Niepowodzenie | Gdy wywołanie zwraca status inny niż `NURT_OK`, jego bufory wyjściowe są puste. Jedynym wyjątkiem jest odpowiedź `nurt_exec_wait` z `NURT_E_CANCELLED`. |
| Zwolnienie | `nurt_buf_free` ustawia bufor na zero; przyjmuje `NULL` i bufory zerowe, więc dwukrotne zwolnienie jest bezpieczne. |
| Uchwyty | Silnik jest zwalniany przez `nurt_engine_close`, wykonanie przez `nurt_exec_free`; oba przyjmują `NULL`. Wykonanie zachowuje potrzebne mu dane silnika, więc może zostać zwolnione po swoim silniku. |

## Silnik {#engine}

`nurt_engine_open` otwiera silnik z zegarem i hostem procesu. Konfiguracja jest
składana z wbudowanych wartości domyślnych, plików systemowych i użytkownika,
pliku projektu, pliku wskazanego przez `NURT_CONFIG`, zmiennych `NURT_*` i
ustawień czasu wykonania; `config_json` jest stosowany na poziomie pliku
`NURT_CONFIG`, po tym pliku. Katalogiem stanu jest `state.dir` albo katalog
platformy.

| Niepowodzenie | Status |
|---|---|
| `config_json` nie jest JSON albo nie jest obiektem `nurt-config/1` | `NURT_E_ARG` |
| Wartość nie pasuje do swojego klucza, na przykład względny `state.dir` | `NURT_E_ARG`; `detail.error.code` to `E_CONFIG` lub `E_PATH_RELATIVE` |
| Nie można odczytać ani utworzyć pliku konfiguracji lub katalogu stanu | `NURT_E_IO` |

| Funkcja | Wynik |
|---|---|
| `nurt_engine_config_get` | Obowiązująca konfiguracja `nurt-config/1`. |
| `nurt_parse` | Drzewo składni i `{"diagnostics": []}`. |
| `nurt_correct` | `{"program": text, "ast": tree, "corrections": [...]}`; `program` jest poprawionym tekstem albo podanym tekstem, gdy nic nie poprawiono. |
| `nurt_check` | Dokument kontroli `nurt-check/1` (`format`, `effect`, `reversibility`, `operations`, `waits`, `paths`, `network`, `programs`, `groups`, `corrections`, `details`) ze składnikiem `bound` zawierającym powiązane drzewo oraz `{"diagnostics": []}`. |
| `nurt_page` | Odpowiedź strony; `limit` 0 zachowuje limit pierwotnego żądania. |
| `nurt_describe` | Tekst tematu `name`, w przeciwnym razie `layer`, w przeciwnym razie `core`, w `language` lub skonfigurowanym języku. Składniki inne niż `layer`, `name` i `language` to `NURT_E_ARG`. |
| `nurt_help` | Tekst tematu; `NULL` prosi o indeks tematów (temat pusty). |
| `nurt_history` | `{"operations": [...]}`; `query_json` zawiera parametry polecenia `history` (`id`, `limit`, `state`, `before`, `since`, `path`, `deleted_only`); nieznany składnik to `NURT_E_ARG`. |
| `nurt_space` | `{"volumes": [...]}`; względny `volume` to błąd języka `E_PATH_RELATIVE`. |
| `nurt_session_end` | Usuwa ustawienia czasu wykonania sesji. Identyfikator pusty, dłuższy niż limit lub ze znakami sterującymi to `NURT_E_ARG`. |
| `nurt_ext_list` | `{"extensions": [...]}`. |
| `nurt_ext_load` | `E_UNSUPPORTED` (`NURT_E_ARG`), gdy biblioteka nie ma hosta rozszerzeń. |
| `nurt_catalog` | Katalog poleceń albo `{"error": ...}` z `E_UNSUPPORTED`, dopóki silnik nie ma katalogu. `what_json` to `null`, `{"group": name}` dla poleceń grupy albo `{"command": name}` dla specyfikacji polecenia; inny składnik to `NURT_E_ARG`, nieznana nazwa to `{"error": ...}`. |

## Informacje {#information}

`nurt_info` nie potrzebuje silnika i zwraca:

| Składnik | Znaczenie |
|---|---|
| `version` | Wersja biblioteki. |
| `abi` | `major`, `minor` i `version` interfejsu. |
| `formats` | Wersje formatów `ast`, `bound`, `report`, `request`, `response`. |
| `schemas` | Nazwy przyjmowane przez `nurt_schema`. |
| `catalog_rev` | Rewizja katalogu poleceń wbudowanego w bibliotekę: `sha256:` i skrót jego kanonicznego JSON; `null`, gdy katalogu nie można załadować. |
| `features` | Funkcje Cargo skompilowane w bibliotece, na przykład `stub`. |
| `target` | `arch`, `os`, `family`, `pointer_width`, `endian`. |
| `build` | `profile`: `debug` lub `release`. |

## Paniki {#panics}

Każda wyeksportowana funkcja działa wewnątrz granicy paniki. Panika jest
przechwytywana, zgłaszana jako `NURT_E_INTERNAL` z komunikatem paniki w ostatnim
błędzie i wysyłana do odbiornika dziennika na poziomie `NURT_LOG_ERROR`. Silnik
pozostaje zdatny do użycia. Granica opiera się na odwijaniu stosu: biblioteka
jest budowana z `panic = "unwind"`.

## Wątki {#threads}

| Element | Zachowanie |
|---|---|
| Silnik | Jeden silnik może być używany z kilku wątków jednocześnie. W jednym procesie może istnieć kilka silników. |
| Wywołania synchroniczne | `nurt_run_json` i pozostałe funkcje z wynikiem działają w wątku wywołującego. |
| Wykonanie | `nurt_exec_start` uruchamia żądanie w wątku silnika i wraca natychmiast. |
| Zdarzenia | Każde wykonanie ma jeden wątek dostarczania. Wywołanie zwrotne zdarzeń jest wywoływane wyłącznie z tego wątku, po jednym zdarzeniu naraz. Zdarzenia są buforowane, więc powolne wywołanie zwrotne nie spowalnia wykonania. |
| Odbiornik dziennika | `nurt_set_log_callback` ustawia jeden odbiornik dla całego procesu; może być wywoływane z dowolnego wątku. Przekazanie `NULL` zatrzymuje zapis dziennika. |
| Ostatni błąd | Przechowywany dla każdego wątku. |

### Zdarzenia wykonania {#events-of-an-execution}

Każde zdarzenie jest jednym obiektem JSON przekazanym do wywołania zwrotnego jako
`event_json`, ważnym tylko w czasie wywołania:

```json
{"seq": 1, "time": "2026-11-21T09:12:00Z", "request": "r1", "t": "exec.started"}
{"seq": 2, "time": "2026-11-21T09:12:00Z", "request": "r1", "t": "progress", "command": "find", "phase": "scan", "done": 64, "total": 148, "bytes": null, "message": null}
{"seq": 3, "time": "2026-11-21T09:12:00Z", "request": "r1", "t": "exec.finished", "status": "ok", "error": null}
```

Składniki są takie, jak w `spec/schemas/nurt-event-1.schema.json`, tym samym
dokumencie, który `nurt serve` wysyła w powiadomieniach `nurt/event`.

| Składnik | Znaczenie |
|---|---|
| `seq` | Numer kolejny, od 1, rosnący o 1 w obrębie wykonania. |
| `time` | Czas zdarzenia jako tekst, RFC 3339 w UTC. |
| `request` | Identyfikator żądania jako tekst, ten sam dla każdego zdarzenia wykonania; `null`, gdy żądanie go nie ma. |
| `t` | Typ zdarzenia: `exec.started`, `progress`, `job.started`, `job.output`, `job.progress`, `job.finished`, `log`, `exec.finished`. |
| `id` | Tylko zdarzenia `job.*`: identyfikator zadania w tle. |
| `status` | Tylko `exec.finished`: status raportu (`ok`, `failed` i pozostałe wartości `report.status`). Wykonanie, które kończy się bez odpowiedzi, ma `failed`. |
| `error` | Tylko `exec.finished`: kod błędu, który zakończył wykonanie, albo `null`. |

Pierwszym zdarzeniem jest `exec.started`, a ostatnim `exec.finished`. Po
`exec.finished` nie następuje żadne wywołanie zwrotne, podobnie po
`nurt_exec_free`. Wywołanie zwrotne może wywoływać inne funkcje biblioteki; nie
wywołuje `nurt_exec_free` dla własnego wykonania, `nurt_engine_close` dla
własnego silnika ani `nurt_exec_wait` dla własnego wykonania z limitem czasu
innym niż 0.

## Oczekiwanie i anulowanie {#waiting-and-cancellation}

| Wywołanie | Zachowanie |
|---|---|
| `nurt_exec_wait(x, 0, out)` | Sprawdza bez oczekiwania: `NURT_E_TIMEOUT` i pusty bufor, dopóki wykonanie trwa. |
| `nurt_exec_wait(x, ms, out)` | Czeka nie dłużej niż `ms` milisekund. |
| `nurt_exec_wait(x, NURT_WAIT_FOREVER, out)` | Czeka bez limitu. |
| Po zakończeniu | Każde wywołanie zwraca ten sam status i tę samą odpowiedź. |
| `nurt_exec_cancel(x)` | Żąda anulowania i zwraca `NURT_OK`. Powtórne wywołania nie mają dalszego skutku. |
| Anulowanie trwającego wykonania | Wykonanie zatrzymuje się w następnym punkcie anulowania. `nurt_exec_wait` zwraca `NURT_E_CANCELLED` oraz odpowiedź ze statusem `failed` i błędem `E_CANCELLED`. Zmiany już zatwierdzone pozostają w dzienniku i można je cofnąć. |
| Anulowanie po zakończeniu | Nie ma skutku: wynik pozostaje `NURT_OK` albo statusem niepowodzenia, którym wykonanie się zakończyło. |
| `nurt_exec_free(x)` | Anuluje trwające wykonanie, czeka na jego wątki i zwalnia uchwyt. |
| `nurt_engine_close(e)` | Anuluje trwające wykonania silnika i czeka, aż się zakończą. |

Wykonanie jest zakończone, gdy `exec.finished` zostało dostarczone, więc po
zwróceniu wyniku przez `nurt_exec_wait` żadne wywołanie zwrotne zdarzeń nie
działa dla tego wykonania. `nurt_exec_wait` zwraca `NURT_E_CANCELLED` dokładnie
wtedy, gdy odpowiedź zgłasza `E_CANCELLED`; wykonanie, które zakończyło się przed
punktem anulowania, zachowuje swoją odpowiedź i `NURT_OK`.

## Silnik testowy {#test-engine}

Z funkcją cargo `stub` silnik jest zastępowany dublerem testowym, który odpowiada
z zarejestrowanych przypadków; jego zdarzenia mają te same składniki co zdarzenia
silnika. Funkcja nigdy nie jest włączana w kompilacji wydania. Każdy przypadek
jest plikiem JSON w katalogu wskazanym przez `NURT_STUB_DIR` (domyślnie
`crates/nurt-ffi/tests/fixtures/stub`); wywołanie otrzymuje odpowiedź od
przypadku, którego klucz jest równy skrótowi SHA-256 kanonicznej postaci JSON
(RFC 8785) `{"function": ..., "input": ...}`. `NURT_STUB_PANIC` zawiera
rozdzieloną przecinkami listę wyeksportowanych funkcji, które wywołują panikę
wewnątrz swojej granicy, na potrzeby testów `NURT_E_INTERNAL`.

## Przykłady {#examples}

Uruchomienie żądania:

```c
#include <stdio.h>
#include "nurt.h"

int main(void) {
    nurt_engine *engine = NULL;
    nurt_buf response = {0};
    if (nurt_abi_check(NURT_ABI_VERSION) != NURT_OK ||
        nurt_engine_open(NULL, &engine) != NURT_OK) {
        nurt_buf error = {0};
        nurt_last_error(NULL, &error);
        fprintf(stderr, "%s\n", error.ptr ? (const char *)error.ptr : "unknown failure");
        nurt_buf_free(&error);
        return 1;
    }
    if (nurt_run_json(engine, "{\"nurt\":\"1.0\",\"program\":\"find \\\"/srv/app\\\"\"}",
                      &response) == NURT_OK) {
        printf("%s\n", (const char *)response.ptr);
    }
    nurt_buf_free(&response);
    nurt_engine_close(engine);
    return 0;
}
```

Oczekiwanie na wykonanie z terminem:

```c
nurt_exec *exec = NULL;
nurt_buf response = {0};
nurt_exec_start(engine, request_json, on_event, &state, &exec);
if (nurt_exec_wait(exec, 5000, &response) == NURT_E_TIMEOUT) {
    nurt_exec_cancel(exec);
    nurt_exec_wait(exec, NURT_WAIT_FOREVER, &response);
}
nurt_buf_free(&response);
nurt_exec_free(exec);
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
