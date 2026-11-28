---
command: package.list
group: package
summary: Wyświetla zainstalowane pakiety.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# package.list

## Składnia {#synopsis}

```text
package.list [name: value]...
```

## Opis {#description}

`package.list` zgłasza zainstalowane pakiety menedżerów pakietów znalezionych w systemie. Rozpoczyna potok i nie przyjmuje wejścia. Polecenie odczytuje bazy pakietów i lokalne indeksy repozytoriów i nie uruchamia żadnego programu; znak [P] katalogu go nie dotyczy.

| Menedżer | Odczytywana baza |
|---|---|
| apt | `/var/lib/dpkg/status` i `/var/lib/dpkg/status.d/` |
| dnf, zypper | baza RPM: `rpmdb.sqlite` lub `Packages.db` w `/usr/lib/sysimage/rpm` albo `/var/lib/rpm` |
| pacman | `local/` poniżej `DBPath` z `/etc/pacman.conf`, domyślnie `/var/lib/pacman` |
| brew | `Cellar` i `Caskroom` poniżej `HOMEBREW_PREFIX`, `/opt/homebrew`, `/usr/local` lub `/home/linuxbrew/.linuxbrew` |
| winget | baza instalacji App Installer poniżej `LOCALAPPDATA` oraz klucze odinstalowania rejestru |
| scoop | `apps` poniżej `SCOOP` lub `~\scoop` oraz poniżej `SCOOP_GLOBAL` |
| choco | `lib` poniżej `ChocolateyInstall`, domyślnie `C:\ProgramData\chocolatey` |

`name` zachowuje pakiety, których nazwa zawiera tekst, bez uwzględniania wielkości liter. Z `outdated: true` polecenie odczytuje lokalne indeksy repozytoriów menedżera i zachowuje pakiety, dla których najnowsza oferowana wersja odpowiedniej architektury jest większa niż wersja zainstalowana; `available_version` zawiera tę wersję. Wersje są porównywane według reguł menedżera: polityka Debiana dla apt, `rpmvercmp` dla dnf i zypper, `vercmp` dla pacman, wersja z rewizją dla Homebrew oraz segmenty numeryczne i alfabetyczne dla winget, Scoop i Chocolatey.

Gdy znaleziono kilku menedżerów, a `manager` ma wartość `auto`, rekord każdego pakietu wskazuje jego menedżera. Przypięcia i priorytety apt nie są oceniane. Pakiety etapowej aktualizacji poniżej 100 procent nie są oferowane jako nowsze wersje. Brakujące lub uszkodzone indeksy są zgłaszane jako ostrzeżenia i pomijane; polecenie kończy się błędem `E_FORMAT` tylko wtedy, gdy podano `manager`, a każdy indeks tego menedżera jest nieczytelny. Chocolatey nie przechowuje lokalnego indeksu, więc `outdated` niczego dla niego nie znajduje.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text` | brak | Tylko pakiety, których nazwa zawiera ten tekst. |
| `outdated` | `bool` | `false` | Tylko pakiety, dla których dostępna jest nowsza wersja. |
| `manager` | `enum(auto\|apt\|dnf\|pacman\|zypper\|brew\|winget\|choco\|scoop)` | `auto` | Menedżer pakietów; auto wybiera menedżera systemu. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `package` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `package_manager` |

## Przykłady {#examples}

Wyświetlenie zainstalowanych pakietów, których nazwa zawiera `ssl`.

```nurt example
package.list name: "ssl"
```

Wyświetlenie pakietów, które mają nowszą wersję w repozytoriach.

```nurt example
package.list outdated: true
```

Wyświetlenie pakietów tylko jednego menedżera.

```nurt example
package.list manager: pacman |> sort by: name
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera
pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego
uruchamiania polecenia.

```text
matched 1210  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_EXTERNAL_MISSING` | Nie znaleziono żadnego obsługiwanego menedżera pakietów albo podany nie jest zainstalowany. | Przekaż `manager` zainstalowanego menedżera. |
| `E_FORMAT` | Każdy indeks repozytorium podanego menedżera jest nieczytelny. | Odśwież indeksy za pomocą menedżera. |
| `E_NOT_FOUND` | Brakuje bazy pakietów menedżera. | Przekaż `manager` zainstalowanego menedżera. |
| `E_PERMISSION` | Konto nie może otworzyć bazy pakietów. | Uruchom z kontem, które może odczytać bazę. |
| `E_FORMAT` | Baza pakietów jest uszkodzona. | Napraw bazę za pomocą menedżera. |
| `E_LIMIT` | Pakiety przekraczają limit rekordów. | Zawęź listę za pomocą `name`. |

## Powiązane {#related}

`package.search`, `package.upgrade`.
