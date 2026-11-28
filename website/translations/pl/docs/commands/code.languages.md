---
command: code.languages
group: code
summary: Wyświetla obsługiwane języki wraz z rozszerzeniami plików, wersjami gramatyk i stanem serwera języka.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.languages

## Składnia {#synopsis}

```text
code.languages
```

## Opis {#description}

Wyświetla jeden rekord `language` dla każdego języka z wbudowaną gramatyką,
w kolejności rejestru gramatyk, a następnie języki, które mają wyłącznie
skonfigurowany serwer języka, w kolejności alfabetycznej.

Pola serwera pochodzą z puli serwerów języka. Polecenie nie uruchamia serwerów
i nie czeka na żądania w toku, więc odpowiada, gdy inne polecenia korzystają
z serwerów. Serwer jest zgłaszany jako znaleziony, gdy jego program znajduje
się w `PATH`, a `code.servers_allow` na niego zezwala. Bez puli serwerów pole
`server` podaje skonfigurowany program, a `server_found` ma wartość `false`.

## Parametry {#parameters}

Polecenie nie ma parametrów.

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `language` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie każdego języka.

```nurt example
code.languages
```

Wyświetlenie języków, których serwer jest zainstalowany.

```nurt example
code.languages |> filter server_found
```

Pokazanie działających serwerów wraz z ich katalogami głównymi.

```nurt example
code.languages |> filter server_running |> select language server server_pid server_root
```

## Błędy {#errors}

Polecenie nie zgłasza własnych błędów.

## Powiązane {#related}

`code.diagnostics`, `code.rename`, klucze konfiguracji `code.servers`,
`code.servers_allow`, `code.max_servers`.
