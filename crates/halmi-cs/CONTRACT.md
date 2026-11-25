# halmi-cs contract

## Purpose and layer

C# language adapter for halmi: hand lexer + declaration scanner on
the shared `halmi-lex` machinery (halmi-java shape with C#
declarations). Zero dependencies; no tree-sitter in the product (the
tree-sitter-c_sharp oracle is a test-side differential only).

## Counted definitions

`method_declaration` / `constructor_declaration` parity with
tree-sitter-c_sharp:

* Methods and constructors with attribute runs (`[STAThread]`,
  `[Obsolete("x")]`) folded into the span; expression-bodied members
  (`=> expr;`, including object/collection initializers inside);
  `async`/generic methods; interface bodyless methods; local
  functions (one depth below member level, gated by a type token
  before the name).
* Not counted: properties (`{ get; set; }`, `=> expr` computed),
  accessors, indexers, destructors, enum members, lambdas, field
  initializers (`= call();`), and `new StringBuilder()` constructor
  calls (`new`/`return`/`throw` never precede a local function).
* Qualification: container chain without namespace prefixes
  (`Widget::Area`, `Widget::Local`); file-scoped `namespace X;` and
  block namespaces do not qualify names. Containers:
  class/struct/interface/record/enum with generic and `where T : X`
  constraints (the name follows the keyword, not the constraint
  type).

## Lexing

`//` and `/* */` comments; raw strings `"""..."""`; verbatim strings
`@"..."` (with `""` escapes, multiline); interpolated `$"..."` /
`$@"..."` (with `{{` escapes); chars; UTF-8 BOM skip; numbers with
`0x`/`0b`, `_` separators, and suffixes; `@identifiers`.

## Metric rules

* Halstead: control/LINQ/allocation keywords (`if`, `else`, `for`,
  `foreach`, `while`, `do`, `switch`, `case`, `default`, `break`,
  `continue`, `return`, `throw`, `try`, `catch`, `finally`, `new`,
  `typeof`, `nameof`, `sizeof`, `lock`, `using`, `yield`, `await`,
  LINQ verbs, `operator`, `checked`, `unchecked`) are operators.
  `decl_op` is the declarator `(`.
* Cyclomatic complexity: 1 + one per `if`, `for`, `foreach`,
  `while`, `case`, `catch`, `&&`, `||`.
* Physical LOC; nesting = brace depth inside the body (body = 0).
* `io_params`: parameter names + 1 when the return type is not
  exactly `void` (constructors add nothing).

## Conformance

Differential vs tree-sitter-c_sharp on the local corpus
(TensorAST engine-csharp-v1 fixtures + Program.cs, 16 files):
28/28 exact spans (100%). Historical, maintainer-local run with a private
oracle binary and an unvendored corpus: UNREPRODUCIBLE from this repo.
8 total tests (1 in-crate and 7 external
integration tests), mutation-verified (local-function gate). The
attribute-context recognizer (`is_attribute_context`) is pinned by the
custom/stacked attribute fixture but not mutation-discriminated: replacing
it with `true` leaves every test and the corpus unchanged, because array
suffixes and attribute runs reach the same span through either branch.

## No-claim boundaries

* No async state-machine modeling; `await` is an operator token.
* Source generators and analyzers (what the compiler emits) are
  invisible by design.
* The local corpus is small; attribute-heavy real-world code is
  validated by the fixture set only.
* A multi-line tuple return type is not folded into the span: the method
  starts on its name's line, where tree-sitter-c_sharp starts on the
  modifier line. Unchanged by the attribute-context cleanup.
