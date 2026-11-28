---
group: system
summary: Procesy, porty, dyski, host, środowisko, dzienniki systemowe i wstrzymywanie procesów.
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Grupa system

## Zakres {#scope}

Grupa zgłasza stan komputera: uruchomione procesy, porty sieciowe,
zamontowane woluminy, informacje o hoście, zmienne środowiskowe, konta, pliki
otwarte przez procesy i dziennik systemowy. `sleep` należy do grupy, ponieważ
czeka według zegara programu i niczego nie odczytuje.

Polecenia działające na stanie zgłaszanym tutaj należą do innych grup:

| Zadanie | Polecenie | Grupa |
|---|---|---|
| Zatrzymanie procesu, zadania lub usługi | `stop` | core |
| Odczyt dziennika operacji Danaco Nurt | `journal.*` | journal |
| Odczyt i zmiana plików, w tym ścieżek `registry:` | `read`, `write`, `list` | core |
| Zarządzanie usługami | `services`, `start`, `restart` | administration |

`suspend` i `resume` są wymienione na końcu tabeli poleceń. Ich strony są
utrzymywane razem z poleceniami, które uruchamiają programy i nimi sterują.

## Polecenia {#commands}

Każde polecenie źródłowe nie przyjmuje wejścia i daje rekordy jednego rodzaju.

| Polecenie | Kategoria | Wejście i wyjście | Odwracalność | Strona |
|---|---|---|---|---|
| `processes` | source | brak do `process` | read | [processes](../commands/processes.md) |
| `ports` | source | brak do `port` | read | [ports](../commands/ports.md) |
| `disks` | source | brak do `disk` | read | [disks](../commands/disks.md) |
| `host` | source | brak do `host` | read | [host](../commands/host.md) |
| `environment` | source | brak do `environment_variable` | read | [environment](../commands/environment.md) |
| `open_files` | source | brak do `process` | read | [open_files](../commands/open_files.md) |
| `accounts` | source | brak do `account` | read | [accounts](../commands/accounts.md) |
| `sleep` | run | brak do brak | read | [sleep](../commands/sleep.md) |
| `logs` | source | brak do `log_event` | read | [logs](../commands/logs.md) |

Polecenia grupy opisane na innych stronach:

| Polecenie | Kategoria | Wejście i wyjście | Odwracalność |
|---|---|---|---|
| `suspend` | run | `job` lub `process` do `effect` | irreversible |
| `resume` | run | `job` lub `process` do `effect` | irreversible |

## Rekordy {#records}

Pola, które niesie każdy rekord danego rodzaju. Pola opcjonalne są wymienione
na stronie polecenia.

| Rekord | Wymagane pola |
|---|---|
| `process` | `pid`, `parent_pid`, `name`, `command`, `user`, `memory`, `started`, `state` |
| `port` | `protocol`, `local`, `address`, `port`, `state` |
| `disk` | `mount`, `device`, `filesystem`, `total`, `free`, `used`, `type`, `case_sensitive`, `readonly` |
| `host` | `os`, `os_version`, `architecture`, `cpus`, `memory_total`, `hostname`, `user`, `home`, `temp`, `timezone`, `path_style`, `nurt_version`, `features` |
| `environment_variable` | `name`, `value`, `scope` |
| `account` | `name`, `kind`, `id` |
| `log_event` | `time`, `source`, `level`, `message` |

`open_files` daje rekordy `process` procesów, które utrzymują ścieżkę.
Wartości środowiska nigdy nie są maskowane. Surowe atrybuty systemu są
zachowywane w `os_raw` w rodzajach, które mają to pole, i pozostają poza
gwarancją stabilności rekordu.

## Zakres zmiennych środowiskowych {#scope-of-environment-variables}

Parametr `scope` polecenia `environment` wybiera, skąd odczytywane są
zmienne:

| Wartość | Źródło |
|---|---|
| `process` | środowisko procesu, który uruchamia program; wartość domyślna |
| `user` | trwałe zmienne konta; tylko Windows |
| `machine` | trwałe zmienne komputera; tylko Windows |

`scope` polecenia `environment` i `scope` polecenia `services` mają różne
wartości i różne znaczenie. Dla `services` wybiera instancję menedżera usług
(`system` lub `user`), zobacz grupę `administration`.

Środowisko jest odczytywane przez dostawcę `env:` poleceń plikowych albo przez
`environment`. Dostawca adresuje jedną zmienną jako ścieżkę i zwraca tekst.
`environment` zwraca rekordy, które można filtrować i sortować, i odczytuje
zakresy trwałe.

## Różnice między platformami {#platform-differences}

| Polecenie | Linux | macOS | Windows |
|---|---|---|---|
| `processes` | tablica procesów jądra | tablica procesów jądra | migawka procesów systemu |
| `ports` | tablice gniazd jądra | tablice gniazd systemu | tablice IP Helper |
| `disks` | tablica montowań | tablica montowań | woluminy z literami dysków |
| `host` | wersja jądra w `kernel` | wersja jądra w `kernel` | `os_version` to numer kompilacji |
| `environment` | zakres `process` | zakres `process` | zakresy `process`, `user` i `machine` |
| `accounts` | bazy kont i grup | katalog lokalny | konta i grupy lokalne; `id` to SID |
| `logs` | dziennik systemd odczytywany natywnym czytnikiem | ujednolicony dziennik | Dziennik zdarzeń |

Konta zgłaszają `uid` jako UID w systemach Linux i macOS oraz jako SID w
systemie Windows.

Odczyt danych należących do innych kont może wymagać uprawnienia. Procesy,
gniazda i otwarte pliki innych kont są zgłaszane z polami, które system
ujawnia; pola opcjonalne, takie jak `pid`, `process`, `cwd` lub `executable`,
są nieobecne, gdy system je ukrywa. Polecenie, które w ogóle nie może
odczytać swojego źródła, kończy się błędem `E_PRIVILEGE`; `fix` wskazuje
wymagane uprawnienie. Źródło odczytane częściowo jest zgłaszane jako
ostrzeżenie `permission_denied_skipped`, które nie przerywa programu.

### Polecenie logs

`logs` zwraca zdarzenia z dziennika systemu operacyjnego bez uruchamiania
programu. Najnowsze zdarzenia są pierwsze. `level` jest progiem: zwracane są
zdarzenia o tej ważności i ważniejsze.

| Parametr | Linux | macOS | Windows |
|---|---|---|---|
| `source` | program, który zapisał zdarzenie | proces | dostawca |
| `unit` | jednostka systemd | podsystem | kanał; wartość domyślna `System` i `Application` |
| `since`, `until` | czas zdarzenia | czas zdarzenia | `TimeCreated` |

Linux odczytuje pliki dziennika systemd czytnikiem formatu plików Danaco
Nurt. W systemie Linux bez dziennika polecenie kończy się błędem
`E_NOT_FOUND`. W systemie Windows kanał `Security` wymaga uprawnienia i bez
niego kończy się błędem `E_PRIVILEGE`. Każde zdarzenie zawiera pole `cursor`,
które wskazuje jego pozycję w dzienniku.

## Programy zewnętrzne {#external-programs}

| Polecenie | Znacznik | Warunek |
|---|---|---|
| `host` | [U] | `versions: true` uruchamia każdy zlokalizowany program jednokrotnie z `--version` |

Żadne inne polecenie grupy nie uruchamia programu. Znacznik [U] wskazuje
program użytkownika, który polecenie uruchamia tylko na żądanie.

## Przykłady {#examples}

Pokazanie pięciu najbardziej obciążonych procesów o nazwie `node`:

```nurt example
processes name: "node" |> sort cpu order: desc |> first 5
```

Wylistowanie nasłuchującego portu 8080 i procesu, który go utrzymuje:

```nurt example
ports state: listen |> filter port == 8080
```

Wylistowanie dysków stałych z mniej niż 10 GB wolnego miejsca:

```nurt example
disks type: fixed |> filter free < 10gb |> select mount free
```

Odczyt ostrzeżeń jednej usługi z dziennika systemowego:

```nurt example
logs unit: "nginx.service" level: warning limit: 20
```

Wylistowanie procesów, które mają otwarty plik:

```nurt example
open_files "/home/user/app/data.db"
```

Zgłoszenie wersji programów znalezionych w ścieżce wyszukiwania:

```nurt example
host programs: ["git", "python3"] versions: true
```

Oczekiwanie przez dwie sekundy:

```nurt example
sleep 2s
```

## Powiązane {#related}

[Grupa administration](administration.md) zarządza usługami, które zgłaszają
`processes` i `ports`. [Grupa program](program.md) opisuje zainstalowane
programy. Stroną polecenia `sleep` jest [sleep](../commands/sleep.md).
