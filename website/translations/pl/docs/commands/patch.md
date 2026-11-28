---
command: patch
group: text
summary: Stosuje różnicę w formacie unified lub git do plików pod katalogiem głównym.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# patch

## Składnia {#synopsis}

```text
patch [<diff>] root: <path> (diff: <text> | from: <path>) [name: value]...
```

## Opis {#description}

`patch` stosuje różnicę w formacie unified, zapisaną przez `diff -u`, albo w
formacie git, zapisaną przez `git diff`, do plików poniżej `root`. Różnica jest
podawana jako tekst w `diff` albo odczytywana z pliku `from`; wymagane jest
dokładnie jedno z nich. Tekst poza sekcjami plików, taki jak komunikat
rewizji, jest pomijany.

Cała różnica jest stosowana w pamięci, zanim cokolwiek się zmieni. Fragment
(hunk) jest umieszczany w wierszu swojego nagłówka, przesuniętym o odsunięcie
poprzedniego fragmentu; gdy starych wierszy tam nie ma, są wyszukiwane w
coraz większych odległościach. Z `fuzz: n` do `n` wierszy kontekstu na każdym
końcu fragmentu może się różnić. Fragment, którego nie można umieścić, kończy
całe polecenie błędem `E_PATCH_REJECTED` i żaden plik się nie zmienia.
Kolejne sekcje plików widzą wynik wcześniejszych, a każdy plik jest zapisywany
raz z jego końcową zawartością, w jednej operacji, którą `undo` cofa w całości.

Ścieżki w różnicy są względne wobec `root`; `strip: n` usuwa `n` początkowych
składowych, jak `patch -p n` (różnice git wymagają `strip: 1` ze względu na
przedrostki `a/` i `b/`; wiersze `rename` i `copy` tracą o jedną składową
mniej). Ścieżki bezwzględne i ścieżki z `..` są odrzucane.

Zmieniony plik zachowuje swoje kodowanie, znacznik kolejności bajtów i
zakończenia wierszy: różnica z zakończeniami wierszy `lf` stosuje się do pliku
z zakończeniami `crlf`, a dodane wiersze przyjmują zakończenie wiersza pliku.
Nowe pliki są zapisywane w UTF-8 z zakończeniami wierszy z różnicy, a
brakujące katalogi są tworzone.

Nagłówki git tworzą (`new file mode`), usuwają (`deleted file mode`),
przemianowują (`rename from`, `rename to`) i kopiują (`copy from`, `copy to`)
pliki. Zmiana nazwy bez zmiany zawartości przemianowuje plik; zmiana nazwy ze
zmianami zapisuje nowy plik i usuwa stary. Sekcja pliku bez nagłówków git,
której stary plik nie istnieje, a której fragmenty tylko dodają wiersze,
tworzy plik. Tak jak w GNU `patch`, plik, który taka sekcja opróżnia, jest
usuwany, gdy znacznik czasu jego nowej strony jest epoką Unix, którą `diff -N`
zapisuje dla brakującego pliku; w przeciwnym razie pusty plik jest zachowywany
i zgłaszany jako `file_emptied`.

Zmiana trybu git (`old mode`, `new mode`) jest stosowana do pliku, którego
zawartość się nie zmienia. Dla pliku, którego zawartość się zmienia, oraz dla
nowego pliku z bitami wykonania tryb nie jest stosowany, a zmiana ma flagę
`mode_not_applied`.

`reverse: true` cofa zmiany opisane przez różnicę: dodane wiersze są
usuwane, utworzone pliki są kasowane, a zmiany nazw są odwracane.

Tekst każdego utworzonego i zmienionego pliku znanego formatu jest sprawdzany
po zastosowaniu łatki: format dokumentu z grupy documents, w przeciwnym razie
język z grupy code. Liczą się tylko błędy składni, których nie miała
poprzednia zawartość; plik utworzony przez zmianę nazwy jest porównywany z
plikiem, z którego został przemianowany. Z `validate: reject` taki błąd w
dowolnym pliku wycofuje całą łatkę błędem `E_ROLLED_BACK`; z `warn` jest
zgłaszany jako ostrzeżenie, a łatka zostaje zachowana; `off` pomija
sprawdzanie. Usunięte pliki i zmiany trybu nie są sprawdzane, a bez grup
documents i code nic nie jest sprawdzane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `root` | `path` | wymagany | Katalog, względem którego są ścieżki w różnicy. |
| `diff` | `text` | jeden z `patch_source` | Tekst różnicy. |
| `from` | `path` | jeden z `patch_source` | Plik zawierający różnicę. |
| `reverse` | `bool` | `false` | Stosuje różnicę w odwrotnym kierunku. |
| `fuzz` | `int` | `0` | Liczba wierszy kontekstu, które mogą się różnić przy umieszczaniu fragmentu. |
| `strip` | `int` | `0` | Liczba początkowych składowych ścieżki usuwanych ze ścieżek w różnicy. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Wycofuje zmianę, gdy wprowadza nowy błąd składni. |
| `if_match` | `text \| map<text>` | brak | Oczekiwany etag łatanego pliku albo mapa ścieżki na etag dla kilku plików; inny plik nie jest zmieniany. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

Rekord `effect` ma jedną zmianę `change` na każdy cel w kolejności wykonania:
utworzone katalogi, zmiany nazw, następnie pozostałe pliki w kolejności
ścieżek. Zmiana zawartości pliku ma `added`, `removed` i `diff` (rekordy
`hunk` zmiany, z trzema wierszami kontekstu); zmieniony plik ma także
`encoding_before`, `encoding_after`, `line_ending_before`,
`line_ending_after`, `bom_before` i `bom_after`. Flagi `hunk_offset` i
`hunk_fuzz` oznaczają plik, którego fragmenty zostały umieszczone w innym
wierszu albo z pominiętymi wierszami kontekstu.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

Uwagi: `unchanged` dla pliku, który różnica pozostawia bez zmian,
`file_emptied` dla pliku pozostawionego pustym, `encoding_preserved` i
`line_ending_preserved` dla plików zapisanych w kodowaniu innym niż UTF-8 lub
z zakończeniami wierszy innymi niż `lf`.

## Przykłady {#examples}

Zastosowanie poprawki zapisanej przez `git diff` do projektu z zakończeniami
wierszy `crlf`.

```nurt example
patch from: "/srv/patches/fix-timeout.diff" root: "/srv/app" strip: 1
```

```text expected
op 41  patch  ok
targets 1 file
  ~ /srv/app/src/config.rs  +1 -1
```

Zastosowanie różnicy git, która tworzy, usuwa i przemianowuje pliki.

```nurt example
patch from: "/srv/patches/layout.diff" root: "/srv/app" strip: 1
```

```text expected
op 42  patch  ok
targets 5
  + /srv/app/docs/
  + /srv/app/src/support/
  > /srv/app/src/util.rs -> /srv/app/src/support/util.rs
  - /srv/app/INSTALL.txt
  + /srv/app/docs/setup.md  +2
```

Cofnięcie pierwszego przykładu.

```nurt example
patch from: "/srv/patches/fix-timeout.diff" root: "/srv/app" strip: 1 reverse: true
```

Różnica, która już nie pasuje do pliku, niczego nie zmienia.

```nurt example
patch from: "/srv/patches/fix-timeout.diff" root: "/srv/app" strip: 1
```

```text expected
error E_PATCH_REJECTED  stage 1  /srv/app/src/config.rs:1
  hunk 1 does not apply to src/config.rs at line 1; check the file version or raise fuzz
```

Różnica, która usuwa zamykający nawias klamrowy w `package.json`, jest
wycofywana wraz ze swoją zmianą w `README.md`.

```nurt example
patch from: "/srv/patches/bump.diff" root: "/srv/app" strip: 1
```

```text expected
error E_ROLLED_BACK  stage 1
  the change adds 1 syntax error to `/srv/app/package.json` and was rolled back: correct the new text or pass `validate: warn`
```

Ta sama różnica z `validate: warn` jest stosowana, a nowy błąd jest zgłaszany
jako ostrzeżenie.

```nurt example
patch from: "/srv/patches/bump.diff" root: "/srv/app" strip: 1 validate: warn
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_PATCH_REJECTED` | Fragment nie pasuje do swojego pliku (`problem: hunk_rejected`, z oczekiwanymi i rzeczywistymi wierszami oraz położeniem) albo plik do usunięcia ma inną zawartość (`problem: delete_mismatch`). | Sprawdź wersję pliku, dostosuj `strip` albo zwiększ `fuzz`. |
| `E_FORMAT` | Różnica jest nieprawidłowa, nie zawiera sekcji pliku, jest różnicą binarną, nie wskazuje pliku, ma mniej składowych ścieżki niż `strip` albo ma ścieżkę bezwzględną lub ścieżkę z `..` (`problem` wskazuje przypadek). | Podaj tekstową różnicę w formacie unified lub git ze ścieżkami względnymi wobec `root`. |
| `E_NOT_FOUND` | Plik do zmiany nie istnieje albo `root` lub `from` nie istnieje. | Sprawdź `root` i `strip`. |
| `E_EXISTS` | Plik do utworzenia już istnieje. | Usuń plik albo najpierw zastosuj różnicę w odwrotnym kierunku. |
| `E_STALE` | Plik nie ma etagu z `if_match` albo zmienił się po zaplanowaniu. | Odczytaj plik ponownie i powtórz wywołanie. |
| `E_MISSING_PARAM` | Brak zarówno `diff`, jak i `from`, albo brak `root`. | Podaj różnicę i katalog główny. |
| `E_PARAM_CONFLICT` | Podano jednocześnie `diff` i `from`. | Podaj jedno z nich. |
| `E_INVALID_VALUE` | `strip` lub `fuzz` jest ujemne, `root` nie jest katalogiem (`problem: not_a_directory`) albo ścieżka różnicy jest katalogiem lub dowiązaniem (`problem: not_a_file`). | Popraw wartość. |
| `E_UNKNOWN_VALUE` | `validate` nie jest `off`, `warn` ani `reject`. | Użyj jednej z wymienionych wartości. |
| `E_ROLLED_BACK` | Łatka dodaje błąd składni do pliku, a `validate` ma wartość `reject` (`problem: validate_failed`, jeden wiersz `sample` na każdy nowy błąd). | Popraw różnicę albo przekaż `validate: warn`. |
| `E_BINARY` | Plik do zmiany ma zawartość binarną. | Łataj tylko pliki tekstowe. |
| `E_ENCODING` | Plik nie jest poprawny w wykrytym kodowaniu albo dodany znak nie może być w nim przedstawiony. | Przekonwertuj plik poleceniem `convert`. |
| `E_LIMIT` | Plik jest większy niż limit plików tekstowych. | Łataj mniejsze pliki. |
| `E_PERMISSION` | Plik nie może zostać odczytany ani zapisany. | Sprawdź uprawnienia. |
| `E_IO` | Plik nie może zostać odczytany ani zapisany. | Sprawdź uprawnienia. |

## Powiązane {#related}

`diff` zwraca różnice plików jako rekordy `hunk`. `replace`, `insert` i `edit`
zmieniają pliki według zawartości, a nie według numerów wierszy. `undo` cofa
zastosowaną łatkę.
