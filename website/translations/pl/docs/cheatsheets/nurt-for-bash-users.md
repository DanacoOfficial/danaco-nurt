---
title: Nurt dla użytkowników basha
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cheatsheets/nurt-for-bash-users.md -->
<!-- Responsibility:   Documents Nurt for bash users. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Nurt dla użytkowników basha {#nurt-for-bash-users}

Każdy wiersz zestawia polecenie Bash z jego postacią w Nurt. Ostatnia kolumna podaje, czy Nurt poprawia nawyk automatycznie, odrzuca go błędem zawierającym poprawkę, czy odrzuca go błędem bez programu i prosi o program z danego wiersza.

Ścieżki pochodzą z hosta z systemem Linux, a parametr wywołania `base` ma wartość `/home/user/app`.

## Pliki i katalogi {#files-and-directories}

| Reguła | Bash | Nurt | Wynik |
|---|---|---|---|
| B01 | `rm -rf /home/user/app/build` | `remove "/home/user/app/build"` | poprawiane automatycznie: rekurencja jest domyślna, a brakująca ścieżka jest pomijana |
| B02 | `rm /home/user/app/build/*.tmp` | `find "/home/user/app/build" glob: "*.tmp" depth: 1 hidden: false \|> remove on_missing: error` | poprawiane automatycznie: wzorzec glob jest rozwijany tak jak w bashu, bez rekurencji |
| B03 | `mv /home/user/a.txt /home/user/archive` | `move "/home/user/a.txt" to: "/home/user/archive"` | poprawiane automatycznie |
| B04 | `cp -r /home/user/app /home/user/backup` | `copy "/home/user/app" to: "/home/user/backup" preserve: false` | poprawiane automatycznie |
| B05 | `mkdir -p /home/user/app/out` | `create "/home/user/app/out" type: directory` | poprawiane automatycznie |
| B06 | `touch /home/user/app/a.txt` | `create "/home/user/app/a.txt" type: file on_exists: touch` | poprawiane automatycznie |
| B07 | `chmod -R 755 /home/user/app/bin` | `attributes "/home/user/app/bin" mode: "755" recursive: true` | poprawiane automatycznie |
| B09 | `ln -s /home/user/app/src /home/user/link` | `create "/home/user/link" type: link target: "/home/user/app/src"` | poprawiane automatycznie |
| B13 | `ls -la /home/user/app` | `list "/home/user/app"` | poprawiane automatycznie |
| B14 | `find /home/user/app -name '*.go' -type f` | `find "/home/user/app" glob: "**/*.go" type: file` | poprawiane automatycznie |
| B37 | `stat /home/user/app/a.txt` | `info "/home/user/app/a.txt"` | poprawiane automatycznie |

## Odczyt i wyszukiwanie tekstu {#reading-and-searching-text}

| Reguła | Bash | Nurt | Wynik |
|---|---|---|---|
| B10 | `cat /home/user/app/a.txt` | `read "/home/user/app/a.txt"` | poprawiane automatycznie |
| B11 | `tail -n 20 /home/user/app/a.log` | `read "/home/user/app/a.log" last: 20` | poprawiane automatycznie |
| B12 | `wc -l /home/user/app/a.txt` | `lines "/home/user/app/a.txt" \|> count` | poprawiane automatycznie |
| B15 | `grep -rni 'todo' /home/user/app/src` | `search /todo/ "/home/user/app/src" case_sensitive: false` | poprawiane automatycznie: wzorzec w składni podstawowego wyrażenia regularnego jest konwertowany z założeniem, które nazywa raport |
| B16 | `sed -i 's/oldName/newName/g' /home/user/app/a.go` | `replace /oldName/ "newName" in: ["/home/user/app/a.go"]` | poprawiane automatycznie |
| B17 | `sed -n '10,20p' /home/user/app/a.go` | `read "/home/user/app/a.go" lines: 10..20` | poprawiane automatycznie |
| B37b | `sha256sum /home/user/app/a.tar.gz` | `hash "/home/user/app/a.tar.gz"` | poprawiane automatycznie |
| B37c | `diff /home/user/a.txt /home/user/b.txt` | `diff "/home/user/a.txt" "/home/user/b.txt"` | poprawiane automatycznie |

## Zapis tekstu {#writing-text}

| Reguła | Bash | Nurt | Wynik |
|---|---|---|---|
| B19 | `echo "dist/" >> /home/user/app/.gitignore` | `write "/home/user/app/.gitignore" "dist/\n" mode: append` | poprawiane automatycznie |

## Archiwa {#archives}

| Reguła | Bash | Nurt | Wynik |
|---|---|---|---|
| B34 | `tar -xzf /home/user/dl/data.tar.gz -C /home/user/data` | `archive.extract "/home/user/dl/data.tar.gz" into: "/home/user/data"` | poprawiane automatycznie |
| B34b | `gzip /home/user/app/a.log` | `compress "/home/user/app/a.log" keep: false` | poprawiane automatycznie |

## Procesy i system {#processes-and-system}

| Reguła | Bash | Nurt | Wynik |
|---|---|---|---|
| B35a | `ps aux` | `processes` | poprawiane automatycznie |
| B35b | `df -h` | `disks` | poprawiane automatycznie |
| B35c | `ss -ltnp` | `ports protocol: tcp` | poprawiane automatycznie |
| B35d | `printenv` | `environment` | poprawiane automatycznie |
| B35e | `which git` | `program.find "git"` | poprawiane automatycznie |
| B36 | `kill -9 4132` | `processes pid: 4132 \|> stop signal: kill` | poprawiane automatycznie |

## Uruchamianie programów {#running-programs}

| Reguła | Bash | Nurt | Wynik |
|---|---|---|---|
| B29 | `npm run build &` | `run ["npm", "run", "build"] cwd: "/home/user/app" background: true` | błąd bez poprawki: `&` nie jest częścią języka; należy zapisać program z background: true i katalogiem jako cwd |
| B30 | `timeout 60 make test` | `run ["make", "test"] cwd: "/home/user/app" timeout: 60s` | błąd z poprawką: nie ma katalogu bieżącego; poprawka przekazuje katalog jako cwd lub base |
| B20 | `cd /home/user/app && make` | `run ["make"] cwd: "/home/user/app"` | poprawiane automatycznie: konwertowana jest tylko zmiana na katalog bezwzględny w tym samym wierszu |
| B31 | `CI=1 npm test` | `run ["npm", "test"] cwd: "/home/user/app" env: {"CI": "1"}` | błąd z poprawką: nie ma katalogu bieżącego; poprawka przekazuje katalog jako cwd lub base |

## Sieć {#network}

| Reguła | Bash | Nurt | Wynik |
|---|---|---|---|
| B32 | `curl -s https://example.com/health` | `http.request "https://example.com/health" follow_redirects: false` | poprawiane automatycznie |
| B32b | `wget -O /home/user/a.tar.gz https://example.com/a.tar.gz` | `http.download "https://example.com/a.tar.gz" "/home/user/a.tar.gz"` | poprawiane automatycznie |

## Git {#git}

| Reguła | Bash | Nurt | Wynik |
|---|---|---|---|
| B38 | `git -C /home/user/app status` | `git.status "/home/user/app"` | poprawiane automatycznie |

## Usługi i pakiety {#services-and-packages}

| Reguła | Bash | Nurt | Wynik |
|---|---|---|---|
| B40 | `systemctl restart nginx` | `services name: "nginx" \|> restart` | poprawiane automatycznie |
| B40b | `apt install -y ripgrep` | `package.install ["ripgrep"] manager: apt` | poprawiane automatycznie |

## Przepływ sterowania {#control-flow}

| Reguła | Bash | Nurt | Wynik |
|---|---|---|---|
| B22 | `mkdir /home/user/out && cp /home/user/a.txt /home/user/out` | `create "/home/user/out" type: directory parents: false` | błąd z poprawką: dwa polecenia zmieniające nie są uruchamiane w jednym programie; poprawką jest jedno wywołanie na zmianę, to jest pierwsze |
| B25 | `rm -rf "$BUILD"/*` | `find "/home/user/app/build" glob: "*" depth: 1 \|> remove` | błąd bez poprawki: w tekście nie ma zmiennych; należy zapisać katalog |
| B28 | `sudo systemctl restart nginx` | `services name: "nginx" \|> restart privilege: elevated` | poprawiane automatycznie: program działa z parametrem wywołania privilege: elevated; sudo -u NAME daje as_user |

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
