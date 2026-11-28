<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/reference/serve-protocol.md -->
<!-- Responsibility:   Documents `nurt serve` protocol reference (`nurt-rpc/1`). -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Referencja protokołu `nurt serve` (`nurt-rpc/1`) {#nurt-serve-protocol-reference-nurt-rpc1}

`nurt serve` uruchamia silnik za serwerem JSON-RPC 2.0 na standardowym wejściu i
standardowym wyjściu. Aplikacje osadzające, które nie mogą załadować biblioteki C
(`docs/reference/c-abi.md`), używają tego protokołu; oba interfejsy zwracają te
same dokumenty JSON i odwzorowują niepowodzenia na te same kody statusu.

## Składnia {#synopsis}

```
nurt serve [--framing ndjson|content-length]
```

| Opcja | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `--framing` | `ndjson` \| `content-length` | `ndjson` | Rozgraniczanie komunikatów w obu strumieniach. |

Standardowe wyjście przenosi wyłącznie komunikaty protokołu. Diagnostyka procesu
trafia na standardowe wyjście błędów albo, po `initialize` z włączonymi
zdarzeniami, do zdarzeń `log`.

## Ramkowanie {#framing}

| Ramkowanie | Postać jednego komunikatu |
|---|---|
| `ndjson` | Jeden dokument JSON na wiersz, UTF-8, zakończony `\n`. `\r` przed `\n` oraz puste wiersze są ignorowane. Serwer nigdy nie zapisuje wysunięcia wiersza wewnątrz komunikatu. |
| `content-length` | Wiersze nagłówków zakończone `\r\n`, pusty wiersz, a następnie dokładnie `Content-Length` bajtów JSON. Pozostałe nagłówki są ignorowane. Serwer zapisuje wyłącznie nagłówek `Content-Length`. |

Odebrany komunikat dłuższy niż limit rozmiaru (16 MiB, chyba że serwer
skonfigurowano inaczej; zgłaszany w `initialize` jako `limits.max_message_bytes`)
jest pomijany i otrzymuje odpowiedź `-32600` z `id: null`. Uszkodzony blok
nagłówków ramkowania `content-length` otrzymuje odpowiedź `-32700` i kończy
połączenie z kodem wyjścia 1.

## Komunikaty {#messages}

Każdy komunikat jest obiektem JSON-RPC 2.0 z `"jsonrpc": "2.0"`.

| Komunikat | Składniki | Zachowanie serwera |
|---|---|---|
| Żądanie | `id` (ciąg znaków, liczba lub `null`), `method`, `params` (obiekt, tablica lub brak) | Otrzymuje dokładnie jedną odpowiedź z tym samym `id`. |
| Powiadomienie | `method`, `params`, bez `id` | Wykonywane; nigdy bez odpowiedzi, także przy błędzie. |
| Odpowiedź wysłana przez klienta | `id`, `result` lub `error` | Ignorowana. Serwer nie wysyła żądań. |
| Wsad (tablica) | | Odrzucany z `-32600`; należy wysyłać jedno żądanie na komunikat. |

## Cykl życia połączenia {#connection-lifecycle}

1. Klient wysyła `initialize`. Przed nim każda metoda poza `initialize` i
   `shutdown` otrzymuje odpowiedź `-32600`.
2. Klient wysyła żądania. Kilka żądań może być w toku; odpowiedzi przychodzą w
   kolejności kończenia żądań i są dopasowywane po `id`.
3. Klient wysyła `shutdown` albo zamyka wejście. Serwer przestaje czytać,
   czeka, aż każde żądanie w toku otrzyma odpowiedź, odpowiada na `shutdown`
   wartością `null` i kończy działanie. Komunikaty po `shutdown` nie są czytane.

| Kod wyjścia | Znaczenie |
|---|---|
| `0` | Odebrano `shutdown` albo wejście się skończyło. |
| `1` | Nie dało się odczytać wejścia, ramkowanie było uszkodzone albo nie dało się zapisać wyjścia. |

Gdy zapis na wyjście się nie powiedzie, serwer anuluje żądania w toku i nie
czyta kolejnego komunikatu: kończy działanie z kodem 1 przed następnym
odczytem albo, gdy odczyt już oczekuje, gdy tylko ten odczyt wróci; komunikat,
który zwrócił, nie jest przetwarzany. Serwer nigdy nie czeka na koniec wejścia.

## Metody {#methods}

| Metoda | Parametry | Wynik |
|---|---|---|
| `initialize` | zob. [`initialize`](#initialize) | zob. [`initialize`](#initialize) |
| `run` | koperta żądania `nurt-request/1` | odpowiedź `nurt-response/1`; żądanie, którego jedyną wadą jest `ast` niebędące drzewem `nurt-ast/1`, otrzymuje błąd języka `E_AST_SCHEMA` (status raportu `failed`), a nie `-32602` |
| `check` | koperta żądania `nurt-request/1` | dokument kontroli `nurt-check/1` ze składnikami `bound` (powiązane drzewo), `ast_hash` (skrót powiązanego drzewa z `bound`, jak w `spec/syntax-tree.md` sekcja 6, `sha256:<hex>`; nieobecny tylko wtedy, gdy drzewo nie jest znane, nigdy dla programu podanego jako tekst) i `diagnostics` |
| `parse` | `{program: string, variant?: "a" \| "b" \| "c"}` | `{ast: nurt-ast/1, diagnostics: []}` |
| `print` | `{ast: nurt-ast/1, variant?: "a" \| "b" \| "c"}` | `{text: string}` |
| `correct` | `{program: string, variant?: "a" \| "b" \| "c"}` | `{ast: nurt-ast/1, corrections?: [...], ran?: string}`; `ran` jest poprawionym tekstem programu, gdy silnik go poprawił |
| `page` | `{cursor: string, limit?: integer}` | odpowiedź `nurt-response/1` z następną stroną |
| `describe` | `{subject?: string, language?: string}` | `{description: {text: string, tokens: integer, estimate: boolean}}`; opis tematu jest polem `text` |
| `help` | `{topic?: string, language?: string}` | `{text: string}` |
| `schema` | `{name: string}` | opublikowany schemat JSON `name` |
| `catalog` | `{what?: object \| null}` lub brak | katalog poleceń `nurt-catalog-result/1` |
| `history` | `{query?: object \| string \| null}` lub brak | `{operations: [...]}` |
| `space` | `{volume?: absolute path}` | `{volumes: [...]}` |
| `info` | brak | opis silnika: wersja, formaty, funkcje, system |
| `cancel` | `{request_id}` lub `{job_id}` | `{cancelled: boolean}` |
| `shutdown` | brak | `null` |

`variant` domyślnie przyjmuje wariant konfiguracji; `subject` i `topic`
domyślnie przyjmują `core`. `schema` przyjmuje `ast`, `bound`, `report`,
`request`, `response`, `config`, `catalog`, `catalog-result`, `value`, `tool`,
`fs-snapshot`, `check`, `rpc` i `event`. `what` wybiera
`{"group": name}` (polecenia grupy) lub `{"command": name}` (specyfikację
polecenia); `null` lub brak zwraca grupy. `query` jest zapytaniem `history` jako
obiekt lub jako tekst obiektu; obiekt bez `query` jest sam zapytaniem. Wybór lub
zapytanie o innym kształcie jest błędem JSON-RPC `-32602`. Błąd języka dowolnej
metody jest częścią wyniku, a nie błędem JSON-RPC: wynikiem jest wtedy
odpowiedź `nurt-response/1`, której raport ma status `failed`, błąd w
`report.error` oraz, jako `report.kind`, nazwę metody (dla `run` rodzaj
programu).

### `initialize` {#initialize}

| Parametr | Typ | Wymagany | Opis |
|---|---|---|---|
| `protocol` | ciąg znaków | tak | `nurt-rpc/<major>[.<minor>]`; wersja główna musi wynosić `1`. |
| `client` | obiekt `{name, version}` | nie | Nazwa i wersja klienta; informacyjne. |
| `abi` | liczba całkowita | nie | Oczekiwana wersja interfejsu C `(major << 16) \| minor`; porównywana według wersji głównej, gdy serwer osadza interfejs C. |
| `language` | ciąg znaków | nie | Oczekiwana wersja języka; informacyjna. |
| `ast` | ciąg znaków | nie | Oczekiwana wersja `nurt-ast/1`; wersja główna musi się zgadzać. |
| `session` | ciąg znaków | nie | Sesja używana przez żądania `run`, `check` i `page`, w których brak `options.session`. |
| `config_overrides` | obiekt | nie | Musi być nieobecny lub pusty; konfiguracja jest stała po uruchomieniu serwera. |
| `capabilities.events` | wartość logiczna | nie, `false` | Wysyłaj powiadomienia `nurt/event` dla wykonań. |

Nieznane składniki są ignorowane.

| Składnik wyniku | Typ | Opis |
|---|---|---|
| `protocol` | ciąg znaków | `nurt-rpc/1`. |
| `version` | ciąg znaków | Wersja Danaco Nurt. |
| `abi` | liczba całkowita \| `null` | Wersja osadzonego interfejsu C; `null`, gdy serwer go nie osadza. |
| `language` | `null` | Silnik nie zgłasza wersji języka. |
| `ast` | ciąg znaków | Wersja `nurt-ast/1`. |
| `report` | ciąg znaków | Wersja `nurt-report/1`. |
| `catalog_rev` | `null` | Silnik nie zgłasza rewizji katalogu. |
| `features` | tablica ciągów znaków | Funkcje kompilacji, na przykład `stub`. |
| `groups` | tablica ciągów znaków | Grupy poleceń, których usługi dostarcza silnik. |
| `limits.max_inflight` | liczba całkowita | Liczba żądań wykonywanych w tym samym czasie (domyślnie 16). |
| `limits.max_message_bytes` | liczba całkowita | Limit rozmiaru jednego odebranego komunikatu (domyślnie 16777216). |

Błędy: `-32602` dla nieprawidłowych parametrów lub niepustego
`config_overrides`, `-32004` (`NURT_E_ABI`) dla innej wersji głównej `protocol`,
`abi` lub `ast` (`data.supported` nazywa obsługiwaną wersję), `-32600`, gdy
połączenie jest już zainicjowane.

### `cancel` {#cancel}

Podawany jest dokładnie jeden ze składników.

| Parametr | Typ | Opis |
|---|---|---|
| `request_id` | ciąg znaków \| liczba | `id` żądania w toku na tym połączeniu. Porównanie zachowuje typ JSON: `1` i `"1"` różnią się. |
| `job_id` | ciąg znaków | Identyfikator zadania w tle. Niedostępny w tym serwerze: wynikiem jest odpowiedź `failed` z `E_UNSUPPORTED`. |

`cancelled` ma wartość `true`, gdy żądanie było w toku. Anulowanie jest
idempotentne; zakończone lub nieznane żądanie daje `false` i brak błędu.
Anulowane `run` otrzymuje odpowiedź `-32002` (`NURT_E_CANCELLED`), której
`data.response` jest odpowiedzią `failed` z `E_CANCELLED`, a jego ostatnim
zdarzeniem jest `exec.finished` z `error: "E_CANCELLED"`. Metody inne niż `run`
kończą się normalnie.

## Błędy {#errors}

Obiekt `error` odpowiedzi jest używany wyłącznie dla błędów protokołu i
wywołania. Błędy języka są częścią wyniku.

| Kod | Nazwa | Przyczyna |
|---|---|---|
| `-32700` | Parse error | Komunikat nie jest JSON albo ramkowanie jest uszkodzone. |
| `-32600` | Invalid request | Komunikat nie jest obiektem żądania; wsad; komunikat jest za długi; połączenie nie jest zainicjowane albo jest już zainicjowane; osiągnięto limit żądań w toku; w toku jest żądanie z tym samym `id`. |
| `-32601` | Method not found | Metoda jest nieznana; komunikat wymienia znane metody. |
| `-32602` | Invalid params | Parametry nie pasują do metody. |
| `-32603` | Internal error | Serwer nie zdołał zserializować wyniku. |
| `-32000 - s` | Application error | Status `s` interfejsu C; `data.status` go nazywa. |

| Kod | `nurt_status` | Przyczyna w serwerze |
|---|---|---|
| `-32001` | `NURT_E_TIMEOUT` | Nieużywany przez serwer. |
| `-32002` | `NURT_E_CANCELLED` | `run` zostało anulowane; `data.response` zawiera odpowiedź. |
| `-32003` | `NURT_E_ARG` | Zarejestrowany przypadek zaślepki kończy się tym statusem. |
| `-32004` | `NURT_E_ABI` | Niezgodność wersji w `initialize`. |
| `-32005` | `NURT_E_IO` | Zarejestrowany przypadek zaślepki kończy się tym statusem. |
| `-32006` | `NURT_E_INTERNAL` | Wykonanie żądania zakończyło się paniką (`data.panic`) albo żaden przypadek zaślepki nie odpowiada na wywołanie. |

## Współbieżność {#concurrency}

Każde żądanie działa we własnym wątku; `initialize`, `cancel` i `shutdown`
obsługuje wątek odczytujący, więc anulowanie nigdy nie czeka w kolejce za
żądaniem, które anuluje. Co najwyżej `limits.max_inflight` żądań i powiadomień
działa w tym samym czasie; żądanie ponad limit otrzymuje odpowiedź `-32600`.
`id` może zostać użyte ponownie po wysłaniu odpowiedzi na poprzednie żądanie z
tym `id`.

## Zdarzenia {#events}

Po `initialize` z `capabilities.events = true` każde wykonanie `run` wysyła
powiadomienia z metodą `nurt/event`. Parametrami jest dostarczone zdarzenie
kontraktu:

| Składnik | Typ | Opis |
|---|---|---|
| `seq` | liczba całkowita | Numer zdarzenia w obrębie wykonania, od 1. |
| `time` | ciąg znaków | Czas emisji, RFC 3339 UTC. |
| `request` | ciąg znaków \| `null` | `id` żądania jako tekst; `null` dla powiadomienia. |
| `t` | ciąg znaków | Typ zdarzenia. |

| `t` | Dalsze składniki |
|---|---|
| `exec.started` | `ast_hash`, `effect` (`effect` jest nieobecne, gdy silnik go nie zgłasza) |
| `progress` | `command`, `phase`, `done`, `total`, `bytes`, `message` |
| `job.started` | `id`, `label`, `command`, `cwd`, `pid`, `started` |
| `job.output` | `id`, `stream`, `cursor`, `lines`, `bytes` |
| `job.progress` | `id`, `percent`, `line` |
| `job.finished` | `id`, `state`, `exit_code`, `signal`, `elapsed`, `output_file`, `changes` |
| `log` | `level`, `message`, `source` |
| `exec.finished` | `status`, `error` (kod błędu raportu, `null`, gdy raport go nie ma; także dla programu, który zakończył się niepowodzeniem przed zaplanowaniem, na przykład `E_SYNTAX`) |

Zdarzenia jednego wykonania są wysyłane w kolejności `seq`; `exec.started` jest
pierwsze, a `exec.finished` ostatnie, i wszystkie poprzedzają odpowiedź na
żądanie.

## Tryb zaślepki {#stub-mode}

Kompilacja z funkcją `stub` może odpowiadać na podstawie zarejestrowanych
przypadków zaślepki biblioteki C (`crates/nurt-ffi/tests/fixtures/stub/`) zamiast
silnika, na potrzeby testów wiązań przed udostępnieniem silnika. Wywołanie
otrzymuje odpowiedź od przypadku, którego klucz, skrót SHA-256 kanonicznego JSON
`{"function", "input"}`, jest równy kluczowi wywołania; wejście wywołania jest
budowane jak w interfejsie C. `initialize` i `info` zgłaszają funkcję `stub`.
Wywołanie bez przypadku otrzymuje odpowiedź `-32006` oraz `data.key`.

## Przykłady {#examples}

Sesja rozdzielana znakami nowego wiersza (`>` wysyła klient, `<` wysyła serwer):

```
> {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol":"nurt-rpc/1","capabilities":{"events":true}}}
< {"jsonrpc":"2.0","id":1,"result":{"protocol":"nurt-rpc/1","version":"0.1.2","abi":null,"language":null,"ast":"1.1","report":"1.0","catalog_rev":null,"features":[],"groups":[],"limits":{"max_inflight":16,"max_message_bytes":16777216}}}
> {"jsonrpc":"2.0","id":2,"method":"run","params":{"nurt":"1.0","program":"find \"/srv/app\" glob: \"*.go\""}}
< {"jsonrpc":"2.0","method":"nurt/event","params":{"seq":1,"time":"2026-11-21T09:12:00Z","request":"2","t":"exec.started"}}
< {"jsonrpc":"2.0","method":"nurt/event","params":{"seq":2,"time":"2026-11-21T09:12:00Z","request":"2","t":"exec.finished","status":"ok","error":null}}
< {"jsonrpc":"2.0","id":2,"result":{"nurt":"1.0","report":{"nurt":"1.0","status":"ok","kind":"find"}}}
> {"jsonrpc":"2.0","id":3,"method":"shutdown"}
< {"jsonrpc":"2.0","id":3,"result":null}
```

Anulowanie żądania w toku:

```
> {"jsonrpc":"2.0","id":"scan","method":"run","params":{"nurt":"1.0","program":"find \"/srv/archive\""}}
> {"jsonrpc":"2.0","id":5,"method":"cancel","params":{"request_id":"scan"}}
< {"jsonrpc":"2.0","id":5,"result":{"cancelled":true}}
< {"jsonrpc":"2.0","id":"scan","error":{"code":-32002,"message":"the request was cancelled","data":{"status":"NURT_E_CANCELLED","response":{"nurt":"1.0","report":{"nurt":"1.0","status":"failed","kind":"find","error":{"code":"E_CANCELLED","message":"the execution was cancelled"}}}}}}
```

To samo żądanie z ramkowaniem `content-length`:

```
Content-Length: 44\r\n
\r\n
{"jsonrpc":"2.0","id":3,"method":"shutdown"}
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
