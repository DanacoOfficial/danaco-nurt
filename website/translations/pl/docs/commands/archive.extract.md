---
command: archive.extract
group: archive
summary: Rozpakowuje archiwum do katalogu, pomijając wpisy, które wskazują poza ten katalog.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# archive.extract

## Składnia {#synopsis}

```text
archive.extract <path> into: <path> [name: value]...
```

## Opis {#description}

`archive.extract` rozpakowuje składniki archiwum `path` do katalogu `into`,
który jest tworzony wraz z brakującymi katalogami nadrzędnymi, jeżeli nie
istnieje. Format jest rozpoznawany na podstawie zawartości pliku, tak jak
w `archive.list`.

Przed zapisaniem czegokolwiek sprawdzana jest każda ścieżka składnika i każdy
cel dowiązania. Składnik nie jest rozpakowywany i jest zgłaszany jako wpis
szczególny `archive_path_escape`, gdy jego ścieżka:

- zawiera element `..` albo element złożony wyłącznie z kropek i spacji;
- jest bezwzględna, zaczyna się od litery dysku lub zawiera `\`;
- przechodzi przez dowiązanie symboliczne utworzone przez to samo archiwum albo
  przez istniejące dowiązanie symboliczne wewnątrz `into`;
- jest dowiązaniem symbolicznym, którego cel jest bezwzględny lub prowadzi poza
  `into`;
- jest dowiązaniem twardym do pliku, który nie jest rozpakowywany z tego samego
  archiwum.

Przy `allow_outside: true` takie składniki są rozpakowywane do ścieżki zapisanej
w archiwum, liczonej względem `into`, i nadal są zgłaszane. Składnik, którego
nazwa jest pusta, zawiera NUL lub nie jest poprawnym UTF-8, nigdy nie jest
rozpakowywany. Urządzenia, kolejki FIFO i inne pliki specjalne nigdy nie są
rozpakowywane i są zgłaszane jako `special_file`.

`only` wybiera składniki według ich ścieżki w archiwum, według reguł wzorców
z `archive.list`. `strip_components` usuwa początkowe elementy ze ścieżek
składników i celów dowiązań twardych; składnik, który pozostał bez elementów,
jest pomijany. Gdy kilka składników ma ten sam cel, rozpakowywany jest
ostatni z nich.

Istniejący zwykły plik jest zastępowany, zachowywany lub zgłaszany jako błąd
zgodnie z `on_conflict`. Istniejący katalog otrzymuje zawartość archiwum.
Istniejący wpis innego typu niż składnik jest zachowywany przy
`on_conflict: skip`, a w pozostałych przypadkach jest błędem; dowiązanie
symboliczne o tym samym celu co składnik pozostaje bez zmian.

Zwykłe pliki otrzymują uprawnienia zapisane w archiwum; bity set-user-id
i set-group-id są zachowywane tylko wtedy, gdy jawnie podano
`preserve_permissions: true`, a proces działa jako `root`, w przeciwnym
razie są odrzucane, dzięki czemu archiwum rozpakowane przez proces bez
uprzywilejowania lub bez tej opcji nigdy nie tworzy programu uprzywilejowanego.
Przy `preserve_permissions: false` odrzucane są także bit lepki oraz
uprawnienie zapisu grupy i pozostałych. Zastąpiony plik zachowuje własne
uprawnienia. Katalogi zawsze zapewniają kontu pełny dostęp, aby można było
zapisać ich zawartość. Pliki otrzymują jako czas modyfikacji czas ich zapisu.

Rozmiar po rozpakowaniu oraz stosunek rozmiaru po rozpakowaniu do rozmiaru
archiwum są ograniczone (1 TiB oraz stosunek 1000 powyżej 64 MiB, dlatego
archiwum o rozmiarze 300 KiB, które deklaruje 300 MiB, jest odrzucane z błędem
`E_LIMIT`); archiwum, które deklaruje więcej, kończy się niepowodzeniem przed
zapisaniem czegokolwiek.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik archiwum. |
| `into` | `path` | wymagany | Katalog, do którego archiwum jest rozpakowywane; tworzony, gdy brakuje. |
| `only` | `glob` | brak | Tylko wpisy, których ścieżka pasuje do tego wzorca. |
| `strip_components` | `int` | `0` | Liczba początkowych elementów ścieżki usuwanych ze ścieżek wpisów. |
| `on_conflict` | `enum(overwrite\|skip\|error)` | `overwrite` | Zachowanie, gdy plik docelowy już istnieje. |
| `preserve_permissions` | `bool` | `true` | Stosuje uprawnienia zapisane w archiwum. |
| `allow_outside` | `bool` | `false` | Rozpakowuje wpisy, których rozwiązana ścieżka leży poza katalogiem docelowym. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

Wpisy szczególne: `archive_path_escape` dla każdego odrzuconego składnika
wychodzącego poza katalog albo, przy `allow_outside`, rozpakowanego, ze
składnikiem w postaci `archive:<archive>!/<member>` i przyczyną
(`parent_component`, `absolute`, `drive_letter`, `backslash`, `invalid_name`,
`link_outside`, `through_link`, `hard_link_outside`); `special_file` dla plików
specjalnych; `overwrote_existing` z liczbą zastąpionych plików.

## Przykłady {#examples}

Rozpakowanie skompresowanego archiwum tar do nowego katalogu:

```nurt example
archive.extract "/srv/dl/data.tar.gz" into: "/srv/data"
```

```text expected
effect archive.extract  files: 2  directories: 2  bytes: 18
```

Rozpakowanie plików Markdown z wydania bez jego katalogu głównego:

```nurt example
archive.extract "/home/user/dl/project-1.2.zip" into: "/home/user/docs" only: "*.md" strip_components: 1
```

Rozpakowanie wydania na istniejącą instalację z zachowaniem plików, które już tam są:

```nurt example
archive.extract "/srv/dl/release.tar" into: "/srv/app" on_conflict: skip
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_FORMAT` | plik nie jest archiwum obsługiwanego formatu albo archiwum jest uszkodzone | Sprawdź plik. |
| `E_ENCRYPTED` | wybrany składnik jest zaszyfrowany | Odszyfruj archiwum jego własnym narzędziem albo wyłącz składnik za pomocą `only`. |
| `E_LIMIT` | deklarowany rozmiar po rozpakowaniu lub stopień kompresji przekracza limit | Sprawdź archiwum; może to być bomba dekompresyjna. |
| `E_EXISTS` | cel istnieje, a `on_conflict` ma wartość `error`, istniejący wpis ma inny typ niż składnik albo `into` nie jest katalogiem | Ustaw `on_conflict`, usuń wpis albo wybierz inny `into`. |
| `E_STALE` | archiwum lub cel zmienił się po sporządzeniu planu | Uruchom polecenie ponownie. |
| `E_INVALID_VALUE` | `strip_components` jest ujemne albo `only` nie jest poprawnym wzorcem | Popraw wartość. |
| `E_UNKNOWN_VALUE` | wartość `on_conflict` jest nieznana | Użyj jednej z wymienionych wartości. |
| `E_PATH_RELATIVE` | ścieżka jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |

## Powiązane {#related}

`archive.list` pokazuje składniki przed rozpakowaniem; `archive.create`
tworzy archiwum; `undo` cofa rozpakowanie.
