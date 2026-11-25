# halmi-c contract

## Purpose and layer

C language adapter for halmi: hand lexer + definition scanner on the
shared `halmi-lex` machinery (`MetricsCollector`, `cfamily`
punctuators). Zero dependencies; no tree-sitter in the product (the
tree-sitter-c oracle is a test-side differential only).

## Counted definitions

* `type name(params) { ... }` at file scope: `function_definition`
  parity with tree-sitter-c. The storage class, return type, and a
  preceding `__attribute__((...))` / `__declspec(...)` group are folded
  into the span, including one newline crossing for a return type on
  its own line.
* Old-style K&R parameter declarations (`int f(a, b) int a; char b;
  {`) are part of the signature: between `)` and `{` a gap is a
  signature only if it is empty, or every statement after the last `;`
  is newline-only.
* Not counted: prototypes (`;` terminator), `typedef`s of function
  pointers, preprocessor macro bodies (`preproc_function_def`),
  calls/macros at file scope, `__attribute__`-style pseudo-declarators
  (skip list), and macro invocation wrappers like `ATOMIC_GEN(uint8_t,
  1)` whose gap contains non-attribute parenthesized groups.
* Macro-decorated declarators (`bool ATOMIC(is_lock_free) (params)
  {`): consecutive `(...)` groups after the name are re-scanned as the
  parameter list.

## Preprocessor model

Directive lines (`#...`, with backslash continuations) emit no code
tokens; continuation newlines still emit Newline tokens so span
boundaries survive. `#if/#ifdef/#ifndef` push a marker, `#else/#elif`
reset the body brace depth to the `#if` entry depth, `#endif` pops:
brace matching follows single-branch semantics (a compiler with all
macros undefined) while declarations in both branches remain visible.
General backslash-newline splices outside strings are handled.

## Metric rules

* Halstead: identifiers/numbers/strings/chars are operands; control
  keywords (`if`, `else`, `for`, `while`, `do`, `switch`, `case`,
  `default`, `break`, `continue`, `return`, `goto`, `sizeof`) are
  operators. `decl_op` is the declarator `(` (C has no `fn` keyword).
* Cyclomatic complexity: 1 + one per `if`, `for`, `while`, `case`,
  `&&`, `||` (`do` is not counted; its `while` is).
* Physical LOC; nesting = brace depth inside the body (body = 0).
* `io_params`: parameter names by last-identifier-before-separator
  heuristic (type-then-name order); a lone `void` parameter counts as
  zero; +1 when the return type is not exactly `void`.

## Error model

Unterminated strings/chars/comments, stray characters, and
unterminated parameter lists surface as structured per-file errors.

## Conformance

Differential vs tree-sitter-c over 1,205 local C/H files: pure C
9,765/9,835 exact spans (99.3%); the remainder are files with C++
content in `.h` (owned by the future C++ adapter), quickjs.c
label-macro interpreter code where the oracle itself is wrong, and a
handful of ±1 residuals. Historical, maintainer-local run with a private
oracle binary and an unvendored corpus: UNREPRODUCIBLE from this repo.
8 total tests (1 in-crate and 7 external integration tests), mutation-verified
(prototype-vs-definition guard, K&R guard).

## No-claim boundaries

* No macro evaluation; conditional blocks use the single-branch brace
  model above.
* Assembler/macro-heavy code (quickjs `SWITCH/CASE`) is span-correct
  by brace matching, not by the oracle's error-recovery shape.
* C++ files routed here as `.h` degrade gracefully (errors or partial
  spans) and are out of scope.
