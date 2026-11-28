---
name: documents
description: Odczyt i zmiana plików ustrukturyzowanych JSON, YAML, TOML, XML, INI, CSV i innych według klucza.
commands: [read, set, query, sql, write, diff, find]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/documents.md -->
<!-- Responsibility:   Documents documents. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# dokumenty {#documents}

## Ścieżki kluczy {#key-paths}

Klucz wskazuje wartość wewnątrz dokumentu. Dostępne formy to: `a.b` dla wpisu mapy, `a[0].b` dla elementu listy, `a[*]` dla każdego elementu listy, `a.**` dla wszystkich potomków, `a["dotted.key"]` dla klucza zawierającego kropkę, `a[?name == "x"]` dla elementów spełniających warunek oraz `a[+]` dla nowego elementu na końcu listy.

## Odczyt {#reading}

`read` z parametrem `key` zwraca wartość spod ścieżki klucza. `query` stosuje filtr jq do dokumentu, pliku lub odpowiedzi i zwraca elementy, które kolejne etapy potoku mogą filtrować i wybierać.

```nurt
read "/srv/app/values.yaml" key: "image.tag"
query ".dependencies | keys" path: "/srv/app/package.json"
```

## Zmiana {#changing}

`set` zapisuje wartość pod ścieżką klucza i zachowuje układ, komentarze, kolejność kluczy oraz styl pliku; zmieniają się wyłącznie bajty zmienianej wartości. Format jest wykrywany na podstawie pliku i można go podać parametrem `format`. `create: true` dodaje brakujący klucz, a `values` ustawia kilka kluczy w jednym wywołaniu.

```nurt
set "/srv/app/package.json" key: "scripts.test" value: "vitest"
```

## Walidacja {#validation}

Zapis, który wprowadza nowy błąd składni do znanego formatu, jest wycofywany. `validate: warn` zgłasza błąd i zachowuje zmianę, a `validate: off` pomija sprawdzenie. Obsługiwane formaty to `json`, `jsonc`, `yaml`, `toml`, `xml`, `ini`, `csv`, `tsv`, `properties`, `env` i `plist`. Zmiana pliku, który został zmodyfikowany po odczytaniu, kończy się błędem `E_STALE`; zob. przewodnik `multi-agent`.

## Bazy danych {#databases}

`sql` wykonuje instrukcje na pliku bazy danych SQLite. Wywołanie, którego instrukcje tylko odczytują dane, otwiera bazę danych w trybie tylko do odczytu. Instrukcja, która cokolwiek zmienia, otwiera ją do zapisu, a zmiana jest rejestrowana w dzienniku jak każda inna zmiana: plik bazy danych oraz jego pliki `-wal`, `-shm` i `-journal` są najpierw zachowywane, a `undo` je przywraca. Parametr `write: true` jest opcjonalny; `write: false` utrzymuje wywołanie w trybie tylko do odczytu. `BEGIN`, `COMMIT` i `END` łączą się z transakcją wywołania, `ROLLBACK` kończy ją błędem `E_ROLLED_BACK`, a `ATTACH`, `VACUUM INTO`, `VACUUM`, `load_extension` oraz pragmy `temp_store_directory` i `data_store_directory` są dozwolone (zob. stronę polecenia).

```nurt
sql "/srv/app/app.db" "select id, name from users where active = 1 limit 5"
```

## Błędy {#errors}

`E_KEY_NOT_FOUND` wskazuje klucz, który nie istnieje, i wymienia klucze na najbliższym poziomie. `E_KEY_SYNTAX` oznacza nieprawidłową ścieżkę klucza. `E_DOC_PARSE` oznacza, że plik jest niepoprawny w swoim formacie, a `E_FORMAT` oznacza, że nie udało się ustalić formatu.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
