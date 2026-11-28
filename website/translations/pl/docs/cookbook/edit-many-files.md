---
category: edit-many-files
title: Edycja wielu plików
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/edit-many-files.md -->
<!-- Responsibility:   Documents Editing many files. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Edycja wielu plików {#editing-many-files}

## Zastąpienie identyfikatora we wszystkich plikach Go {#replace-an-identifier-in-all-go-files}

Zastąp nazwę funkcji w każdym pliku Go w katalogu.

```nurt example
replace /oldName\(/ "newName(" in: ["/home/user/app/src"] glob: "**/*.go"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"x := oldName(1)\n"}]}
```

```text expected
/home/user/app/src/a.go
```

```bash
grep -rl 'oldName(' --include='*.go' /home/user/app/src | xargs sed -i 's/oldName(/newName(/g'
```

## Zastąpienie tylko przy oczekiwanej liczbie plików {#replace-only-when-the-expected-number-of-files-matches}

Zatrzymaj się przed jakąkolwiek zmianą, gdy wzorzec występuje w innej liczbie plików niż oczekiwana.

```nurt example
search /oldName\(/ "/home/user/app/src" glob: "**/*.go" |> assert paths: 1 |> replace "newName("
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"x := oldName(1)\n"}]}
```

```text expected
/home/user/app/src/a.go
```

## Zastąpienie tekstu dosłownego {#replace-a-literal-text}

Zastąp tekst dosłowny zawierający znaki specjalne wyrażeń regularnych.

```nurt example
replace "a.b(c)" "x" in: ["/home/user/app/notes.txt"] mode: literal
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/notes.txt","type":"file","content_utf8":"call a.b(c) now\n"}]}
```

```text expected
/home/user/app/notes.txt
```

## Zastąpienie tylko pierwszego wystąpienia w każdym pliku {#replace-only-the-first-occurrence-in-each-file}

Zastąp pierwsze wystąpienie tekstu w każdym pliku, a pozostałe zostaw.

```nurt example
replace "http://" "https://" in: ["/home/user/app/site"] glob: "**/*.html" occurrence: 1
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/site/index.html","type":"file","content_utf8":"http://a http://b\n"}]}
```

```text expected
/home/user/app/site/index.html
```

## Zastąpienie słowa bez naruszania dłuższych identyfikatorów {#replace-a-word-without-touching-longer-identifiers}

Zastąp wyłącznie całe słowo, tak aby dłuższe identyfikatory, które je zawierają, pozostały bez zmian.

```nurt example
replace "id" "uid" in: ["/home/user/app/src"] glob: "**/*.py" whole_word: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.py","type":"file","content_utf8":"id = 1\nvalid = 2\n"}]}
```

```text expected
/home/user/app/src/a.py
```

## Zastąpienie z weryfikacją budowaniem {#replace-and-verify-with-a-build}

Zastąp nazwę i uruchom budowanie w tym samym programie; gdy budowanie zakończy się błędem, zmiany są wycofywane.

```nurt example
search /oldName\(/ "/home/user/app/src" glob: "**/*.go"
|> replace "newName("
|> verify (run ["go", "build", "./..."] cwd: "/home/user/app" |> assert exit_code: 0)
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"x := oldName(1)\n"}]}
```

```text expected
/home/user/app/src/a.go
```

## Podgląd zastąpienia {#preview-a-replacement}

Oblicz cele zastąpienia bez zmieniania czegokolwiek.

```nurt example
replace /oldName\(/ "newName(" in: ["/home/user/app/src"] glob: "**/*.go"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"x := oldName(1)\n"}]}
```

```text expected
/home/user/app/src/a.go
```

Parametr wywołania żądania `preview: true` zmienia program w podgląd.

## Usunięcie pasujących wierszy z wielu plików {#remove-the-matching-lines-of-many-files}

Usuń z pliku wiersze zawierające instrukcję debugowania.

```nurt example
lines "/home/user/app/app.js" |> filter text ~ /console\.log/ |> remove
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.js","type":"file","content_utf8":"a();\nconsole.log(1);\n"}]}
```

```text expected
/home/user/app/app.js
```

```bash
sed -i '/console\.log/d' /home/user/app/app.js
```

## Wstawienie wiersza po pasującym wierszu {#insert-a-line-after-a-matching-line}

Wstaw import po klauzuli package w pliku Go.

```nurt example
lines "/home/user/app/main.go" |> filter text == "package main" |> insert r#"import "fmt""# position: after
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n"}]}
```

```text expected
/home/user/app/main.go
```

```bash
sed -i '/^package main$/a import "fmt"' /home/user/app/main.go
```

## Dopisanie wiersza do pliku {#append-a-line-to-a-file}

Dopisz jeden wiersz do istniejącego pliku.

```nurt example
write "/home/user/app/.gitignore" "node_modules/\n" mode: append
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/.gitignore","type":"file","content_utf8":"dist/\n"}]}
```

```text expected
/home/user/app/.gitignore
```

```bash
echo "node_modules/" >> /home/user/app/.gitignore
```

## Sortowanie pliku w miejscu {#sort-a-file-in-place}

Posortuj wiersze pliku i zapisz wynik do tego samego pliku.

```nurt example
lines "/home/user/app/names.txt" |> sort text |> write "/home/user/app/names.txt" mode: overwrite
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/names.txt","type":"file","content_utf8":"b\na\n"}]}
```

```text expected
/home/user/app/names.txt
```

## Zmiana zakończeń wierszy w wielu plikach {#change-the-line-endings-of-many-files}

Przekonwertuj pliki tekstowe na zakończenia wierszy LF.

```nurt example
convert "/home/user/app/a.txt" line_ending: lf
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/a.txt","type":"file","content_utf8":"a\r\nb\r\n"}]}
```

```text expected
/home/user/app/a.txt
```

```bash
dos2unix /home/user/app/a.txt
```

## Konwersja kodowania pliku {#convert-the-encoding-of-a-file}

Przekonwertuj plik z Windows-1250 na UTF-8.

```nurt example
convert "/home/user/legacy/plik.txt" encoding: "utf-8"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/legacy/plik.txt","type":"file","content_utf8":"text\n"}]}
```

```text expected
/home/user/legacy/plik.txt
```

```bash
iconv -f windows-1250 -t utf-8 /home/user/legacy/plik.txt
```

## Zastosowanie łatki {#apply-a-patch}

Zastosuj diff w formacie unified do katalogu projektu.

```nurt example
patch "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+two\n" root: "/home/user/app"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/a.txt","type":"file","content_utf8":"one\n"}]}
```

```text expected
/home/user/app/a.txt
```

```bash
patch -p1 -d /home/user/app < change.diff
```

## Zastąpienie bez względu na wielkość liter {#replace-regardless-of-letter-case}

Zastąp nazwę produktu w każdym pliku Markdown niezależnie od wielkości liter.

```nurt example
replace "acme" "Acme" in: ["/home/user/app/docs"] glob: "**/*.md" case_sensitive: false
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/docs/a.md","type":"file","content_utf8":"ACME and acme\n"}]}
```

```text expected
/home/user/app/docs/a.md
```

```bash
grep -rli 'acme' --include='*.md' /home/user/app/docs | xargs sed -i 's/acme/Acme/gI'
```

## Zastąpienie tylko w komentarzach {#replace-only-inside-comments}

Zmień pisownię w komentarzach i pozostaw kod bez zmian.

```nurt example
replace "colour" "color" in: ["/home/user/app/src"] glob: "**/*.go" scope: comments
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"// colour of the text\nvar colour = 1\n"}]}
```

```text expected
/home/user/app/src/a.go
```

## Zastąpienie obejmujące wiele wierszy {#replace-across-lines}

Połącz wywołanie podzielone na dwa wiersze.

```nurt example
replace /open\(\n\s+path\)/ "open(path)" in: ["/home/user/app/src"] glob: "**/*.js" multiline: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.js","type":"file","content_utf8":"open(\n  path)\n"}]}
```

```text expected
/home/user/app/src/a.js
```

## Zastąpienie z ograniczeniem liczby zmian {#replace-with-a-bound-on-the-number-of-changes}

Zatrzymaj się przed jakąkolwiek zmianą, gdy wzorzec występuje mniej niż dwa razy lub więcej niż pięć razy.

```nurt example
replace "timeout = 30" "timeout = 60" in: ["/home/user/app/conf"] glob: "*.conf" expect: 2..5
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/conf/a.conf","type":"file","content_utf8":"timeout = 30\n"},{"path":"/home/user/app/conf/b.conf","type":"file","content_utf8":"timeout = 30\n"}]}
```

```text expected
/home/user/app/conf/a.conf
```

## Zastąpienie w plikach jednego języka {#replace-in-files-of-one-language}

Zastąp nazwę w plikach jednego języka, gdziekolwiek się znajdują.

```nurt example
replace "getUser" "fetchUser" in: ["/home/user/app"] file_type: ["ts"]
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.ts","type":"file","content_utf8":"getUser(1)\n"}]}
```

```text expected
/home/user/app/src/a.ts
```

## Zastąpienie z poszanowaniem reguł ignorowania {#replace-and-respect-the-ignore-rules}

Zastąp nazwę w plikach, których repozytorium nie ignoruje.

```nurt example
replace "v1/api" "v2/api" in: ["/home/user/app"] gitignore: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/client.py","type":"file","content_utf8":"url = 'v1/api'\n"}]}
```

```text expected
/home/user/app/client.py
```

## Usunięcie wierszy według numeru {#remove-lines-by-number}

Usuń dwa pierwsze wiersze pliku, na przykład baner.

```nurt example
lines "/home/user/app/export.csv" 1..2 |> remove
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/export.csv","type":"file","content_utf8":"banner\ngenerated\nid,name\n"}]}
```

```text expected
/home/user/app/export.csv
```

```bash
sed -i '1,2d' /home/user/app/export.csv
```

## Wstawienie nagłówka na początku pliku {#insert-a-header-at-the-start-of-a-file}

Wstaw wiersz shebang na początku skryptu.

```nurt example
insert "#!/bin/sh" in: "/home/user/app/run.sh" position: start
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/run.sh","type":"file","content_utf8":"echo run\n"}]}
```

```text expected
/home/user/app/run.sh
```

```bash
sed -i '1i #!/bin/sh' /home/user/app/run.sh
```

## Wstawienie wiersza na końcu pliku {#insert-a-line-at-the-end-of-a-file}

Wstaw wiersz zamykający na końcu pliku.

```nurt example
insert "# end of file" in: "/home/user/app/app.conf" position: end ensure_newline: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.conf","type":"file","content_utf8":"a = 1"}]}
```

```text expected
/home/user/app/app.conf
```

## Skopiowanie niepustych wierszy do innego pliku {#copy-the-non-empty-lines-to-another-file}

Zapisz niepuste wiersze pliku do nowego pliku.

```nurt example
lines "/home/user/app/in.txt" |> filter text != "" |> write "/home/user/app/out.txt" mode: create
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/in.txt","type":"file","content_utf8":"a\n\nb\n"}]}
```

```text expected
/home/user/app/out.txt
```

```bash
grep -v '^$' /home/user/app/in.txt > /home/user/app/out.txt
```

## Zapis wielu plików w jednej transakcji {#write-several-files-in-one-transaction}

Utwórz dwa pliki razem; gdy jednego z nich nie można utworzyć, nie powstaje żaden.

```nurt example
write files: [{path: "/home/user/app/a.txt", text: "one\n"}, {path: "/home/user/app/b.txt", text: "two\n"}] mode: create
```

```text expected
/home/user/app/a.txt
```

## Konwersja wszystkich plików tekstowych na LF {#convert-all-text-files-to-lf}

Przekonwertuj zakończenia wierszy każdego pliku tekstowego w katalogu.

```nurt example
find "/home/user/app/docs" glob: "**/*.txt" |> convert line_ending: lf
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/docs/a.txt","type":"file","content_utf8":"a\r\n"}]}
```

```text expected
/home/user/app/docs/a.txt
```

## Dodanie znacznika kolejności bajtów {#add-a-byte-order-mark}

Dodaj znacznik kolejności bajtów do pliku odczytywanego przez starszy program.

```nurt example
convert "/home/user/legacy/list.csv" bom: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/legacy/list.csv","type":"file","content_utf8":"a;b\n"}]}
```

```text expected
/home/user/legacy/list.csv
```

## Odwrócenie łatki {#revert-a-patch}

Zastosuj diff w formacie unified w odwrotnym kierunku, aby cofnąć opisywaną przez niego zmianę.

```nurt example
patch "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+two\n" root: "/home/user/app" reverse: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/a.txt","type":"file","content_utf8":"two\n"}]}
```

```text expected
/home/user/app/a.txt
```

```bash
patch -R -p1 -d /home/user/app < change.diff
```

## Zastosowanie łatki z pliku {#apply-a-patch-from-a-file}

Zastosuj diff zapisany w pliku i usuń jeden początkowy składnik ścieżki.

```nurt example
patch from: "/home/user/dl/change.diff" root: "/home/user/app" strip: 1
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/dl/change.diff","type":"file","content_utf8":"--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+two\n"},{"path":"/home/user/app/a.txt","type":"file","content_utf8":"one\n"}]}
```

```text expected
/home/user/app/a.txt
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
