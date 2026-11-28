---
command: host
group: system
summary: Opisuje system operacyjny, sprzęt, bieżące konto oraz katalogi domowy i tymczasowy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# host

## Składnia {#synopsis}

```text
host [name: value]...
```

## Opis {#description}

`host` zwraca jeden rekord `host`, który opisuje urządzenie, system operacyjny i
konto uruchamiające konsolę. Ścieżki w `home`, `temp`, `programs.path` i
`programs.all_paths` zawsze używają `/` jako separatora; w systemie Windows
zaczynają się od litery dysku, na przykład `C:/Users/user`.

| Pole | Linux | macOS | Windows |
|---|---|---|---|
| `os_version` | `VERSION_ID` z `/etc/os-release` | wersja produktu | wersja systemu |
| `kernel` | wydanie jądra | wydanie jądra | wersja jądra |
| `memory_total`, `memory_free` | `MemTotal`, `MemAvailable` z `/proc/meminfo` | statystyki pamięci hosta | stan pamięci |
| `uptime`, `boot_time` | `/proc/uptime`, `btime` z `/proc/stat` | czas uruchomienia | czas od uruchomienia |
| `virtualization` | znaczniki kontenera, `/proc/1/cgroup`, dostawca oprogramowania układowego | flaga hiperwizora | nieraportowane |
| `uid`, `groups` | identyfikatory użytkownika i grup procesu | jak w systemie Linux | identyfikator zabezpieczeń, grupy lokalne |
| `locale` | `LC_ALL`, `LC_MESSAGES`, `LANG` | jak w systemie Linux | ustawienia regionalne użytkownika |

`features` wymienia możliwości dostępne dla konsoli: `journal`, `service`,
`reflink`, `hardlink`, `exchange`, `xattr`, `acl` i `case_sensitive`.

`service` to stan usługi działającej w tle dla danej instancji. Ma wartość
`running`, gdy plik sygnału życia `service.hb` w katalogu stanu nie jest
starszy niż `space.heartbeat_max_age_ms`, proces w nim wskazany istnieje, a
usługa odpowiada na `ping` na swoim kanale sterowania w ciągu 50 ms. Ma wartość
`stopped`, gdy usługa jest zainstalowana (jednostka systemd
`nurt-service.service`, lista właściwości launchd `nurt.service.plist` albo
usługa Windows `NurtService`), ale nie działa, a w pozostałych przypadkach
`not_installed`.

`programs` rozwiązuje każdą nazwę w `PATH` procesu konsoli, z rozszerzeniami z
`PATHEXT` w systemie Windows, i zwraca jeden `program_location` na nazwę, w
podanej kolejności. `path` to pierwsze dopasowanie, a `all_paths` każde
dopasowanie w kolejności rozwiązywania.

Przy `versions: true` pole `version` jest odczytywane z metadanych pliku
programu: zasobu wersji pliku PE, `Info.plist` pakietu aplikacji albo tego
osadzonego w pliku Mach-O, albo noty `.note.package` pliku ELF. Program bez
takich metadanych ma puste `version`. Żaden program nie jest uruchamiany.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `programs` | `list<text>` | `[]` | Nazwy programów do zlokalizowania w PATH. |
| `versions` | `bool` | `false` | Uruchamia każdy zlokalizowany program z --version, aby odczytać jego wersję. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `host` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| External use | `user_program`; używane, gdy podano `versions`; Uruchamia każdy zlokalizowany program jednokrotnie z --version. |

## Przykłady {#examples}

Opisz urządzenie:

```nurt example
host
```

```text expected
os: linux
architecture: x86_64
```

Zlokalizuj dwa programy w `PATH`:

```nurt example
host programs: ["git", "cargo"]
```

Zlokalizuj program i odczytaj jego wersję z metadanych pliku:

```nurt example
host programs: ["git"] versions: true |> select programs
```

Sprawdź, czy usługa działająca w tle jest uruchomiona:

```nurt example
host |> select service
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_TYPE` | `programs` nie jest listą nazw albo `versions` nie jest wartością logiczną. | Przekaż listę wartości tekstowych, na przykład `["git"]`. |

## Powiązane {#related}

`environment`, `accounts`, `processes`, `disks`, `program.find`, `service`.
