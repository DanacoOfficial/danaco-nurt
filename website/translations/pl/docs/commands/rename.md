---
command: rename
group: core
summary: Zmienia nazwy wpisów według wzorca lub na nową nazwę.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# rename

## Składnia {#synopsis}

```text
rename [<pattern>] [<with>] path: <path> (with: <text> | to: <text>) [name: value]...
entry |> rename [<pattern>] [<with>] (with: <text> | to: <text>) [name: value]...
```

## Opis {#description}

`rename` zmienia nazwę wpisów w obrębie ich katalogu; `move` zmienia katalog. Nowa nazwa jest podawana przez `to`, obliczane dla każdego rekordu wejściowego, lub wyprowadzana ze starej nazwy: tekstowy `pattern` zastępuje każde wystąpienie tekstu wartością `with`, a `pattern` w postaci wyrażenia regularnego zastępuje każde dopasowanie szablonem `with`, w którym `$1`, `${name}` i `$$` oznaczają odpowiednio grupę numerowaną, grupę nazwaną i znak dolara. `to` i `with` wzajemnie się wykluczają.

Nowa nazwa jest pojedynczą nazwą: nazwa zawierająca `/`, `\`, nazwa pusta, `.` i `..` są odrzucane. Wpis, którego nowa nazwa jest równa starej, jest liczony jako `unchanged`.

`on_conflict` rozstrzyga, co dzieje się, gdy wpis o nowej nazwie istnieje: `error` kończy się błędem przed jakąkolwiek zmianą, `skip` zachowuje oba wpisy, `overwrite` zastępuje istniejący plik lub dowiązanie. Istniejący katalog nigdy nie jest zastępowany. Gdy nowa nazwa jest starą nazwą innego wpisu zmienianego w tym samym wywołaniu, ten wpis jest zmieniany najpierw; zmiany nazw tworzące cykl są odrzucane.

Na woluminie, który nie rozróżnia wielkości liter, zmiana nazwy zmieniająca tylko wielkość liter zmienia nazwę samego wpisu, a nowa nazwa różniąca się od innej istniejącej nazwy tylko wielkością liter jest konfliktem zgłaszanym jako `case_collision`.

`check_references: true` zgłasza wystąpienia całych słów starych nazw w innych plikach jako `references_left`, według tych samych reguł co `remove`.

Rekord wpisu z `etag` musi nadal mieć ten znacznik jednostki; zmieniony wpis kończy się błędem `E_STALE` przed jakąkolwiek zmianą. Gdy nic nie zostanie przemianowane, wynik zgłasza `nothing_matched`.

| Godny uwagi kod | Znaczenie |
|---|---|
| `case_collision` | nowa nazwa różni się od istniejącej nazwy tylko wielkością liter na woluminie, który nie rozróżnia wielkości liter |
| `references_left` | inne pliki nadal wymieniają zmieniony wpis pod jego starą nazwą |
| `nothing_matched` | nic nie zostało przemianowane |

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Wpis do zmiany nazwy w postaci źródłowej. |
| `pattern` | `text \| regex` | none | Część nazwy do zastąpienia; używana z with. |
| `with` | `text` | jeden z `replacement` | Zastępnik wzorca w nazwie. |
| `to` | `text` | jeden z `replacement` | Nowa nazwa. |
| `on_conflict` | `enum(overwrite\|skip\|error)` | `error` | Zachowanie, gdy wpis o nowej nazwie istnieje. |
| `check_references` | `bool` | `false` | Zgłasza odwołania do starych nazw pozostałe po zmianie. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `entry` | `effect` |

Rekord `effect` ma jedną zmianę `change` z akcją `rename` dla każdego przemianowanego wpisu, z `path` ustawionym na starą ścieżkę i `to` na nową ścieżkę, w kolejności wykonywania zmian nazw. Jego podsumowanie `summary` zlicza `renamed`, `overwritten`, `skipped` i `unchanged`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zmiana nazwy pliku:

```nurt example
rename path: "/home/user/draft.md" to: "final.md"
```

```text expected
rename: 1 target, renamed 1
```

Zastąpienie rozszerzenia w każdej wyświetlonej nazwie:

```nurt example
find "/home/user/photos" glob: "*.jpeg" |> rename ".jpeg" ".jpg"
```

Przepisanie dat w nazwach za pomocą szablonu wyrażenia regularnego:

```nurt example
list "/home/user/logs" |> rename /(\d{4})-(\d{2})-(\d{2})/ "$3.$2.$1"
```

Zmiana tylko wielkości liter w nazwie:

```nurt example
rename path: "/Volumes/data/readme.md" to: "README.md"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | brak nowej nazwy: ani `to`, ani `pattern` z `with` | podaj `to` albo `pattern` i `with` |
| `E_PARAM_CONFLICT` | jednocześnie `to` i `with` albo `pattern` z `to` | podaj jedną postać nowej nazwy |
| `E_INVALID_VALUE` | nowa nazwa zawiera katalog, jest pusta, `.` lub `..` albo `pattern` jest pusty | użyj `move`, aby zmienić katalog |
| `E_EXISTS` | wpis o nowej nazwie istnieje przy `on_conflict: error`, jest katalogiem albo zmiany nazw tworzą cykl | wybierz inną nazwę lub wartość `on_conflict` |
| `E_REGEX` | wzorzec wyrażenia regularnego jest niepoprawny albo szablon wskazuje grupę, której wzorzec nie ma | popraw wzorzec lub szablon |
| `E_NOT_FOUND` | wpis nie istnieje | sprawdź ścieżkę |
| `E_STALE` | wpis wejściowy zmienił się od odczytu | odczytaj wpisy ponownie |
| `E_IN_USE` | Windows: wpis jest otwarty w innym procesie; błąd wyświetla procesy | zamknij wpis w tym procesie |

## Powiązane {#related}

`move`, `copy`, `remove`, `replace`, `list`, `find`.
