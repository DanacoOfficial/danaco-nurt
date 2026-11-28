---
command: verify
group: core
summary: Sprawdza zmianę potokiem i wycofuje zmianę, gdy sprawdzenie zawodzi.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# verify

## Składnia {#synopsis}

```text
effect |> verify <check>
```

## Opis {#description}

`verify` uruchamia potok `check` po skutku programu, z rekordami `effect` jako
jego wejściem, i odczytuje całe jego wyjście. Gdy sprawdzenie się powiedzie,
rekordy `effect` przechodzą dalej bez zmian. Gdy którykolwiek etap
sprawdzenia zawiedzie, na przykład `assert` wewnątrz niego, zmiana jest
wycofywana z dziennika, a program kończy się błędem `E_ROLLED_BACK`, który
zawiera błąd sprawdzenia; gdy wycofanie nie może przywrócić każdego pliku,
błędem jest `E_ROLLBACK_FAILED` z listą plików, których dotyczy.

Sprawdzenie zwykle uruchamia budowanie lub test (`run … |> assert exit_code: 0`)
albo ponownie odczytuje zmienione pliki. Jest wykonywane na etapie `6`, po
etapach, które odczytują rekordy skutku.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `check` | `pipeline` | wymagany | Potok, który sprawdza wynik zmiany. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `effect` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `check` |
| Skutek | `read` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Nieudane sprawdzenie wycofuje poprzedzającą zmianę z dziennika. |

## Przykłady {#examples}

Zastąpienie wywołania i zachowanie zmiany tylko wtedy, gdy projekt nadal się buduje:

```nurt example
search /oldName\(/ "/home/user/project/src" |> replace "newName(" |> verify (run ["go", "build", "./..."] cwd: "/home/user/project" |> assert exit_code: 0)
```

Wycofanie zapisu, gdy plik nie zawiera już wymaganego klucza:

```nurt example
write "/home/user/project/config.toml" text: "name = 1" mode: overwrite |> verify (lines "/home/user/project/config.toml" |> assert contains: "version")
```

Zachowanie zmiany wersji tylko wtedy, gdy testy przechodzą:

```nurt example
write "/home/user/project/VERSION" text: "1.4.0" mode: overwrite |> verify (run ["make", "test"] cwd: "/home/user/project" |> assert exit_code: 0)
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_ROLLED_BACK` | sprawdzenie zawiodło, a zmiana została wycofana; przyczyna zawiera błąd sprawdzenia | popraw zmianę lub sprawdzenie i uruchom program ponownie |
| `E_ROLLBACK_FAILED` | wycofanie nie mogło przywrócić każdego pliku; pliki są wymienione | przywróć wymienione pliki za pomocą `undo` lub ręcznie |
| `E_MISSING_PARAM` | brak potoku sprawdzenia | zapisz potok w nawiasach po `verify` |
| `E_TYPE` | `check` nie jest potokiem | ujmij potok sprawdzenia w nawiasy |

## Powiązane {#related}

`assert`, `undo`, `history`.
