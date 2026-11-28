---
group: administration
summary: Usługi systemowe, dostawca rejestru, CIM, zapora sieciowa, trasy, DNS i zegar.
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Grupa administration

## Zakres {#scope}

Grupa wylicza i kontroluje usługi systemu operacyjnego oraz odpytuje obiekty
zarządzania systemu Windows. Usługi mają jeden model we wszystkich systemach:
menedżer usług systemu (systemd, menedżer sterowania usługami systemu Windows
lub launchd) jest ukryty za rekordem `service`, a polecenia zmieniające
usługę przyjmują rekordy `service` jako wejście.

Rejestr systemu Windows nie jest poleceniem tej grupy. Jest dostawcą ścieżek:
`registry:/HKCU/Software/Example` adresuje klucz, a
`registry:/HKCU/Software/Example/Mode` wartość, i działają na nich polecenia
plikowe grupy core (`list`, `find`, `read`, `write`, `remove`, `copy`,
`move`). Poprzednia wartość jest zachowana w dzienniku operacji, więc zmianę
rejestru wykonaną za pomocą `write` można cofnąć.

| Zadanie | Polecenie | Grupa |
|---|---|---|
| Zatrzymanie usługi | `stop` | core |
| Wstrzymanie lub wznowienie procesu | `suspend`, `resume` | system |
| Odczyt i zmiana wartości rejestru | `read`, `write`, `remove` | core |
| Cofnięcie zmiany rejestru | `undo` | core |

## Polecenia {#commands}

| Polecenie | Kategoria | Wejście i wyjście | Odwracalność | Strona |
|---|---|---|---|---|
| `services` | source | brak do `service` | read | [services](../commands/services.md) |
| `start` | run | `service` do `effect` | reversible_if_preserved | [start](../commands/start.md) |
| `restart` | run | `service` do `effect` | irreversible | [restart](../commands/restart.md) |
| `enable` | change | `service` do `effect` | reversible_if_preserved | [enable](../commands/enable.md) |
| `disable` | change | `service` do `effect` | reversible_if_preserved | [disable](../commands/disable.md) |
| `windows.cim` | change | brak do `cim_object`, `effect` | read | [windows.cim](../commands/windows.cim.md) |
| `firewall.rules` | source | brak do `map` | read | [firewall.rules](../commands/firewall.rules.md) |
| `firewall.allow` | change | brak do `effect` | reversible | [firewall.allow](../commands/firewall.allow.md) |
| `firewall.deny` | change | brak do `effect` | reversible | [firewall.deny](../commands/firewall.deny.md) |
| `firewall.remove` | change | brak do `effect` | reversible | [firewall.remove](../commands/firewall.remove.md) |
| `network.route_add` | change | brak do `effect` | reversible | [network.route_add](../commands/network.route_add.md) |
| `network.route_remove` | change | brak do `effect` | reversible | [network.route_remove](../commands/network.route_remove.md) |
| `network.dns_set` | change | brak do `effect` | reversible | [network.dns_set](../commands/network.dns_set.md) |
| `clock.timezone_set` | change | brak do `effect` | reversible | [clock.timezone_set](../commands/clock.timezone_set.md) |
| `clock.set` | change | brak do `effect` | irreversible | [clock.set](../commands/clock.set.md) |
| `clock.ntp` | change | brak do `effect` | reversible | [clock.ntp](../commands/clock.ntp.md) |
| `accounts.create` | change | brak do `effect` | reversible_if_preserved | [accounts.create](../commands/accounts.create.md) |
| `accounts.modify` | change | brak do `effect` | reversible_if_preserved | [accounts.modify](../commands/accounts.modify.md) |
| `accounts.remove` | change | brak do `effect` | reversible_if_preserved | [accounts.remove](../commands/accounts.remove.md) |
| `accounts.password` | change | brak do `effect` | irreversible | [accounts.password](../commands/accounts.password.md) |
| `groups.create` | change | brak do `effect` | reversible_if_preserved | [groups.create](../commands/groups.create.md) |
| `groups.remove` | change | brak do `effect` | reversible_if_preserved | [groups.remove](../commands/groups.remove.md) |
| `groups.add_member` | change | brak do `effect` | reversible_if_preserved | [groups.add_member](../commands/groups.add_member.md) |
| `groups.remove_member` | change | brak do `effect` | reversible_if_preserved | [groups.remove_member](../commands/groups.remove_member.md) |
| `mount` | change | brak do `effect` | reversible_if_preserved | [mount](../commands/mount.md) |
| `unmount` | change | brak do `effect` | reversible_if_preserved | [unmount](../commands/unmount.md) |
| `sysctl.set` | change | brak do `effect` | reversible_if_preserved | [sysctl.set](../commands/sysctl.set.md) |
| `modules.load` | change | brak do `effect` | reversible_if_preserved | [modules.load](../commands/modules.load.md) |
| `modules.unload` | change | brak do `effect` | reversible_if_preserved | [modules.unload](../commands/modules.unload.md) |
| `power.restart` | change | brak do `effect` | reversible_if_preserved | [power.restart](../commands/power.restart.md) |
| `power.shutdown` | change | brak do `effect` | reversible_if_preserved | [power.shutdown](../commands/power.shutdown.md) |
| `power.cancel` | change | brak do `effect` | irreversible | [power.cancel](../commands/power.cancel.md) |
| `disks.partition` | change | brak do `effect` | irreversible | [disks.partition](../commands/disks.partition.md) |
| `disks.format` | change | brak do `effect` | irreversible | [disks.format](../commands/disks.format.md) |

## Rekordy {#records}

| Rekord | Wymagane pola |
|---|---|
| `service` | `name`, `state`, `start_type`, `manager` |
| `cim_object` | `class`, `path`, `properties` |
| `effect` | `status`, `kind` |

Wartościami `service.state` są `running`, `stopped`, `starting`,
`stopping`, `failed`, `paused` i `unknown`. Wartościami
`service.start_type` są `automatic`, `manual`, `disabled`, `on_demand`,
`static` i `unknown`. Pole `manager` ma wartość `systemd`, `windows` lub
`launchd`. Pole `state_raw` zachowuje stan w postaci zgłoszonej przez
menedżera usług.

Rekord `effect` poleceń `start`, `restart`, `enable` i `disable` wskazuje
usługę w `target` (na przykład `service:nginx`), akcję w `action` oraz stan
usługi w `before` i `after`.

## Model usług {#service-model}

`services` wybiera usługi według `name` (tekst lub wyrażenie regularne),
według `state` i według `scope`. `scope` wybiera instancję menedżera usług:

| Zakres | Instancja menedżera usług |
|---|---|
| `system` | usługi komputera; wartość domyślna |
| `user` | usługi konta, które uruchamia program |

`scope` polecenia `services` nie jest `scope` polecenia `environment`;
wartości i znaczenie się różnią, zobacz grupę `system`.

`start` i `restart` przyjmują usługi z wejścia i czekają do `wait`
(wartość domyślna `10s`) na żądany stan. `enable` sprawia, że usługi
uruchamiają się przy starcie systemu, a dla zakresu `user` przy logowaniu.
`disable` usuwa to uruchamianie. Program ma jeden etap, który zmienia świat,
więc `start`, `restart`, `enable` i `disable` są ostatnim etapem, który cokolwiek
zmienia: `services name: "nginx"
|> restart` wybiera w kroku odczytu i uruchamia ponownie w kroku zastosowania.

Zmiana stanu usługi jest skutkiem poza systemem plików. Dziennik operacji
zapisuje stan przed i po; `undo` uruchamia polecenie przeciwne dla `start`,
`enable` i `disable`, a usługa, której stanu sprzed zmiany nie przywraca żadne
polecenie, pozostaje nieodwracalna. `restart` nie ma odwrotności. Katalog
klasyfikuje `start`, `enable` i `disable` jako `reversible_if_preserved`, a
`restart` jako `irreversible`; raport skutków wylicza pozostałe skutki
nieodwracalne. Podgląd pokazuje usługi, które zostałyby zmienione, i niczego
nie zmienia.

## Zapora sieciowa, trasy, DNS i zegar {#firewall-routes-dns-and-the-clock}

`firewall.rules`, `firewall.allow`, `firewall.deny` i `firewall.remove`
odczytują i zmieniają reguły zapory sieciowej systemu: nftables, firewalld lub
ufw w systemie Linux, pf w systemie macOS oraz Zaporę Windows Defender.
`network.route_add`, `network.route_remove` i `network.dns_set` zmieniają
tablicę routingu i serwery nazw. `clock.timezone_set`, `clock.ntp` i
`clock.set` zmieniają strefę czasową, czas sieciowy i zegar.

Każde z tych poleceń zmienia system i przyjmuje `privilege`; wymagają one
administratora, więc wywołanie zapisuje się z `privilege: elevated`. Etap
nigdy sam nie uruchamia `sudo`: wykonawca uruchamia etap w procesie z
podwyższonymi uprawnieniami, a program, który zgłasza brak uprawnień, kończy
wywołanie błędem `E_PRIVILEGE`.

Raport zmiany zawiera zmianę akcji `state` dla jednego celu z wartością przed
i po: `firewall:<rule>` (`absent` lub `present`), `route:<destination>`,
`dns:<interface>`, `clock:timezone` i `clock:ntp`. Z tych wartości silnik
wyprowadza program, który `undo` uruchamia, z uprawnieniami oryginalnego
wywołania:

| Polecenie | `undo` uruchamia |
|---|---|
| `firewall.allow`, `firewall.deny` | `firewall.remove` reguły |
| `firewall.remove` | `firewall.allow` lub `firewall.deny` każdej usuniętej reguły |
| `network.route_add` | `network.route_remove` trasy |
| `network.route_remove` | `network.route_add` każdej usuniętej trasy |
| `network.dns_set` | `network.dns_set` z poprzednimi serwerami i domenami; zmiana `/etc/resolv.conf` jest przywracana przez dziennik |
| `clock.timezone_set` | `clock.timezone_set` z poprzednią strefą |
| `clock.ntp` | `clock.ntp` z poprzednim stanem |
| `clock.set` | nieodwracalne |

Zmiana, którą system już zawiera (istniejąca reguła, ta sama strefa), jest
zgłaszana jako `unchanged`, nie otwiera wpisu dziennika i nie uruchamia
żadnego programu. Podgląd odczytuje system i wylicza skutki, niczego nie
zmieniając.

## Różnice między platformami {#platform-differences}

| Aspekt | Linux | macOS | Windows |
|---|---|---|---|
| Menedżer usług | systemd, przez interfejs D-Bus | launchd | menedżer sterowania usługami |
| Wartość `manager` | `systemd` | `launchd` | `windows` |
| Odczyt usług | D-Bus | pliki list właściwości i `launchctl` | API menedżera sterowania usługami |
| Zmiana usług | D-Bus | `launchctl` | API menedżera sterowania usługami |
| `scope: user` | instancja użytkownika systemd konta | zadania użytkownika launchd konta | usługi konta, gdy system je udostępnia |
| `windows.cim` | `E_OS_ONLY` | `E_OS_ONLY` | WMI przez COM |

Zmiana usługi o zakresie `system` wymaga uprawnienia. Bez niego polecenie
kończy się błędem `E_PRIVILEGE`, a `fix` wskazuje uprawnienie. Błąd menedżera
usług jest zgłaszany jako `E_SERVICE`.

`windows.cim` odpytuje instancje klasy CIM w `namespace` (wartość domyślna
`root/cimv2`) z opcjonalnym warunkiem WQL `where` i listą `properties`. W
systemach Linux i macOS kończy się błędem `E_OS_ONLY`.

## Programy zewnętrzne {#external-programs}

| Polecenie | Znacznik | Warunek |
|---|---|---|
| `services` | [X] | tylko macOS: stan usług jest odczytywany przez `launchctl` |
| `start` | [X] | tylko macOS: usługi są uruchamiane przez `launchctl` |
| `restart` | [X] | tylko macOS: usługi są uruchamiane ponownie przez `launchctl` |
| `enable` | [X] | tylko macOS: usługi są włączane przez `launchctl` |
| `disable` | [X] | tylko macOS: usługi są wyłączane przez `launchctl` |
| `firewall.rules` | [X] | `nft`, `firewall-cmd`, `ufw` lub `pfctl` odczytują reguły; Windows używa PowerShell |
| `firewall.allow` | [X] | te same programy zmieniają reguły |
| `firewall.deny` | [X] | te same programy zmieniają reguły |
| `firewall.remove` | [X] | te same programy zmieniają reguły |
| `network.route_add` | [X] | `ip`, `route` lub PowerShell |
| `network.route_remove` | [X] | `ip`, `route` lub PowerShell |
| `network.dns_set` | [X] | `resolvectl`, `networksetup` lub PowerShell; `/etc/resolv.conf` jest zapisywany bez programu |
| `clock.timezone_set` | [X] | `timedatectl`, `systemsetup` lub `tzutil` |
| `clock.set` | [X] | `date` lub PowerShell |
| `clock.ntp` | [X] | `timedatectl`, `systemsetup` lub PowerShell |
| `accounts.create` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `accounts.modify` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `accounts.remove` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `accounts.password` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `groups.create` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `groups.remove` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `groups.add_member` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `groups.remove_member` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `mount` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `unmount` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `sysctl.set` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `modules.load` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `modules.unload` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `power.restart` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `power.shutdown` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `power.cancel` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `disks.partition` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |
| `disks.format` | [X] | program systemowy systemu operacyjnego wskazany na stronie polecenia |

Znacznik [X] wskazuje składnik systemu operacyjnego, który polecenie uruchamia
jako program. Polecenia dla kont, grup, montowań, parametrów jądra, zasilania i
dysków uruchamiają programy systemu operacyjnego w każdym systemie; strona
każdego polecenia je wymienia. System macOS nie udostępnia publicznego
interfejsu dla tych operacji, więc `launchctl` jest jedynym składnikiem
systemu operacyjnego, który polecenia tej grupy uruchamiają. Uruchamiają go
przez interfejs procesów i zgłaszają w sprawdzeniu programu. Linux i Windows
używają interfejsów systemowych i nie uruchamiają żadnego programu. Gdy
`launchctl` jest niedostępny w systemie macOS, polecenie kończy się błędem
`E_EXTERNAL_MISSING`.

## Przykłady {#examples}

Ponowne uruchomienie usługi `nginx`:

```nurt example
services name: "nginx" |> restart
```

Wylistowanie uruchomionych usług, które startują automatycznie:

```nurt example
services state: "running" |> filter start_type == "automatic" |> select name state
```

Ponowne uruchomienie usługi i oczekiwanie do 30 sekund, aż zacznie działać:

```nurt example
services "nginx" |> restart wait: 30s
```

Wyłączenie uruchamiania usługi przy starcie systemu:

```nurt example
services name: "cups" |> disable
```

Odczyt wartości rejestru, a następnie jej zmiana; poprzednia wartość
pozostaje w dzienniku:

```nurt example
read "registry:/HKCU/Software/Example/Mode"
write "registry:/HKCU/Software/Example/Mode" "3" value_type: dword mode: overwrite
```

Odczyt dwóch właściwości systemu operacyjnego w systemie Windows:

```nurt example
windows.cim "Win32_OperatingSystem" properties: ["Caption", "Version"]
```

## Powiązane {#related}

[Grupa system](system.md) wylicza procesy i porty. [Grupa schedule](schedule.md)
uruchamia programy o ustalonych porach. `stop` grupy core zatrzymuje usługi.

## Konta, grupy, montowania, jądro, zasilanie i dyski {#accounts-groups-mounts-kernel-power-and-disks}

Polecenia `accounts.create`, `accounts.modify`, `accounts.remove`,
`accounts.password`, `groups.create`, `groups.remove`, `groups.add_member`,
`groups.remove_member`, `mount`, `unmount`, `sysctl.set`, `modules.load`,
`modules.unload`, `power.restart`, `power.shutdown`, `power.cancel`,
`disks.partition` i `disks.format` zmieniają system za pomocą jego własnych
programów i są źródłami jednego rekordu `effect` na każdy zmieniony cel.
Każde z nich najpierw odczytuje stan, który zmienia, zgłasza `unchanged`, gdy
system ma już żądany stan, i zapisuje stan przed i po jako dokumenty JSON w
`before` i `after` celu (`account:<name>`, `group:<name>`, `mount:<path>`,
`sysctl:<key>`, `module:<name>`, `power:schedule`, `disk:<device>`). `undo`
wyprowadza polecenie odwrotne z tych stanów:

| Polecenie | Cofnięcie |
|---|---|
| `accounts.create` | `accounts.remove` z `keep_home: true` |
| `accounts.remove` | `accounts.create` z zapisanymi wartościami; katalog domowy wraca z dziennikiem |
| `accounts.modify` | `accounts.modify` z zapisanymi wartościami |
| `accounts.password` | nieodwracalne |
| `groups.create`, `groups.remove` | parami; usunięta grupa wraca ze swoimi członkami |
| `groups.add_member`, `groups.remove_member` | parami |
| `mount`, `unmount` | parami; `/etc/fstab` jest przywracany przez dziennik |
| `sysctl.set` | `sysctl.set` z poprzednią wartością; plik `persistent` jest przywracany przez dziennik |
| `modules.load`, `modules.unload` | parami |
| `power.restart`, `power.shutdown` | `power.cancel`, gdy zaplanowane; nieodwracalne, gdy natychmiastowe |
| `power.cancel`, `disks.partition`, `disks.format` | nieodwracalne |

Polecenia przyjmują `privilege` jak każde polecenie zmieniające; wykonawca
uruchamia wywołanie z `privilege: elevated` w procesie z podwyższonymi
uprawnieniami, a `undo` uruchamia odwrotność przez ten sam backend.
`disks.partition` i `disks.format` odmawiają pracy na zamontowanej partycji i
wyliczają w swoim podglądzie partycje, systemy plików, punkty montowania oraz
rozmiar danych, które zostaną utracone.
