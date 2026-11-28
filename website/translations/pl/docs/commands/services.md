---
command: services
group: administration
summary: Wyświetla usługi systemowe lub użytkownika wraz z ich stanem i typem uruchamiania.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# services

## Składnia {#synopsis}

```text
services [<name>] [name: value]...
```

## Opis {#description}

Odczyt stanu jednostek systemowych nie wymaga uprawnień; `start`, `stop`, `restart`, `enable` i `disable` na jednostkach systemowych wymagają administratora, co zapewnia `privilege: elevated` na ostatnim etapie (zobacz te polecenia).

`services` wyświetla usługi menedżera usług systemu: systemd w systemie Linux, menedżer kontroli usług w systemie Windows i launchd w systemie macOS. Rozpoczyna potok i tworzy jeden rekord `service` dla każdej usługi; w systemie Linux rekordy są sortowane według nazwy.

Tekstowe `name` wybiera jedną usługę w sposób, w jaki menedżer rozwiązuje nazwy: w systemie Linux `nginx` i `nginx.service` oznaczają tę samą jednostkę, a instancje szablonów, takie jak `getty@tty1`, są akceptowane; w systemie Windows porównanie ignoruje wielkość liter. Gdy menedżer nie zna takiej usługi, polecenie kończy się błędem `E_NOT_FOUND`. `name` w postaci wyrażenia regularnego zachowuje usługi, których nazwa do niego pasuje.

`state` zachowuje usługi, których stan, stan surowy lub jedna część stanu surowego oddzielona znakiem `/` jest równa tekstowi, bez uwzględniania wielkości liter. `state: "failed"` pasuje do stanu wspólnego; `state: "exited"` pasuje do surowego stanu systemd `active/exited`.

W systemie Linux polecenie łączy załadowane jednostki z plikami jednostek, które są zainstalowane, ale nie są załadowane, więc wyłączona usługa, która nigdy nie działała, jest wyświetlana jako `stopped`. Jednostki innych typów i szablony jednostek nie są wyświetlane. Menedżer jest osiągany przez D-Bus; żaden program zewnętrzny nie jest uruchamiany. `scope: user` odczytuje menedżera bieżącego konta na magistrali sesji.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text \| regex` | none | Nazwa usługi lub wzorzec. |
| `state` | `text` | none | Tylko usługi w tym stanie. |
| `scope` | `enum(system\|user)` | `system` | Usługi systemowe lub usługi bieżącego konta. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `service` |

Rekord `service` zawiera `name`, `state` (`running`, `stopped`, `starting`, `stopping`, `failed`, `paused`, `unknown`), `start_type` (`automatic`, `manual`, `disabled`, `on_demand`, `static`, `unknown`), `manager`, `scope` oraz, gdy menedżer je zgłasza, `state_raw`, `description`, `pid`, `path`, `account` i `os_raw`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| External use | `system_program`: `launchctl`; W systemie macOS stan usług jest odczytywany przez składnik launchctl systemu operacyjnego. |

## Przykłady {#examples}

Pokazanie jednej usługi:

```nurt example
services name: "cron"
```

Wyświetlenie usług, które zakończyły się niepowodzeniem:

```nurt example
services state: "failed"
```

Wyświetlenie działających usług bieżącego konta:

```nurt example
services scope: user state: "running"
```

Ponowne uruchomienie jednej usługi:

```nurt example
services name: "nginx" |> restart
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego uruchamiania polecenia.

```text
matched 230  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Żadna usługa nie ma podanej nazwy. | Wyświetl usługi poleceniem `services` i użyj nazwy z listy. |
| `E_REGEX` | Wzorzec `name` nie jest poprawnym wyrażeniem regularnym. | Popraw wzorzec. |
| `E_INVALID_VALUE` | `scope` nie jest ani `system`, ani `user`. | Użyj `scope: system` lub `scope: user`. |
| `E_PRIVILEGE` | Menedżer odmówił pokazania usługi; `required` wskazuje uprawnienie. | Uruchom jako konto z tym uprawnieniem. |
| `E_SERVICE` | Menedżer zgłosił błąd lub jest nieosiągalny. `os_code` ma wartość `no_systemd`, gdy systemd nie jest menedżerem usług, i `no_user_bus`, gdy konto nie ma magistrali sesji. | Użyj `scope: system` lub uruchom w systemie zarządzanym przez systemd. |
| `E_UNSUPPORTED` | Zakres nie jest dostępny w tym systemie. | Użyj `scope: system`. |

## Powiązane {#related}

`start`, `restart`, `enable` i `disable` działają na rekordach `services`; `stop` je zatrzymuje. `logs unit:` pokazuje dziennik usługi. Strona grupy `administration` wyświetla pozostałe polecenia grupy.
