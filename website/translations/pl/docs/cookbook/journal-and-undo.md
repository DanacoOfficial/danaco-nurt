---
category: journal-and-undo
title: Dziennik, cofanie i bezpieczeństwo
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/journal-and-undo.md -->
<!-- Responsibility:   Documents Journal, undo and safety. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Dziennik, cofanie i bezpieczeństwo {#journal-undo-and-safety}

## Wyświetlanie historii operacji {#show-the-history-of-operations}

Pokaż pięć ostatnich operacji dziennika.

```nurt example
history limit: 5
```

```text expected
id
```

## Cofnięcie operacji {#undo-an-operation}

Cofnij jedną operację według jej numeru.

```nurt example
undo 18
```

```text expected
18
```

## Cofnięcie z uwzględnieniem późniejszych zmian {#undo-with-later-changes-included}

Cofnij operację wraz z późniejszymi operacjami, które dotknęły tych samych plików.

```nurt example
undo 18 on_conflict: include_later
```

```text expected
18
```

## Cofnięcie zmian jednego pliku {#undo-the-changes-of-one-file}

Cofnij zmiany jednego pliku dokonane od danej operacji.

```nurt example
undo file: "/home/user/app/src/a.go" since: 18
```

```text expected
/home/user/app/src/a.go
```

## Ponowienie cofniętej operacji {#redo-an-undone-operation}

Zastosuj ponownie cofniętą operację.

```nurt example
redo 18
```

```text expected
18
```

## Wyświetlanie szczegółów operacji {#show-the-details-of-an-operation}

Pokaż pliki jednej operacji.

```nurt example
history 18 details: true
```

```text expected
18
```

## Wyszukiwanie operacji, które dotknęły ścieżki {#find-the-operations-that-touched-a-path}

Pokaż operacje, które zmieniły ścieżkę.

```nurt example
history path: "/home/user/app/src/a.go"
```

```text expected
/home/user/app/src/a.go
```

## Przypięcie operacji {#pin-an-operation}

Zachowaj operację w dzienniku przez 24 godziny.

```nurt example
journal.pin 18 until: 24h
```

```text expected
18
```

## Wyświetlanie miejsca dziennika {#show-the-space-of-the-journal}

Pokaż wolne miejsce oraz miejsce zajęte przez dziennik woluminu.

```nurt example
journal.space
```

```text expected
free
```

## Wyświetlanie stanu dziennika {#show-the-state-of-the-journal}

Pokaż, czy dziennik jest włączony i ile operacji zawiera.

```nurt example
journal.status
```

```text expected
journal
```

## Ustawienie limitów przechowywania {#set-the-retention-limits}

Zachowaj ostatnie 100 operacji sesji.

```nurt example
journal.limits steps: 100
```

```text expected
100
```

## Usunięcie wygasłych wpisów {#purge-expired-entries}

Usuń z dziennika wpisy do danej operacji włącznie.

```nurt example
journal.purge up_to: 17
```

```text expected
17
```

## Zabezpieczenie zmiany licznikiem {#guard-a-change-with-a-count}

Zatrzymaj się przed jakąkolwiek zmianą, gdy polecenie obejmuje więcej plików, niż oczekiwano.

```nurt example
find "/home/user/app/build" glob: "**/*.o" |> assert count: 12 |> remove
```

```text expected
E_ASSERT
```

## Weryfikacja zmiany budowaniem {#check-a-change-with-a-build}

Uruchom budowanie po zmianie i wycofaj ją, gdy budowanie zakończy się błędem.

```nurt example
search /oldName\(/ "/home/user/app/src" glob: "**/*.go" |> replace "newName(" |> verify (run ["go", "build", "./..."] cwd: "/home/user/app" |> assert exit_code: 0)
```

```text expected
newName
```

## Wyświetlanie operacji cofalnych tylko częściowo {#list-the-operations-that-were-only-partly-undoable}

Wyświetl operacje, których zmiany są w dzienniku zachowane tylko częściowo.

```nurt example
history state: partial limit: 10
```

```text expected
partial
```

## Wyświetlanie operacji, które zostały cofnięte {#list-the-operations-that-were-undone}

Wyświetl operacje, które zostały cofnięte i można je ponowić.

```nurt example
history state: undone
```

```text expected
undone
```

## Wyświetlanie operacji, które usunęły pliki {#list-the-operations-that-removed-files}

Wyświetl operacje, które usunęły co najmniej jeden plik.

```nurt example
history deleted: true limit: 20
```

```text expected
id
```

## Cofnięcie wszystkich operacji do danego punktu {#undo-every-operation-back-to-a-point}

Cofnij wszystkie operacje nowsze niż operacja 12, od najnowszej, w jednej transakcji.

```nurt example
undo to: 12
```

```text expected
12
```

## Cofnięcie operacji wszystkich sesji {#undo-operations-of-all-sessions}

Cofnij operacje wszystkich sesji nowsze niż operacja 12.

```nurt example
undo to: 12 all_sessions: true
```

```text expected
12
```

## Cofnięcie tego, co możliwe, i zgłoszenie reszty {#undo-what-is-possible-and-report-the-rest}

Cofnij pliki operacji, których nie dotykają późniejsze zmiany, a pozostałe zgłoś.

```nurt example
undo 18 on_conflict: partial
```

```text expected
18
```

## Anulowanie cofnięcia przy konflikcie {#cancel-an-undo-on-conflict}

Pozostaw wszystkie pliki bez zmian, gdy późniejsza zmiana dotyka pliku operacji.

```nurt example
undo 18 on_conflict: abort
```

```text expected
18
```

## Cofnięcie zmian pliku z ostatniej godziny {#undo-the-changes-of-a-file-made-in-the-last-hour}

Przywróć jeden plik do stanu sprzed godziny.

```nurt example
undo file: "/home/user/app/src/a.go" since: ago(1h)
```

```text expected
/home/user/app/src/a.go
```

## Zwolnienie przypiętej operacji {#release-a-pinned-operation}

Pozwól dziennikowi zwolnić operację, która była przypięta.

```nurt example
journal.pin 18 release: true
```

```text expected
18
```

## Ograniczenie wieku dziennika dla konta {#limit-the-age-of-the-journal-for-the-account}

Zachowaj operacje z ostatnich siedmiu dni dla bieżącego konta.

```nurt example
journal.limits age: 7d scope: user
```

```text expected
7d
```

## Zabezpieczenie zastąpienia liczbą plików {#guard-a-replacement-with-a-file-count}

Zatrzymaj się przed zastąpieniem, gdy wyszukiwanie znajdzie więcej niż pięć plików.

```nurt example
search /legacyApi/ "/home/user/app/src" |> assert paths: 1..5 |> replace "currentApi"
```

```text expected
E_ASSERT
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
