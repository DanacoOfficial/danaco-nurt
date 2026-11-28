---
category: edit-single-file
title: Edycja pojedynczego pliku
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/edit-single-file.md -->
<!-- Responsibility:   Documents Editing a single file. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Edycja pojedynczego pliku {#editing-a-single-file}

## Tworzenie pliku {#create-a-file}

Utwórz nowy plik o podanej zawartości; program kończy się błędem, gdy plik istnieje.

```nurt example
write "/home/user/app/notes.txt" "first\n" mode: create
```

```text expected
/home/user/app/notes.txt
```

## Nadpisanie pliku {#overwrite-a-file}

Zastąp całą zawartość istniejącego pliku.

```nurt example
write "/home/user/app/notes.txt" "second\n" mode: overwrite
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/notes.txt","type":"file","content_utf8":"first\n"}]}
```

```text expected
/home/user/app/notes.txt
```

## Tworzenie pliku z wieloma wierszami {#create-a-file-with-several-lines}

Utwórz plik wielowierszowy bez zmiany znaczenia cudzysłowów.

```nurt example
write "/home/user/app/Dockerfile" r#"FROM alpine:3.20
RUN apk add --no-cache curl
CMD ["sh"]
"# mode: create
```

```text expected
/home/user/app/Dockerfile
```

```bash
cat > /home/user/app/Dockerfile <<'EOF'
FROM alpine:3.20
RUN apk add --no-cache curl
CMD ["sh"]
EOF
```

## Dopisanie wiersza na początku pliku {#prepend-a-line-to-a-file}

Dodaj wiersz nagłówka na początku pliku.

```nurt example
write "/home/user/app/notes.txt" "# Notes\n" mode: prepend
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/notes.txt","type":"file","content_utf8":"a\n"}]}
```

```text expected
/home/user/app/notes.txt
```

## Wstawienie wiersza pod podanym numerem {#insert-a-line-at-a-line-number}

Wstaw komentarz przed wierszem 3 pliku.

```nurt example
insert "// updated" in: "/home/user/app/main.go" line: 3 position: before
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"a\nb\nc\n"}]}
```

```text expected
/home/user/app/main.go
```

## Zastosowanie kilku zmian jako jednej transakcji {#apply-several-changes-as-one-transaction}

Zastosuj trzy zmiany w jednym pliku; gdy któraś z nich nie daje się zastosować, nie zostanie zastosowana żadna.

```nurt example
edit "/home/user/app/main.go" changes: [
  {replace: "oldName(", with: "newName(", occurrence: 1},
  {insert: "// updated", line: 3, position: before},
  {delete_lines: 10..12}
] expect: 3
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"oldName(1)\n"}]}
```

```text expected
/home/user/app/main.go
```

## Ustawienie klucza w pliku JSON {#set-a-key-in-a-json-file}

Ustaw jeden klucz pliku JSON i zachowaj układ pozostałej części.

```nurt example
set "/home/user/app/package.json" key: "scripts.test" value: "vitest"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/package.json","type":"file","content_utf8":"{\n  \"scripts\": {\n    \"test\": \"jest\"\n  }\n}\n"}]}
```

```text expected
/home/user/app/package.json
```

```bash
jq '.scripts.test = "vitest"' package.json > p.tmp && mv p.tmp package.json
```

## Ustawienie klucza w pliku TOML {#set-a-key-in-a-toml-file}

Zmień wersję w manifeście TOML i zachowaj jego komentarze.

```nurt example
set "/home/user/app/Cargo.toml" key: "package.version" value: "0.4.0"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/Cargo.toml","type":"file","content_utf8":"[package]\nversion = \"0.3.9\" # bump on release\n"}]}
```

```text expected
/home/user/app/Cargo.toml
```

## Ustawienie kilku kluczy naraz {#set-several-keys-at-once}

Ustaw kilka kluczy pliku YAML w jednej operacji.

```nurt example
set "/home/user/app/config.yaml" values: {"server.port": 8080, "server.debug": false}
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/config.yaml","type":"file","content_utf8":"server:\n  port: 80\n"}]}
```

```text expected
/home/user/app/config.yaml
```

## Odczyt klucza dokumentu strukturalnego {#read-a-key-of-a-structured-document}

Odczytaj jedną wartość z dokumentu JSON.

```nurt example
read "/home/user/app/package.json" key: "scripts.test"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/package.json","type":"file","content_utf8":"{\"scripts\":{\"test\":\"jest\"}}"}]}
```

```text expected
jest
```

## Zapytanie do dokumentu {#query-a-document}

Wybierz nazwy wszystkich zależności manifestu JSON.

```nurt example
read "/home/user/app/package.json" |> query ".dependencies | keys[]"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/package.json","type":"file","content_utf8":"{\"dependencies\":{\"left-pad\":\"1.0.0\"}}"}]}
```

```text expected
left-pad
```

## Wykonanie zapytania w bazie SQLite {#run-a-query-on-a-sqlite-database}

Odczytaj aktywnych użytkowników z bazy SQLite.

```nurt example
sql "/home/user/app/app.db" "select id, name from users where active = ? limit 5" params: [1]
```

```text expected
id
```

```bash
sqlite3 /home/user/app/app.db "select id, name from users where active = 1 limit 5"
```

## Zapis pliku tylko wtedy, gdy w międzyczasie się nie zmienił {#write-a-file-only-when-it-did-not-change-meanwhile}

Zastąp tekst tylko wtedy, gdy plik nadal ma zawartość odczytaną wcześniej.

```nurt example
replace "debug" "info" in: ["/home/user/app/app.conf"] if_match: "1a2b3c4d"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.conf","type":"file","content_utf8":"level = debug\n"}]}
```

```text expected
E_STALE
```

## Zapis wartości rejestru Windows {#write-a-windows-registry-value}

Zapisz wartość DWORD w rejestrze bieżącego użytkownika.

```nurt example
write "registry:/HKCU/Software/Example/Mode" "3" value_type: dword mode: overwrite
```

```text expected
registry:/HKCU/Software/Example/Mode
```

```powershell
Set-ItemProperty HKCU:\Software\Example -Name Mode -Value 3 -Type DWord
```

## Dopisanie wiersza do pliku logu {#append-a-line-to-a-log-file}

Dopisz wiersz na końcu pliku i zachowaj jego zawartość.

```nurt example
write "/home/user/app/notes.txt" "done\n" mode: append ensure_newline: true
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/notes.txt","type":"file","content_utf8":"start"}]}
```

```text expected
/home/user/app/notes.txt
```

```bash
printf '\ndone\n' >> /home/user/app/notes.txt
```

## Tworzenie pliku z zakończeniami wierszy Windows {#create-a-file-with-windows-line-endings}

Utwórz plik wsadowy z zakończeniami wierszy CRLF.

```nurt example
write "/home/user/app/run.cmd" "@echo off\necho run\n" mode: create line_ending: crlf
```

```text expected
/home/user/app/run.cmd
```

## Tworzenie pliku w nieistniejącym katalogu {#create-a-file-in-a-missing-directory}

Utwórz plik razem z katalogami jego ścieżki.

```nurt example
write "/home/user/app/out/logs/today.txt" "start\n" mode: create parents: true
```

```text expected
/home/user/app/out/logs/today.txt
```

```bash
mkdir -p /home/user/app/out/logs && printf 'start\n' > /home/user/app/out/logs/today.txt
```

## Zapis pliku JSON {#write-a-json-file}

Zapisz plik JSON; dokument, który nie jest poprawnym JSON, zostaje odrzucony.

```nurt example
write "/home/user/app/settings.json" r#"{"debug": false}"# mode: create format: json
```

```text expected
/home/user/app/settings.json
```

## Zastąpienie tekstu w jednym pliku {#replace-a-text-in-one-file}

Zastąp jeden tekst w jednym pliku.

```nurt example
replace "localhost" "db.internal" in: ["/home/user/app/app.conf"]
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.conf","type":"file","content_utf8":"host = localhost\n"}]}
```

```text expected
/home/user/app/app.conf
```

```bash
sed -i 's/localhost/db.internal/g' /home/user/app/app.conf
```

## Wstawienie wiersza po ostatnim wierszu {#insert-a-line-after-the-last-line}

Wstaw wiersz na końcu pliku.

```nurt example
insert "exit 0" in: "/home/user/app/run.sh" position: end
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/run.sh","type":"file","content_utf8":"echo run\n"}]}
```

```text expected
/home/user/app/run.sh
```

## Usunięcie zakresu wierszy {#delete-a-range-of-lines}

Usuń wiersze od 10 do 12 pliku.

```nurt example
edit "/home/user/app/main.go" changes: [{delete_lines: 10..12}]
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\n"}]}
```

```text expected
/home/user/app/main.go
```

```bash
sed -i '10,12d' /home/user/app/main.go
```

## Edycja pliku tylko wtedy, gdy zawartość się zgadza {#edit-a-file-only-when-the-content-matches}

Zastosuj zmianę tylko wtedy, gdy plik nadal ma zawartość odczytaną wcześniej.

```nurt example
edit "/home/user/app/main.go" changes: [{replace: "oldName(", with: "newName("}] if_match: "1a2b3c4d"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/main.go","type":"file","content_utf8":"oldName(1)\n"}]}
```

```text expected
E_STALE
```

## Usunięcie klucza pliku konfiguracyjnego {#remove-a-key-of-a-configuration-file}

Usuń jeden klucz pliku JSON.

```nurt example
read "/home/user/app/package.json" key: "scripts.lint" |> remove
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/package.json","type":"file","content_utf8":"{\"scripts\":{\"lint\":\"eslint .\",\"test\":\"jest\"}}"}]}
```

```text expected
/home/user/app/package.json
```

## Ustawienie wartości w pliku INI {#set-a-value-in-an-ini-file}

Ustaw wartość w pliku INI i zachowaj pozostałe wiersze.

```nurt example
set "/home/user/app/app.ini" key: "database.host" value: "db.internal" format: ini
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/app.ini","type":"file","content_utf8":"[database]\nhost = localhost\n"}]}
```

```text expected
/home/user/app/app.ini
```

## Ustawienie zmiennej pliku środowiskowego {#set-a-variable-of-an-environment-file}

Zmień zmienną pliku dotenv.

```nurt example
set "/home/user/app/.env" key: "LOG_LEVEL" value: "debug" format: env
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/.env","type":"file","content_utf8":"LOG_LEVEL=info\n"}]}
```

```text expected
/home/user/app/.env
```

## Odczyt klucza pliku YAML {#read-a-key-of-a-yaml-file}

Odczytaj jedną zagnieżdżoną wartość pliku YAML.

```nurt example
read "/home/user/app/config.yaml" key: "server.port"
```

```json fs-before
{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/app/config.yaml","type":"file","content_utf8":"server:\n  port: 8080\n"}]}
```

```text expected
8080
```

## Odczyt wierszy pliku {#read-lines-of-a-file}

Odczytaj wiersze od 5 do 15 pliku wraz z ich numerami.

```nurt example
read "/home/user/app/main.go" lines: 5..15 line_numbers: true
```

```text expected
main.go
```

```bash
sed -n '5,15p' /home/user/app/main.go | cat -n
```

## Odczyt ostatnich wierszy pliku {#read-the-last-lines-of-a-file}

Odczytaj ostatnie dwadzieścia wierszy logu.

```nurt example
read "/home/user/app/app.log" last: 20
```

```text expected
app.log
```

```bash
tail -n 20 /home/user/app/app.log
```

## Zapis do bazy SQLite {#write-to-a-sqlite-database}

Wstaw wiersz do bazy SQLite.

```nurt example
sql "/home/user/app/app.db" "insert into users (name, active) values (?, ?)" params: ["ann", 1]
```

```text expected
/home/user/app/app.db
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
