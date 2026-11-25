# halmi-lua contract

## Purpose and layer

Lua language adapter for halmi: hand lexer + `end`-block scanner on
the shared `halmi-lex` machinery. Zero dependencies; no tree-sitter
(the tree-sitter-lua oracle is a test-side differential only).

## Counted definitions

`function_declaration` parity: `function a.b.c()`, `function M:m()`
(colon methods keep their written name), `local function name()`
(`local` folds into the span), and nested named functions inside
bodies. Anonymous `function ( ... )` expressions (callbacks) do not
count.

## Block model

`function`/`if`/`do`/`repeat` open, `end`/`until` close; `for`/`while`
never need counting because their `do` covers them; `then`/`else`/
`elseif` continue the current block.

## Lexing

`--` line and `--[[...]]` long comments (any `=` level); dq/sq
strings; long strings `[[...]]`/`[=[...]=]`; UTF-8 BOM skip; hex and
float literals.

## Metric rules

* Halstead: control keywords (`if`, `elseif`, `else`, `while`, `for`,
  `repeat`, `until`, `do`, `then`, `end`, `break`, `goto`, `return`,
  `local`, `function`, `and`, `or`, `not`, `in`) are operators.
  `decl_op` is `function`.
* Cyclomatic complexity: 1 + one per `if`, `elseif`, `while`, `for`,
  `and`, `or`.
* Physical LOC; nesting = block depth inside the body.
* `io_params`: parameter names (lone `self`/`...` cleared) + 1.

## Conformance

Differential vs tree-sitter-lua on the fetched corpus (lite editor,
Penlight: 142 files): 1,122/1,122 exact spans (100%). Historical,
maintainer-local run with a private oracle binary and an unvendored corpus:
UNREPRODUCIBLE from this repo. 5 total tests (1 in-crate and 4 external integration tests);
the `local`-fold mutant is line-invariant (same-line token) and
therefore unobservable.

## No-claim boundaries

* goto/labels are token-level only.
* Coroutine semantics are invisible.
