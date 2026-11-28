<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/architecture/c-interface.md -->
<!-- Responsibility:   Documents C interface. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Interfejs C {#c-interface}

Biblioteka `libnurt` (skrzynka `nurt-ffi`) udostępnia silnik dowolnemu językowi
przez funkcje zadeklarowane w `crates/nurt-ffi/include/nurt.h`. Nagłówek
jest utrzymywany ręcznie i stanowi wzorzec interfejsu.

## Artefakty {#artifacts}

| System | Biblioteka współdzielona | Biblioteka statyczna |
|---|---|---|
| Linux | `libnurt.so.1` (SONAME, główna wersja interfejsu) | `libnurt.a` |
| macOS | `libnurt.1.dylib` (install name `@rpath/libnurt.1.dylib`) | `libnurt.a` |
| Windows | `nurt.dll` z biblioteką importową `nurt.dll.lib` | `nurt.lib` (zdefiniuj `NURT_STATIC`) |

W celach musl archiwum wydania biblioteki zawiera wyłącznie bibliotekę statyczną `libnurt.a`. W musl biblioteka jest statyczna
(`libnurt.a`) i nie wolno jej łączyć w jednym programie z inną biblioteką statyczną napisaną w Rust.

## Wersjonowanie {#versioning}

- `NURT_ABI_MAJOR` i `NURT_ABI_MINOR` podają wersję nagłówka;
  `nurt_abi_version()` zwraca wersję biblioteki jako
  `(major << 16) | minor`.
- Wiązanie wywołuje `nurt_abi_check(NURT_ABI_VERSION)` jednokrotnie; kończy się to błędem
  `NURT_E_ABI`, gdy wersje główne się różnią lub biblioteka jest starsza niż
  nagłówek.
- W obrębie jednej wersji głównej interfejs tylko się rozrasta: dodawane są funkcje, kody statusu
  i pola JSON; nic nie jest usuwane ani zmieniane.

## Dane i pamięć {#data-and-memory}

- Łańcuchy znaków są w UTF-8 i zakończone `'\0'`; wartości złożone to JSON w
  opublikowanych formatach (`nurt-ast/1`, `nurt-bound/1`, `nurt-request/1`,
  `nurt-config/1`, odpowiedzi, zdarzenia, błędy).
- Wyniki są zwracane w `nurt_buf { ptr, len, cap }`: `ptr` jest NULL dokładnie
  wtedy, gdy `len` wynosi 0, po danych następuje bajt `'\0'`, `cap` jest prywatne, a
  bufor zwalnia się przez `nurt_buf_free`, które jest idempotentne i akceptuje
  NULL.
- Gdy wywołanie kończy się niepowodzeniem, jego bufory wyjściowe pozostają puste. Jedynym wyjątkiem jest
  `nurt_exec_wait` z `NURT_E_CANCELLED`, które zwraca raport anulowanego
  wykonania.

## Kody statusu {#status-codes}

`nurt_status` zgłasza wyłącznie niepowodzenia wywołania. Błędy języka są
zwracane z `NURT_OK` wewnątrz wyniku JSON (`error` z kodem `E_*`).

| Kod | Wartość | Znaczenie |
|---|---|---|
| `NURT_OK` | 0 | wywołanie się powiodło |
| `NURT_E_TIMEOUT` | 1 | `nurt_exec_wait`: nadal trwa |
| `NURT_E_CANCELLED` | 2 | `nurt_exec_wait`: zakończone przez `nurt_exec_cancel` |
| `NURT_E_ARG` | 3 | wskaźnik zerowy, nieprawidłowy UTF-8 lub nieprawidłowy argument JSON |
| `NURT_E_ABI` | 4 | niezgodna wersja interfejsu |
| `NURT_E_IO` | 5 | nie można odczytać konfiguracji, dziennika, katalogu stanu ani rozszerzenia |
| `NURT_E_INTERNAL` | 6 | awaria wewnętrzna, w tym panika przechwycona na granicy |

Możliwe kody każdej funkcji są wymienione na początku `nurt.h`.
Szczegóły ostatniego niepowodzenia wątku wywołującego zwraca
`nurt_last_error`, które działa także bez silnika.

## Wątki i funkcje zwrotne {#threads-and-callbacks}

- Każda funkcja jest bezpieczna wątkowo; w jednym procesie może istnieć kilka silników.
- Zdarzenia wykonania są dostarczane do `nurt_event_cb` z jednego wątku
  biblioteki na wykonanie, pojedynczo, w rosnącej kolejności `seq`, kończąc na
  `exec.finished`. Funkcja zwrotna nie może zwolnić własnego wykonania.
- Komunikaty diagnostyczne trafiają do odbiorcy ustawionego w skali procesu przez `nurt_set_log_callback`
  wraz z `nurt_log_level`.
- Żadna panika nie przekracza granicy. Po `fork()` bez `exec()` silnik
  procesu nadrzędnego nie nadaje się do użycia w procesie potomnym.

## Konfiguracja {#configuration}

`nurt_engine_open(config_json, &engine)` przyjmuje `NULL` lub obiekt JSON w
formacie `nurt-config/1` (zobacz `spec/configuration.md`). Jest stosowany na
poziomie pliku `NURT_CONFIG`, więc aplikacja osadzająca może ustawić zaufane
projekty i dolne granice ochrony dziennika; ustawienia czasu wykonania nie mogą
zastąpić zaufania.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
