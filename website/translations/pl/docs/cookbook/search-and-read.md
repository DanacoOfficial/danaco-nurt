---
category: search-and-read
title: Wyszukiwanie i odczyt
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/search-and-read.md -->
<!-- Responsibility:   Documents Searching and reading. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Wyszukiwanie i odczyt {#searching-and-reading}

## Wyświetlanie wpisów katalogu {#list-the-entries-of-a-directory}

Wyświetl pliki i katalogi znajdujące się bezpośrednio w katalogu projektu.

```nurt example
list "/home/user/app"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n"},{"path":"/home/user/app/docs","type":"dir"}]}
```

```text expected
/home/user/app/main.go
```

```bash
ls -la /home/user/app
```

```powershell
Get-ChildItem C:\Users\user\app
```

## Wyszukiwanie największych plików Go zmienionych w ostatniej dobie {#find-the-largest-go-files-changed-in-the-last-day}

Znajdź pliki Go większe niż 1 MB, zmienione w ciągu ostatnich 24 godzin, od największego.

```nurt example
find "/home/user/app" glob: "**/*.go"
|> filter size > 1mb and modified > ago(24h)
|> sort size order: desc
|> first 20
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/gen/schema.go","type":"file","content_utf8":"package gen\n"}]}
```

```text expected
matched 0
```

```bash
find /home/user/app -name '*.go' -size +1M -mtime -1 -printf '%s %p\n' | sort -rn | head -20
```

```powershell
Get-ChildItem C:\Users\user\app -Recurse -Filter *.go | Where-Object { $_.Length -gt 1MB -and $_.LastWriteTime -gt (Get-Date).AddDays(-1) } | Sort-Object Length -Descending | Select-Object -First 20
```

## Zliczanie plików jednego typu {#count-the-files-of-one-type}

Policz pliki TypeScript projektu.

```nurt example
find "/home/user/app" glob: "**/*.ts" |> count
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/a.ts","type":"file","content_utf8":""},{"path":"/home/user/app/b.ts","type":"file","content_utf8":""}]}
```

```text expected
2
```

```bash
find /home/user/app -name '*.ts' | wc -l
```

## Wyszukiwanie plików według fragmentu nazwy {#find-files-by-a-name-fragment}

Znajdź pliki, których nazwa zawiera słowo `config`, w dowolnym katalogu.

```nurt example
find "/home/user/app" glob: "**/*config*"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/config.toml","type":"file","content_utf8":""},{"path":"/home/user/app/src/main.rs","type":"file","content_utf8":""}]}
```

```text expected
/home/user/app/config.toml
```

```bash
find /home/user/app -name '*config*'
```

## Wyszukiwanie wzorca tekstowego w plikach źródłowych {#find-a-text-pattern-in-source-files}

Znajdź każde wywołanie funkcji w plikach Go katalogu.

```nurt example
search /oldName\(/ "/home/user/app/src" glob: "**/*.go"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"x := oldName(1)\n"}]}
```

```text expected
/home/user/app/src/a.go
```

```bash
grep -rn 'oldName(' --include='*.go' /home/user/app/src
```

```powershell
Get-ChildItem C:\Users\user\app\src -Recurse -Filter *.go | Select-String 'oldName\('
```

## Wyświetlanie plików zawierających wzorzec {#list-the-files-that-contain-a-pattern}

Wyświetl same ścieżki plików zawierających wzorzec, bez pasujących wierszy.

```nurt example
search /pending/ "/home/user/app/src" output: files
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"// pending\n"},{"path":"/home/user/app/src/b.go","type":"file","content_utf8":"x\n"}]}
```

```text expected
/home/user/app/src/a.go
```

```bash
grep -rl 'pending' /home/user/app/src
```

## Wyszukiwanie bez rozróżniania wielkości liter {#search-without-regard-to-letter-case}

Wyszukaj słowo w pliku logu i zaakceptuj dowolną wielkość liter.

```nurt example
search "error" "/home/user/app/app.log" case_sensitive: false
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.log","type":"file","content_utf8":"ok\nERROR disk full\n"}]}
```

```text expected
disk full
```

```bash
grep -in 'error' /home/user/app/app.log
```

## Wyświetlanie kontekstu wokół trafienia {#show-context-around-a-match}

Pokaż trzy wiersze przed każdym trafieniem wzorca i po nim.

```nurt example
search /panic/ "/home/user/app/app.log" context: 3
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.log","type":"file","content_utf8":"a\nb\npanic: boom\nc\n"}]}
```

```text expected
panic: boom
```

```bash
grep -C 3 'panic' /home/user/app/app.log
```

## Odczyt pliku {#read-a-file}

Odczytaj całą zawartość pliku tekstowego.

```nurt example
read "/home/user/app/README.md"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/README.md","type":"file","content_utf8":"# App\n"}]}
```

```text expected
# App
```

```bash
cat /home/user/app/README.md
```

```powershell
Get-Content C:\Users\user\app\README.md
```

## Odczyt zakresu wierszy {#read-a-range-of-lines}

Odczytaj wiersze od 10 do 20 pliku źródłowego wraz z numerami wierszy.

```nurt example
read "/home/user/app/main.go" lines: 10..20 line_numbers: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n"}]}
```

```text expected
main.go
```

```bash
sed -n '10,20p' /home/user/app/main.go
```

## Odczyt ostatnich wierszy logu {#read-the-last-lines-of-a-log}

Odczytaj ostatnie 50 wierszy pliku logu.

```nurt example
read "/home/user/app/app.log" last: 50
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.log","type":"file","content_utf8":"started\n"}]}
```

```text expected
started
```

```bash
tail -n 50 /home/user/app/app.log
```

```powershell
Get-Content C:\Users\user\app\app.log -Tail 50
```

## Odczyt wierszy wokół wzorca {#read-the-lines-around-a-pattern}

Odczytaj wiersze wokół pierwszego wiersza pasującego do wzorca.

```nurt example
read "/home/user/app/main.go" around: /func main/ context: 3
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n\nfunc main() {}\n"}]}
```

```text expected
func main
```

## Wyświetlanie metadanych pliku {#show-the-metadata-of-a-file}

Pokaż rozmiar, czasy i uprawnienia pliku wraz z jego skrótem SHA-256.

```nurt example
info "/home/user/app/main.go" hash: sha256
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n"}]}
```

```text expected
/home/user/app/main.go
```

```bash
stat /home/user/app/main.go && sha256sum /home/user/app/main.go
```

## Zliczanie wierszy pliku {#count-the-lines-of-a-file}

Policz wiersze pliku.

```nurt example
lines "/home/user/app/main.go" |> count
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"a\nb\nc\n"}]}
```

```text expected
3
```

```bash
wc -l /home/user/app/main.go
```

## Wyświetlanie katalogu według czasu modyfikacji {#list-a-directory-by-modification-time}

Wyświetl wpisy katalogu, od ostatnio zmodyfikowanego.

```nurt example
list "/home/user/app" sort: modified order: desc
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n"},{"path":"/home/user/app/go.mod","type":"file","content_utf8":"module app\n"}]}
```

```text expected
/home/user/app/main.go
```

```bash
ls -lt /home/user/app
```

## Wyszukiwanie samych katalogów {#find-only-directories}

Znajdź katalogi projektu, których nazwa zawiera `test`.

```nurt example
find "/home/user/app" type: directory glob: "**/*test*"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/tests","type":"dir"},{"path":"/home/user/app/src","type":"dir"}]}
```

```text expected
/home/user/app/tests
```

```bash
find /home/user/app -type d -name '*test*'
```

## Wyszukiwanie plików z pominięciem katalogów zależności i budowania {#find-files-without-dependency-and-build-directories}

Znajdź pliki JavaScript i pomiń katalogi zależności oraz budowania.

```nurt example
find "/home/user/app" glob: "**/*.js" exclude_preset: [deps, build]
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/index.js","type":"file","content_utf8":""},{"path":"/home/user/app/node_modules/x/index.js","type":"file","content_utf8":""}]}
```

```text expected
/home/user/app/index.js
```

## Wyszukiwanie całego słowa {#search-for-a-whole-word}

Znajdź słowo `id` i pomiń dłuższe słowa, które je zawierają.

```nurt example
search "id" "/home/user/app/src" whole_word: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"x := id\ny := width\n"}]}
```

```text expected
x := id
```

```bash
grep -rnw 'id' /home/user/app/src
```

## Zliczanie trafień w każdym pliku {#count-the-matches-per-file}

Policz wiersze zawierające wzorzec w każdym pliku.

```nurt example
search /deprecated/ "/home/user/app/src" output: counts
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"// deprecated a\n// deprecated b\n"}]}
```

```text expected
/home/user/app/src/a.go
```

```bash
grep -rc 'deprecated' /home/user/app/src
```

## Wyświetlanie plików bez wzorca {#list-the-files-without-a-pattern}

Wyświetl pliki, które nie zawierają informacji o licencji.

```nurt example
search /Copyright/ "/home/user/app/src" glob: "**/*.go" output: unmatched
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/src/a.go","type":"file","content_utf8":"// Copyright 2026\n"},{"path":"/home/user/app/src/b.go","type":"file","content_utf8":"package b\n"}]}
```

```text expected
/home/user/app/src/b.go
```

```bash
grep -rL 'Copyright' --include='*.go' /home/user/app/src
```

## Wyodrębnienie samego pasującego tekstu {#extract-only-the-matching-text}

Wyodrębnij numery wersji z pliku, bez reszty wiersza.

```nurt example
search /v\d+\.\d+\.\d+/ "/home/user/app/CHANGELOG.md" only_matching: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/CHANGELOG.md","type":"file","content_utf8":"## v1.2.3\n"}]}
```

```text expected
v1.2.3
```

```bash
grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' /home/user/app/CHANGELOG.md
```

## Wyszukiwanie wzorca obejmującego wiele wierszy {#search-a-pattern-that-spans-lines}

Znajdź funkcję, która zaczyna się od pustego ciała.

```nurt example
search /func main\(\) \{\n\}/ "/home/user/app/main.go" multiline: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"func main() {\n}\n"}]}
```

```text expected
func main
```

## Odczyt jednej funkcji {#read-one-function}

Odczytaj definicję funkcji według jej nazwy.

```nurt example
read "/home/user/app/main.go" symbol: "main"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n\nfunc main() {}\n"}]}
```

```text expected
func main
```

## Odczyt zarysu pliku {#read-the-outline-of-a-file}

Odczytaj deklaracje pliku bez ich ciał.

```nurt example
read "/home/user/app/main.go" view: outline
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n\nfunc main() {}\n"}]}
```

```text expected
main
```

## Wyświetlanie metadanych kilku plików {#show-the-metadata-of-several-files}

Pokaż metadane dwóch plików w jednym wywołaniu.

```nurt example
info ["/home/user/app/main.go", "/home/user/app/go.mod"]
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"package main\n"},{"path":"/home/user/app/go.mod","type":"file","content_utf8":"module app\n"}]}
```

```text expected
/home/user/app/go.mod
```

```bash
stat /home/user/app/main.go /home/user/app/go.mod
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
