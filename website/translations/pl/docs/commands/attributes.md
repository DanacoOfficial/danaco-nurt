---
command: attributes
group: core
summary: Zmienia uprawnienia, konto, czasy i atrybuty rozszerzone wpisów.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# attributes

## Składnia {#synopsis}

```text
attributes <path>... [name: value]...
entry |> attributes [name: value]...
```

## Opis {#description}

`attributes` zmienia metadane podanych wpisów albo, w postaci etapu, wpisów
wejściowych. Wymagana jest co najmniej jedna zmiana. Każdy cel musi istnieć;
brakujący cel powoduje niepowodzenie wywołania przed wprowadzeniem jakiejkolwiek
zmiany.

Etap planowania odczytuje każdy cel i oblicza jego zmianę na podstawie bieżącego
stanu. Cel, którego metadane mają już żądane wartości, jest pomijany i zgłaszany
jako `unchanged`. W postaci etapu wpis wejściowy z `etag` jest sprawdzany
względem bieżącego stanu; zmieniony wpis kończy się błędem `E_STALE`.

Dowiązanie podane jako cel jest rozwiązywane: zmiana dotyczy wpisu, na który
wskazuje dowiązanie, a dowiązanie do nieistniejącego wpisu kończy się błędem
`E_NOT_FOUND`. Przy `recursive: true` zmieniane są także wpisy poniżej katalogu;
dowiązania poniżej niego nie są ani śledzone, ani zmieniane.

Przy pustym wejściu wywołanie niczego nie zmienia i zgłasza `nothing_matched`.

| Parametr | Linux | macOS | Windows |
|---|---|---|---|
| `mode` | bity uprawnień | bity uprawnień | tylko atrybut tylko do odczytu, na podstawie uprawnienia zapisu konta; zmiana ma flagę `acl_unchanged` |
| `owner`, `group` | identyfikatory konta i grupy | identyfikatory konta i grupy | `E_UNSUPPORTED` |
| `acl` | ACL dostępu POSIX | `E_UNSUPPORTED` | `E_UNSUPPORTED` |
| `readonly` | `true` usuwa każde uprawnienie zapisu, `false` dodaje uprawnienie zapisu konta | jak w systemie Linux | atrybut tylko do odczytu |
| `hidden` | wynika z nazwy; inna wartość daje flagę `hidden_unsupported` | jak w systemie Linux | atrybut ukryty |
| `created` | nie można ustawić; flaga `created_unsupported` | czas utworzenia | czas utworzenia |
| `xattrs` | atrybuty rozszerzone | atrybuty rozszerzone | `E_UNSUPPORTED` |

Flaga wskazuje żądany element, którego system nie mógł zastosować. Pojawia się
w polu `flags` zmiany albo w szczegółach wpisu `unchanged`, gdy nic innego się
nie zmieniło.

Zmiana konta pliku wymaga uprawnień administratora; wywołanie przez inne konto
kończy się błędem `E_PRIVILEGE` przed wprowadzeniem jakiejkolwiek zmiany.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | wymagany | Wpisy do zmiany w postaci źródła. |
| `mode` | `text` | brak | Uprawnienia, na przykład "0644" lub "u+x". |
| `owner` | `text` | brak | Konto pliku. |
| `group` | `text` | brak | Grupa pliku. |
| `acl` | `list<map<any>>` | brak | Wpisy kontroli dostępu. |
| `readonly` | `bool` | brak | Atrybut tylko do odczytu. |
| `hidden` | `bool` | brak | Atrybut ukryty. |
| `modified` | `instant` | brak | Czas modyfikacji. |
| `accessed` | `instant` | brak | Czas dostępu. |
| `created` | `instant` | brak | Czas utworzenia. |
| `xattrs` | `map<text>` | brak | Atrybuty rozszerzone do ustawienia. |
| `recursive` | `bool` | `false` | Stosuje zmianę do zawartości katalogów. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `entry` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Nadanie skryptowi uprawnienia wykonywania dla konta pliku:

```nurt example
attributes "/home/user/run.sh" mode: "u+x"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/run.sh", "type": "file", "mode": "0644", "content_utf8": "content\n" }
] }
```

Ograniczenie pliku do jego konta i ustawienie czasu modyfikacji:

```nurt example
attributes "/home/user/notes.txt" mode: "0600" modified: "2026-01-31T12:00:00Z"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "content\n" }
] }
```

Usunięcie uprawnienia zapisu grupy i pozostałych w całym drzewie:

```nurt example
attributes "/home/user/site" mode: "go-w" recursive: true
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/site", "type": "dir", "mode": "0777" },
  { "path": "/home/user/site/index.html", "type": "file", "mode": "0777", "content_utf8": "content\n" },
  { "path": "/home/user/site/css/main.css", "type": "file", "content_utf8": "content\n" }
] }
```

Ustawienie plików znalezionych przez `find` jako tylko do odczytu:

```nurt example
find "/home/user/site" type: file |> attributes readonly: true
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/site/index.html", "type": "file", "content_utf8": "content\n" },
  { "path": "/home/user/site/css/main.css", "type": "file", "content_utf8": "content\n" }
] }
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | nie zażądano żadnej zmiany | podaj co najmniej jeden z parametrów `mode`, `owner`, `group`, `acl`, `readonly`, `hidden`, `modified`, `accessed`, `created`, `xattrs` |
| `E_INVALID_VALUE` | tryb, czas, konto, grupa lub wpis ACL jest niepoprawny | użyj postaci z tabeli parametrów |
| `E_TYPE` | parametr ma wartość innego typu | przekaż teksty do `xattrs` i mapy do `acl` |
| `E_NOT_FOUND` | cel lub wpis, na który wskazuje dowiązanie, nie istnieje | sprawdź ścieżkę |
| `E_STALE` | wpis wejściowy zmienił się od chwili odczytu | odczytaj wpisy ponownie |
| `E_PRIVILEGE` | zmiana konta pliku wymaga uprawnień administratora | uruchom jako administrator albo pomiń `owner` |
| `E_PERMISSION` | system odmówił zmiany | sprawdź konto pliku i uprawnienia jego katalogu |
| `E_UNSUPPORTED` | system lub wolumin nie ma postaci dla żądanego elementu | zobacz tabelę w części Uwagi |

## Powiązane {#related}

`info`, `create`, `copy`, `undo`.
