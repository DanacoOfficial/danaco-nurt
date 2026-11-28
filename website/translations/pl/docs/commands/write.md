---
command: write
group: core
summary: Zapisuje tekst, bajty lub rekordy do pliku.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# write

## Składnia {#synopsis}

```text
write [<path>] [<text>] mode: <enum(create|overwrite|append|prepend)> (path: <path> | files: <list<map<any>>>) [name: value]...
```

## Opis {#description}

`write` zapisuje jeden plik albo partię plików podanych w `files` w jednej
transakcji: zapisane są wszystkie pliki albo żaden. Zawartością jest `text`,
zawartość wskazana przez `from`, bajty `data` lub rekordy wejściowe. `size`
obcina lub rozszerza plik bez zawartości; `data` z `offset` zapisuje bajty w
miejscu.

`mode` nie ma wartości domyślnej. `create` kończy się błędem `E_EXISTS`, gdy
plik istnieje; `overwrite` zastępuje zawartość; `append` dodaje tekst na
końcu; `prepend` dodaje go na początku. Każdy tryb tworzy brakujący plik.

Nowy plik jest w UTF-8 bez znacznika kolejności bajtów, z końcami wierszy
według klucza konfiguracji `text.new_file_line_ending` (domyślnie `lf`).
Istniejący plik zachowuje swoje kodowanie, znacznik kolejności bajtów i końce
wierszy, chyba że `encoding`, `bom` lub `line_ending` podają inne wartości;
tekst jest dopisywany w kodowaniu i z końcami wierszy pliku. Znak, którego
kodowanie nie może przedstawić, kończy się błędem `E_ENCODING` w jego wierszu
i kolumnie, zanim cokolwiek zostanie zapisane, a poprawką jest wywołanie
`convert`.

Dowiązanie w ostatnim składniku `path` jest podążane: zapisywany jest plik,
który wskazuje, a dowiązanie pozostaje dowiązaniem. Katalog nigdy nie jest
zapisywany. Brakujące katalogi nadrzędne są tworzone, chyba że `parents` ma
wartość `false`; każdy z nich jest osobnym celem. Zawartość identyczna z
plikiem nie jest zapisywana i jest zgłaszana jako `unchanged`.

Rekordy wejściowe są zapisywane w `format`; `auto` podąża za rozszerzeniem
pliku (`.json`, `.yaml`, `.yml`, `.toml`, `.csv`, `.tsv`), w przeciwnym razie
`text`. `json` zapisuje tablicę obiektów z wcięciami; `csv` i `tsv` zapisują
wiersz nagłówka z nazwą każdego pola, a po nim jeden wiersz na rekord; `text`
zapisuje jeden wiersz na rekord, pole `text`, gdy rekord je ma, w przeciwnym
razie rekord jako zwięzły JSON. `yaml` i `toml` wymagają grupy documents.

Ścieżka `registry:/` zapisuje wartość rejestru w systemie Windows, typu
`value_type` (domyślnie `string`); `mode` to `create` lub `overwrite`.

Tekst zapisany do pliku znanego formatu jest sprawdzany po zapisie: format
dokumentu z grupy documents, w przeciwnym razie język z grupy code. Liczą się
tylko błędy składni, których poprzednia zawartość nie miała. Przy
`validate: reject` taki błąd wycofuje cały zapis z `E_ROLLED_BACK`; przy
`warn` jest zgłaszany jako ostrzeżenie, a zapis zostaje zachowany; `off`
pomija kontrolę. Bajty, rozmiary i wartości rejestru nie są sprawdzane, a bez
grup documents i code nic nie jest sprawdzane.

**Uprawnienie.** Pliki, których konto nie może zmieniać (`/etc/hosts`, pliki innych kont), są zapisywane z `privilege: elevated`. Wpis jest w dzienniku konta z właścicielem, grupą, trybem i kontrolą dostępu poprzedniej zawartości, konto ma dostęp do kopii, a `undo` przywraca je przez ten sam backend.

```nurt
write "/etc/hosts" "127.0.0.1 localhost\n" mode: append privilege: elevated
```

Przy `privilege: auto` etap działa z kontem i jest powtarzany jednokrotnie z `elevated`, gdy cel kończy się błędem `E_PERMISSION` lub `E_PRIVILEGE`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `mode` | `enum(create\|overwrite\|append\|prepend)` | wymagany | Tryb zapisu; nie ma wartości domyślnej. |
| `path` | `path` | jeden z `target` | Plik do zapisu. |
| `text` | `text` | brak | Tekst do zapisu. |
| `from` | `pipeline \| stream<any> \| path` | brak | Potok, nazwany wynik lub plik, którego zawartość jest zapisywana. |
| `encoding` | `text` | `"auto"` | Kodowanie tekstu; auto zachowuje kodowanie istniejącego pliku. |
| `line_ending` | `enum(lf\|crlf\|cr\|keep)` | `keep` | Końce wierszy zapisywanego tekstu. |
| `bom` | `bool` | jak w istniejącym pliku | Zapisuje znacznik kolejności bajtów. |
| `parents` | `bool` | `true` | Tworzy brakujące katalogi nadrzędne. |
| `ensure_newline` | `bool` | `false` | Kończy plik końcem wiersza. |
| `format` | `enum(auto\|json\|yaml\|toml\|csv\|tsv\|text)` | `auto` | Serializacja zapisywanych rekordów; auto podąża za rozszerzeniem pliku. |
| `files` | `list<map<any>>` | jeden z `target` | Partia plików zapisywanych w jednej transakcji, każdy ze ścieżką, tekstem i opcjonalnym trybem. |
| `value_type` | `enum(string\|dword\|qword\|multi_string\|expand_string\|binary)` | brak | Typ wartości rejestru; tylko dla ścieżek registry:. |
| `file_mode` | `text` | brak | Uprawnienia nowego pliku, takie jak "0644". |
| `size` | `size` | brak | Obcina lub rozszerza plik do tego rozmiaru. |
| `data` | `bytes` | brak | Bajty do zapisu. |
| `offset` | `int` | brak | Przesunięcie w bajtach, od którego zapisywane są dane. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Wycofuje zapis, który dodaje nowy błąd składni do znanego formatu. |
| `if_match` | `text \| map<text>` | brak | Oczekiwany etag pliku albo mapa ze ścieżki na etag dla plików. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `none`, `any` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Utworzenie pliku z trzema wierszami:

```nurt example
write "/home/user/app/Dockerfile" "FROM alpine:3.20\nRUN apk add --no-cache curl\nCMD [\"sh\"]\n" mode: create
```

```text expected
+ /home/user/app/Dockerfile
```

Dopisanie wiersza do istniejącego pliku o rozmiarze 120 bajtów:

```nurt example
write "/home/user/app/.gitignore" "dist/\n" mode: append
```

```text expected
120b → 126b
```

Zastąpienie pliku, który używa końców wierszy CRLF; nowy tekst je zachowuje:

```nurt example
write "/home/user/app/config.ini" "[main]\nport = 8080\n" mode: overwrite
```

Zapis dwóch plików w jednej transakcji; brakujący katalog `docs` jest tworzony:

```nurt example
write files: [{path: "/home/user/app/a.txt", text: "a\n"}, {path: "/home/user/app/docs/b.txt", text: "b\n"}] mode: create
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | brak `mode` albo brak zarówno `path`, jak i `files` | podaj `mode: create`, `overwrite`, `append` lub `prepend` |
| `E_EXISTS` | `mode: create`, a plik istnieje | użyj `mode: overwrite` lub `mode: append` |
| `E_PARAM_CONFLICT` | dwa źródła zawartości, `path` z `files` albo `offset` bez `data` | podaj jedno źródło zawartości |
| `E_ENCODING` | znak, którego kodowanie pliku nie może przedstawić | najpierw przekonwertuj plik do UTF-8 za pomocą `convert` albo podaj `encoding` |
| `E_STALE` | plik zmienił się po odczytaniu jego etagu lub po planie | odczytaj plik ponownie i użyj nowego etagu |
| `E_NOT_FOUND` | brakuje katalogu nadrzędnego, a `parents` ma wartość `false`, albo dowiązanie wskazuje brakujący wpis | ustaw `parents: true` lub utwórz cel |
| `E_INVALID_VALUE` | ścieżka jest katalogiem, `file_mode` nie jest ósemkowy albo `value_type` podano dla pliku | popraw wartość |
| `E_ROLLED_BACK` | zapisany tekst dodaje błąd składni, a `validate` ma wartość `reject` | popraw tekst lub przekaż `validate: warn` |
| `E_UNSUPPORTED` | `format: yaml` lub `toml` bez grupy documents | załaduj grupę lub użyj `format: json` |
| `E_OS_ONLY` | ścieżka `registry:/` poza systemem Windows | zapisuj zamiast tego pliki |
| `E_LIMIT` | plik odczytany na potrzeby zmiany jest większy niż limit tekstu | zapisz za pomocą `data` i `offset` albo `size` |

## Powiązane {#related}

`create`, `replace`, `insert`, `edit`, `convert`, `read`, `undo`.
