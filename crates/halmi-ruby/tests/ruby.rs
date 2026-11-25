//! The adapter-port harness plus Ruby-specific edges.

use halmi_core::adapter::LanguageAdapter;
use halmi_core::report::FunctionReport;
use halmi_ruby::RubyAdapter;

fn names(a: &halmi_core::adapter::SourceAnalysis) -> Vec<String> {
    a.functions.iter().map(|f| f.name.clone()).collect()
}

fn find<'a>(reports: &'a [FunctionReport], name: &str) -> &'a FunctionReport {
    reports
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} not found in {:?}", names_of(reports)))
}

fn names_of(reports: &[FunctionReport]) -> Vec<String> {
    reports.iter().map(|f| f.name.clone()).collect()
}

const SRC: &str = r"
module Demo
  class Widget
    def initialize(size)
      @size = size
    end

    def area
      return 0 unless @size
      if @size > 0 && @size < 100
        @size * @size
      else
        0
      end
    end

    def self.make(s)
      new(s)
    end

    def name=(v)
      @name = v
    end

    def hidden
      yield if block_given?
    end

    class << self
      def meta
        1
      end
    end
  end
end

def top_level(a, b = 2)
  l = lambda { |x| x + 1 }
  a + b
end
";

#[test]
fn defs_and_end_blocks() {
    let a = RubyAdapter.analyze("t.rb", SRC);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    for expected in [
        "Demo::Widget::initialize",
        "Demo::Widget::area",
        "Demo::Widget::make",
        "Demo::Widget::name=",
        "Demo::Widget::hidden",
        "Demo::Widget::meta",
        "top_level",
    ] {
        assert!(
            n.contains(&expected.to_string()),
            "{expected} missing: {n:?}"
        );
    }
    assert_eq!(n.len(), 7, "lambdas excluded: {n:?}");

    let area = find(&a.functions, "Demo::Widget::area");
    assert_eq!((area.start_line, area.end_line), (8, 15));
    assert_eq!(area.cyclomatic, 4, "1 + unless + if + &&");

    let setter = find(&a.functions, "Demo::Widget::name=");
    assert_eq!((setter.start_line, setter.end_line), (21, 23));
}

#[test]
fn modifier_conditionals_do_not_open_blocks() {
    let src = "def f(x)\n  return 0 if x.nil?\n  x while false\nend\n";
    let a = RubyAdapter.analyze("t.rb", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let f = find(&a.functions, "f");
    assert_eq!((f.start_line, f.end_line), (1, 4));
}

#[test]
fn loop_do_does_not_double_open() {
    let src = "def f\n  while true do\n    break\n  end\n  items.each do |x|\n    p x\n  end\n  for x in [1] do\n    p x\n  end\nend\n";
    let a = RubyAdapter.analyze("loops.rb", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let f = find(&a.functions, "f");
    assert_eq!((f.start_line, f.end_line), (1, 11));
    assert_eq!(f.cyclomatic, 3);
}

#[test]
fn iterator_do_inside_a_loop_remains_a_block() {
    let src = "def f\n  while true\n    items.each do |x|\n      p x\n    end\n  end\nend\n";
    let a = RubyAdapter.analyze("nested_loops.rb", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let f = find(&a.functions, "f");
    assert_eq!((f.start_line, f.end_line), (1, 7));
    assert_eq!(f.cyclomatic, 2);
}

#[test]
fn self_dot_class_in_params() {
    let src = "class M\n  def setup_blocks(base = self.class)\n    base.each do |b|\n      p b\n    end\n  end\nend\n";
    let a = RubyAdapter.analyze("t.rb", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let d = find(&a.functions, "M::setup_blocks");
    assert_eq!((d.start_line, d.end_line), (2, 6));
}

#[test]
fn heredocs_and_percent_literals() {
    let src = "def render\n  txt = <<~EOS\n    hello #{name}\n  EOS\n  re = %r{a/b}\n  list = %w[a b c]\n  bare = %{<xml/>}\n  bang = %!command!
  slash = %q/def fake/
  pipe = %q/def other_fake/
  regex = %r/def third_fake/
  txt\nend\n";
    let a = RubyAdapter.analyze("t.rb", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let r = find(&a.functions, "render");
    assert_eq!((r.start_line, r.end_line), (1, 13));
}

#[test]
fn char_literals_cover_utf8_and_truncation() {
    let a = RubyAdapter.analyze("t.rb", "def f\n  ?é\nend\n");
    assert_eq!(a.error, None, "{:?}", a.error);

    let a = RubyAdapter.analyze("t.rb", "def f\n  ?\\");
    assert!(a.error.is_some(), "truncated character literal must error");
}

#[test]
fn end_data_section_stops_lexing() {
    let src = "def f\n  1\nend\n\n__END__\ngarbage def broken(\n";
    let a = RubyAdapter.analyze("t.rb", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a).len(), 1);
}

#[test]
fn regex_literals_lex() {
    let src = "def check(s)\n  s =~ /a\\/b/[0-9]+\n  s.sub(/Template$/, 'Test')\nend\n";
    let a = RubyAdapter.analyze("t.rb", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let c = find(&a.functions, "check");
    assert_eq!((c.start_line, c.end_line), (1, 4));
}

#[test]
fn backtick_command_literals_close_and_allow_interpolation() {
    let src = "class Shell\n  def sha\n    `git rev-parse --short HEAD`.strip\n  end\n\n  def files(dir)\n    %x{ls -1 #{Shellwords.escape(dir)}}.split(\"\\n\")\n  end\n\n  def count(dir)\n    `ls -1 #{dir} | wc -l`.to_i\n  end\nend\n";
    let a = RubyAdapter.analyze("shell.rb", src);
    assert!(a.error.is_none(), "{:?}", a.error);
    let mut got = names(&a);
    got.sort();
    assert_eq!(got, ["Shell::count", "Shell::files", "Shell::sha"]);
}

#[test]
fn percent_x_command_literals_keep_block_balance() {
    let src = "def files(dir)\n  %x{ls -1 #{Shellwords.escape(dir)}}.split(\"\\n\")\nend\n\ndef count(dir)\n  %x(echo #{dir} | wc -l).to_i\nend\n";
    let a = RubyAdapter.analyze("x.rb", src);
    assert!(a.error.is_none(), "{:?}", a.error);
    let spans: Vec<(&str, u32, u32)> = a
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.start_line, f.end_line))
        .collect();
    assert_eq!(spans, [("files", 1, 3), ("count", 5, 7)]);
}

#[test]
fn endless_definitions_end_at_the_logical_line() {
    let src = "class Point\n  def +(other) = Point.new(x + other.x, y + other.y)\n  def sum = [1,\n             2].sum\n  def to_s = \"pt\"\n\n  def norm\n    1\n  end\nend\n";
    let a = RubyAdapter.analyze("endless.rb", src);
    assert!(a.error.is_none(), "{:?}", a.error);
    let spans: Vec<(&str, u32, u32)> = a
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.start_line, f.end_line))
        .collect();
    assert_eq!(
        spans,
        [
            ("Point::+", 2, 2),
            ("Point::sum", 3, 4),
            ("Point::to_s", 5, 5),
            ("Point::norm", 7, 9)
        ]
    );

    let src = "class Point\n  class << self\n    def opts.topdict(name) = (name.length > 1 ? top.long : top.short)\n    def alias(from, to) = (dict = topdict(from); dict[to] = dict[from])\n  end\nend\n";
    let a = RubyAdapter.analyze("receiver.rb", src);
    assert!(a.error.is_none(), "{:?}", a.error);
    let spans: Vec<(&str, u32, u32)> = a
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.start_line, f.end_line))
        .collect();
    assert_eq!(spans, [("Point::topdict", 3, 3), ("Point::alias", 4, 4)]);
}

#[test]
fn paren_less_setter_keeps_its_block_and_the_enclosing_owner() {
    let src =
        "class Runnable\n  def name= o\n    @NAME = o\n  end\n\n  def run\n    1\n  end\nend\n";
    let a = RubyAdapter.analyze("setter.rb", src);
    assert!(a.error.is_none(), "{:?}", a.error);
    let spans: Vec<(&str, u32, u32)> = a
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.start_line, f.end_line))
        .collect();
    assert_eq!(spans, [("Runnable::name=", 2, 4), ("Runnable::run", 6, 8)]);
}

#[test]
fn endless_definitions_inside_a_body_do_not_open_a_block() {
    let src = "module M\n  class << self\n    def process(args)\n      opts = Object.new\n      def opts.topdict(name) = (name.length > 1 ? top.long : top.short)\n      def opts.alias(from, to) = (dict = topdict(from); dict[to] = dict[from])\n      opts\n    end\n  end\nend\n";
    let a = RubyAdapter.analyze("nested.rb", src);
    assert!(a.error.is_none(), "{:?}", a.error);
    let spans: Vec<(&str, u32, u32)> = a
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.start_line, f.end_line))
        .collect();
    assert_eq!(
        spans,
        [
            ("M::process", 3, 8),
            ("M::topdict", 5, 5),
            ("M::alias", 6, 6)
        ]
    );
}

#[test]
fn paren_less_default_parameters_are_not_endless_definitions() {
    let src = "module M\n  class << self\n    def process_args args = [] # :nodoc:\n      options = { :io => $stdout }\n      orig_args = args.dup\n    end\n\n    def run(args) = process_args(args)\n  end\nend\n";
    let a = RubyAdapter.analyze("defaults.rb", src);
    assert!(a.error.is_none(), "{:?}", a.error);
    let spans: Vec<(&str, u32, u32)> = a
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.start_line, f.end_line))
        .collect();
    assert_eq!(spans, [("M::process_args", 3, 6), ("M::run", 8, 8)]);
}

#[test]
fn lexer_errors_surfaced() {
    let a = RubyAdapter.analyze("t.rb", "def f\n  s = \"oops\nend\n");
    assert!(a.error.is_some(), "unterminated construct must error");
}
