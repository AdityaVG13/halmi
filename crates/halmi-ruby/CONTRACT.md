# halmi-ruby contract

## Purpose and layer

Ruby language adapter for halmi: hand lexer + `end`-block scanner on
the shared `halmi-lex` machinery. `def` triggers candidates. Zero
dependencies; no tree-sitter (the tree-sitter-ruby oracle is a
test-side differential only).

## Counted definitions

`method` / `singleton_method` parity: instance methods, `def self.x`,
`def Const.x`, `def obj.x` (the last method name of the receiver chain
is the name; the receiver is dropped, so tree-sitter parity holds),
setters, operators, `class << self` members; bodies are
`end`-terminated (def/do/if/unless/case/while/until/for/begin/class/
module and `{` open; `end`/`}` close; modifier conditionals and
`self.class`-style dotted keywords do not open). A setter is `name=`
with the `=` adjacent to the name (`def name=(v)`, `def name= v`); the
enclosing block is `end`-terminated. Endless definitions
(`def name(params) = expr`, Ruby 3; whitespace before `=`) span the
logical line: the body runs to the first line break at bracket depth
zero that is not a continuation. Endless definitions inside a body do
not open a block, so they never consume the enclosing `end`.
Qualification is the
module/class container chain (`Demo::Widget::area`). Lambdas, blocks,
and `alias` do not count.

## Lexing

`#` comments; dq strings with `#{}` interpolation (nested strings
inside); sq strings; heredocs `<<~`/`<<-`/`<<ID` with quoted ids;
`%-literals (`%w`, `%i`, `%q`, `%Q`, `%r`, `%s`, `%x`, `%W`, uppercase forms,
and bare `%(...)`, `%[...]`, `%{...}`, `%<...>`, `%!...!`, `%/.../`, `%|...|`); regexes
(expression-position heuristic: line-start `/` always opens; division
after identifiers/literals/closers); symbols `:sym`, `:"interp"`;
char literals `?x`, `?"`, `?\`; `$globals` (including `$'`, `$;`,
`$:`, `$1`); `@ivar`/`@@cvar`; backticks and `%x{...}` (both interpolate);
`__END__` data sections stop
lexing; backslash-newline splices; UTF-8 fallback.

## Metric rules

* Halstead: control keywords (`if`, `elsif`, `else`, `unless`,
  `case`, `when`, `while`, `until`, `for`, `in`, `do`, `begin`,
  `rescue`, `ensure`, `return`, `break`, `next`, `redo`, `retry`,
  `yield`, `raise`, `throw`, `catch`, `and`, `or`, `not`, `super`,
  `self`, `new`, `defined?`, `lambda`, `proc`) are operators.
  `decl_op` is `def`.
* Cyclomatic complexity: 1 + one per `if`, `elsif`, `unless`, `when`,
  `while`, `until`, `for`, `rescue`, `&&`, `||` (modifier forms
  count).
* Physical LOC; nesting = block depth inside the body.
* `io_params`: parameter names + 1 (every Ruby method returns).

## Conformance

Differential vs tree-sitter-ruby through `tools/oracle_diff.py` on a
local 32-file Ruby corpus (`HALMI_CORPUS_RB`, a private tree-sitter oracle
binary not in this repository; historical, UNREPRODUCIBLE from this repo):
190/190 exact spans, 0 files with span differences; 2 malformed
fixtures (`half_assignment.rb`, `half_call.rb`) where the oracle itself
reports >30% ERROR nodes are excluded from the span count (halmi
reports 1 and 2 functions there). Historical, UNREPRODUCIBLE (the
fetched corpus is not in this tree): sinatra/rack/sinatra-contrib,
242 files, 1,739/1,743 exact spans (99.8%), residual 1-2-span diffs in
3 files. 17 total tests (1 in-crate and 16 external integration tests),
failure-first (every endless/setter/receiver regression was red before
its fix) and mutation-verified (the is_opener gating mutant kills the
modifier-conditionals test).

## No-claim boundaries

* Regex/division disambiguation is heuristic (expression-position);
  pathological cases degrade to division.
* Multi-line `/x` regexes close at the first flag-suffixed slash.
* Metaprogramming (`define_method`, `method_missing`) is invisible.
