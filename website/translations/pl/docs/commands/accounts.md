---
command: accounts
group: system
summary: Wyświetla konta użytkowników i grupy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# accounts

## Składnia {#synopsis}

```text
accounts [name: value]...
```

## Opis {#description}

`accounts` wyświetla lokalne konta użytkowników i grupy jako rekordy `account`.
Najpierw występują użytkownicy, w kolejności bazy kont, a po nich grupy.

| Pole | Użytkownicy | Grupy |
|---|---|---|
| `id` | UID; w systemie Windows identyfikator zabezpieczeń | GID; w systemie Windows identyfikator zabezpieczeń |
| `groups` | grupa podstawowa, a następnie grupy, które wymieniają użytkownika jako członka | członkowie |
| `home`, `shell` | katalog domowy i powłoka logowania | nieustawione |
| `full_name` | pierwsza część pola komentarza; w systemie Windows imię i nazwisko | nieustawione |
| `disabled` | zablokowane hasło w systemie Linux; `UF_ACCOUNTDISABLE` w systemie Windows | nieustawione |
| `last_login` | ostatnie logowanie w systemie Windows | nieustawione |
| `system` | konto systemu operacyjnego | grupa systemu operacyjnego |

| System | Źródło | `system` |
|---|---|---|
| Linux | bazy użytkowników i grup biblioteki systemowej, łącznie z każdym skonfigurowanym źródłem | identyfikator poniżej `UID_MIN` z `/etc/login.defs`, a gdy nie jest ustawiony, 1000 |
| macOS | bazy użytkowników i grup biblioteki systemowej; konta lokalne dostarcza usługa katalogowa | identyfikator poniżej 500 lub nazwa zaczynająca się od `_` |
| Windows | konta lokalne i grupy lokalne | identyfikator względny poniżej 1000; grupy wbudowane |

`nobody`, `nogroup` i `nfsnobody` są kontami systemowymi w każdym systemie. W
systemie Linux pole `disabled` pochodzi z `/etc/shadow`: konto, którego pole
hasła zaczyna się od `!` albo ma wartość `*`, jest wyłączone. Gdy plik nie może
zostać odczytany, pole `disabled` nie jest ustawiane, a raport zawiera jedną
notatkę `permission_denied_skipped`. System macOS nie zgłasza pola `disabled`.
Konta domenowe w systemie Windows nie są wyświetlane.

Parametr `name` wybiera konto lub grupę o tej nazwie; wielkość liter jest
rozróżniana w systemach Linux i macOS, a w systemie Windows nie ma znaczenia.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `kind` | `enum(user\|group\|all)` | `all` | Użytkownicy, grupy lub jedni i drudzy. |
| `name` | `text` | brak | Tylko konto lub grupa o tej nazwie. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `account` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie kont osób:

```nurt example
accounts kind: user |> filter not system
```

```text expected
kind: user
system: false
```

Wyświetlenie członków jednej grupy:

```nurt example
accounts kind: group name: "docker" |> select groups
```

Wyświetlenie wyłączonych kont:

```nurt example
accounts kind: user |> filter disabled
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_TYPE` | `kind` nie jest jedną z wymienionych wartości albo `name` nie jest tekstem. | Użyj `user`, `group` lub `all` oraz nazwy w postaci tekstu. |
| `E_PRIVILEGE` | Baza kont odmawia dostępu. | Uruchom konsolę jako administrator. |
| `E_IO` | Bazy kont nie można odczytać. | Sprawdź, czy istnieją `/etc/passwd` i `/etc/group`. |

## Powiązane {#related}

`host`, `processes`, `open_files`.
