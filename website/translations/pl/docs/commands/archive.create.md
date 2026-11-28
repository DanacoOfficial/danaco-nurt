---
command: archive.create
group: archive
summary: Tworzy archiwum z wpisów wejściowych.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# archive.create

## Składnia {#synopsis}

```text
entry |> archive.create <to> [name: value]...
```

## Opis {#description}

`archive.create` zapisuje nowe archiwum `to` zawierające pliki, katalogi
i dowiązania wskazane przez wejściowe rekordy `entry`, na przykład rekordy
polecenia `find` lub `list`. Katalog dodaje całe swoje drzewo. Wpis podany
więcej niż raz, na przykład katalog i pliki, które `find` wymienia w jego
wnętrzu, jest zapisywany jeden raz.

Nazwą składnika jest ścieżka wpisu względem `base`. Bez `base` jest to pole
`relative` rekordu, które ustawiają `find` i `list`, a w pozostałych
przypadkach nazwa wpisu. Wpis poza `base` jest błędem; sam wpis `base` dodaje
swoją zawartość bez własnego składnika.

Składniki są zapisywane w kolejności bajtowej ich nazw, z uprawnieniami
i czasami modyfikacji plików oraz bez nazw kont, więc to samo drzewo zawsze
daje to samo archiwum. Dowiązania symboliczne są zapisywane jako dowiązania;
przy `follow_links: true` zapisywane są zamiast nich pliki i katalogi, do
których prowadzą, a dowiązanie prowadzące z powrotem do zapisywanego katalogu
jest pomijane i zgłaszane jako `link_loop_skipped`. Urządzenia, kolejki FIFO
i inne pliki specjalne są pomijane i zgłaszane jako `special_file`.

`format: auto` wybiera format na podstawie rozszerzenia `to`: `.zip`, `.jar`,
`.whl`, `.apk`, `.tar`, `.tar.gz` lub `.tgz`, `.tar.zst` lub `.tzst`,
`.tar.xz` lub `.txz`, `.tar.bz2` lub `.tbz2`, `.7z`. `level` jest poziomem
kompresji formatu: od 0 do 9 dla zip (0 zapisuje składniki bez kompresji)
i 7z oraz zakres algorytmu strumieniowego dla skompresowanego tar (gzip, xz
i bzip2 od 0 do 9, zstd od 1 do 22). Archiwum 7z jest składane w pamięci
i ma limit 1 GiB.

Puste wejście tworzy puste archiwum.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `to` | `path` | wymagany | Plik archiwum do utworzenia. |
| `format` | `enum(auto\|zip\|tar\|tar_gz\|tar_zst\|tar_xz\|tar_bz2\|seven_zip)` | `auto` | Format archiwum; wartość auto wybiera go na podstawie rozszerzenia pliku. |
| `base` | `path` | brak | Katalog, względem którego liczone są nazwy wpisów w archiwum. |
| `level` | `int` | brak | Poziom kompresji formatu. |
| `follow_links` | `bool` | `false` | Zapisuje cele dowiązań symbolicznych zamiast samych dowiązań. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `entry` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zarchiwizowanie wyniku kompilacji aplikacji z nazwami zaczynającymi się od `dist`:

```nurt example
find "/srv/app/dist" |> archive.create "/srv/out/dist.zip" base: "/srv/app"
```

```text expected
effect archive.create  /srv/out/dist.zip  files: 2  directories: 1
```

Zarchiwizowanie wpisów katalogu jako archiwum tar skompresowanego za pomocą gzip:

```nurt example
list "/srv/app" |> archive.create "/srv/out/app.tar.gz"
```

Zarchiwizowanie źródeł jako archiwum tar skompresowanego za pomocą zstd na wysokim poziomie:

```nurt example
find "/srv/app/src" depth: 0 |> archive.create "/srv/out/app.bin" format: tar_zst level: 19
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_EXISTS` | `to` istnieje | Najpierw je usuń albo wybierz inną nazwę |
| `E_NOT_FOUND` | katalog `to` lub wpis wejściowy nie istnieje albo śledzone dowiązanie jest zerwane | Sprawdź ścieżki |
| `E_INVALID_VALUE` | formatu nie można ustalić na podstawie rozszerzenia `to`, `level` jest poza zakresem formatu, wpis leży poza `base` albo nazwa nie jest poprawnym Unicode | Ustaw `format`, popraw `level` lub `base` albo zmień nazwę wpisu |
| `E_UNKNOWN_VALUE` | wartość `format` jest nieznana | Użyj jednej z wymienionych wartości |
| `E_STALE` | plik wejściowy zmienił się w trakcie zapisu albo `to` został utworzony po sporządzeniu planu | Uruchom polecenie ponownie |
| `E_LIMIT` | archiwum 7z przekracza limit pamięci | Utwórz zamiast tego archiwum zip lub tar |
| `E_TYPE` | rekord wejściowy nie ma pola `path` | Przekaż rekordy plików |

## Powiązane {#related}

`archive.list` wyświetla składniki nowego archiwum; `archive.extract`
je rozpakowuje; `compress` kompresuje pojedyncze pliki.
