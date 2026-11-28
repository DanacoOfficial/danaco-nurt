---
category: system-and-processes
title: System i procesy
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/system-and-processes.md -->
<!-- Responsibility:   Documents System and processes. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# System i procesy {#system-and-processes}

## Wyświetlanie procesów według pamięci {#list-the-processes-by-memory}

Wyświetl pięć procesów zużywających najwięcej pamięci.

```nurt example
processes |> sort memory order: desc |> first 5
```

```text expected
pid
```

```bash
ps aux --sort=-%mem | head -6
```

## Wyszukiwanie procesu według nazwy {#find-a-process-by-name}

Znajdź procesy, których nazwa pasuje do wzorca.

```nurt example
processes /node/
```

```text expected
node
```

## Wyświetlanie, kto nasłuchuje na porcie {#show-who-listens-on-a-port}

Pokaż proces nasłuchujący na porcie 8080.

```nurt example
ports state: listen |> filter port == 8080
```

```text expected
8080
```

```bash
ss -ltnp | grep :8080
```

## Zatrzymanie procesu {#stop-a-process}

Zatrzymaj proces o podanym identyfikatorze.

```nurt example
processes pid: 4132 |> stop
```

```text expected
4132
```

## Zabicie procesu ignorującego zakończenie {#kill-a-process-that-ignores-termination}

Zabij proces, który nie reaguje na sygnał zakończenia.

```nurt example
processes pid: 4132 |> stop signal: kill
```

```text expected
4132
```

## Wyświetlanie dysków {#show-the-disks}

Pokaż zamontowane woluminy i ich wolne miejsce.

```nurt example
disks
```

```text expected
free
```

```bash
df -h
```

## Wyświetlanie hosta {#show-the-host}

Pokaż system operacyjny, użytkownika i zainstalowane programy.

```nurt example
host programs: ["git", "python3", "node"] versions: true
```

```text expected
git
```

## Wyświetlanie zmiennych środowiskowych {#show-the-environment-variables}

Pokaż zmienne środowiskowe procesu, którego nazwa zawiera `PATH`.

```nurt example
environment /PATH/
```

```text expected
PATH
```

## Wyszukiwanie programu w ścieżce wyszukiwania {#find-the-program-in-the-search-path}

Znajdź, gdzie program jest zainstalowany.

```nurt example
program.find "git"
```

```text expected
git
```

## Wyszukiwanie procesów trzymających otwarty plik {#find-the-processes-that-hold-a-file-open}

Znajdź procesy, które utrzymują otwarty plik.

```nurt example
open_files "/home/user/app/app.log"
```

```text expected
pid
```

## Odczyt dziennika systemowego {#read-the-system-log}

Odczytaj 50 najnowszych zdarzeń błędów dziennika systemowego.

```nurt example
logs level: error limit: 50
```

```text expected
error
```

## Odczyt dziennika jednej jednostki {#read-the-log-of-one-unit}

Odczytaj dziennik jednej jednostki usługi od godziny wstecz.

```nurt example
logs unit: "nginx" since: ago(1h)
```

```text expected
nginx
```

## Wyświetlanie kont {#show-the-accounts}

Wyświetl konta użytkowników hosta.

```nurt example
accounts kind: user
```

```text expected
user
```

## Wstrzymanie procesu {#suspend-a-process}

Wstrzymaj proces do czasu jego wznowienia.

```nurt example
processes pid: 4132 |> suspend
```

```text expected
4132
```

## Wznowienie procesu {#resume-a-process}

Wznów wstrzymany proces.

```nurt example
processes pid: 4132 |> resume
```

```text expected
4132
```

## Wyświetlanie procesów konta {#list-the-processes-of-an-account}

Wyświetl procesy uruchomione dla jednego konta.

```nurt example
processes user: "deploy"
```

```text expected
deploy
```

```bash
ps -u deploy
```

## Wyświetlanie katalogów roboczych procesów {#show-the-working-directory-of-processes}

Pokaż katalogi robocze procesów, których nazwa pasuje do wzorca.

```nurt example
processes /node/ cwd: true
```

```text expected
node
```

## Wyświetlanie portów UDP {#show-the-udp-ports}

Pokaż wszystkie porty używające protokołu UDP, nie tylko nasłuchujące.

```nurt example
ports state: all protocol: udp
```

```text expected
udp
```

```bash
ss -uanp
```

## Wyszukiwanie dysku z najmniejszą ilością wolnego miejsca {#find-the-disk-with-the-least-free-space}

Pokaż wolumin z najmniejszą ilością wolnego miejsca.

```nurt example
disks |> sort free order: asc |> first 1
```

```text expected
free
```

```bash
df -h --output=source,avail | sort -h -k2 | head -2
```

## Wyświetlanie zmiennych konta {#show-the-variables-of-the-account}

Pokaż zmienną środowiskową `PATH` w postaci ustawionej dla konta, a nie dla sesji.

```nurt example
environment "PATH" scope: user
```

```text expected
PATH
```

## Przeszukiwanie ostrzeżeń dziennika systemowego {#search-the-warnings-of-the-system-log}

Pokaż ostrzeżenia z ostatniej doby wspominające o przekroczeniu czasu.

```nurt example
logs level: warning message: /timeout/ since: ago(24h)
```

```text expected
timeout
```

```bash
journalctl -p warning --since '-1 day' | grep -i timeout
```

## Odczyt okna dziennika jednostki {#read-a-window-of-the-log-of-a-unit}

Odczytaj dziennik jednostki między dwoma punktami w czasie.

```nurt example
logs unit: "sshd" since: ago(2h) until: ago(1h)
```

```text expected
sshd
```

## Wyszukiwanie zainstalowanego programu według słowa kluczowego {#find-an-installed-program-by-a-keyword}

Przeszukaj ścieżkę, pakiety i aplikacje pod kątem programów obsługujących JSON.

```nurt example
program.search "json" source: [path, package] limit: 10
```

```text expected
json
```

## Odczyt opisu programu {#read-the-description-of-a-program}

Odczytaj kartę programu wraz z jego przewodnikiem użycia.

```nurt example
program.describe "jq" include_guide: true
```

```text expected
jq
```

## Wyświetlanie wersji zainstalowanych programów {#show-the-versions-of-installed-programs}

Pokaż lokalizacje i wersje kilku programów.

```nurt example
program.find ["git", "make"] versions: true
```

```text expected
git
```

## Zatrzymanie procesu po okresie karencji {#stop-a-process-after-a-grace-period}

Zakończ proces i zabij go, gdy nie zakończy się w ciągu pięciu sekund.

```nurt example
processes /^runaway$/ |> stop grace: 5s
```

```text expected
runaway
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
