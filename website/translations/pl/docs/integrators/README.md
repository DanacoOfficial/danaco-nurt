<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/integrators/README.md -->
<!-- Responsibility:   Documents Integrator guide. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Przewodnik integratora {#integrator-guide}

Opis dla hostów, które udostępniają agentowi AI narzędzie `nurt`: środowisk uruchomieniowych agentów, integracji z SDK, edytorów i usług uruchamiających Danaco Nurt w imieniu modelu.

| Strona | Zawartość |
|---|---|
| [Interfejsy](#interfaces) | `nurt serve`, ABI C i serwer MCP |
| [Definicja narzędzia](tool-definition.md) | `nurt schema tool`, formaty, reguły dla poszczególnych platform, grupy poleceń ładowane na żądanie |
| [Wywołania narzędzia](tool-calls.md) | argumenty i żądania, poprawki i `accept`, strony i kursory |
| [Polityka](policy.md) | polityka hosta z `check` i `preview`: zabronione polecenia, dozwolone katalogi, cele HTTP |
| [Zdalne instancje i zdarzenia](remote-and-events.md) | wysyłanie drzew składni na inne urządzenie, zdarzenia zadań w tle |
| [Wersje i zgodność](versioning.md) | wersje formatów, negocjacja protokołu, reguły zgodności |

Przykłady dla SDK modeli i frameworków agentów: `integrations/examples/`.

## Interfejsy {#interfaces}

| Interfejs | Transport | Zastosowanie | Zdarzenia | Izolacja |
|---|---|---|---|---|
| `nurt serve --stdio` | JSON-RPC 2.0 (`nurt-rpc/1`) przez standardowe wejście i wyjście procesu potomnego | środowiska uruchomieniowe agentów i usługi; jeden proces na sesję lub przestrzeń roboczą | powiadomienia `nurt/event` w tym samym strumieniu | osobny proces; awaria konsoli nie zatrzymuje hosta |
| `nurt serve --socket <path>` | zarezerwowane dla transportu przez gniazdo domeny Unix lub nazwany potok; opcja jest rozpoznawana i kończy się kodem wyjścia 3 w każdej kompilacji wydania 0.1.2 | brak; połącz się przez `nurt serve --stdio` | brak | brak |
| `libnurt` (`nurt.h`) | ABI C w procesie hosta | krótkie wywołania bez skutków: `nurt_check`, `nurt_describe`, `nurt_correct`; hosty bez procesów potomnych | funkcja zwrotna `nurt_event_cb` | brak; błąd w bibliotece zatrzymuje hosta |
| `nurt mcp --stdio` | Model Context Protocol przez standardowe wejście i wyjście | klienci MCP: asystenci desktopowi i edytorowi, frameworki z obsługą MCP | powiadomienia MCP | osobny proces |
| `nurt mcp --http <address>` | Model Context Protocol, strumieniowy HTTP | serwer współdzielony przez sieć | powiadomienia MCP | osobny proces |

Podpolecenie `nurt mcp` jest obecne w kompilacjach z funkcją `mcp`; kompilacja bez niej zgłasza to podpolecenie jako niedostępne. `nurt serve` jest obecne w każdej kompilacji. Ramkowanie, metody, cykl życia, zdarzenia i kody błędów `nurt serve` są zdefiniowane w [`docs/reference/serve-protocol.md`](../reference/serve-protocol.md); ten przewodnik ich nie powtarza.

Wybór:

1. Host, który uruchamia własną pętlę agenta, używa `nurt serve --stdio`. Nadzoruje proces potomny: restartuje go po zakończeniu i kończy trwające żądania błędem wywołania. Dziennik jest na dysku, więc zrestartowana konsola odzyskuje ukończone operacje.
2. Host, który tylko analizuje programy (polityka, podświetlanie, poprawki), może wywoływać `libnurt` w procesie.
3. Host, który już obsługuje MCP, uruchamia `nurt mcp`; kod nie jest potrzebny.

Wiązania językowe (`bindings/`) opakowują `nurt serve` i `libnurt` tym samym API w Pythonie, TypeScripcie, Go, Javie i .NET.

Każdy interfejs przyjmuje kopertę żądania `nurt-request/1` i zwraca odpowiedź `nurt-response/1` z raportem skutków `nurt-report/1`. Błędy języka (nieznane polecenie, nieudana asercja) są częścią raportu; błędy wywołania (nieprawidłowy JSON, nieznana metoda, zatrzymany proces) są błędami interfejsu.

## Wywołania `nurt serve` {#calls-to-nurt-serve}

Żądanie jest komunikatem JSON-RPC 2.0 w wersji `nurt-rpc/1`; `check` przyjmuje te same parametry co `run`. Metody, parametry i wyniki są wymienione w [opisie protokołu](../reference/serve-protocol.md):

<!-- schema: rpc -->
```json
{"jsonrpc": "2.0", "id": 1, "method": "check", "params": {"nurt": "1.0", "program": "list \"/home/user/app\""}}
```

Błąd wywołania zastępuje wynik obiektem `error`; kody i ich odwzorowanie na statusy ABI C (`-32000` minus status) są zdefiniowane w opisie protokołu:

<!-- schema: rpc -->
```json
{"jsonrpc": "2.0", "id": 2, "error": {"code": -32002, "message": "the call was cancelled", "data": {"status": "NURT_E_CANCELLED"}}}
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
