# halmi-java contract

## Purpose and layer

Java language adapter for halmi: hand lexer + declaration scanner on
the shared `halmi-lex` machinery (halmi-c/halmi-cpp shape). Zero
dependencies; no tree-sitter in the product (the tree-sitter-java
oracle is a test-side differential only).

## Counted definitions

`method_declaration` / `constructor_declaration` /
`static_initializer` parity with tree-sitter-java:

* Methods and constructors at class-body level, with modifier and
  annotation runs (`@Override`, `@SuppressWarnings("x")`, `@Mod(...)`)
  folded into the span. Qualification is the full container chain
  (`Widget::Color::GREEN::specialty`); constructors are
  `C::C`; static initializers are `C::<clinit>`.
* Bodyless methods (`abstract`, `native`, interface methods) count
  when a type/modifier precedes the name; field initializers
  (`= call();`) do not.
* Methods of anonymous classes count, including qualified
  (`new java.lang.Runnable() {}`) and diamond (`new Foo<>() {}`)
  forms; the walk descends into function bodies with depth
  compensation.
* Enum semantics: the constants section (before the member-level
  `;`) treats `CONST`, `CONST {`, and `CONST(args) {` as containers,
  not methods; methods inside constant bodies count. `class X
  extends Y<X> implements Z<W> {` headers (multi-line, generic)
  resolve to `X`.
* Lambdas, instance initializer blocks, and calls inside bodies do
  not count (depth-gated: candidates only fire at the innermost
  container's member level).

## Lexing

`//` and `/* */` comments; text blocks `"""..."""` (backslash-newline
inside still counts lines); strings/chars with escapes; UTF-8 BOM
skip; numbers with `0x`/`0b`, `_` separators, and suffixes; `$` in
identifiers.

## Metric rules

* Halstead: control/allocation keywords (`if`, `else`, `for`,
  `while`, `do`, `switch`, `case`, `default`, `break`, `continue`,
  `return`, `throw`, `try`, `catch`, `finally`, `new`, `instanceof`,
  `synchronized`, `assert`) are operators. `decl_op` is the
  declarator `(`.
* Cyclomatic complexity: 1 + one per `if`, `for`, `while`, `case`,
  `catch`, `&&`, `||`.
* Physical LOC; nesting = brace depth inside the body (body = 0).
* `io_params`: parameter names + 1 when the return type is not
  exactly `void` (constructors add nothing).

## Conformance

Differential vs tree-sitter-java over the GT5-Unofficial corpus
(3,331 files, GregTech New Horizons): 37,630/37,789 exact spans
(99.6%); residual is ~116 files with 1-2 span diffs each
(annotation/array-type edge formats). Historical, maintainer-local run with
a private oracle binary and an unvendored corpus: UNREPRODUCIBLE from this
repo. 10 total tests (1 in-crate and 9 external integration tests),
mutation-verified (enum-constants gate).

## No-claim boundaries

* No type inference or overload resolution; `C::C` constructors are
  name-based.
* Inner-class shadowing uses the written container chain.
* Lombok-style annotation processing (`@Getter` generating methods)
  is invisible by design.
