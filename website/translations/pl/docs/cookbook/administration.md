---
category: administration
title: Administracja, pakiety i harmonogramy
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/administration.md -->
<!-- Responsibility:   Documents Administration, packages and schedules. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Administracja, pakiety i harmonogramy {#administration-packages-and-schedules}

## Wyświetlanie usług {#list-the-services}

Wyświetl usługi, których nazwa pasuje do wzorca.

```nurt example
services /nginx/
```

```text expected
nginx
```

## Restart usługi {#restart-a-service}

Uruchom usługę ponownie.

```nurt example
services name: "nginx" |> restart
```

```text expected
nginx
```

```bash
systemctl restart nginx
```

```powershell
Restart-Service nginx
```

## Włączanie usługi przy starcie systemu {#enable-a-service-at-boot}

Włącz usługę tak, aby uruchamiała się przy starcie systemu.

```nurt example
services name: "nginx" |> enable
```

```text expected
nginx
```

## Zapytania o klasy WMI {#query-wmi-classes}

Odczytaj klasę CIM w systemie Windows.

```nurt example
windows.cim "Win32_OperatingSystem" properties: ["Caption", "Version"]
```

```text expected
Caption
```

```powershell
Get-CimInstance -ClassName Win32_OperatingSystem
```

## Odczyt wartości rejestru {#read-a-registry-value}

Odczytaj wartość rejestru Windows jako ścieżkę.

```nurt example
read "registry:/HKCU/Software/Example/Mode"
```

```text expected
Mode
```

## Wyświetlanie zainstalowanych pakietów {#list-the-installed-packages}

Wyświetl pakiety, dla których dostępna jest nowsza wersja.

```nurt example
package.list outdated: true
```

```text expected
package
```

## Wyszukiwanie pakietu {#search-a-package}

Wyszukaj program w menedżerze pakietów.

```nurt example
package.search "ripgrep"
```

```text expected
ripgrep
```

## Instalacja pakietu {#install-a-package}

Zainstaluj pakiet za pomocą menedżera pakietów hosta.

```nurt example
package.install ["ripgrep"]
```

```text expected
ripgrep
```

```bash
apt install ripgrep
```

## Aktualizacja pakietów {#upgrade-the-packages}

Zaktualizuj wszystkie pakiety hosta.

```nurt example
package.upgrade
```

```text expected
package
```

## Dzienne zadanie w harmonogramie {#schedule-a-daily-job}

Uruchamiaj program kopii zapasowej codziennie o 03:00.

```nurt example
schedule.create "backup" cron: "0 3 * * *" action: (run ["backup"] cwd: "/home/user")
```

```text expected
backup
```

```bash
(crontab -l; echo '0 3 * * * cd /home/user && backup') | crontab -
```

## Wyświetlanie harmonogramów {#list-the-schedules}

Wyświetl aktywne harmonogramy.

```nurt example
schedule.list state: active
```

```text expected
backup
```

## Wstrzymanie harmonogramu {#pause-a-schedule}

Wstrzymaj harmonogram bez jego usuwania.

```nurt example
schedule.pause "s4"
```

```text expected
s4
```

## Natychmiastowe uruchomienie harmonogramu {#run-a-schedule-now}

Uruchom jednokrotnie akcję harmonogramu, poza jego terminem.

```nurt example
schedule.run "s4"
```

```text expected
s4
```

## Historia harmonogramu {#show-the-history-of-a-schedule}

Pokaż ostatnie uruchomienia harmonogramu.

```nurt example
schedule.history "s4" limit: 10
```

```text expected
s4
```

## Wyszukiwanie programu do zadania {#find-a-program-for-a-task}

Przeszukaj programy i pakiety pod kątem zadania.

```nurt example
program.search "ocr pdf" limit: 3
```

```text expected
ocr
```

## Zatrzymanie usługi {#stop-a-service}

Zatrzymaj usługę i poczekaj na jej zakończenie.

```nurt example
services name: "nginx" |> stop grace: 10s
```

```text expected
nginx
```

```bash
systemctl stop nginx
```

```powershell
Stop-Service nginx
```

## Wyłączanie usługi przy starcie systemu {#disable-a-service-at-boot}

Zapobiegnij uruchamianiu usługi przy starcie systemu.

```nurt example
services name: "cups" |> disable
```

```text expected
cups
```

```bash
systemctl disable cups
```

## Uruchomienie usługi i oczekiwanie na nią {#start-a-service-and-wait-for-it}

Uruchom usługę i poczekaj do 30 sekund, aż zacznie działać.

```nurt example
services name: "postgresql" |> start wait: 30s
```

```text expected
postgresql
```

## Wyświetlanie usług bieżącego użytkownika {#list-the-services-of-the-current-user}

Wyświetl usługi menedżera użytkownika zamiast menedżera systemowego.

```nurt example
services scope: user
```

```text expected
service
```

```bash
systemctl --user list-units --type=service
```

## Zapytanie o działające usługi Windows {#query-the-running-windows-services}

Odczytaj dwie właściwości działających usług przez CIM.

```nurt example
windows.cim "Win32_Service" where: "State = 'Running'" properties: ["Name", "State"]
```

```text expected
Name
```

```powershell
Get-CimInstance -ClassName Win32_Service -Filter "State = 'Running'"
```

## Instalacja pakietu w podanej wersji {#install-a-package-in-a-given-version}

Zainstaluj dokładną wersję pakietu za pomocą wybranego menedżera pakietów.

```nurt example
package.install ["nodejs"] version: "20.11.0" manager: apt
```

```text expected
nodejs
```

## Usuwanie pakietu {#remove-a-package}

Usuń pakiet za pomocą menedżera pakietów hosta.

```nurt example
package.remove ["ripgrep"]
```

```text expected
ripgrep
```

```bash
apt remove ripgrep
```

## Aktualizacja wybranych pakietów {#upgrade-selected-packages}

Zaktualizuj dwa pakiety, pozostałe pozostaw bez zmian.

```nurt example
package.upgrade ["openssl", "curl"]
```

```text expected
openssl
```

## Harmonogram w stałym odstępie czasu {#schedule-a-job-at-an-interval}

Uruchamiaj program czyszczący co sześć godzin.

```nurt example
schedule.create "cleanup" every: 6h action: (run ["cleanup"] cwd: "/home/user")
```

```text expected
cleanup
```

## Cotygodniowe zadanie ze strefą czasową {#schedule-a-weekly-job-with-a-time-zone}

Uruchamiaj raport w każdy poniedziałek o 08:00 w podanej strefie czasowej, najwyżej dwanaście razy.

```nurt example
schedule.create "report" cron: "0 8 * * 1" timezone: "Europe/Warsaw" max_runs: 12 action: (run ["report"] cwd: "/home/user")
```

```text expected
report
```

## Wznowienie wstrzymanego harmonogramu {#resume-a-paused-schedule}

Wznów harmonogram, który został wstrzymany.

```nurt example
schedule.resume "s4"
```

```text expected
s4
```

## Usuwanie harmonogramu {#remove-a-schedule}

Usuń harmonogram, który nie jest już potrzebny.

```nurt example
schedule.remove ["s4"]
```

```text expected
s4
```

## Lokalizacja programu {#locate-a-program}

Pokaż, gdzie program jest zainstalowany, oraz jego wersję.

```nurt example
program.find ["git"] versions: true
```

```text expected
git
```

```bash
command -v git && git --version
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
