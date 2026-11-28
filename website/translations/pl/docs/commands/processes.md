---
command: processes
group: system
summary: Wyświetla uruchomione procesy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# processes

## Składnia {#synopsis}

```text
processes [<name>] [name: value]...
```

## Opis {#description}

`processes` wyświetla procesy hosta jako rekordy `process`, uporządkowane według `pid`. `name`, `user` i `pid` wybierają procesy; wszystkie podane warunki muszą być spełnione. Tekstowe `name` odpowiada całej nazwie procesu, a w systemie Windows także nazwie z dopiskiem `.exe`; wyrażenie regularne odpowiada dowolnej części nazwy. `user` odpowiada nazwie konta lub identyfikatorowi konta (`uid`).

Z `tree: true` wybrane procesy są korzeniami drzewa. Wynik zawiera każdy korzeń, a po nim wszystkich jego potomków w kolejności w głąb, z potomkami uporządkowanymi według `pid`, wraz z polami `depth` (0 dla korzenia) i `children`. Wybrany proces będący potomkiem innego wybranego procesu pojawia się raz, pod swoim przodkiem. Bez warunków korzeniami są procesy, których proces nadrzędny nie występuje w tabeli. Powiązania z procesem nadrzędnym tworzące cykl są przerywane przy powtórzonym procesie i zgłaszane jako ostrzeżenie.

`cpu` to użycie procesora w procentach jednego rdzenia. Gdy reszta potoku nazywa używane pola i wśród nich jest `cpu`, polecenie odczytuje czas procesora dwukrotnie, w odstępie 200 ms, i zgłasza użycie w tym przedziale. W przeciwnym razie `cpu` jest średnią z całego czasu życia procesu, obliczoną z jednego odczytu bez oczekiwania. Proces kończący się między dwoma odczytami jest pomijany; proces uruchomiony między nimi nie ma `cpu`.

`memory` to pamięć rezydentna. `uid` to efektywny identyfikator użytkownika w systemach Linux i macOS oraz identyfikator zabezpieczeń tokenu procesu w systemie Windows. `cwd` jest odczytywane tylko z `cwd: true`. Bez warunków wejściowych polecenie czyta tabelę leniwie, więc następujący po nim limit zatrzymuje odczyt wcześniej.

| Dane | Linux | macOS | Windows |
|---|---|---|---|
| tabela procesów | `/proc/<pid>/stat`, `status` | `proc_listpids`, `proc_pidinfo` | Toolhelp snapshot |
| `command` | `/proc/<pid>/cmdline` | `sysctl(KERN_PROCARGS2)` | blok środowiska procesu |
| `executable` | `/proc/<pid>/exe` | `proc_pidpath` | `QueryFullProcessImageNameW` |
| `cwd` | `/proc/<pid>/cwd` | `PROC_PIDVNODEPATHINFO` | blok środowiska procesu |
| `state` | litera stanu z `stat` | `pbi_status` | zawsze `running` |

Pola procesów innych kont, których konto nie może odczytać, takie jak `executable`, `command` i `cwd`, pozostają puste. Raport zawiera jedną notatkę `source_unavailable` z liczbą takich procesów. W systemie Linux wątek jądra ma pusty `command`, a jego nazwa jest w nawiasach kwadratowych, na przykład `[kthreadd]`. Stan bez odpowiednika to `other`, a `os_raw.state` zawiera kod systemu.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text \| regex` | none | Nazwa procesu lub wzorzec. |
| `user` | `text` | none | Tylko procesy tego konta. |
| `pid` | `int` | none | Tylko proces o tym identyfikatorze. |
| `cwd` | `bool` | `false` | Dołącza katalog roboczy każdego procesu. |
| `tree` | `bool` | `false` | Zwraca procesy jako drzewo z listami potomków i głębokością. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `process` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie procesów o nazwie `nginx` wraz z ich procesami potomnymi:

```nurt example
processes name: "nginx" tree: true
```

Wyświetlenie procesów jednego konta wraz z ich katalogami roboczymi:

```nurt example
processes user: "www-data" cwd: true
```

Wyświetlenie jednego procesu według identyfikatora:

```nurt example
processes pid: 1
```

Wybór procesów według wzorca nazwy:

```nurt example
processes /^python[0-9.]*$/
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego uruchamiania polecenia.

```text
matched 410  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | `name` nie jest poprawnym wyrażeniem regularnym lub `pid` jest ujemny. | Popraw wzorzec lub identyfikator. |
| `E_PRIVILEGE` | Tabela procesów nie jest odczytywalna dla konta. | Uruchom z uprawnieniem wskazanym w `required`. |
| `E_IO` | Nie można odczytać tabeli procesów, na przykład `/proc` nie jest zamontowany. | Zamontuj `proc` w `/proc`. |
| `E_CANCELLED` | Wykonanie zostało anulowane podczas pomiaru `cpu`. | Uruchom program ponownie. |

## Powiązane {#related}

`ports`, `open_files`, `suspend`, `resume`, `stop`.
