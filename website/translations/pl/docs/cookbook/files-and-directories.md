---
category: files-and-directories
title: Pliki i katalogi
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/files-and-directories.md -->
<!-- Responsibility:   Documents Files and directories. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Pliki i katalogi {#files-and-directories}

## Usunięcie katalogu {#remove-a-directory}

Usuń katalog wraz z zawartością.

```nurt example
remove "/home/user/app/build"
```

```text expected
/home/user/app/build
```

```bash
rm -rf /home/user/app/build
```

## Usunięcie plików według wzorca {#remove-the-files-of-a-pattern}

Usuń pliki tymczasowe z katalogu budowania.

```nurt example
find "/home/user/app/build" glob: "**/*.tmp" |> remove
```

```text expected
nothing_matched
```

## Usunięcie zawartości katalogu {#remove-the-content-of-a-directory}

Usuń wszystko wewnątrz katalogu i zachowaj sam katalog.

```nurt example
find "/home/user/app/build" glob: "*" depth: 1 |> remove
```

```text expected
nothing_matched
```

## Usunięcie pliku ze spacją w nazwie {#remove-a-file-with-a-space-in-its-name}

Usuń plik, którego nazwa zawiera spację; ścieżka jest jedną wartością.

```nurt example
remove "/home/user/data/my file.txt"
```

```text expected
my file.txt
```

## Kopiowanie katalogu {#copy-a-directory}

Skopiuj katalog i zachowaj atrybuty.

```nurt example
copy "/home/user/app" to: "/home/user/backup" preserve: true
```

```text expected
/home/user/backup
```

```bash
cp -a /home/user/app /home/user/backup
```

## Kopiowanie kilku plików do katalogu {#copy-several-files-into-a-directory}

Skopiuj dwa pliki do katalogu.

```nurt example
copy ["/home/user/a.txt", "/home/user/b.txt"] into: "/home/user/out"
```

```text expected
/home/user/out
```

## Przenoszenie starych logów {#move-the-old-logs}

Przenieś pliki logów starsze niż siedem dni do katalogu archiwum.

```nurt example
find "/home/user/logs" glob: "*.log" depth: 1 |> filter modified < ago(7d) |> move into: "/home/user/logs/old"
```

```text expected
nothing_matched
```

```bash
find /home/user/logs -maxdepth 1 -name '*.log' -mtime +7 -exec mv {} /home/user/logs/old/ \;
```

## Zmiana nazwy pliku {#rename-a-file}

Zmień nazwę pliku wewnątrz jego katalogu.

```nurt example
rename path: "/home/user/a.txt" to: "b.txt"
```

```text expected
b.txt
```

## Zmiana nazw plików według wzorca {#rename-files-by-a-pattern}

Zastąp rozszerzenie plików katalogu.

```nurt example
find "/home/user/photos" glob: "*.jpeg" depth: 1 |> rename /\.jpeg$/ ".jpg"
```

```text expected
nothing_matched
```

## Tworzenie katalogu wraz z katalogami nadrzędnymi {#create-a-directory-with-its-parents}

Utwórz katalog zagnieżdżony.

```nurt example
create "/home/user/app/out/reports" type: directory
```

```text expected
/home/user/app/out/reports
```

## Tworzenie dowiązania symbolicznego {#create-a-symbolic-link}

Utwórz dowiązanie do katalogu.

```nurt example
create "/home/user/current" type: link target: "/home/user/app/releases/42"
```

```text expected
/home/user/current
```

## Zmiana uprawnień {#change-permissions}

Nadaj skryptowi uprawnienie do wykonywania.

```nurt example
attributes "/home/user/app/run.sh" mode: "755"
```

```text expected
/home/user/app/run.sh
```

## Wyświetlanie rozmiaru podkatalogów {#show-the-size-of-the-subdirectories}

Wyświetl drzewo katalogu wraz z rozmiarem każdego katalogu.

```nurt example
list "/home/user/app" view: sizes depth: 2
```

```text expected
/home/user/app
```

```bash
du -h --max-depth=2 /home/user/app
```

## Wyszukiwanie duplikatów według skrótu {#find-duplicates-by-hash}

Oblicz skrót każdego pliku katalogu i zachowaj grupy o wspólnym skrócie.

```nurt example
find "/home/user/photos" |> hash |> group by: hash
```

```text expected
sha256
```

## Porównanie dwóch plików {#compare-two-files}

Pokaż różnice między dwoma plikami.

```nurt example
diff "/home/user/a.txt" "/home/user/b.txt"
```

```text expected
hunk
```

## Wyświetlanie największych wpisów {#list-the-largest-entries}

Wyświetl wpisy katalogu, od największego.

```nurt example
list "/home/user/app" sort: size order: desc
```

```text expected
/home/user/app
```

```bash
ls -lS /home/user/app
```

## Wyświetlanie drzewa katalogów {#show-a-directory-tree}

Pokaż drzewo katalogu do trzeciego poziomu.

```nurt example
list "/home/user/app" view: tree depth: 3
```

```text expected
/home/user/app
```

```bash
tree -L 3 /home/user/app
```

## Wyświetlanie samych podkatalogów {#list-only-the-subdirectories}

Wyświetl podkatalogi katalogu bez wpisów ukrytych.

```nurt example
list "/home/user/app" type: directory hidden: false
```

```text expected
/home/user/app
```

## Wyszukiwanie największych plików {#find-the-largest-files}

Znajdź dziesięć największych plików katalogu.

```nurt example
find "/home/user/data" |> filter size > 100mb |> sort size order: desc |> first 10
```

```text expected
nothing_matched
```

```bash
find /home/user/data -type f -size +100M -exec ls -lS {} + | head -10
```

## Wyszukiwanie plików zmienionych dzisiaj {#find-the-files-changed-today}

Znajdź pliki zmienione w ciągu ostatnich 24 godzin.

```nurt example
find "/home/user/app/src" |> filter modified > ago(1d)
```

```text expected
nothing_matched
```

```bash
find /home/user/app/src -type f -mtime -1
```

## Wyszukiwanie pliku według przybliżonej nazwy {#find-a-file-by-an-approximate-name}

Znajdź pliki, których ścieżka przypomina podaną nazwę.

```nurt example
find "/home/user/app" fuzzy: "readme" exclude_preset: [vcs, deps]
```

```text expected
README
```

## Wyświetlanie skrótu pliku {#show-the-hash-of-a-file}

Pokaż skrót BLAKE3 pliku wraz z jego rozmiarem.

```nurt example
hash "/home/user/dl/image.iso" algorithm: blake3
```

```text expected
blake3
```

```bash
b3sum /home/user/dl/image.iso
```

## Kopiowanie bez nadpisywania {#copy-without-overwriting}

Skopiuj katalog i zachowaj pliki, które już istnieją w katalogu docelowym.

```nurt example
copy "/home/user/photos" to: "/home/user/backup/photos" on_conflict: skip
```

```text expected
/home/user/backup/photos
```

```bash
rsync -a --ignore-existing /home/user/photos/ /home/user/backup/photos/
```

## Przeniesienie pliku {#move-a-file}

Przenieś plik pod inną ścieżkę.

```nurt example
move "/home/user/dl/report.pdf" to: "/home/user/docs/report.pdf"
```

```text expected
/home/user/docs/report.pdf
```

```bash
mv /home/user/dl/report.pdf /home/user/docs/report.pdf
```

## Aktualizacja czasu modyfikacji {#update-the-modification-time}

Utwórz plik, gdy go brak, a w przeciwnym razie zaktualizuj jego czas modyfikacji.

```nurt example
create "/home/user/app/.stamp" type: file on_exists: touch
```

```text expected
/home/user/app/.stamp
```

```bash
touch /home/user/app/.stamp
```

## Tworzenie kilku katalogów {#create-several-directories}

Utwórz katalogi układu projektu jednym wywołaniem.

```nurt example
create ["/home/user/app/src", "/home/user/app/docs", "/home/user/app/tests"] type: directory
```

```text expected
/home/user/app/src
```

```bash
mkdir -p /home/user/app/src /home/user/app/docs /home/user/app/tests
```

## Tworzenie katalogu tymczasowego {#create-a-temporary-directory}

Utwórz katalog, który Nurt usuwa na końcu sesji.

```nurt example
create "/home/user/tmp/work" type: directory temporary: true
```

```text expected
/home/user/tmp/work
```

## Zmiana konta i grupy drzewa {#change-the-account-and-group-of-a-tree}

Ustaw konto i grupę drzewa katalogów.

```nurt example
attributes "/home/user/www" owner: "deploy" group: "deploy" recursive: true
```

```text expected
/home/user/www
```

```bash
chown -R deploy:deploy /home/user/www
```

## Ustawienie pliku jako tylko do odczytu {#make-a-file-read-only}

Ustaw atrybut tylko do odczytu pliku.

```nurt example
attributes "/home/user/app/config.lock" readonly: true
```

```text expected
/home/user/app/config.lock
```

```bash
chmod a-w /home/user/app/config.lock
```

## Błąd, gdy pliku brakuje {#fail-when-a-file-is-missing}

Usuń plik i zgłoś błąd, gdy nie istnieje.

```nurt example
remove "/home/user/app/app.pid" on_missing: error
```

```text expected
E_NOT_FOUND
```

## Porównanie plików z pominięciem białych znaków {#compare-files-ignoring-whitespace}

Pokaż różnice między dwoma plikami i zignoruj zmiany białych znaków.

```nurt example
diff "/home/user/a.txt" "/home/user/b.txt" ignore_whitespace: true context: 1
```

```text expected
hunk
```

```bash
diff -w -U 1 /home/user/a.txt /home/user/b.txt
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
