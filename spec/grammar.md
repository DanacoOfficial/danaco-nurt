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

# Grammar

The grammar of program text in EBNF. The parser of the crate `nurt-syntax` is
the reference; the surface tree of every production is in
[`syntax-tree.md`](syntax-tree.md). The notation: `{ x }` is zero or more,
`[ x ]` is optional, `( a | b )` groups, a quoted word is a terminal, `NL` is a
line break that ends a statement, `IDENT` is `[a-z][a-z0-9_]*`.

## 1. Program and statements

```ebnf
program     = { NL } , statement , { NL , { NL } , statement } , { NL } ;
statement   = let | pipeline | when | each | repeat | while
            | "break" | "continue" | steps | define | params ;
let         = "let" , IDENT , "=" , pipeline ;
pipeline    = head , { "|>" , stage } ;
head        = stage | call | ref ;
ref         = "@" , IDENT , { "." , IDENT } ;
```

A line break inside brackets does not end a statement, and a line that starts
with `|>` continues the pipeline of the line before. Inside the braces of a
block every line break ends a statement.

## 2. Conditions

```ebnf
when        = "when" , condition , block ,
              { "otherwise" , "when" , condition , block } ,
              [ "otherwise" , block ] ;
condition   = expression | "exists" , operand ;
block       = "{" , { NL } , statement , { NL , { NL } , statement } , { NL } , "}" ;
```

`otherwise` may follow the closing brace on the same line or after line
breaks. A condition is an expression of type `bool`: comparisons, `and`, `or`,
`not`, `exists <path>`, a reading pipeline in parentheses, `exit_code` and the
fields of the record of a `let`.

## 3. Loops

```ebnf
each        = "each" , expression , "as" , IDENT , { option } , block ;
repeat      = "repeat" , { option } , block ;
while       = "while" , condition , { option } , block ;
option      = IDENT , [ ":" , expression ] ;
```

Options: `parallel` and `atomic` for `each`; `until`, `every`, `limit` and
`atomic` for `repeat`, where `until` is required; `limit` and `atomic` for
`while`. `limit` is a duration or a number of iterations. An option written
without a value is `true`.

## 4. Several changes in one call

```ebnf
steps       = "steps" , block ;
```

## 5. Functions

```ebnf
define      = "define" , IDENT , "(" , [ param , { "," , param } ] , ")" , block ;
param       = IDENT , ":" , type , [ "=" , expression ] ;
type        = IDENT , [ "<" , type , ">" | "(" , IDENT , { "," , IDENT } , ")" ] ;
call        = IDENT , "(" , [ argument , { "," , argument } ] , ")" ;
```

A call is written with parentheses and commas in all three syntax variants,
for a name defined earlier with `define`.

## 6. Script parameters

```ebnf
params      = "params" , "{" , { NL } , param , { NL , { NL } , param } , { NL } , "}" ;
```

`params` is the first statement of a file.

## 7. Stages and expressions

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

A pipeline in parentheses is a value: in a condition or a source it runs and
yields the only field of its only record, the only record, or the list of
records. A pipeline of the form `((a …) |> b)` is the pipeline `a … |> b`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
