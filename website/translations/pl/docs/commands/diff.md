---
command: diff
group: text
summary: Porównuje dwa pliki albo wejście z plikiem lub nazwanym wynikiem i zwraca różnice w postaci fragmentów (hunk).
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# diff

## Składnia {#synopsis}

```text
diff <from> <to> [name: value]...
entry|content |> diff <against> [name: value]...
```

## Opis {#description}

`diff` porównuje teksty wiersz po wierszu i zwraca jeden rekord `hunk` na
każdy obszar zmiany. Dla tych samych tekstów i tej samej liczby wierszy
kontekstu fragmenty są takie same jak w GNU `diff -u`.

W postaci źródła `from` jest stroną starą, a `to` stroną nową. Dwa pliki dają
fragmenty swojej różnicy. Dwa katalogi są porównywane rekurencyjnie według
ścieżki względnej, w kolejności bajtowej ścieżek: plik obecny tylko w `to` jest
`added`, plik obecny tylko w `from` jest `deleted`, a usunięty i dodany plik
o tej samej niepustej zawartości są zgłaszane jeden raz jako `renamed`.
Dowiązania do zwykłych plików są porównywane jak pliki; dowiązania do katalogów
nie są śledzone. Plik porównywany z katalogiem jest porównywany z plikiem o tej
samej nazwie w tym katalogu.

W postaci etapu każdy rekord wejściowy jest stroną nową, a `against` stroną
starą. Rekord `entry` wskazuje plik; rekord `content` zawiera sam tekst, więc
tekst odczytany lub zmieniony wcześniej w potoku można porównać bez zapisywania
go. `against` jest plikiem, katalogiem (każdy plik wejściowy jest porównywany
z plikiem o tej samej nazwie w nim, a katalog wejściowy z całym drzewem) albo
nazwanym wynikiem, którego rekordy są dopasowywane do wejścia według `path`;
nazwany wynik zawierający jeden rekord jest porównywany z każdym wejściem.
Wejście bez odpowiednika jest `added`. Wpisy, które nie są ani plikami, ani
katalogami, są pomijane.

Pliki są dekodowane przed porównaniem, więc pliki w różnych kodowaniach są
porównywane według swoich znaków, a różnica kodowań jest zgłaszana jako
`encoding_changed`. Znak powrotu karetki należy do wiersza, więc wiersze
z zakończeniami `crlf` i `lf` różnią się, chyba że ustawiono
`ignore_whitespace`. Pliki o zawartości binarnej są porównywane według skrótu
zawartości; różnica daje jeden rekord z `binary: true` i bez wierszy.

Równe wejścia nie dają żadnych rekordów.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `from` | `path` | wymagany | Stary plik w postaci źródła. |
| `to` | `path` | wymagany | Nowy plik w postaci źródła. |
| `against` | `path \| stream<any>` | brak | Plik lub nazwany wynik, z którym wejście jest porównywane w postaci etapu. |
| `context` | `int` | `3` | Liczba wierszy kontekstu wokół każdej zmiany. |
| `ignore_whitespace` | `bool` | `false` | Ignoruje zmiany w białych znakach. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `hunk` |
| `stage` | `entry`, `content` | `hunk` |

Pola rekordu `hunk`:

Dodany lub usunięty pusty plik oraz plik ze zmienioną nazwą dają jeden rekord
bez wierszy.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Porównanie dwóch wersji pliku konfiguracyjnego.

```nurt example
diff "/srv/app/config.old.toml" "/srv/app/config.toml"
```

```text expected
--- /srv/app/config.old.toml
+++ /srv/app/config.toml
@@ -1,3 +1,3 @@
 [server]
-timeout = 30
+timeout = 60
 port = 8080
```

Porównanie dwóch drzew wydań; przeniesiony plik jest zgłaszany jeden raz jako zmieniona nazwa.

```nurt example
diff "/srv/release/1.0" "/srv/release/1.1"
```

```text expected
added     /srv/release/1.1/CHANGELOG.md
deleted   /srv/release/1.0/NOTES.txt
modified  /srv/release/1.1/src/main.rs
renamed   /srv/release/1.0/src/util.rs -> /srv/release/1.1/src/run.rs
```

Porównanie wymienionych źródeł z plikami o tej samej nazwie w kopii, z jednym
wierszem kontekstu.

```nurt example
find "/srv/app/src" glob: "*.rs" type: file |> diff against: "/srv/app.orig/src" context: 1
```

Porównanie dwóch skryptów bez względu na wcięcia i zakończenia wierszy.

```nurt example
diff "/srv/app/build.cmd" "/srv/app/build.new.cmd" ignore_whitespace: true
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera
pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego
uruchamiania polecenia.

```text
matched 140  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Brakuje `from` lub `to` w postaci źródła albo `against` w postaci etapu. | Podaj obie strony. |
| `E_PARAM_CONFLICT` | `against` podano w postaci źródła albo `from` lub `to` w postaci etapu. | Użyj parametrów właściwych dla postaci. |
| `E_INVALID_VALUE` | `context` jest ujemne albo ścieżka nie jest ani zwykłym plikiem, ani katalogiem (`problem: not_a_file`). | Podaj zero lub więcej wierszy oraz pliki lub katalogi. |
| `E_TYPE` | `against` nie jest ani ścieżką, ani nazwanym wynikiem rekordów `entry` lub `content`, albo rekord `content` nie ma `text`. | Przekaż ścieżkę lub wynik `read`. |
| `E_LIMIT` | Plik jest większy niż limit plików tekstowych. | Porównuj mniejsze pliki. |
| `E_ENCODING` | Plik jest niepoprawny w wykrytym kodowaniu. | Skonwertuj plik za pomocą `convert`. |
| `E_NOT_FOUND` | Pliku nie można odczytać. | Sprawdź ścieżkę i jej uprawnienia. |
| `E_PERMISSION` | Pliku nie można odczytać. | Sprawdź ścieżkę i jej uprawnienia. |
| `E_IO` | Pliku nie można odczytać. | Sprawdź ścieżkę i jej uprawnienia. |

## Powiązane {#related}

`patch` stosuje różnicę w formacie unified lub git. `read ... since:` zwraca
różnicę pliku względem wcześniej odczytanej wersji. `hash` porównuje pliki
według zawartości bez odczytywania ich wierszy.
