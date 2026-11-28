---
command: windows.cim
group: administration
summary: Odpytuje instancje CIM przez WMI w systemie Windows albo wywołuje metodę wybranych instancji za pomocą invoke.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# windows.cim

## Składnia {#synopsis}

```text
windows.cim <class> [name: value]...
```

## Opis {#description}

`windows.cim` odczytuje instancje klasy CIM z lokalnej usługi Windows
Management Instrumentation i zwraca jeden rekord `cim_object` na każdą
instancję. Polecenie buduje instrukcję WQL
`SELECT <properties> FROM <class> [WHERE <where>]` ze swoich parametrów;
wszystkie właściwości są odczytywane, gdy `properties` jest pominięte lub
puste.

`class` i każdy element `properties` muszą być nazwami CIM
(`[A-Za-z_][A-Za-z0-9_]*`), więc nie mogą zmienić instrukcji. `where` jest
warunkiem WQL na właściwościach klasy. Musi mieć jeden wiersz ze
zrównoważonymi literałami łańcuchowymi `'` i `"` oraz nie może zawierać form
zapytań asocjacyjnych `ASSOCIATORS OF` ani `REFERENCES OF`. `namespace` jest
zapisywany z separatorami `/`, na przykład `root/Microsoft/Windows/Storage`.

Wartości właściwości są konwertowane na wartości języka według ich typu CIM:

| Typ CIM | Wartość |
|---|---|
| `string`, `char16`, `reference` | `text` |
| `sint8` to `uint64` | `int`; wartości `uint64` powyżej zakresu `int` zachowują wszystkie cyfry |
| `real32`, `real64` | `number` |
| `boolean` | `bool` |
| `datetime`, point in time | `instant` w UTC; przesunięcie wartości jest uwzględniane |
| `datetime`, interval | `duration` |
| tablice | `list` |
| obiekty osadzone | `map` z polem `__class`, do czterech poziomów głębokości |

Właściwości bez wartości są pomijane w `properties`. Wartość `datetime` z
nieznanymi polami (`*`) jest zwracana jako `text`.

`path` zawiera ścieżkę obiektu zgłoszoną przez WMI. Gdy `properties` pomija
właściwości kluczowe klasy, WMI nie zgłasza ścieżki obiektu, a `path` jest
puste.

Pozostała część potoku może ograniczyć liczbę rekordów, na przykład za pomocą
`first`; zapytanie przestaje wtedy odczytywać po tej liczbie instancji.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `class` | `text` | wymagany | Nazwa klasy CIM, na przykład Win32_Process. |
| `where` | `text` | brak | Warunek WQL na właściwościach instancji. |
| `namespace` | `text` | `"root/cimv2"` | Przestrzeń nazw CIM. |
| `properties` | `list<text>` | brak | Właściwości do zwrócenia; wszystkie, gdy pominięte. |
| `invoke` | `text` | brak | Metoda do wywołania na każdej wybranej instancji albo na klasie, gdy where jest pominięte (metody statyczne, takie jak Win32_Process.Create). |
| `arguments` | `map<any>` | brak | Parametry wejściowe metody według nazwy: wartości typu text, int lub bool. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `none` | `cim_object`, `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| Warunki odwracalności | Zapytanie tylko odczytuje. Wywołanie z invoke jest nieodwracalne, chyba że metoda ma udokumentowaną metodę przeciwną (StartService i StopService, Enable i Disable, Pause i Resume), którą cofnięcie wywołuje na tych samych instancjach. |
| Systemy operacyjne | `windows` |

| Akcja | Skutek | Odwracalność | Uwaga |
|---|---|---|---|
| `invoke` | `external` | `irreversible` | Dotyczy przypadku, gdy podano invoke. |

## Przykłady {#examples}

Odczyt nazwy i wersji systemu operacyjnego.

```nurt example
windows.cim "Win32_OperatingSystem" properties: ["Caption", "Version"]
```

Wylistowanie uruchomionych usług.

```nurt example
windows.cim "Win32_Service" where: "State = 'Running'" properties: ["Name", "ProcessId"]
```

Wylistowanie dysków znanych dostawcy zarządzania magazynem.

```nurt example
windows.cim "MSFT_Disk" namespace: "root/Microsoft/Windows/Storage"
```

Znalezienie procesów według nazwy z zachowaniem wierszy poleceń.

```nurt example
windows.cim "Win32_Process" where: "Name = 'svchost.exe'" properties: ["ProcessId", "CommandLine"] |> first 5
```

Zatrzymanie usługi; `undo` uruchamia ją ponownie.

```nurt example
windows.cim "Win32_Service" where: "Name = 'Spooler'" invoke: "StopService" privilege: elevated
```

Uruchomienie procesu przez metodę statyczną klasy.

```nurt example
windows.cim "Win32_Process" invoke: "Create" arguments: {"CommandLine": "notepad.exe"} privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_OS_ONLY` | Polecenie działa tylko w systemie Windows. | W innych systemach użyj `host`, `processes` lub `services`. |
| `E_INVALID_VALUE` | Klasa, właściwość lub przestrzeń nazw nie jest nazwą CIM, warunek jest nieprawidłowy albo WMI odrzuciło zapytanie. | Popraw nazwę lub warunek. |
| `E_NOT_FOUND` | Klasa lub przestrzeń nazw nie istnieje. | Sprawdź nazwę klasy i przestrzeń nazw. |
| `E_PRIVILEGE` | Odczyt przestrzeni nazw wymaga uprawnienia `administrator`. | Uruchom konsolę jako administrator. |
| `E_STAGE_FAILED` | Metoda zwróciła `ReturnValue` inne niż 0; `return_value` je zawiera. | Odczytaj dokumentację metody, aby poznać kod. |
| `E_LIMIT` | Zapytanie zwraca więcej instancji, niż może pomieścić etap. | Zawęź zapytanie za pomocą `where`. |
| `E_IO` | WMI lub COM zawiodło; `os_error` zawiera `HRESULT`. | Sprawdź, czy usługa WMI działa. |

## Powiązane {#related}

`services`, `host`, `processes`, `disks`.

## Uwagi {#notes}

Z `invoke` polecenie wywołuje metodę klasy zamiast odczytywać. `where` wybiera
instancje, które otrzymują wywołanie; bez `where` wywoływana jest metoda
klasy, jak dla metody statycznej `Create` klasy `Win32_Process`. `arguments`
zawiera parametry wejściowe metody według nazwy; wartością jest tekst, liczba
całkowita lub wartość logiczna. `invoke` i nazwy w `arguments` muszą być
nazwami CIM. Wywołanie jest skutkiem programu: podgląd odczytuje wybrane
instancje i pokazuje wywołanie bez jego wykonania, a metoda, która zwraca
`ReturnValue` inne niż 0, kończy wywołanie błędem `E_STAGE_FAILED`; rekord
skutku zawiera `return_value`.

Wywołanie jest zmianą celu `cim:<class>.<method>`. Wywołanie jest
nieodwracalne, chyba że klasa dokumentuje metodę przeciwną, którą `undo`
wywołuje na tych samych instancjach; wywołanie bez `where` nie ma instancji,
na których można powtórzyć metodę przeciwną, więc pozostaje nieodwracalne.

| Klasa | Metoda | Przeciwna |
|---|---|---|
| `Win32_Service` | `StartService`, `StopService` | wzajemnie |
| `Win32_NetworkAdapter`, `Win32_PnPEntity` | `Enable`, `Disable` | wzajemnie |
| `Win32_Printer` | `Pause`, `Resume` | wzajemnie |
