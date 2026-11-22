// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Corpus of structural patterns: every case names a language, a pattern,
//! a source text and the text of every expected match in source order.
//!
//! The corpus is checked against the code service and, when the `ast-grep`
//! program is installed, against the match counts of `ast-grep run`.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use nurt_code::service::CodeAnalysis;
use nurt_contracts::services::{CodeService, LanguageId};

/// One corpus case.
struct Case {
    language: &'static str,
    pattern: &'static str,
    source: &'static str,
    expected: &'static [&'static str],
}

macro_rules! case {
    ($language:literal, $pattern:literal, $source:literal, [$($expected:literal),* $(,)?]) => {
        Case {
            language: $language,
            pattern: $pattern,
            source: $source,
            expected: &[$($expected),*],
        }
    };
}

/// Smallest number of cases and languages of the corpus.
const MIN_CASES: usize = 200;
const MIN_LANGUAGES: usize = 10;

const CORPUS: &[Case] = &[
    case!(
        "rust",
        "$A.unwrap()",
        "fn f() { let x = a.unwrap(); }",
        ["a.unwrap()"]
    ),
    case!("rust", "$A.unwrap()", "fn f() { a.expect(\"x\"); }", []),
    case!(
        "rust",
        "$A == $A",
        "fn f() { if x == x {} if x == y {} }",
        ["x == x"]
    ),
    case!(
        "rust",
        "let $A = $B;",
        "fn f() { let x = 1; let y = g(); }",
        ["let x = 1;", "let y = g();"]
    ),
    case!(
        "rust",
        "foo($A)",
        "fn f() { foo(1); foo(1, 2); }",
        ["foo(1)"]
    ),
    case!(
        "rust",
        "foo($$$ARGS)",
        "fn f() { foo(); foo(1, 2); }",
        ["foo()", "foo(1, 2)"]
    ),
    case!(
        "rust",
        "foo($A, $$$REST)",
        "fn f() { foo(); foo(1, 2, 3); }",
        ["foo(1, 2, 3)"]
    ),
    case!(
        "rust",
        "$X.clone()",
        "fn f() { a.clone(); b.c.clone(); }",
        ["a.clone()", "b.c.clone()"]
    ),
    case!(
        "rust",
        "Some($V)",
        "fn f() { let a = Some(1); let b = None; }",
        ["Some(1)"]
    ),
    case!(
        "rust",
        "if $C { $$$B }",
        "fn f() { if a { b(); } }",
        ["if a { b(); }"]
    ),
    case!(
        "rust",
        "&mut $A",
        "fn f() { g(&mut x); g(&x); }",
        ["&mut x"]
    ),
    case!(
        "rust",
        "$A as $T",
        "fn f() { let a = b as u32; }",
        ["b as u32"]
    ),
    case!(
        "rust",
        "$A?",
        "fn f() -> R { let a = g()?; Ok(a) }",
        ["g()?"]
    ),
    case!(
        "rust",
        "return $A",
        "fn f() -> i32 { return 1; }",
        ["return 1"]
    ),
    case!(
        "rust",
        "$A + 1",
        "fn f() { let a = b + 1; let c = b + 2; }",
        ["b + 1"]
    ),
    case!(
        "rust",
        "fn $F() { $$$B }",
        "fn a() { x(); }\nfn b(y: i32) {}\n",
        ["fn a() { x(); }"]
    ),
    case!(
        "python",
        "print($A)",
        "print(1)\nprint(1, 2)\n",
        ["print(1)"]
    ),
    case!(
        "python",
        "print($$$A)",
        "print()\nprint(1, 2)\n",
        ["print()", "print(1, 2)"]
    ),
    case!("python", "$A == $A", "a == a\na == b\n", ["a == a"]),
    case!(
        "python",
        "len($X) == 0",
        "if len(items) == 0:\n    pass\n",
        ["len(items) == 0"]
    ),
    case!(
        "python",
        "$A.append($B)",
        "xs.append(1)\nxs.extend(y)\n",
        ["xs.append(1)"]
    ),
    case!(
        "python",
        "return $A",
        "def f():\n    return 1\n",
        ["return 1"]
    ),
    case!(
        "python",
        "import $M",
        "import os\nfrom x import y\n",
        ["import os"]
    ),
    case!(
        "python",
        "$A is None",
        "if a is None:\n    pass\nif a is not None:\n    pass\n",
        ["a is None"]
    ),
    case!(
        "python",
        "open($F)",
        "with open(p) as f:\n    pass\n",
        ["open(p)"]
    ),
    case!("python", "$F($$$A)", "a(b(1))\n", ["a(b(1))", "b(1)"]),
    case!(
        "python",
        "lambda $X: $Y",
        "f = lambda x: x + 1\n",
        ["lambda x: x + 1"]
    ),
    case!(
        "python",
        "[$X for $X in $Y]",
        "a = [i for i in xs]\nb = [i * 2 for i in xs]\n",
        ["[i for i in xs]"]
    ),
    case!(
        "python",
        "$A[0]",
        "x = items[0]\ny = items[1]\n",
        ["items[0]"]
    ),
    case!(
        "python",
        "isinstance($A, str)",
        "isinstance(a, str)\nisinstance(a, int)\n",
        ["isinstance(a, str)"]
    ),
    case!("python", "not $A", "if not ok:\n    pass\n", ["not ok"]),
    case!(
        "python",
        "raise $E",
        "raise ValueError(\"x\")\n",
        ["raise ValueError(\"x\")"]
    ),
    case!(
        "javascript",
        "console.log($A)",
        "console.log(1);\nconsole.log(1, 2);\n",
        ["console.log(1)"]
    ),
    case!(
        "javascript",
        "console.log($$$A)",
        "console.log(1);\nconsole.log(1, 2);\n",
        ["console.log(1)", "console.log(1, 2)"]
    ),
    case!("javascript", "$A === $A", "a === a; a === b;", ["a === a"]),
    case!(
        "javascript",
        "const $A = $B",
        "const a = 1;\nlet b = 2;\n",
        ["const a = 1;"]
    ),
    case!(
        "javascript",
        "require($M)",
        "const fs = require(\"fs\");\n",
        ["require(\"fs\")"]
    ),
    case!(
        "javascript",
        "$A.then($B)",
        "p.then(f);\np.catch(g);\n",
        ["p.then(f)"]
    ),
    case!(
        "javascript",
        "new $C($$$A)",
        "const d = new Date();\nconst m = new Map(x);\n",
        ["new Date()", "new Map(x)"]
    ),
    case!(
        "javascript",
        "typeof $A === \"undefined\"",
        "if (typeof x === \"undefined\") {}\nif (typeof x === \"object\") {}\n",
        ["typeof x === \"undefined\""]
    ),
    case!(
        "javascript",
        "$A ? $B : $C",
        "const v = a ? 1 : 2;\n",
        ["a ? 1 : 2"]
    ),
    case!(
        "javascript",
        "($A) => $B",
        "const f = (x) => x + 1;\nconst g = (x, y) => x;\n",
        ["(x) => x + 1"]
    ),
    case!(
        "javascript",
        "await $P",
        "async function f() { await g(); }\n",
        ["await g()"]
    ),
    case!(
        "javascript",
        "$A.length",
        "const n = xs.length + ys.size;\n",
        ["xs.length"]
    ),
    case!(
        "javascript",
        "JSON.parse($A)",
        "JSON.parse(s); JSON.stringify(o);",
        ["JSON.parse(s)"]
    ),
    case!(
        "javascript",
        "document.getElementById($ID)",
        "document.getElementById(\"a\"); document.querySelector(\"b\");",
        ["document.getElementById(\"a\")"]
    ),
    case!(
        "javascript",
        "$A += 1",
        "i += 1; i += 2; i -= 1;",
        ["i += 1"]
    ),
    case!(
        "javascript",
        "function $F($$$P) { $$$B }",
        "function add(a, b) { return a + b; }\n",
        ["function add(a, b) { return a + b; }"]
    ),
    case!(
        "typescript",
        "$A as any",
        "const x = y as any;\nconst z = y as string;\n",
        ["y as any"]
    ),
    case!(
        "typescript",
        "console.log($$$A)",
        "console.log(\"a\", b);\n",
        ["console.log(\"a\", b)"]
    ),
    case!(
        "typescript",
        "let $A: $T = $V",
        "let a: number = 1;\nlet b = 2;\n",
        ["let a: number = 1;"]
    ),
    case!("typescript", "$A!", "const v = maybe!;\n", ["maybe!"]),
    case!(
        "typescript",
        "interface $I { $$$M }",
        "interface P { x: number; }\ntype Q = {};\n",
        ["interface P { x: number; }"]
    ),
    case!(
        "typescript",
        "function $F($$$P): $R { $$$B }",
        "function f(a: number): string { return \"\"; }\n",
        ["function f(a: number): string { return \"\"; }"]
    ),
    case!(
        "typescript",
        "$A?.$B",
        "const v = a?.b;\nconst w = a.b;\n",
        ["a?.b"]
    ),
    case!(
        "typescript",
        "import { $$$N } from \"m\"",
        "import { a, b } from \"m\";\nimport c from \"n\";\n",
        ["import { a, b } from \"m\";"]
    ),
    case!(
        "typescript",
        "new Promise($F)",
        "new Promise((r) => r(1));\n",
        ["new Promise((r) => r(1))"]
    ),
    case!(
        "typescript",
        "$A === null",
        "if (a === null) {}\n",
        ["a === null"]
    ),
    case!(
        "typescript",
        "throw new Error($M)",
        "throw new Error(\"x\");\n",
        ["throw new Error(\"x\");"]
    ),
    case!(
        "typescript",
        "$X.map($F)",
        "xs.map(f).filter(g);\n",
        ["xs.map(f)"]
    ),
    case!(
        "typescript",
        "type $T = $V",
        "type A = string;\ninterface B {}\n",
        ["type A = string;"]
    ),
    case!("typescript", "$A ?? $B", "const v = a ?? b;\n", ["a ?? b"]),
    case!(
        "typescript",
        "enum $E { $$$M }",
        "enum Color { Red, Green }\n",
        ["enum Color { Red, Green }"]
    ),
    case!(
        "go",
        "fmt.Println($$$A)",
        "package main\nfunc main() { fmt.Println(1, 2); fmt.Printf(\"x\") }\n",
        ["fmt.Println(1, 2)"]
    ),
    case!(
        "go",
        "if err != nil { $$$B }",
        "package main\nfunc f() error { if err != nil { return err }; return nil }\n",
        ["if err != nil { return err }"]
    ),
    case!(
        "go",
        "$A == $A",
        "package main\nfunc f() { _ = x == x; _ = x == y }\n",
        ["x == x"]
    ),
    case!(
        "go",
        "errors.New($M)",
        "package main\nvar e = errors.New(\"x\")\n",
        ["errors.New(\"x\")"]
    ),
    case!(
        "go",
        "$X := $Y",
        "package main\nfunc f() { a := 1; b = 2 }\n",
        ["a := 1"]
    ),
    case!(
        "go",
        "len($S)",
        "package main\nfunc f() { n := len(s) + cap(s) }\n",
        ["len(s)"]
    ),
    case!(
        "go",
        "defer $F()",
        "package main\nfunc f() { defer close(); defer g(1) }\n",
        ["defer close()"]
    ),
    case!(
        "go",
        "go $F($$$A)",
        "package main\nfunc f() { go run(1) }\n",
        ["go run(1)"]
    ),
    case!(
        "go",
        "make([]$T, $N)",
        "package main\nfunc f() { a := make([]int, 3); b := make(map[string]int) }\n",
        ["make([]int, 3)"]
    ),
    case!(
        "go",
        "$A.Close()",
        "package main\nfunc f() { r.Close(); r.Open() }\n",
        ["r.Close()"]
    ),
    case!(
        "go",
        "func $F() { $$$B }",
        "package main\nfunc a() { x() }\nfunc b(y int) {}\n",
        ["func a() { x() }"]
    ),
    case!(
        "go",
        "return $A, nil",
        "package main\nfunc f() (int, error) { return 1, nil }\n",
        ["return 1, nil"]
    ),
    case!(
        "go",
        "for _, $V := range $L { $$$B }",
        "package main\nfunc f() { for _, v := range xs { use(v) } }\n",
        ["for _, v := range xs { use(v) }"]
    ),
    case!(
        "go",
        "panic($M)",
        "package main\nfunc f() { panic(\"x\") }\n",
        ["panic(\"x\")"]
    ),
    case!(
        "go",
        "$S[$I]",
        "package main\nfunc f() { a := xs[0] }\n",
        ["xs[0]"]
    ),
    case!(
        "java",
        "System.out.println($A)",
        "class A { void f() { System.out.println(1); System.out.print(2); } }",
        ["System.out.println(1)"]
    ),
    case!(
        "java",
        "$A.equals($B)",
        "class A { boolean f() { return a.equals(b); } }",
        ["a.equals(b)"]
    ),
    case!(
        "java",
        "new $T($$$A)",
        "class A { void f() { Object o = new ArrayList<>(); } }",
        ["new ArrayList<>()"]
    ),
    case!(
        "java",
        "$A == null",
        "class A { void f() { if (a == null) {} } }",
        ["a == null"]
    ),
    case!(
        "java",
        "throw new $E($$$A);",
        "class A { void f() { throw new IllegalStateException(\"x\"); } }",
        ["throw new IllegalStateException(\"x\");"]
    ),
    case!(
        "java",
        "$L.size() == 0",
        "class A { boolean f() { return xs.size() == 0; } }",
        ["xs.size() == 0"]
    ),
    case!(
        "java",
        "return $A;",
        "class A { int f() { return 1; } }",
        ["return 1;"]
    ),
    case!(
        "java",
        "$A.get($B)",
        "class A { void f() { m.get(k); m.put(k, v); } }",
        ["m.get(k)"]
    ),
    case!(
        "java",
        "$A != $A",
        "class A { void f() { if (x != x) {} if (x != y) {} } }",
        ["x != x"]
    ),
    case!(
        "java",
        "String.format($$$A)",
        "class A { String f() { return String.format(\"%d\", 1); } }",
        ["String.format(\"%d\", 1)"]
    ),
    case!(
        "java",
        "$X++",
        "class A { void f() { i++; j--; } }",
        ["i++"]
    ),
    case!(
        "java",
        "Thread.sleep($N)",
        "class A { void f() { Thread.sleep(10); } }",
        ["Thread.sleep(10)"]
    ),
    case!(
        "java",
        "$A instanceof $T",
        "class A { boolean f() { return o instanceof String; } }",
        ["o instanceof String"]
    ),
    case!(
        "java",
        "Integer.parseInt($S)",
        "class A { int f() { return Integer.parseInt(s); } }",
        ["Integer.parseInt(s)"]
    ),
    case!(
        "java",
        "$A.isEmpty()",
        "class A { boolean f() { return s.isEmpty() || t.isBlank(); } }",
        ["s.isEmpty()"]
    ),
    case!(
        "c",
        "printf($$$A)",
        "int main() { printf(\"%d\", 1); puts(\"x\"); }",
        ["printf(\"%d\", 1)"]
    ),
    case!(
        "c",
        "malloc($N)",
        "void f() { char *p = malloc(10); }",
        ["malloc(10)"]
    ),
    case!(
        "c",
        "free($P)",
        "void f() { free(p); free(q); }",
        ["free(p)", "free(q)"]
    ),
    case!(
        "c",
        "$A == NULL",
        "void f() { if (p == NULL) {} }",
        ["p == NULL"]
    ),
    case!(
        "c",
        "sizeof($T)",
        "void f() { int n = sizeof(x); }",
        ["sizeof(x)"]
    ),
    case!("c", "$A += $B", "void f() { a += 1; a -= 1; }", ["a += 1"]),
    case!("c", "return $A;", "int f() { return 0; }", ["return 0;"]),
    case!(
        "c",
        "$X->$F",
        "void f() { int a = s->x; int b = s.y; }",
        ["s->x"]
    ),
    case!(
        "c",
        "strcpy($D, $S)",
        "void f() { strcpy(a, b); strncpy(a, b, 3); }",
        ["strcpy(a, b)"]
    ),
    case!("c", "!$A", "void f() { if (!ok) {} }", ["!ok"]),
    case!("c", "$A++", "void f() { i++; }", ["i++"]),
    case!("c", "exit($C)", "void f() { exit(1); }", ["exit(1)"]),
    case!("c", "$A[$I]", "void f() { int x = a[0]; }", ["a[0]"]),
    case!(
        "c",
        "memset($P, 0, $N)",
        "void f() { memset(b, 0, 4); memset(b, 1, 4); }",
        ["memset(b, 0, 4)"]
    ),
    case!(
        "c",
        "if ($C) return $V;",
        "int f() { if (x) return 1; return 0; }",
        ["if (x) return 1;"]
    ),
    case!(
        "cpp",
        "std::cout << $A",
        "int main() { std::cout << x; }",
        ["std::cout << x"]
    ),
    case!(
        "cpp",
        "std::move($A)",
        "void f() { g(std::move(v)); }",
        ["std::move(v)"]
    ),
    case!(
        "cpp",
        "new $T($$$A)",
        "void f() { auto p = new Foo(1); }",
        ["new Foo(1)"]
    ),
    case!("cpp", "delete $P", "void f() { delete p; }", ["delete p"]),
    case!(
        "cpp",
        "$A.push_back($B)",
        "void f() { v.push_back(1); v.pop_back(); }",
        ["v.push_back(1)"]
    ),
    case!(
        "cpp",
        "$A == nullptr",
        "void f() { if (p == nullptr) {} }",
        ["p == nullptr"]
    ),
    case!(
        "cpp",
        "std::make_unique<$T>($$$A)",
        "void f() { auto p = std::make_unique<Foo>(1, 2); }",
        ["std::make_unique<Foo>(1, 2)"]
    ),
    case!("cpp", "return $A;", "int f() { return 1; }", ["return 1;"]),
    case!(
        "cpp",
        "$A->$M($$$P)",
        "void f() { p->run(1); q.run(2); }",
        ["p->run(1)"]
    ),
    case!(
        "cpp",
        "static_cast<$T>($E)",
        "void f() { int a = static_cast<int>(x); }",
        ["static_cast<int>(x)"]
    ),
    case!(
        "cpp",
        "throw $E",
        "void f() { throw std::runtime_error(\"x\"); }",
        ["throw std::runtime_error(\"x\");"]
    ),
    case!(
        "cpp",
        "$A.size()",
        "void f() { auto n = v.size(); }",
        ["v.size()"]
    ),
    case!(
        "cpp",
        "std::sort($B, $E)",
        "void f() { std::sort(v.begin(), v.end()); }",
        ["std::sort(v.begin(), v.end())"]
    ),
    case!(
        "cpp",
        "$A && $A",
        "void f() { if (a && a) {} if (a && b) {} }",
        ["a && a"]
    ),
    case!(
        "cpp",
        "printf($$$A)",
        "void f() { printf(\"x\"); }",
        ["printf(\"x\")"]
    ),
    case!(
        "csharp",
        "Console.WriteLine($A)",
        "class A { void F() { Console.WriteLine(1); Console.Write(2); } }",
        ["Console.WriteLine(1)"]
    ),
    case!(
        "csharp",
        "$A == null",
        "class A { void F() { if (a == null) {} } }",
        ["a == null"]
    ),
    case!(
        "csharp",
        "new $T($$$A)",
        "class A { void F() { var l = new List<int>(); } }",
        ["new List<int>()"]
    ),
    case!(
        "csharp",
        "$A.ToString()",
        "class A { void F() { var s = x.ToString(); } }",
        ["x.ToString()"]
    ),
    case!(
        "csharp",
        "await $T",
        "class A { async Task F() { await G(); } }",
        ["await G()"]
    ),
    case!(
        "csharp",
        "return $A;",
        "class A { int F() { return 1; } }",
        ["return 1;"]
    ),
    case!(
        "csharp",
        "throw new $E($$$A);",
        "class A { void F() { throw new ArgumentException(\"x\"); } }",
        ["throw new ArgumentException(\"x\");"]
    ),
    case!(
        "csharp",
        "$A.Count == 0",
        "class A { bool F() { return xs.Count == 0; } }",
        ["xs.Count == 0"]
    ),
    case!(
        "csharp",
        "string.IsNullOrEmpty($S)",
        "class A { bool F() { return string.IsNullOrEmpty(s); } }",
        ["string.IsNullOrEmpty(s)"]
    ),
    case!(
        "csharp",
        "$A ?? $B",
        "class A { void F() { var v = a ?? b; } }",
        ["a ?? b"]
    ),
    case!(
        "csharp",
        "$A is null",
        "class A { void F() { if (a is null) {} } }",
        ["a is null"]
    ),
    case!("csharp", "$X++", "class A { void F() { i++; } }", ["i++"]),
    case!(
        "csharp",
        "Math.Max($A, $B)",
        "class A { int F() { return Math.Max(a, b); } }",
        ["Math.Max(a, b)"]
    ),
    case!(
        "csharp",
        "$A.Add($B)",
        "class A { void F() { xs.Add(1); xs.Remove(1); } }",
        ["xs.Add(1)"]
    ),
    case!(
        "csharp",
        "$A != $A",
        "class A { void F() { if (x != x) {} } }",
        ["x != x"]
    ),
    // Siblings after the last node of a pattern are not compared.
    case!(
        "ruby",
        "puts $A",
        "puts 1\nputs 1, 2\n",
        ["puts 1", "puts 1, 2"]
    ),
    case!("ruby", "$A.nil?", "x.nil?\ny.empty?\n", ["x.nil?"]),
    case!("ruby", "$A == $A", "a == a\na == b\n", ["a == a"]),
    case!(
        "ruby",
        "require $M",
        "require \"json\"\nload \"x\"\n",
        ["require \"json\""]
    ),
    case!(
        "ruby",
        "raise $E",
        "raise ArgumentError\n",
        ["raise ArgumentError"]
    ),
    case!("ruby", "$A << $B", "xs << 1\n", ["xs << 1"]),
    case!(
        "ruby",
        "attr_reader $$$A",
        "class A\n  attr_reader :x, :y\nend\n",
        ["attr_reader :x, :y"]
    ),
    case!("ruby", "$A.to_s", "a.to_s + b.to_i\n", ["a.to_s"]),
    case!(
        "ruby",
        "def $M\n  $$$B\nend",
        "def run\n  go\nend\ndef stop(x)\nend\n",
        ["def run\n  go\nend", "def stop(x)\nend"]
    ),
    case!("ruby", "$A ? $B : $C", "v = a ? 1 : 2\n", ["a ? 1 : 2"]),
    case!(
        "ruby",
        "return $A",
        "def f\n  return 1\nend\n",
        ["return 1"]
    ),
    case!("ruby", "$A.length", "n = xs.length\n", ["xs.length"]),
    case!(
        "ruby",
        "File.read($P)",
        "File.read(p)\nFile.write(p, x)\n",
        ["File.read(p)"]
    ),
    case!("ruby", "!$A", "if !ok\n  x\nend\n", ["!ok"]),
    case!("ruby", "$H[$K]", "v = h[:a]\n", ["h[:a]"]),
    case!(
        "php",
        "echo $A;",
        "<?php echo 1; echo 1, 2;",
        ["echo 1;", "echo 1, 2;"]
    ),
    case!("php", "isset($A)", "<?php if (isset($x)) {}", ["isset($x)"]),
    case!(
        "php",
        "$A == $A",
        "<?php if ($a == $a) {} if ($a == $b) {}",
        ["$a == $a"]
    ),
    case!(
        "php",
        "strlen($S)",
        "<?php $n = strlen($s);",
        ["strlen($s)"]
    ),
    case!(
        "php",
        "$A->$M()",
        "<?php $o->run(); $o->go(1);",
        ["$o->run()"]
    ),
    case!(
        "php",
        "new $C($$$A)",
        "<?php $d = new DateTime();",
        ["new DateTime()"]
    ),
    case!(
        "php",
        "return $A;",
        "<?php function f() { return 1; }",
        ["return 1;"]
    ),
    case!(
        "php",
        "array_map($F, $A)",
        "<?php array_map($f, $xs); array_filter($xs);",
        ["array_map($f, $xs)"]
    ),
    case!(
        "php",
        "count($A) === 0",
        "<?php if (count($xs) === 0) {}",
        ["count($xs) === 0"]
    ),
    case!("php", "$A + $B", "<?php $c = $a + $b;", ["$a + $b"]),
    case!(
        "php",
        "throw new $E($M);",
        "<?php throw new Exception(\"x\");",
        ["throw new Exception(\"x\");"]
    ),
    case!(
        "php",
        "in_array($N, $H)",
        "<?php in_array($x, $xs);",
        ["in_array($x, $xs)"]
    ),
    case!("php", "$A ?? $B", "<?php $v = $a ?? $b;", ["$a ?? $b"]),
    case!(
        "php",
        "print_r($A)",
        "<?php print_r($x); var_dump($x);",
        ["print_r($x)"]
    ),
    case!(
        "php",
        "$A::$M()",
        "<?php Foo::bar(); $o->bar();",
        ["Foo::bar()"]
    ),
    case!(
        "kotlin",
        "println($A)",
        "fun main() { println(1); print(2) }",
        ["println(1)"]
    ),
    case!(
        "kotlin",
        "$A == $A",
        "fun f() { if (a == a) {} }",
        ["a == a"]
    ),
    case!("kotlin", "$A!!", "fun f() { val x = y!! }", ["y!!"]),
    case!(
        "kotlin",
        "listOf($$$A)",
        "val xs = listOf(1, 2)\nval ys = setOf(1)\n",
        ["listOf(1, 2)"]
    ),
    case!("kotlin", "$A ?: $B", "val v = a ?: b\n", ["a ?: b"]),
    case!(
        "kotlin",
        "return $A",
        "fun f(): Int { return 1 }",
        ["return 1"]
    ),
    case!("kotlin", "$A.size", "val n = xs.size\n", ["xs.size"]),
    case!(
        "kotlin",
        "throw $E",
        "fun f() { throw IllegalStateException(\"x\") }",
        ["throw IllegalStateException(\"x\")"]
    ),
    case!(
        "kotlin",
        "$A.toString()",
        "val s = x.toString()\n",
        ["x.toString()"]
    ),
    case!(
        "kotlin",
        "require($C)",
        "fun f(x: Int) { require(x > 0) }",
        ["require(x > 0)"]
    ),
    case!("kotlin", "$A in $B", "val ok = a in xs\n", ["a in xs"]),
    case!(
        "kotlin",
        "$A as $T",
        "val s = x as String\n",
        ["x as String"]
    ),
    case!(
        "kotlin",
        "$A > 0",
        "val p = n > 0\nval q = n < 0\n",
        ["n > 0"]
    ),
    case!(
        "kotlin",
        "check($C)",
        "fun f() { check(ok) }",
        ["check(ok)"]
    ),
    case!(
        "kotlin",
        "$A.isEmpty()",
        "val e = s.isEmpty()\n",
        ["s.isEmpty()"]
    ),
    case!(
        "bash",
        "echo $A",
        "echo hello\necho a b\n",
        ["echo hello", "echo a b"]
    ),
    case!(
        "bash",
        "rm -rf $P",
        "rm -rf /tmp/x\nrm -f y\n",
        ["rm -rf /tmp/x"]
    ),
    case!("bash", "cd $D", "cd /srv\nls\n", ["cd /srv"]),
    case!("bash", "export $V", "export PATH\n", ["export PATH"]),
    case!(
        "bash",
        "curl $$$A",
        "curl -s http://x\nwget y\n",
        ["curl -s http://x"]
    ),
    case!(
        "bash",
        "mkdir -p $D",
        "mkdir -p a/b\nmkdir c\n",
        ["mkdir -p a/b"]
    ),
    case!("bash", "test -f $F", "test -f a.txt\n", ["test -f a.txt"]),
    case!(
        "bash",
        "cat $F | grep $P",
        "cat a | grep b\ncat a\n",
        ["cat a | grep b"]
    ),
    case!(
        "bash",
        "sudo $$$A",
        "sudo apt update\n",
        ["sudo apt update"]
    ),
    case!(
        "bash",
        "git commit $$$A",
        "git commit -m x\ngit push\n",
        ["git commit -m x"]
    ),
    case!(
        "bash",
        "chmod +x $F",
        "chmod +x run.sh\n",
        ["chmod +x run.sh"]
    ),
    case!("bash", "source $F", "source env.sh\n", ["source env.sh"]),
    case!("bash", "exit $C", "exit 1\n", ["exit 1"]),
    case!(
        "bash",
        "cp $A $B",
        "cp a b\ncp -r a b\n",
        ["cp a b", "cp -r a b"]
    ),
    case!("bash", "sleep $N", "sleep 5\n", ["sleep 5"]),
    case!(
        "swift",
        "print($A)",
        "print(1)\nprint(1, 2)\n",
        ["print(1)"]
    ),
    case!("swift", "$A == $A", "let b = a == a\n", ["a == a"]),
    case!("swift", "$A!", "let v = x!\n", ["x!"]),
    case!("swift", "$A ?? $B", "let v = a ?? b\n", ["a ?? b"]),
    case!(
        "swift",
        "return $A",
        "func f() -> Int { return 1 }\n",
        ["return 1"]
    ),
    case!("swift", "$A.count", "let n = xs.count\n", ["xs.count"]),
    case!(
        "swift",
        "fatalError($M)",
        "fatalError(\"x\")\n",
        ["fatalError(\"x\")"]
    ),
    case!("swift", "$A.append($B)", "xs.append(1)\n", ["xs.append(1)"]),
    case!("swift", "$A > $B", "let p = a > b\n", ["a > b"]),
    case!("swift", "$A.isEmpty", "let e = s.isEmpty\n", ["s.isEmpty"]),
    case!(
        "swift",
        "throw $E",
        "func f() throws { throw E.bad }\n",
        ["throw E.bad"]
    ),
    case!("swift", "assert($C)", "assert(ok)\n", ["assert(ok)"]),
    case!("swift", "$A.map($F)", "let ys = xs.map(f)\n", ["xs.map(f)"]),
    case!("swift", "min($A, $B)", "let m = min(a, b)\n", ["min(a, b)"]),
    case!(
        "swift",
        "$A + 1",
        "let c = n + 1\nlet d = n + 2\n",
        ["n + 1"]
    ),
];

fn language(id: &str) -> LanguageId {
    LanguageId::new(id).unwrap()
}

/// Texts of the matches of a case, or the error message.
fn matches(service: &CodeAnalysis, case: &Case) -> Result<Vec<String>, String> {
    let id = language(case.language);
    let pattern = service
        .compile_pattern(case.pattern, &id)
        .map_err(|error| format!("compile: {}", error.message))?;
    let found = service
        .syntax_search(case.source.as_bytes(), pattern.as_ref())
        .map_err(|error| format!("search: {}", error.message))?;
    Ok(found.into_iter().map(|found| found.text).collect())
}

#[test]
fn corpus_has_enough_cases_and_languages() {
    let languages: BTreeSet<&str> = CORPUS.iter().map(|case| case.language).collect();
    assert!(CORPUS.len() >= MIN_CASES, "{} cases", CORPUS.len());
    assert!(languages.len() >= MIN_LANGUAGES, "{languages:?}");
}

#[test]
fn every_corpus_pattern_finds_the_expected_matches() {
    let service = CodeAnalysis::builtin();
    let failures: Vec<String> = CORPUS
        .iter()
        .filter_map(|case| {
            let outcome = matches(&service, case);
            let expected: Vec<String> = case.expected.iter().map(|s| (*s).to_owned()).collect();
            (outcome.as_ref() != Ok(&expected)).then(|| {
                format!(
                    "{} `{}`: expected {:?}, got {:?}",
                    case.language, case.pattern, expected, outcome
                )
            })
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn captures_hold_the_text_of_single_and_multiple_metavariables() {
    let service = CodeAnalysis::builtin();
    let python = language("python");
    let pattern = service
        .compile_pattern("$F($FIRST, $$$REST)", &python)
        .unwrap();
    assert_eq!(pattern.metavariables(), ["F", "FIRST", "REST"]);
    let found = service
        .syntax_search(b"call(1, 2, 3)\n", pattern.as_ref())
        .unwrap();
    let captures: Vec<(&str, &str, usize)> = found[0]
        .captures
        .iter()
        .map(|c| (c.name.as_str(), c.text.as_str(), c.spans.len()))
        .collect();
    assert_eq!(
        captures,
        [("F", "call", 1), ("FIRST", "1", 1), ("REST", "2, 3", 2)]
    );
}

#[test]
fn reused_metavariables_require_equal_nodes() {
    let service = CodeAnalysis::builtin();
    let go = language("go");
    let pattern = service.compile_pattern("$A = $A", &go).unwrap();
    let found = service
        .syntax_search(
            b"package main\nfunc f() { x = x; x = y; a.b = a.b }\n",
            pattern.as_ref(),
        )
        .unwrap();
    let texts: Vec<&str> = found.iter().map(|f| f.text.as_str()).collect();
    assert_eq!(texts, ["x = x", "a.b = a.b"]);
}

#[test]
fn match_positions_count_lines_and_characters() {
    let service = CodeAnalysis::builtin();
    let js = language("javascript");
    let pattern = service.compile_pattern("f($A)", &js).unwrap();
    let found = service
        .syntax_search("const é = 1;\nlet a = f(é);\n".as_bytes(), pattern.as_ref())
        .unwrap();
    let span = found[0].span;
    assert_eq!((span.start.line, span.start.column), (2, 9));
    assert_eq!((span.end.line, span.end.column), (2, 13));
    assert_eq!(found[0].node_kind, "call_expression");
}

/// Name of the `ast-grep` program, when it is installed.
fn ast_grep() -> Option<&'static str> {
    ["ast-grep", "sg"].into_iter().find(|program| {
        Command::new(program)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    })
}

/// Number of matches reported by `ast-grep run` for a case.
fn ast_grep_count(program: &str, dir: &Path, index: usize, case: &Case) -> usize {
    let file = dir.join(format!("case{index}.{}", extension(case.language)));
    std::fs::write(&file, case.source).unwrap();
    let output = Command::new(program)
        .args([
            "run",
            "--pattern",
            case.pattern,
            "--lang",
            case.language,
            "--json=compact",
        ])
        .arg(&file)
        .output()
        .unwrap();
    let found: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
    found.as_array().map_or(0, Vec::len)
}

fn extension(language: &str) -> &'static str {
    match language {
        "rust" => "rs",
        "python" => "py",
        "javascript" => "js",
        "typescript" => "ts",
        "go" => "go",
        "java" => "java",
        "c" => "c",
        "cpp" => "cpp",
        "csharp" => "cs",
        "ruby" => "rb",
        "php" => "php",
        "kotlin" => "kt",
        "bash" => "sh",
        "swift" => "swift",
        _ => "txt",
    }
}

#[test]
fn corpus_counts_agree_with_ast_grep_cli_when_installed() {
    let Some(program) = ast_grep() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let service = CodeAnalysis::builtin();
    let counts: Vec<(usize, usize)> = CORPUS
        .iter()
        .enumerate()
        .map(|(index, case)| {
            let ours = matches(&service, case).map_or(0, |found| found.len());
            (ours, ast_grep_count(program, dir.path(), index, case))
        })
        .collect();
    // The program reports no match for a pattern that its grammar of the
    // language rejects or lacks; only the cases it answers are compared.
    let disagreements: Vec<String> = CORPUS
        .iter()
        .zip(&counts)
        .filter_map(|(case, &(ours, theirs))| {
            (theirs > 0 && ours != theirs).then(|| {
                format!(
                    "{} `{}`: {ours} here, {theirs} in ast-grep",
                    case.language, case.pattern
                )
            })
        })
        .collect();
    assert!(disagreements.is_empty(), "{}", disagreements.join("\n"));
}
