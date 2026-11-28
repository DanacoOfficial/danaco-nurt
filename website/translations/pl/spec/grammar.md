<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/grammar.md -->
<!-- Responsibility:   Documents Grammar. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Gramatyka {#grammar}

Gramatyka tekstu programu w EBNF. Wzorcem jest parser skrzynki `nurt-syntax`;
drzewo powierzchniowe każdej produkcji znajduje się w
[`syntax-tree.md`](syntax-tree.md). Notacja: `{ x }` oznacza zero lub więcej,
`[ x ]` oznacza element opcjonalny, `( a | b )` grupuje, słowo w cudzysłowie to
terminal, `NL` to koniec wiersza kończący instrukcję, a `IDENT` to `[a-z][a-z0-9_]*`.

## 1. Program i instrukcje {#1-program-and-statements}

```ebnf
program     = { NL } , statement , { NL , { NL } , statement } , { NL } ;
statement   = let | pipeline | when | each | repeat | while
            | "break" | "continue" | steps | define | params ;
let         = "let" , IDENT , "=" , pipeline ;
pipeline    = head , { "|>" , stage } ;
head        = stage | call | ref ;
ref         = "@" , IDENT , { "." , IDENT } ;
```

Koniec wiersza wewnątrz nawiasów nie kończy instrukcji, a wiersz zaczynający się
od `|>` kontynuuje potok z poprzedniego wiersza. Wewnątrz nawiasów klamrowych
bloku każdy koniec wiersza kończy instrukcję.

## 2. Warunki {#2-conditions}

```ebnf
when        = "when" , condition , block ,
              { "otherwise" , "when" , condition , block } ,
              [ "otherwise" , block ] ;
condition   = expression | "exists" , operand ;
block       = "{" , { NL } , statement , { NL , { NL } , statement } , { NL } , "}" ;
```

`otherwise` może następować po klamrze zamykającej w tym samym wierszu lub po
końcach wierszy. Warunek jest wyrażeniem typu `bool`: porównania, `and`, `or`,
`not`, `exists <path>`, potok odczytu w nawiasach, `exit_code` oraz pola rekordu
instrukcji `let`.

## 3. Pętle {#3-loops}

```ebnf
each        = "each" , expression , "as" , IDENT , { option } , block ;
repeat      = "repeat" , { option } , block ;
while       = "while" , condition , { option } , block ;
option      = IDENT , [ ":" , expression ] ;
```

Opcje: `parallel` i `atomic` dla `each`; `until`, `every`, `limit` i `atomic` dla
`repeat`, gdzie `until` jest wymagana; `limit` i `atomic` dla `while`. `limit`
jest czasem trwania lub liczbą iteracji. Opcja zapisana bez wartości ma wartość
`true`.

## 4. Kilka zmian w jednym wywołaniu {#4-several-changes-in-one-call}

```ebnf
steps       = "steps" , block ;
```

## 5. Funkcje {#5-functions}

```ebnf
define      = "define" , IDENT , "(" , [ param , { "," , param } ] , ")" , block ;
param       = IDENT , ":" , type , [ "=" , expression ] ;
type        = IDENT , [ "<" , type , ">" | "(" , IDENT , { "," , IDENT } , ")" ] ;
call        = IDENT , "(" , [ argument , { "," , argument } ] , ")" ;
```

Wywołanie jest zapisywane z nawiasami okrągłymi i przecinkami we wszystkich
trzech wariantach składni, dla nazwy zdefiniowanej wcześniej za pomocą `define`.

## 6. Parametry skryptu {#6-script-parameters}

```ebnf
params      = "params" , "{" , { NL } , param , { NL , { NL } , param } , { NL } , "}" ;
```

`params` jest pierwszą instrukcją pliku.

## 7. Etapy i wyrażenia {#7-stages-and-expressions}

```ebnf
stage       = command , { argument }                        (* variant A *)
            | command , "(" , [ argument , { "," , argument } ] , ")"   (* variant B *)
            | command , { IDENT , "=" , expression } ;      (* variant C *)
command     = IDENT , { "." , IDENT } ;
argument    = [ IDENT , ":" ] , expression ;
expression  = or ;
or          = and , { "or" , and } ;
and         = not , { "and" , not } ;
not         = [ "not" ] , comparison ;
comparison  = sum , [ ( "==" | "!=" | "<" | "<=" | ">" | ">=" | "~" | "!~" | "in" ) , sum ] ;
sum         = product , { ( "+" | "-" ) , product } ;
product     = unary , { ( "*" | "/" ) , unary } ;
unary       = [ "-" ] , postfix ;
postfix     = operand , { "." , IDENT } ;
operand     = literal | ref | IDENT | call | "(" , ( expression | pipeline ) , ")" | list | map ;
```

Potok w nawiasach jest wartością: w warunku lub źródle jest wykonywany i daje
jedyne pole jedynego rekordu, jedyny rekord albo listę rekordów. Potok postaci
`((a …) |> b)` jest potokiem `a … |> b`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
