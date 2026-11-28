<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/providers/registry.md -->
<!-- Responsibility:   Documents registry:. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# registry: {#registry}

## Składnia {#synopsis}

```text
registry:/<root>/<key>/…/<key>
registry:/<root>/<key>/…/<value name>
registry:/<root>/<key>/…/(default)
```

## Opis {#description}

Dostawca `registry:` przedstawia rejestr systemu Windows jako drzewo ścieżek. Klucz
jest katalogiem i opisuje go rekord `entry` z `type: directory`;
wartość opisuje rekord `registry_value`. Polecenia `list`,
`find`, `read`, `info`, `write`, `remove`, `copy` i `move` przyjmują ścieżki
`registry:` zamiast ścieżek plików. Dostawca istnieje wyłącznie w systemie
Windows; gdzie indziej każda ścieżka `registry:` jest odrzucana podczas wiązania
z `E_OS_ONLY`.

Dostęp do rejestru odbywa się przez funkcje rejestru Win32. Żaden program
zewnętrzny nie jest uruchamiany.

## Ścieżki {#paths}

| Element | Reguła |
|---|---|
| Korzeń | `HKLM`, `HKCU`, `HKCR`, `HKU` lub `HKCC`. Pełne nazwy `HKEY_LOCAL_MACHINE`, `HKEY_CURRENT_USER`, `HKEY_CLASSES_ROOT`, `HKEY_USERS` i `HKEY_CURRENT_CONFIG` są akceptowane i pokazywane w postaci skróconej. |
| Separator | `/`. Znak `/` wewnątrz nazwy klucza lub wartości zapisuje się jako `%2F`, a `%` jako `%25`. |
| `\` | Nie rozdziela kluczy. Jest przyjmowany tylko w ostatnim segmencie, jako część nazwy wartości (`registry:/HKCU/Example/C:\Tools\app.exe`). Ścieżka z `\` w segmencie nazywającym klucz albo w jedynym segmencie poniżej korzenia jest odrzucana z błędem `E_INVALID_VALUE`: zapisz `registry:/HKCU/Software/Example`, a nie `registry:/HKCU/Software\Example`. |
| Wielkość liter | Nazwy są porównywane bez rozróżniania wielkości liter. Wyniki pokazują pisownię zapisaną w rejestrze. |
| Ostatni segment | Nazywa wartość, gdy klucz poprzedzający ma wartość o tej nazwie, w przeciwnym razie klucz. |
| `(default)` | Nazywa wartość domyślną klucza, której `name` jest puste w `registry_value`. |
| Widok | Widok 64-bitowy rejestru. Rekordy zawierają widok w polu `view`. |

## Rekordy {#records}

`registry_value`:

| Pole | Typ | Znaczenie |
|---|---|---|
| `path` | path | Klucz zawierający wartość. |
| `name` | text | Nazwa; puste dla wartości domyślnej. |
| `type` | enum(string\|expand_string\|multi_string\|dword\|qword\|binary\|none\|other) | Typ. Dane, których długość nie pasuje do `dword` lub `qword`, oraz typy `REG_DWORD_BIG_ENDIAN`, `REG_LINK` i listy zasobów są pokazywane jako `other`. |
| `value` | any | Tekst dla `string` i `expand_string` (odwołania nie są rozwijane), liczba całkowita dla `dword` i `qword`, lista tekstów dla `multi_string`, bajty dla `binary`, `none` i `other`. |
| `size` | size | Rozmiar danych w bajtach. |
| `view` | enum(64\|32) | Widok rejestru. |

Klucz jest rekordem `entry` z `type: directory`, `modified` ustawionym na czas
ostatniego zapisu do klucza i `size` równym 0. Klucz będący dowiązaniem symbolicznym jest rekordem `entry` z
`type: link` i `link_target`; `find` do niego nie wchodzi.

## Zapis {#writing}

`write "registry:/…" "<value>" mode: <mode> value_type: <type>` ustawia wartość
i tworzy brakujące klucze. `mode` jest wymagane, tak jak dla plików, i ma wartość `create`
(wartość nie może istnieć) lub `overwrite`. Wartość jest parametrem tekstowym
`write`; bez `value_type` jest zapisywana jako `string`. Liczby są zapisywane
jako tekst i konwertowane na dany typ.

| `value_type` | Akceptowana wartość |
|---|---|
| `string`, `expand_string` | tekst |
| `dword` | tekst z liczbą dziesiętną lub szesnastkową `0x` od 0 do 4294967295 |
| `qword` | tekst z liczbą dziesiętną lub szesnastkową `0x` od 0 do 18446744073709551615 |
| `multi_string` | tekst z jednym łańcuchem w wierszu |
| `binary` | bajty w parametrze `data` |

`remove` usuwa wartość albo klucz ze wszystkimi jego podkluczami i wartościami. Klucza
głównego nie można usunąć. `copy` i `move` kopiują klucz wraz z poddrzewem albo
wartość; miejsce docelowe nie może istnieć, a brakujące klucze nad nim są
tworzone. `move` jest jedną operacją dziennika.

## Polecenia {#commands}

| Polecenie | Zachowanie dla ścieżek `registry:` |
|---|---|
| `read` | Zwraca rekord `registry_value` każdej wartości. Klucz jest odrzucany z `E_IO` (`kind: is_a_directory`): zamiast tego należy go wylistować. Selektory wierszy nie mają zastosowania. |
| `info` | Zwraca rekord `entry` klucza lub rekord `registry_value` wartości; ścieżka, która niczego nie nazywa, daje `entry` z `exists: false`. |
| `list` | Najpierw klucze, potem wartości, do `depth` poziomów (domyślnie 1). `type` wybiera wartości (`file`), klucze (`directory`), klucze będące dowiązaniami (`link`) lub wszystko (`any`). |
| `find` | Klucze i wartości poniżej klucza do `depth` poziomów (domyślnie bez ograniczenia). `type` domyślnie ma wartość `file`, która wybiera wartości. `glob` jest dopasowywany bez rozróżniania wielkości liter do nazwy i do ścieżki poniżej klucza. Sam klucz jest kandydatem dla `type: any` i `type: directory`, gdy nie podano `glob`. |
| `write` | Ustawia wartość, jak opisano w sekcji Zapis. `mode` ma wartość `create` lub `overwrite`. |
| `remove` | Usuwa wartości i klucze nazwane przez `path` lub przez przekazane potokiem rekordy `entry` kluczy. Klucz wewnątrz klucza, który również jest usuwany, jest usuwany jednokrotnie, razem z kluczem nadrzędnym. Obowiązuje `on_missing`. |
| `copy`, `move` | `to` nazywa miejsce docelowe jednego źródła, `into` klucz, który przyjmuje każde źródło pod jego własną nazwą. Źródła i miejsce docelowe muszą być ścieżkami rejestru. Miejsce docelowe nie może istnieć, niezależnie od `on_conflict`. |

`list`, `find`, `read` i `info` zwracają rekordy rodzajów nazwanych w sekcji Rekordy.
`remove`, `copy` i `move` deklarują swoje cele dziennikowi tak jak
zmiany plików; `move` deklaruje kopię i usunięcie źródła w
jednej operacji. Polecenie, które przyjmuje wyłącznie ścieżki plików, odrzuca ścieżkę `registry:`
z `E_INVALID_VALUE`, a host bez rejestru odrzuca ją z
`E_OS_ONLY`.

## Skutek {#effect}

Każda zmiana jest odwracalna przez `undo`. Przed zmianą dziennik przechowuje:

| Cel | Zachowany stan |
|---|---|
| wartość | istnienie, kod typu i dokładne bajty, w tym tekst zapisany bez kończącego NUL |
| klucz | istnienie i całe poddrzewo: klasę, wartości z kodami typów i bajtami, podklucze oraz uznaniową listę kontroli dostępu |
| brakujące klucze | głębokość, na której ścieżka nie istniała; cofnięcie usuwa klucze utworzone przez operację |

Cofnięcie przywraca zachowany stan dokładnie; czas ostatniego zapisu przywróconych
kluczy jest czasem przywrócenia. Klucze ulotne i klucze będące dowiązaniami symbolicznymi
wewnątrz poddrzewa nie są zachowywane: raport oznacza je
`irreversible: previous_content_not_kept`. Stan większy niż
`journal.max_op_size` nie jest zachowywany i jest oznaczany
`irreversible: too_large_for_journal`; zmiana mimo to jest wykonywana.

Zmiana poniżej `HKCU/Environment` lub
`HKLM/SYSTEM/CurrentControlSet/Control/Session Manager/Environment` oraz jej
cofnięcie są ogłaszane uruchomionym programom przez `WM_SETTINGCHANGE`, aby nowe
procesy przez nie uruchamiane widziały zmienione zmienne. Samego ogłoszenia
nie można cofnąć.

## Przykłady {#examples}

Odczyt nazwy produktu zainstalowanej edycji systemu Windows:

```nurt
read "registry:/HKLM/SOFTWARE/Microsoft/Windows NT/CurrentVersion/ProductName"
```

Ustawienie trwałej zmiennej środowiskowej `EDITOR` bieżącego konta:

```nurt
write "registry:/HKCU/Environment/EDITOR" "code" mode: overwrite value_type: string
```

Usunięcie klucza wraz z poddrzewem i jego przywrócenie:

```nurt
remove "registry:/HKCU/Software/Example"
undo
```

Odczyt wartości domyślnej klucza:

```nurt
read "registry:/HKCU/Software/Example/(default)"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa naprawa |
|---|---|---|
| `E_OS_ONLY` | rejestr istnieje wyłącznie w systemie Windows | uruchom program w systemie Windows |
| `E_NOT_FOUND` | klucz lub wartość nie istnieje | sprawdź ścieżkę przez `list` |
| `E_EXISTS` | miejsce docelowe `copy` lub `move` istnieje | najpierw usuń miejsce docelowe |
| `E_PERMISSION` | odmowa dostępu do klucza lub próba usunięcia klucza głównego | uruchom z kontem mającym dostęp |
| `E_PRIVILEGE` | zmiana wymaga uprawnienia `administrator` | uruchom jako administrator |
| `E_TYPE` | wartość nie pasuje do `value_type` | przekaż wartość typu wymienionego wyżej |
| `E_INVALID_VALUE` | liczba jest poza zakresem, tekst zawiera NUL lub ścieżka nie daje się sparsować | popraw wartość lub ścieżkę |

## Powiązane {#related}

`environment` odczytuje trwałe zmienne środowiskowe; `list`, `find`, `read`,
`info`, `write`, `remove`, `copy`, `move` i `undo` opisują polecenia.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
