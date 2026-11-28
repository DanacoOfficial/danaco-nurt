---
command: sleep
group: system
summary: Oczekuje przez zadany czas.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# sleep

## Składnia {#synopsis}

```text
sleep <duration>
```

## Opis {#description}

`sleep` blokuje program na podany czas, a następnie wraca bez wyjścia.
Rozpoczyna potok i nie przyjmuje wejścia.

Oczekiwanie jest odmierzane zegarem wykonania i przerywane przez anulowanie
oraz przez limit czasu programu. Anulowany program zwraca `E_CANCELLED` w
ciągu 100 ms od sygnału. Oczekiwanie `0s` wraca natychmiast.

Oczekiwanie, które osiąga limit czasu programu, kończy się na limicie błędem
`E_TIMEOUT`. Błąd zawiera żądany czas w `expected` i czas faktycznie
odczekany w `actual`, oba w milisekundach. Gdy program nie ustawia `timeout`,
jego limit czasu jest podnoszony do najdłuższego `sleep`, jaki zawiera, więc
oczekiwanie dłuższe niż domyślny limit `run` nie jest ucinane; jawny `timeout`
programu zawsze obowiązuje.

Polecenie niczego nie zmienia. Nie otwiera operacji dziennika i nie ma wpisu w
historii operacji. Odczekany czas jest zgłaszany jako zdarzenie postępu, z
odczekanymi milisekundami jako `done` i żądanymi milisekundami jako `total`.
Sprawdzenie programu (`nurt_check`) zgłasza `waits: true` i żądany czas
trwania bez oczekiwania.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `duration` | `duration` | wymagany | Czas oczekiwania. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `none` | `none` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| Oczekiwanie | zawsze blokuje wywołanie |

## Przykłady {#examples}

Oczekiwanie przez pół sekundy:

```nurt example
sleep 500ms
```

Oczekiwanie przez dwie sekundy:

```nurt example
sleep 2s
```

Utworzenie harmonogramu, którego akcja czeka dwie sekundy, a następnie
jednorazowe uruchomienie akcji poza jej terminami:

```nurt example
schedule.create "pause" every: 10m action: (sleep 2s)
schedule.run "s1"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | Czas trwania jest ujemny albo jego koniec wykracza poza zakres zegara. | Podaj nieujemny czas trwania, taki jak `500ms` lub `2s`. |
| `E_CANCELLED` | Wykonanie zostało anulowane przed oczekiwaniem lub w jego trakcie. | Brak; anulowanie zażądał wywołujący. |
| `E_TIMEOUT` | Limit czasu programu zakończył oczekiwanie. `expected` zawiera żądane milisekundy, a `actual` odczekane milisekundy. | Ustaw dłuższy `timeout` programu lub krótszy czas trwania. |

## Powiązane {#related}

`schedule.create` i `schedule.run` używają `sleep` w akcji harmonogramu. `run`
uruchamia program i czeka na niego za pomocą parametru `wait`. Strona grupy
`system` wylicza pozostałe polecenia grupy.
