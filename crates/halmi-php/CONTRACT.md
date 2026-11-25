# halmi-php contract

## Purpose and layer

PHP language adapter for halmi: hand lexer + declaration scanner on
the shared `halmi-lex` machinery. The `function` keyword triggers
candidates; `function (` closures and `fn` arrows do not count. Zero
dependencies; no tree-sitter (the tree-sitter-php oracle is a
test-side differential only).

## Counted definitions

`function_definition` / `method_declaration` parity: top-level
functions, class/trait/interface/enum methods (qualified
`Widget::area`), constructors (`__construct`), and bodyless
`abstract`/`interface` methods (`;` terminator). Modifier and
`#[Attribute]` runs fold into the span (attributes lex as tokens, not
comments). Not counted: closures, arrow functions.

## Lexing

`//`, `#` (but not `#[`), `/* */`; dq strings (may span lines) with
`{$...}` interpolation; sq strings (may span lines); heredoc/nowdoc
(`<<<EOT`, `<<<'EOT'`,
indented closers); `$var`, `$1`-style globals; `?->`; namespace-qualified
`\Foo\Bar` identifiers; UTF-8 BOM skip.

## Metric rules

* Halstead: control keywords (`if`, `else`, `elseif`, `for`,
  `foreach`, `while`, `do`, `switch`, `case`, `default`, `break`,
  `continue`, `return`, `throw`, `try`, `catch`, `finally`, `new`,
  `clone`, `echo`, `print`, `instanceof`, `unset`, `isset`, `empty`,
  `list`, `include`, `require`, `yield`, `match`) are operators.
  `decl_op` is `function`.
* Cyclomatic complexity: 1 + one per `if`, `elseif`, `for`,
  `foreach`, `while`, `case`, `catch`, `&&`, `||`.
* Physical LOC; nesting = brace depth inside the body (body = 0).
* `io_params`: parameter names + 1 when the return type is not
  `void`/`never` (constructors add nothing).

## Conformance

Differential vs tree-sitter-php on the fetched corpus (php-fig/log,
nikic/iter: 14 files): 181/181 exact spans (100%). Historical,
maintainer-local run with a private oracle binary and an unvendored corpus:
UNREPRODUCIBLE from this repo. 6 total tests (1 in-crate and 5 external integration tests),
mutation-verified (attribute fold; the walk-back gating mutant kills
`methods_ctors_and_attributes`).

## No-claim boundaries

* The corpus is small; framework-generated code (Laravel macros) is
  out of scope.
* `match` expression arms are token-level only.
