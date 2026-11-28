---
command: code.outline
group: code
summary: Wyświetla symbole pliku w postaci zarysu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.outline

## Składnia {#synopsis}

```text
code.outline <path> [name: value]...
```

## Opis {#description}

Wyświetla symbole jednego pliku źródłowego w kolejności występowania w źródle,
jeden rekord `symbol` na definicję; kontener występuje przed symbolami, które
zawiera. Pole `container` podaje nazwę kwalifikowaną symbolu nadrzędnego, więc
`depth` 0 wybiera symbole najwyższego poziomu, 1 dodaje ich składowe i tak
dalej.

Plik w języku z wbudowaną gramatyką jest zarysowywany analizą składniową.
Język jest wykrywany, w tej kolejności, na podstawie modeline, nazwy pliku,
rozszerzenia i wiersza shebang. Plik w innym języku jest zarysowywany przez
jego serwer języka, jeżeli jest dostępny.

Zmienne lokalne zadeklarowane wewnątrz funkcji nie wchodzą do zarysu.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik źródłowy. |
| `depth` | `int` | bez limitu | Maksymalna głębokość zagnieżdżenia symboli. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `symbol` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zarys pliku.

```nurt example
code.outline "/home/user/app/src/server.py"
```

Wyświetlenie wyłącznie symboli najwyższego poziomu.

```nurt example
code.outline "/home/user/app/src/lib.rs" depth: 0
```

Wyświetlenie metod jednej klasy.

```nurt example
code.outline "/home/user/app/src/Cart.java" |> filter container == "Cart" and kind == "method"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano `path`. | Podaj `path`. |
| `E_UNSUPPORTED_LANGUAGE` | Plik nie ma wbudowanej gramatyki i nie jest dostępny żaden serwer języka. | Zainstaluj serwer dla tego języka. |
| `E_INVALID_VALUE` | `depth` jest ujemne. | Podaj głębokość 0 lub większą. |
| `E_LIMIT` | Plik jest większy niż `code.file_bytes_max`. | Zwiększ limit. |
| `E_NOT_FOUND` | Plik nie istnieje. | Sprawdź ścieżkę. |
| `E_EXTERNAL_FAILED` | Serwer języka dla języka bez gramatyki zakończył się awarią. | Zobacz komunikat serwera. |
| `E_TIMEOUT` | Serwer języka dla języka bez gramatyki nie odpowiedział. | Zwiększ `code.timeout`. |

## Powiązane {#related}

`code.symbols`, `read view: outline`, `code.languages`.
