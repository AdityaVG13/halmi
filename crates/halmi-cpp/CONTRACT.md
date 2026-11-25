# halmi-cpp contract

## Purpose and layer

C++ language adapter for halmi: hand lexer + definition scanner on
the shared `halmi-lex` machinery (halmi-c shape extended for C++
declarations). Zero dependencies; no tree-sitter in the product (the
tree-sitter-cpp oracle is a test-side differential only).

## Counted definitions

* `function_definition` parity with tree-sitter-cpp: free functions,
  in-class methods (qualified `Container::name` via a
  class/struct/union/enum/namespace container stack), out-of-line
  definitions keeping their written scope (`Foo::Bar::method`),
  constructors, destructors (qualified `~Name`, template-argument
  forms), operator overloads (`operator+=`, `operator()`,
  `operator[]`, `operator bool`, `operator new`), template
  specializations (`template <> void f<4>(...)`), and
  `= default;` / `= delete;` bodies.
* `template<...>` headers are NOT part of the span (tree-sitter wraps
  them in a separate `template_declaration` node); multi-line return
  types (including `std::tuple<..., std::vector<int>>` with `>>`
  closers) are, via a guarded single-newline crossing.
* Signature gap (`)` to `{`): constructor initializer lists
  (`: a_(x), b_(y)`), trailing return types (`-> T`), `const`,
  `noexcept(...)`, `override`, `final`, `requires (...)`, and
  reference qualifiers are legal; after a `;` only newlines may
  follow (prototype rejection).
* Not counted: prototypes, lambdas, pure-virtual `= 0`, calls and
  macro invocations (guarded by the skip list and the gap grammar),
  and control keywords (the candidate arm admits `Kw` only for
  `operator`).

## Deliberate divergences from the oracle

* Test-framework macro blocks (doctest `TEST_CASE("...") { ... }`,
  gtest `TEST(...)`) are counted as functions: they are the file's
  real test functions (Zig `test` precedent); tree-sitter cannot see
  macros and reports zero.
* Files using macro-generated namespaces
  (`NLOHMANN_JSON_NAMESPACE_BEGIN`) and macro-heavy vendor APIs
  (basisu wasm, gtest amalgamated) carry residual span differences on
  both sides; documented no-claim.

## Lexing

`::` punctuator (added to `cfamily`); UTF-8 BOM skipped; raw strings
`R"delim(...)delim"` including multiline; prefixed strings (`u8`,
`u`, `U`, `L`); digit separators (`1'000'000`); binary literals;
`<<`/`>>` (including as template closers); the C preprocessor model
(single-branch braces, `#if/#else/#endif` markers, backslash splices
anywhere).

## Metric rules

* Halstead: control + allocation keywords (`if`, `else`, `for`,
  `while`, `do`, `switch`, `case`, `default`, `break`, `continue`,
  `return`, `goto`, `sizeof`, `new`, `delete`, `try`, `catch`,
  `throw`, `operator`, `co_*`) are operators; identifiers, numbers,
  strings, chars are operands. `decl_op` is the declarator `(`.
* Cyclomatic complexity: 1 + one per `if`, `for`, `while`, `case`,
  `catch`, `&&`, `||`.
* Physical LOC; nesting = brace depth inside the body (body = 0).
* `io_params`: parameter names (last-identifier-before-separator) +
  1 when the return type is not exactly `void` (constructors and
  destructors add nothing).

## Conformance

Differential vs tree-sitter-cpp over 882 local files (cpp/cc/cxx/
hpp/hxx/hh/ipp): 16,287/17,175 exact spans on non-test files (94.9%)
with the residuals concentrated in the documented macro-namespace
vendor headers. Historical, maintainer-local run with a private oracle binary
and an unvendored corpus: UNREPRODUCIBLE from this repo.
11 total tests (1 in-crate and 10 external integration tests), mutation-verified (template
specialization, K&R/prototype gap rules via halmi-c parity).

## No-claim boundaries

* No template instantiation or overload resolution; a specialization
  is matched syntactically (`name<args>(...) {`).
* Concepts/requires clauses are skipped as signature tokens only.
* Coroutines are tokenized; no suspend-point analysis.
