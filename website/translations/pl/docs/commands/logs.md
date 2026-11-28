---
command: logs
group: system
summary: Odczytuje zdarzenia z dziennika systemowego: dziennika systemd, Dziennika zdarzeń systemu Windows lub ujednoliconego dziennika macOS.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# logs

## Składnia {#synopsis}

```text
logs [name: value]...
```

## Opis {#description}

`logs` odczytuje dziennik systemu operacyjnego i produkuje jeden rekord
`log_event` na każde zdarzenie, od najnowszego. Rozpoczyna potok. Każdy
parametr ma znaczenie późniejszego `filter` na polu o tej samej nazwie;
magazyny używają swoich indeksów do znalezienia pasujących zdarzeń, więc wąskie
zapytanie odczytuje niewiele z dziennika.

`source` i `unit` przyjmują nazwę dosłowną albo wzorzec glob (`*`, `?`,
`[...]`). Nazwa dosłowna, która nie pasuje do żadnego zdarzenia, daje pusty
wynik z uwagą `log_unknown_name`, która wymienia podobne nazwy z dziennika.
`level` jest najniższym zwracanym poziomem: `level: warning` zwraca
ostrzeżenia, błędy i zdarzenia krytyczne. `since` jest włącznie, a `until`
wyłącznie. `message` zachowuje zdarzenia, których komunikat pasuje do
wyrażenia regularnego. `limit` ogranicza liczbę zdarzeń; gdy pozostają starsze
zdarzenia, wykonanie zgłasza uwagę `log_limit_reached` z `cursor` ostatniego
zdarzenia, po którym następują starsze zdarzenia.

Uwaga `permission_denied_skipped` jest wpisem `notable` w raporcie. Pozostałe
uwagi polecenia są zdarzeniami diagnostycznymi wykonania, których komunikat
zaczyna się od kodu uwagi. Uwagi są zgłaszane, gdy kończy się wyjście albo gdy
reszta potoku przestaje je odczytywać.

### Linux

W systemie Linux polecenie odczytuje pliki dziennika systemd bezpośrednio, bez
`journalctl` i bez biblioteki systemd. Odczytuje trwały dziennik w
`/var/log/journal/<machine-id>/` oraz nietrwały dziennik w
`/run/log/journal/<machine-id>/`: pliki aktywne, pliki zarchiwizowane i pliki
przemianowane po nieprawidłowym zamknięciu systemu (`*.journal~`). Dzienniki
innych przestrzeni nazw, kontenerów i zdalnych hostów nie są odczytywane.
Obsługiwane są pliki zapisane z kompresją XZ, LZ4 lub zstd, z haszem z kluczem
i w trybie zwartym. Wpisy, które journald dopisuje w trakcie zapytania, nie są
zwracane.

| Parametr | Pola dziennika |
|---|---|
| `source` | `SYSLOG_IDENTIFIER`, w przeciwnym razie `_COMM`, w przeciwnym razie nazwa pliku z `_EXE`, w przeciwnym razie `_TRANSPORT`; `unknown`, gdy żadne nie jest ustawione |
| `unit` | `UNIT` dla komunikatów menedżera usług, `USER_UNIT` dla komunikatów menedżera użytkownika, w przeciwnym razie `_SYSTEMD_USER_UNIT`, w przeciwnym razie `_SYSTEMD_UNIT`; nazwa dosłowna bez przyrostka jednostki otrzymuje `.service`, jak w `journalctl -u` |
| `level` | `PRIORITY` 0-2 to `critical`, 3 to `error`, 4 to `warning`, 5-6 to `info`, 7 to `debug`; brakujący priorytet to 6 |

Plik, który jest uszkodzony, obcięty albo zapisany z nieznaną niezgodną
funkcją, jest pomijany z uwagą `log_file_skipped`; zdarzenia odczytane z niego
przed defektem są zachowywane. Pliki, których konto nie może odczytać, są
pomijane z uwagą `permission_denied_skipped`.

### Windows

W systemie Windows polecenie odczytuje Dziennik zdarzeń. `unit` wskazuje kanał
dziennika zdarzeń, taki jak `System`, `Application`, `Security` lub
`Microsoft-Windows-TaskScheduler/Operational`; nazwy kanałów są porównywane bez
uwzględniania wielkości liter. Bez `unit` odczytywane są kanały `System` i
`Application`. `source` wskazuje dostawcę zdarzeń (`Provider/@Name`). Poziomy
zdarzeń są odwzorowywane następująco: 1 to `critical`, 2 to `error`, 3 to
`warning`, 5 to `debug`; 0, 4 i poziomy dostawcy od 16 to `info`. Komunikat
jest formatowany na podstawie metadanych dostawcy; bez metadanych zawiera dane
jako pary `name=value`, a uwaga `log_unformatted` podaje liczbę takich
zdarzeń.

| Warunek | Wynik |
|---|---|
| nieznany kanał | `E_UNKNOWN_VALUE`, `expected` wymienia najbliższe nazwy kanałów |
| odmowa dla nazwanego kanału, takiego jak `Security` | `E_PRIVILEGE`, `required: administrator` |
| odmowa dla kanału domyślnego | uwaga `permission_denied_skipped` |
| usługa dziennika zdarzeń zatrzymana | `E_NOT_FOUND` |

### macOS

W systemie macOS polecenie odczytuje ujednolicony dziennik. `unit` wskazuje
podsystem, taki jak `com.example.agent`, a `source` wskazuje proces. Usterki
(faults) mają poziom `critical`, błędy `error`, komunikaty typu `default` i
`info` mają poziom `info`, a komunikaty `debug` mają poziom `debug`. System
przechowuje komunikaty debug i info tylko przez krótki czas. Odczyt magazynu
dziennika działającego systemu wymaga uprawnień root; proces działający jako
root, któremu odmówiono dostępu, wymaga Full Disk Access dla aplikacji
terminala.

| Warunek | Wynik |
|---|---|
| brak uprawnień root | `E_PRIVILEGE`, `required: root` |
| root bez Full Disk Access | `E_PRIVILEGE`, `required: full_disk_access` |
| komunikaty utracone przez system | uwaga `log_events_lost` z liczbą |

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `source` | `text` | brak | Źródło zdarzenia: identyfikator, dostawca lub podsystem. |
| `unit` | `text` | brak | Jednostka usługi, kanał zdarzeń lub kategoria. |
| `level` | `enum(critical\|error\|warning\|info\|debug)` | `info` | Najniższy poziom zwracanych zdarzeń. |
| `since` | `instant` | brak | Tylko zdarzenia od tego czasu włącznie. |
| `until` | `instant` | brak | Tylko zdarzenia przed tym czasem. |
| `message` | `regex` | brak | Tylko zdarzenia, których komunikat pasuje do tego wzorca. |
| `limit` | `int` | `100` | Maksymalna liczba zdarzeń, od najnowszego. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `log_event` |

Rekord `log_event` zawiera `time`, `source`, `level` i `message`, a gdy
dziennik je udostępnia, także `id`, `host`, `unit`, `pid`, `boot_id`, `cursor`
i `fields`. W systemie Linux `id` to `MESSAGE_ID`, a `fields` zawiera pozostałe
pola wpisu z wyjątkiem `MESSAGE`, `MESSAGE_ID`, `_HOSTNAME`, `_PID` i
`_BOOT_ID`.

Komunikat dłuższy niż 65536 bajtów oraz wartość innego pola dłuższa niż 16384
bajtów są obcinane na granicy znaku, a pola poza pierwszymi 128 są
odrzucane; zdarzenie jest wtedy oznaczone jako obcięte. Komunikat, który nie
jest poprawnym UTF-8, jest dekodowany ze znakami zastępczymi.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie błędów jednej usługi:

```nurt example
logs unit: "ssh" level: error
```

Wyszukanie nieudanych logowań w komunikatach `sshd`:

```nurt example
logs source: "sshd" message: /Failed password/ limit: 20
```

Wyświetlenie pięciu najnowszych ostrzeżeń i błędów systemu:

```nurt example
logs level: warning |> first 5
```

Odczyt zdarzeń wszystkich jednostek czasomierzy:

```nurt example
logs unit: "*.timer" level: debug
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera
pole `continuation`; polecenie `next` zwraca najpierw zachowane zdarzenia, a
następnie kontynuuje zapytanie od kursora ostatniego zdarzenia, tak aby części
nie nakładały się ani nie pomijały zdarzeń, które nadejdą później.

```text
shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | System nie ma dziennika, który odczytuje `logs`, na przykład system Linux bez dziennika systemd. | Odczytaj tekstowe pliki dziennika, takie jak `/var/log/syslog`, poleceniem `read`. |
| `E_PERMISSION` | Konto nie może odczytać żadnego pliku dziennika. | W systemie Linux dodaj konto do grupy `systemd-journal` lub `adm` albo uruchom jako root. |
| `E_PRIVILEGE` | Dziennik wymaga uprawnienia, którego proces nie ma: nazwanego kanału systemu Windows, takiego jak `Security`, albo magazynu dziennika macOS. | W systemie Windows uruchom z uprawnieniami administratora; w systemie macOS uruchom jako root i w razie potrzeby przyznaj Full Disk Access. |
| `E_UNKNOWN_VALUE` | `unit` wskazuje kanał systemu Windows, który nie istnieje; `expected` wymienia najbliższe nazwy. | Użyj jednego z wymienionych kanałów. |
| `E_OS_ONLY` | System operacyjny nie jest systemem Linux, Windows ani macOS. | Odczytaj tekstowe pliki dziennika poleceniem `read`. |
| `E_FORMAT` | Żaden plik dziennika nie ma czytelnego formatu; `files` wymienia pliki wraz z klasą ich defektu. | Sprawdź pliki; pliki zapisane przez nowszy systemd wymagają nowszego wydania Danaco Nurt. |
| `E_INVALID_VALUE` | Parametr ma wartość niewłaściwej postaci, na przykład kursor z innego systemu. | Użyj kursora zwróconego przez `logs` w tym samym systemie. |
| `E_REGEX` | Wzorzec `message` nie jest poprawnym wyrażeniem regularnym. | Popraw wzorzec. |
| `E_PATTERN_SYNTAX` | Wzorzec glob w `source` lub `unit` jest nieprawidłowy. | Zamknij nawias albo poprzedź znak specjalny znakiem ucieczki. |
| `E_LIMIT` | `limit` jest mniejszy niż 1 albo większy niż 100000. | Podaj `limit` z dozwolonego zakresu albo zawęź zapytanie za pomocą `since` i `until`. |
| `E_CANCELLED` | Wykonanie zostało anulowane między dwoma zdarzeniami. | Uruchom program ponownie. |
| `E_TIMEOUT` | Limit czasu programu upłynął w trakcie odczytu dziennika. | Zawęź zapytanie za pomocą `since`, `unit` lub `source`. |

## Powiązane {#related}

`services` wyświetla usługi, których nazwy `unit` przyjmuje `logs`. `filter`,
`group` i `sort` przetwarzają rekordy `log_event` dalej. Strona grupy `system`
wymienia pozostałe polecenia tej grupy.
