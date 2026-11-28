---
command: set
group: documents
summary: Ustawia wartości pod ścieżkami kluczy w dokumencie JSON, YAML, TOML, XML, INI, CSV, properties, env lub plist, zachowując jego układ.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# set

## Składnia {#synopsis}

```text
set <path> [name: value]...
item |> set <value> [name: value]...
```

## Opis {#description}

`set` zmienia wartości pod ścieżkami kluczy dokumentu strukturalnego i
zachowuje każdy bajt poza zmienianymi wartościami: wcięcia, cudzysłowy,
komentarze, kolejność kluczy, końce wierszy, kodowanie i znacznik kolejności
bajtów. Wartość jest zapisywana w stylu sąsiednich wartości; nowy klucz jest
dodawany na końcu swojej mapy.

W formie źródłowej dokumentem jest plik `path`. `key` z `value` ustawia jedną
ścieżkę klucza; `values` ustawia kilka ścieżek kluczy, w kolejności mapy.
Wszystkie ścieżki kluczy jednego wywołania są zmieniane w jednej transakcji:
gdy którakolwiek z nich zawiedzie, nic nie jest zapisywane. Dwie ścieżki
kluczy jednego pliku, które są równe lub zagnieżdżone (`a` i `a.b`), kończą
się błędem `E_KEY_CONFLICT`, ponieważ druga zmiana dotyczyłaby wyniku
pierwszej; dopisania (`list[+]`) nigdy nie powodują konfliktu.

W formie etapu, po `read … key:` lub `query`, każdy rekord wejściowy `item`
wskazuje plik, ścieżkę klucza i znacznik encji pliku. Wartość każdego
elementu jest ustawiana na `value`, które może być wyrażeniem elementu.
Elementy jednego pliku są zmieniane razem, w kolejności wejścia. Elementy są
przypisywane do pliku według tożsamości pliku (urządzenie i numer pliku), więc
dwie nazwy jednego pliku, takie jak dowiązanie symboliczne lub ścieżka przez
katalog z dowiązaniem, oznaczają jeden plik; plik, który jeszcze nie istnieje,
jest identyfikowany przez swoją znormalizowaną ścieżkę.

Ścieżki kluczy używają składni typu `key`: `a.b`, `list[0]`, `list[-1]`,
`list[+]` do dopisania, `list[*]` dla każdego elementu oraz `#1:` do wyboru
dokumentu strumienia YAML z wieloma dokumentami. Indeks równy długości listy
dopisuje element; większy indeks kończy się błędem `E_KEY_CONFLICT`. Ścieżka
klucza pasująca do kilku węzłów zmienia je wszystkie, a `replacements` je
zlicza. Niejawne powtórzenie, takie jak powtórzony element XML lub zduplikowany
klucz, bez `[*]` kończy się błędem `E_KEY_CONFLICT` i wylicza kandydatów
`[0]`, `[1]`, ….

Przy `create: true` brakujące klucze i pośrednie mapy są tworzone, a zmiana
zgłasza zdarzenie godne uwagi `keys_created`. Brakujący plik jest tworzony,
gdy każda zmiana wywołania ustawia wartość; nowy dokument zaczyna się jako
pusty albo z najmniejszą mapą główną swojego formatu (`{}` dla JSON, pusty
`<dict>` dla list właściwości, pierwszy klucz jako element główny dla XML).
Przy `create: false` brakujący klucz kończy się błędem `E_KEY_NOT_FOUND`,
który wylicza najbliższe istniejące klucze jako kandydatów.

Każda zmiana przechodzi dwie kontrole formatu: bajty poza zmienianymi węzłami
muszą pozostać takie same, a zmieniony dokument musi dać się odczytać jako
oryginalna wartość z naniesioną zmianą. Konstrukcja, której nie można zmienić
bez utraty jej układu, kończy się błędem `E_UNSUPPORTED_CONSTRUCT` z nazwą
konstrukcji; plik nie jest zmieniany.

`validate` porównuje problemy składniowe dokumentu przed zmianą i po niej.
Liczy się tylko błąd, który wprowadza zmiana; błędy obecne przed zmianą są
pomijane. Przy `reject` taki błąd powoduje niepowodzenie wywołania, przy
`warn` zmiana jest zapisywana, ostrzeżenie trafia do dziennika, a zmiana
otrzymuje flagę `validate_warning`, przy `off` kontrola jest pomijana.

Wywołanie, które pozostawia dokument bez zmian, na przykład wartość równa
bieżącej, niczego nie zapisuje i zgłasza zdarzenie godne uwagi `unchanged`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Dokument do zmiany w formie źródłowej. |
| `key` | `key` | brak | Ścieżka klucza do ustawienia, taka jak "scripts.test" lub "deps[+]" do dopisania do listy. |
| `value` | `any` | brak | Nowa wartość pod ścieżką klucza. |
| `values` | `map<any>` | brak | Kilka ścieżek kluczy i ich wartości, ustawianych w jednej transakcji. |
| `create` | `bool` | `true` | Tworzy brakujące klucze i tabele pośrednie. |
| `format` | `enum(auto\|json\|jsonc\|yaml\|toml\|xml\|ini\|csv\|tsv\|properties\|env\|plist)` | `auto` | Format dokumentu; auto wybiera go na podstawie rozszerzenia pliku. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Wycofuje zmianę, gdy wprowadza nowy błąd składni. |
| `if_match` | `text` | brak | Oczekiwany etag dokumentu; inny dokument nie jest zmieniany. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `item` | `effect` |

Rekord `effect` wylicza jedną zmianę na każdy zapisany plik z polami `action`
(`modify` lub `create`), `size_before`, `size_after`, `hash_before`,
`replacements` i `flags` oraz zdarzenia godne uwagi `keys_created`,
`alias_shared`, `merge_override`, `duplicate_key` i `unchanged`, zgłaszane
przez formaty.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

Dziennik operacji zachowuje każdy plik docelowy przed pierwszym zapisem.
Każdy plik jest zapisywany ze znacznikiem encji odczytanym podczas planowania
jako oczekiwanym stanem; plik zmieniony w międzyczasie kończy się błędem
`E_STALE` i żaden plik nie jest zapisywany. `undo` przywraca każdy zmieniony
plik bajt w bajt i usuwa pliki utworzone przez wywołanie.

## Przykłady {#examples}

Ustawienie jednego klucza pliku JSON:

```nurt example
set "/home/user/app/package.json" key: "scripts.test" value: "vitest run"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/package.json", "type": "file", "content_utf8": "{\n  \"name\": \"app\",\n  \"scripts\": {\n    \"test\": \"jest\"\n  }\n}\n" }
] }
```

```text expected
1 file changed
```

Ustawienie dwóch kluczy pliku YAML w jednej transakcji:

```nurt example
set "/home/user/app/compose.yaml" values: {"services.web.image": "nginx:1.27", "services.web.ports[+]": "8443:443"}
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/compose.yaml", "type": "file", "content_utf8": "services:\n  web:\n    image: nginx:1.25\n    ports:\n      - \"8080:80\"\n" }
] }
```

Zmiana wartości odczytanej przez `read … key:`:

```nurt example
read "/home/user/app/config.toml" key: "server.port" |> set 8081
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/config.toml", "type": "file", "content_utf8": "# server settings\n[server]\nport = 8080 # public port\n" }
] }
```

Utworzenie brakującego pliku z zagnieżdżonym kluczem:

```nurt example
set "/home/user/app/settings.json" key: "editor.tabSize" value: 2
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_KEY_NOT_FOUND` | ścieżka klucza nie istnieje, a `create` ma wartość `false`; `candidates` wylicza najbliższe klucze | popraw ścieżkę klucza lub ustaw `create: true` |
| `E_KEY_CONFLICT` | dwie ścieżki kluczy jednego pliku się pokrywają, indeks wykracza poza koniec listy albo ścieżka klucza pasuje do powtórzenia bez `[*]` | zmień pokrywające się klucze w osobnych wywołaniach, użyj `[+]` albo wybierz jednego kandydata |
| `E_STALE` | dokument różni się od `if_match` lub od znacznika encji elementu albo zmienił się między planowaniem a zapisem | odczytaj dokument ponownie i powtórz zmianę |
| `E_DOC_PARSE` | dokumentu nie można odczytać albo zmiana wprowadza błąd składni przy `validate: reject` (`problem`: `validate`) | popraw dokument lub wartość albo użyj `validate: warn` |
| `E_UNSUPPORTED_CONSTRUCT` | zmiana nie może zachować układu konstrukcji; `found` wskazuje konstrukcję | zmień konstrukcję ręcznie albo użyj `replace` |
| `E_NOT_FOUND` | plik nie istnieje, a `create` ma wartość `false`, albo usunięcie wskazuje brakujący plik | sprawdź ścieżkę lub ustaw `create: true` |
| `E_MISSING_PARAM` | nie podano ani `key` z `value`, ani `values` | dodaj `key` i `value` albo `values` |
| `E_TYPE` | `values` nie jest mapą albo wejście nie składa się z rekordów `item` | przekaż mapę ścieżek kluczy na wartości |
| `E_INVALID_VALUE` | `if_match` nie jest znacznikiem encji albo `validate` ma inną wartość | przekaż pole `etag` rekordu |
| `E_UNKNOWN_VALUE` | `format` wskazuje format, który nie jest dostępny | użyj jednego z wymienionych formatów |
| `E_LIMIT` | plik jest większy niż limit rozmiaru dokumentu | zwiększ `limits.max_file_bytes_text` |

## Powiązane {#related}

`read` z `key:` do odczytu wartości i ich znaczników encji, `query` do wyboru
wartości filtrami `jq`, `remove` do usuwania kluczy rekordów `item`, `undo` do
cofnięcia zmiany.
