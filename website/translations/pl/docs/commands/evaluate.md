---
command: evaluate
group: text
summary: Oblicza wyrażenie z arytmetyką, czasami, rozmiarami i funkcjami wbudowanymi.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# evaluate

## Składnia {#synopsis}

```text
evaluate <expression>
```

## Opis {#description}

`evaluate` oblicza wartość jednego wyrażenia i zwraca ją wraz z jej typem.
Rozpoczyna potok, więc wartość można przekazać do dalszych etapów, i jest
postacią Nurt poleceń powłoki, które wypisują obliczoną wartość (`date`,
`echo`, `Get-Date`, `Write-Output`).

Wyrażenie używa operatorów i funkcji wbudowanych języka: arytmetyki na liczbach
całkowitych, liczbach, rozmiarach i czasach trwania; `instant - instant` jest
czasem trwania, a `instant +- duration` jest chwilą; `+` łączy teksty. Czas jest
odczytywany z zegara wykonania, więc `now()` i `ago(7d)` dają czas wywołania.
Wyrażenie jest obliczane przez ewaluator wykonania; nie istnieje żadne inne
obliczanie, więc `filter`, `select` i `evaluate` dają tę samą wartość dla tego
samego wyrażenia.

Wynikiem jest jeden rekord `value` z polami `value` i `type`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `expression` | `expression` | wymagany | Wyrażenie do obliczenia. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `value` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Obliczenie liczby całkowitej.

```nurt example
evaluate 6 * 7
```

```text expected
42
```

Połączenie dwóch tekstów.

```nurt example
evaluate "Release " + "1.0"
```

```text expected
Release 1.0
```

Obliczenie czasu sprzed trzech dni.

```nurt example
evaluate now() - 3d
```

Obliczenie rozmiaru.

```nurt example
evaluate 1.5gib / 3
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_TYPE` | operand ma typ, którego operator nie przyjmuje, na przykład `"a" - 1` | przekonwertuj operand albo zmień operator |
| `E_MISSING_PARAM` | brakuje wyrażenia | podaj wyrażenie jako pierwszy argument |

## Powiązane {#related}

`describe`, `filter`, `select`, `help`.
