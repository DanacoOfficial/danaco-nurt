---
command: environment
group: system
summary: Wyświetla zmienne środowiskowe wraz z ich wartościami.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# environment

## Składnia {#synopsis}

```text
environment [<name>] [name: value]...
```

## Opis {#description}

`environment` wyświetla zmienne środowiskowe jako rekordy
`environment_variable` z polami `name`, `value` i `scope`. Wartości są
wyświetlane bez zmian i nigdy nie są maskowane.

`name` wybiera zmienne. Tekst wybiera jedną zmienną według jej nazwy,
porównywanej z uwzględnieniem wielkości liter w systemach Linux i macOS oraz
bez jej uwzględniania w systemie Windows. Wzorzec wybiera każdą zmienną, której
nazwa do niego pasuje.

| `scope` | Zmienne | Systemy |
|---|---|---|
| `process` | środowisko procesu konsoli | wszystkie |
| `user` | trwałe zmienne konta, `registry:/HKCU/Environment` | Windows |
| `machine` | trwałe zmienne komputera, `registry:/HKLM/SYSTEM/CurrentControlSet/Control/Session Manager/Environment` | Windows |

Trwałe zmienne typu `REG_EXPAND_SZ` są wyświetlane bez rozwijania odwołań, takich
jak `%USERPROFILE%`. Wartość, która nie jest poprawnym UTF-8, jest wyświetlana
z nieprawidłowymi bajtami zastąpionymi znakiem U+FFFD; raport zawiera jedną
uwagę `encoding_changed` dla każdej takiej zmiennej.

Systemy Linux i macOS nie mają trwałych zmiennych poza plikami profilu, więc
`scope: user` i `scope: machine` kończą się tam błędem `E_OS_ONLY`. Trwałe
zmienne zmienia się poleceniami `write` i `replace`: w systemie Windows przez
zapis wartości rejestru, co dodatkowo powiadamia działające programy za pomocą
`WM_SETTINGCHANGE`; w systemach Linux i macOS przez edycję pliku profilu, takiego
jak `~/.profile` lub `~/.zshenv`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text \| regex` | brak | Nazwa zmiennej lub wzorzec. |
| `scope` | `enum(process\|user\|machine)` | `process` | Zmienne tego procesu albo trwałe zmienne użytkownika lub komputera w systemie Windows. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `environment_variable` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie ścieżki wyszukiwania procesu konsoli:

```nurt example
environment "PATH"
```

```text expected
name: PATH
scope: process
```

Wyświetlenie zmiennych, których nazwy zaczynają się od `CARGO_`:

```nurt example
environment /^CARGO_/
```

Wyświetlenie trwałych zmiennych konta w systemie Windows:

```nurt example
environment scope: user
```

Ustawienie trwałej zmiennej konta w systemie Windows:

```nurt example
write "registry:/HKCU/Environment/EDITOR" "code" mode: overwrite value_type: string
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_OS_ONLY` | `scope: user` lub `scope: machine` poza systemem Windows. | Użyj `scope: process` albo edytuj plik profilu poleceniem `write` lub `replace`. |
| `E_REGEX` | `name` jest nieprawidłowym wzorcem. | Popraw wzorzec. |
| `E_TYPE` | `name` nie jest ani tekstem, ani wzorcem, albo `scope` nie jest wymienioną wartością. | Przekaż nazwę, wzorzec lub jeden z wymienionych zakresów. |
| `E_PRIVILEGE` | Nie można odczytać klucza rejestru dla tego zakresu. | Uruchom konsolę na koncie, które może odczytać ten klucz. |
| `E_IO` | Nie można odczytać klucza rejestru dla tego zakresu z innej przyczyny. | Sprawdź klucz poleceniem `list "registry:/HKCU/Environment"`. |

## Powiązane {#related}

`host`, `write`, `replace`, `list`.
