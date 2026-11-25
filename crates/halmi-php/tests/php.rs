//! The adapter-port harness plus PHP-specific edges.

use halmi_core::adapter::LanguageAdapter;
use halmi_core::report::FunctionReport;
use halmi_php::PhpAdapter;

fn names(a: &halmi_core::adapter::SourceAnalysis) -> Vec<String> {
    a.functions.iter().map(|f| f.name.clone()).collect()
}

fn find<'a>(reports: &'a [FunctionReport], name: &str) -> &'a FunctionReport {
    reports
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} not found"))
}

const SRC: &str = r"<?php
declare(strict_types=1);

namespace Demo;

final class Widget
{
    private int $size;

    public function __construct(int $size)
    {
        $this->size = $size;
    }

    public function area(): int
    {
        if ($this->size > 0 && $this->size < 100) {
            return $this->size * $this->size;
        }
        return 0;
    }

    #[ReturnTypeWillChange]
    public function key() { }

    public static function make(int $s): self
    {
        return new self($s);
    }

    abstract protected function mustImplement(int $x): string;
}

function add(int $a, int $b): int
{
    return $a + $b;
}

$old = function ($a, $b) use ($c) { return $a + $b + $c; };
$arrow = fn($x) => $x * 2;
";

#[test]
fn methods_ctors_and_attributes() {
    let a = PhpAdapter.analyze("t.php", SRC);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    assert_eq!(n.len(), 6, "{n:?}");
    assert!(n.contains(&"Widget::__construct".to_string()), "{n:?}");
    assert!(n.contains(&"Widget::area".to_string()), "{n:?}");
    assert!(n.contains(&"Widget::key".to_string()), "{n:?}");
    assert!(n.contains(&"Widget::make".to_string()), "{n:?}");
    assert!(n.contains(&"Widget::mustImplement".to_string()), "{n:?}");
    assert!(n.contains(&"add".to_string()), "{n:?}");

    let area = find(&a.functions, "Widget::area");
    assert_eq!((area.start_line, area.end_line), (15, 21));
    assert_eq!(area.cyclomatic, 3, "1 + if + &&");

    let key = find(&a.functions, "Widget::key");
    assert_eq!(
        (key.start_line, key.end_line),
        (23, 24),
        "#[Attribute] folds into the span"
    );

    let bodyless = find(&a.functions, "Widget::mustImplement");
    assert_eq!((bodyless.start_line, bodyless.end_line), (31, 31));
}

#[test]
fn closures_and_arrows_excluded() {
    let a = PhpAdapter.analyze("t.php", SRC);
    assert_eq!(names(&a).len(), 6);
}

#[test]
fn heredocs_lex() {
    let src =
        "<?php\nfunction doc(): string {\n    return <<<EOT\n    hello \"world\"\n    EOT;\n}\n";
    let a = PhpAdapter.analyze("t.php", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let d = find(&a.functions, "doc");
    assert_eq!((d.start_line, d.end_line), (2, 6));
}

#[test]
fn multiline_strings_count_lines() {
    let src = "<?php\nfunction dq(): string {\n    $s = \"a\n  b\";\n    return strlen($s);\n}\nfunction sq(): int {\n    $t = 'x\n  y';\n    return strlen($t) + 1;\n}\nfunction after(): int {\n    return 2;\n}\n";
    let a = PhpAdapter.analyze("t.php", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let dq = find(&a.functions, "dq");
    assert_eq!((dq.start_line, dq.end_line), (2, 6));
    let sq = find(&a.functions, "sq");
    assert_eq!((sq.start_line, sq.end_line), (7, 11));
    let after = find(&a.functions, "after");
    assert_eq!((after.start_line, after.end_line), (12, 14));
}

#[test]
fn lexer_errors_surfaced() {
    let a = PhpAdapter.analyze("t.php", "<?php\n$s = \"oops;\n");
    assert!(a.error.is_some(), "unterminated string must error");
}
