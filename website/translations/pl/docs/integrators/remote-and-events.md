<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/integrators/remote-and-events.md -->
<!-- Responsibility:   Documents Remote instances and events. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Zdalne instancje i zdarzenia {#remote-instances-and-events}

## Drzewa składni dla innego urządzenia {#syntax-trees-for-another-device}

Host, który uruchamia programy na innej maszynie (serwerze, kontenerze, urządzeniu), parsuje program lokalnie i wysyła drzewo składni:

1. `parse` na lokalnej konsoli zwraca po poprawkach drzewo `nurt-ast/1`.
2. Host stosuje swoją politykę przez `check` dla tego samego programu.
3. Host wysyła żądanie z `ast` do zdalnej instancji własnym transportem (SSH, gniazdo, kolejka komunikatów) do działającego tam `nurt serve`:

<!-- schema: request -->
```json
{
  "nurt": "1.0",
  "ast": {
    "nurt": "1.0",
    "program": {
      "statements": [
        {
          "t": "pipeline",
          "head": {
            "t": "stage",
            "name": "list",
            "args": [{"value": {"t": "text", "v": "/srv/app"}}]
          },
          "stages": []
        }
      ]
    }
  },
  "options": {"base": "/srv/app", "session": "s-17"}
}
```

Zdalna instancja nie poprawia drzewa ponownie, więc uruchamiany jest program sprawdzony przez politykę. Drzewo w nowszej wersji, niż instancja obsługuje, jest odrzucane z `E_AST_VERSION`, który wymienia nieznane węzły. Połączenie, uwierzytelnianie i wybór urządzenia należą do hosta.

## Zdarzenia {#events}

`nurt serve` wysyła powiadomienia `nurt/event` bez `id`; parametrami jest dokument `nurt-event/1`, który `libnurt` przekazuje także do `nurt_event_cb`. Każde zdarzenie ma `t` (typ), `seq` (od 1, w obrębie wykonania), `time` oraz, gdy żądanie ma identyfikator, `request`.

| Typ | Zawartość |
|---|---|
| `exec.started` | `ast_hash`, `effect` |
| `exec.finished` | `status`, `error` |
| `progress` | `command`, `phase`, `done`, `total`, `bytes`, `message` |
| `job.started` | `id`, `label`, `command` (lista łańcuchów), `cwd`, `pid`, `started` |
| `job.output` | `id`, `stream` (`stdout`, `stderr`), `cursor`, `lines`, `bytes` |
| `job.progress` | `id`, `percent`, `line` |
| `job.finished` | `id`, `state` (`exited`, `failed`, `stopped`, `timed_out`), `exit_code`, `signal`, `elapsed`, `output_file`, `changes` |
| `log` | `level`, `message`, `source` |

Powiadomienie w strumieniu `nurt serve`:

<!-- schema: rpc -->
```json
{
  "jsonrpc": "2.0",
  "method": "nurt/event",
  "params": {
    "seq": 3,
    "time": "2026-11-22T10:15:02Z",
    "request": "7",
    "t": "job.output",
    "id": "j-4",
    "stream": "stdout",
    "cursor": "o-120",
    "lines": ["compiling", "linking"],
    "bytes": 17
  }
}
```

To samo zdarzenie jako parametr `nurt_event_cb`:

<!-- schema: event -->
```json
{"seq": 4, "time": "2026-11-22T10:15:09Z", "request": "7", "t": "exec.finished", "status": "ok", "error": null}
```

Program uruchomiony przez `run ... background: true` od razu zwraca rekord `job`. Host śledzi zadanie przez zdarzenia lub odczytuje jego wyjście przyrostowo przez `jobs <id> since: <cursor>`; `stop`, `retry` i `remove` działają na zadaniu. Subskrypcje, filtrowanie wierszy wyjścia i dostarczanie zdarzeń do kontekstu modelu należą do hosta.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
