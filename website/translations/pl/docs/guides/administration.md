---
name: administration
description: Sprawdzanie i administrowanie systemem: usługi, dzienniki systemowe, procesy, pakiety, konta i dyski.
commands: [services, logs, processes, ports, disks, host, environment, accounts, package.list, package.install, windows.cim, stop]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/administration.md -->
<!-- Responsibility:   Documents administration. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# administracja {#administration}

## Granice {#boundaries}

Polecenia administracyjne odczytują stan systemu, a w niektórych przypadkach go zmieniają. Zmiany częściej niż inna praca wymagają uprawnień administratora, dlatego parametr wywołania `privilege` jest częścią języka, a zmiany poza systemem plików mają operacje odwrotne.

## Praca z podwyższonymi uprawnieniami {#elevated-work}

Praca wymagająca uprawnień administratora przyjmuje parametr wywołania `privilege`: `account` (wartość domyślna) uruchamia z uprawnieniami konta, `elevated` uruchamia z uprawnieniami administratora, a `auto` uruchamia z uprawnieniami konta i powtarza cały etap jednokrotnie z `elevated`, gdy cel kończy się błędem `E_PERMISSION` lub `E_PRIVILEGE`, zanim jakakolwiek zmiana zostanie zachowana. `as_user: "NAME"` uruchamia operację jako inne konto w systemach Linux i macOS. Oba parametry zapisuje się na etapie (`write "/etc/hosts" "..." privilege: elevated`), podaje w żądaniu albo ustawia w wierszu poleceń przez `--privilege` i `--as-user`; klucz konfiguracji `privilege.default` ustawia wartość dla wywołań, które żadnej nie podają. Nurt uruchamia `sudo`, `doas` lub `run0` (albo działa w tym samym procesie, gdy już jest administratorem), wysyła drzewo składni programu, nigdy tekst powłoki, i nigdy nie pyta o hasło: backend, który pytałby o hasło, jest zgłaszany jako `password_required`. Raport operacji podaje `elevated: true`, `backend` oraz, dla `auto`, `elevation_reason`. Gdy żaden backend nie może wykonać operacji, wywołanie kończy się błędem `E_PRIVILEGE`; jego szczegół `backend` wymienia każdy backend wraz z wynikiem, a `next_steps` wskazuje dalszy sposób postępowania. Silnik osadzający wyłącza podwyższanie uprawnień przez `privilege.allowed = false`.

## Wpisy z podwyższonymi uprawnieniami w dzienniku {#elevated-entries-in-the-journal}

Wpisy z podwyższonymi uprawnieniami znajdują się w dzienniku konta, a konto ma dostęp do ich kopii; `undo N` i `redo N` takiego wpisu przywracają cele przez ten sam backend; `undo` i `redo` bez numeru, które przywracałyby wpis z podwyższonymi uprawnieniami, kończą się błędem `E_PRIVILEGE` i następnym krokiem `undo N privilege: elevated`. Proces z podwyższonymi uprawnieniami zapisuje dziennik konta z uprawnieniami konta w systemach Linux i macOS; w systemie Windows backend `self` jest tym samym kontem.

## Cofnięcie zmian poza systemem plików {#undo-of-changes-outside-the-file-system}

Zmiany poza systemem plików są cofane przez operacje odwrotne, które silnik wyprowadza ze stanu zapisanego w raporcie:

| Operacja | Cofnięcie |
|---|---|
| `package.install` | `package.remove`, gdy pakiet nie był wcześniej zainstalowany; ponowna instalacja zapisanej wersji, gdy wersja się zmieniła |
| `package.remove` | `package.install` zapisanej wersji, o ile repozytorium nadal ją oferuje |
| `package.upgrade` | instalacja zapisanych wersji, o ile repozytorium nadal je oferuje |
| `start`, `stop` | polecenie przeciwne |
| `enable`, `disable` | polecenie przeciwne z zapisanym typem uruchamiania |

`enable` i `disable` są cofane tylko dla typów uruchamiania `disabled` i `automatic`; inne typy uruchamiania czynią wpis nieodwracalnym, a powód podaje raport. Wersja pakietu, której repozytorium już nie oferuje, czyni nieodwracalnym wyłącznie ten pakiet.

Nieodwracalne są `restart`, `stop` procesu oraz `run`: pliki zmienione przez program uruchomiony przez `run` z `privilege: elevated` nie są zachowywane, o czym informuje raport.

## Usługi {#services}

`services` wyświetla usługi wraz z ich stanem i typem uruchamiania. `state` zawęża listę, a `scope` wybiera usługi systemowe lub użytkownika. `start`, `restart`, `enable` i `disable` przyjmują usługi z wejścia, a `stop` kończy usługi, procesy i zadania.

```nurt
services state: "running"
services name: "nginx" |> restart
```

## Procesy i porty {#processes-and-ports}

`processes` wyświetla uruchomione procesy, a `ports` wyświetla porty wraz z procesami, które je zajmują. `open_files` wskazuje procesy, które mają otwarty dany plik, co wyjaśnia `E_IN_USE`.

```nurt
processes name: "node" |> filter memory > 500mb |> select pid name memory
ports state: listen |> filter port == 8080
```

## Dzienniki systemowe {#logs}

`logs` odczytuje zdarzenia z dziennika systemowego: dziennika systemd, Dziennika zdarzeń systemu Windows lub ujednoliconego dziennika macOS. `level` ustawia najniższy poziom ważności, `since` i `until` ograniczają czas, a `message` filtruje tekst.

```nurt
logs level: error since: ago(1h) limit: 50
```

## Pakiety i konta {#packages-and-accounts}

`package.list` i `package.search` odczytują repozytoria pakietów, a `package.install`, `package.remove` i `package.upgrade` zmieniają zainstalowane pakiety za pośrednictwem menedżera systemu. `accounts` wyświetla konta użytkowników i grupy, `host` opisuje system operacyjny, a `environment` wyświetla zmienne wraz z zakresem, w którym są ustawione. W systemie Windows `windows.cim` odpytuje klasy CIM.

```nurt
package.list outdated: true
accounts kind: group
host
```

## Dyski {#disks}

`disks` wyświetla zamontowane woluminy wraz z ich pojemnością i wolnym miejscem. Polecenie `journal.space` podaje, ile z tego miejsca zajmuje dziennik.

```nurt
disks |> filter free < 10gib
```

## Natywne polecenia administracyjne {#native-administration-commands}

Każde z poniższych poleceń zmienia system, przyjmuje `privilege`, zgłasza swój skutek i zapisuje poprzedni stan w dzienniku. Operacja odwrotna to operacja, którą wykonuje `undo`. Polecenia są ładowane z grupą `administration` opisu narzędzia, więc sesja, która nie administruje systemem, ich nie zawiera.

| Grupa | Polecenia | Systemy | Operacja odwrotna |
|---|---|---|---|
| konta | `accounts.create`, `accounts.modify` (powłoka, katalog domowy, grupy, komentarz, blokada), `accounts.remove` (`keep_home: bool`), `accounts.password` | Linux, macOS, Windows | create i remove są wzajemnie odwrotne (katalog domowy jest zachowywany do wygaśnięcia wpisu); `modify` przywraca zapisane wartości; `password` jest nieodwracalne |
| grupy | `groups.create`, `groups.remove`, `groups.add_member`, `groups.remove_member` | Linux, macOS, Windows | parami |
| montowanie | `mount` (urządzenie, cel, typ, opcje, `persistent`), `unmount` | Linux, macOS; litery dysków i montowania folderów w systemie Windows | `mount` i `unmount` |
| jądro | `sysctl.set` (klucz, wartość, `persistent`), `modules.load`, `modules.unload` | Linux; `sysctl` kluczy zapisywalnych w systemie macOS | poprzednia wartość; load i unload |
| zasilanie | `power.restart`, `power.shutdown` (`at`, `delay`, `message`), `power.cancel` | wszystkie | nieodwracalne po wykonaniu; zaplanowane jest cofane przez `power.cancel` |
| dyski | `disks.partition` (create, delete, resize), `disks.format` (system plików, etykieta) | Linux, macOS, Windows | nieodwracalne |

## Sieć, zegar i zarządzanie systemem Windows {#network-clock-and-windows-management}

Polecenia sieci, zegara, usług i obiektów zarządzania systemu Windows podlegają tym samym zasadom: zgłaszają swój skutek, zapisują poprzedni stan i przyjmują `privilege`.

| Grupa | Polecenia | Systemy | Operacja odwrotna |
|---|---|---|---|
| zapora | `firewall.rules` (odczyt), `firewall.allow`, `firewall.deny`, `firewall.remove` (port, protokół, źródło, kierunek, strefa) | nftables, firewalld i ufw w systemie Linux, pf w systemie macOS, Zapora Windows Defender | dodana reguła jest usuwana |
| sieć | `network.route_add`, `network.route_remove`, `network.dns_set` (serwery, domeny wyszukiwania) | wszystkie | poprzednie trasy i DNS |
| zegar | `clock.timezone_set`, `clock.set`, `clock.ntp` (on, off) | wszystkie | poprzednia strefa czasowa i stan NTP; `clock.set` jest nieodwracalne |
| usługi | `start`, `stop`, `restart`, `enable`, `disable` | wszystkie | jak w tabeli powyżej |
| zarządzanie systemem Windows | wywołania metod `windows.cim` (`invoke: <method>`) | Windows | nieodwracalne, chyba że metoda ma udokumentowaną metodę przeciwną |

## Trwałość i podglądy {#persistence-and-previews}

Hasło podaje się jako odwołanie do sekretu silnika, nigdy jako tekst; dziennik go nie przechowuje. `mount` z `persistent: true` zapisuje `/etc/fstab` lub jednostkę montowania systemd, a `sysctl.set` z `persistent: true` zapisuje `/etc/sysctl.d/90-nurt.conf`; oba pliki są rejestrowane w dzienniku jak każdy plik, więc `undo` je przywraca. Podgląd `disks.partition` i `disks.format` wyświetla urządzenie, jego partycje, systemy plików i punkty montowania oraz rozmiar danych, które zostałyby utracone.

```nurt
accounts.create "deploy" shell: "/bin/bash" groups: ["www-data"] privilege: elevated
groups.add_member "www-data" "backup" privilege: elevated
```

## Kilka zmian jako jedna operacja {#several-changes-as-one-operation}

Zmiana systemu rzadko jest jednym poleceniem. `steps` wykonuje kilka potoków zmieniających jako jeden wpis dziennika, w trybie wszystko albo nic; krok oznaczony jako nieodwracalny (restart, program uruchomiony przez `run`, `power.restart`) jest wymieniany przed startem, a po późniejszym niepowodzeniu jest zgłaszany jako niewycofany.

```nurt
steps {
    sysctl.set "net.ipv4.ip_forward" "1" persistent privilege: elevated
    firewall.allow port: 8443 protocol: tcp privilege: elevated
    run ["systemctl", "reload", "app"] cwd: "/" privilege: elevated |> assert exit_code: 0
}
```

`when` wybiera gałąź według stanu systemu, a `each` powtarza zmianę dla listy; `preview` pokazuje wybraną gałąź i każdą iterację bez wprowadzania zmiany. Składnię opisuje [język](../site/language.md).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
