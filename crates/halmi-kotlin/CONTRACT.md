# halmi-kotlin contract

## Purpose and layer

Kotlin language adapter for halmi: hand lexer + declaration scanner
on the shared `halmi-lex` machinery. The `fun` keyword triggers
candidates, so no call-misfire ambiguity exists. Zero dependencies;
no tree-sitter in the product (the tree-sitter-kotlin oracle is a
test-side differential only).

## Counted definitions

`function_declaration` / `secondary_constructor` parity with
tree-sitter-kotlin:

* Every `fun`: top-level, class/object/interface/enum members
  (qualified `Widget::area`), companion objects
  (`Widget::Companion::make`), receiver types (`fun String.shout()`,
  `fun <T> T?.takeUnless(...)` with `.`/`?.`/`::` separators),
  generics, `suspend`/`inline`/`operator`/`external` modifiers,
  annotation runs (`@JvmStatic`, `@Deprecated("x")` on their own
  lines) folded into the span, block bodies `{ ... }`, and expression
  bodies `= expr` that continue across lines until the next line
  starts a declaration (fun/val/var/class/modifiers/`}`/`@`) or a
  literal is followed by an identifier.
* Secondary constructors `constructor() : this(0)` at member level
  (primary constructors in class headers do not fire).
* Not counted: primary constructors, `init` blocks, lambdas, and
  properties.

## Lexing

`//`, `/* */`, nested block comments; raw strings `"""..."""`;
template strings with `$name` and `${expr}` interpolations (nested
strings inside interpolations); chars; backtick identifiers; UTF-8
BOM skip; numbers with `0x`/`0b`, `_`, suffixes.

## Metric rules

* Halstead: control keywords (`if`, `else`, `for`, `while`, `do`,
  `when`, `break`, `continue`, `return`, `throw`, `try`, `catch`,
  `finally`, `is`, `in`, `as`, `fun`) are operators. `decl_op` is
  `fun`.
* Cyclomatic complexity: 1 + one per `if`, `for`, `while`, `when`,
  `catch`, `&&`, `||`.
* Physical LOC; nesting = brace depth inside the body (body = 0).
* `io_params`: parameter names + 1 (every function returns; `Unit`
  counts as a nominal result).

## Conformance

Differential vs tree-sitter-kotlin on the llama.android corpus (11
files): 75/75 exact spans (100%). Historical, maintainer-local run with a
private oracle binary and an unvendored corpus: UNREPRODUCIBLE from this
repo. 8 total tests (1 in-crate and 7 external integration tests), mutation-verified
(receiver/params arm).

## No-claim boundaries

* Multi-line expression bodies use the line-start heuristic above;
  pathological operator-leading continuations are not modeled.
* Coroutine suspension points, delegation (`by`), and DSL builders
  are token-level only.
