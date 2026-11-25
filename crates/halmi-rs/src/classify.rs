//! Single-pass Rust AST classification for Halstead + control-flow metrics.
//!
//! Classification rules (deterministic, token-faithful; see README.md):
//!
//! * Operators -- arithmetic, logical, comparison, bitwise, assignment and
//!   compound assignment, unary `!`/`-`/`&`/`*`, ranges, `?`, `as`, field
//!   access, method calls, calls, indexing, struct/array literals, the `fn`
//!   keyword itself, control keywords (`if`, `match`, `while`, `for`, `loop`,
//!   `return`, `break`, `continue`, `await`, `let`, `unsafe`, `async`,
//!   `yield`), closures, and macros (each macro name is its own operator
//!   kind, e.g. `macro:println`).
//! * Operands -- identifiers (locals, fields, functions, method names,
//!   type names, path segments, tuple indices) and literals, keyed by their
//!   token text. `self` counts as an operand.
//! * Excluded -- separators (`;`, `,`, `::`, braces, grouping parens),
//!   attributes, doc comments, the function's own signature (name, params,
//!   return type), nested items inside a body, and everything inside macro
//!   invocation token streams (a macro counts as one operator).
//!
//! Cyclomatic complexity: 1 + one per `if` (incl. `else if`), `while`,
//! `while let`, `for`, `loop`, match arm, `&&`, `||`, and `?`.
//!
//! Logical LOC: statement count, plus one per match arm (arm bodies in
//! blocks are already counted as statements).
//!
//! Nesting depth: max depth of nested control-flow constructs (`if`
//! branches, match arms, loop bodies). The function body itself is depth 0;
//! `else if` chains stay flat; closures do not add nesting.

use std::collections::HashMap;

use syn::{
    BinOp, Expr, ExprAwait, ExprBreak, ExprCall, ExprCast, ExprContinue, ExprField, ExprForLoop,
    ExprIf, ExprIndex, ExprLet, ExprLoop, ExprMacro, ExprMatch, ExprMethodCall, ExprRange,
    ExprReference, ExprReturn, ExprStruct, ExprTry, ExprUnary, ExprWhile, GenericArgument, Lit,
    Member, Pat, Path, PathArguments, RangeLimits, Stmt, Type, UnOp, visit::Visit,
};

use halmi_core::HalsteadCounts;

pub struct WalkResult {
    pub counts: HalsteadCounts,
    pub cyclomatic: u32,
    pub loc: u32,
    pub nesting: u32,
    pub method_chain: u32,
}

struct FnVisitor {
    ops: HashMap<String, usize>,
    operands: HashMap<String, usize>,
    cc: u32,
    loc: u32,
    nesting: u32,
    max_nesting: u32,
    /// Current method-call chain depth on the visited expression spine;
    /// arguments start a fresh chain, receivers extend it.
    cur_chain: u32,
    max_chain: u32,
}

impl FnVisitor {
    fn op(&mut self, kind: &str) {
        match self.ops.get_mut(kind) {
            Some(c) => *c += 1,
            None => {
                self.ops.insert(kind.to_string(), 1);
            }
        }
    }

    fn operand(&mut self, text: &str) {
        match self.operands.get_mut(text) {
            Some(c) => *c += 1,
            None => {
                self.operands.insert(text.to_string(), 1);
            }
        }
    }

    fn with_nesting(&mut self, f: impl FnOnce(&mut Self)) {
        self.nesting += 1;
        if self.nesting > self.max_nesting {
            self.max_nesting = self.nesting;
        }
        f(self);
        self.nesting -= 1;
    }

    pub(super) fn run(block: &syn::Block) -> WalkResult {
        let mut v = Self {
            ops: HashMap::new(),
            operands: HashMap::new(),
            cc: 1,
            loc: 0,
            nesting: 0,
            max_nesting: 0,
            cur_chain: 0,
            max_chain: 0,
        };
        v.op("fn");
        v.visit_block(block);
        WalkResult {
            counts: HalsteadCounts {
                eta1: v.ops.len(),
                eta2: v.operands.len(),
                n1: v.ops.values().sum(),
                n2: v.operands.values().sum(),
                io_params: 0,
            },
            cyclomatic: v.cc,
            loc: v.loc,
            nesting: v.max_nesting,
            method_chain: v.max_chain,
        }
    }

    fn macro_op(&mut self, mac: &syn::Macro) {
        let name = mac
            .path
            .segments
            .last()
            .map_or_default(|s| s.ident.to_string());
        self.op(&format!("macro:{name}"));
    }

    fn visit_path_segments(&mut self, path: &Path) {
        for seg in &path.segments {
            self.operand(&seg.ident.to_string());
            match &seg.arguments {
                PathArguments::AngleBracketed(args) => {
                    for arg in &args.args {
                        match arg {
                            GenericArgument::Type(t) => self.visit_type(t),
                            GenericArgument::Const(e) => self.visit_expr(e),
                            GenericArgument::AssocType(a) => self.visit_type(&a.ty),
                            GenericArgument::AssocConst(a) => self.visit_expr(&a.value),
                            _ => {}
                        }
                    }
                }
                PathArguments::Parenthesized(args) => {
                    for t in &args.inputs {
                        self.visit_type(t);
                    }
                    if let syn::ReturnType::Type(_, t) = &args.output {
                        self.visit_type(t);
                    }
                }
                PathArguments::None => {}
            }
        }
    }

    fn operand_member(&mut self, member: &Member) {
        match member {
            Member::Named(m) => self.operand(&m.to_string()),
            Member::Unnamed(i) => self.operand(&i.index.to_string()),
        }
    }

    #[expect(
        clippy::ref_option,
        reason = "the signature is fixed by syn::visit_mut::VisitMut"
    )]
    fn visit_path_expr(&mut self, qself: &Option<syn::QSelf>, path: &Path) {
        if let Some(q) = qself {
            self.visit_type(&q.ty);
        }
        self.visit_path_segments(path);
    }
}

const fn bin_op_str(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add(_) => "+",
        BinOp::Sub(_) => "-",
        BinOp::Mul(_) => "*",
        BinOp::Div(_) => "/",
        BinOp::Rem(_) => "%",
        BinOp::And(_) => "&&",
        BinOp::Or(_) => "||",
        BinOp::BitAnd(_) => "&",
        BinOp::BitOr(_) => "|",
        BinOp::BitXor(_) => "^",
        BinOp::Shl(_) => "<<",
        BinOp::Shr(_) => ">>",
        BinOp::Eq(_) => "==",
        BinOp::Ne(_) => "!=",
        BinOp::Lt(_) => "<",
        BinOp::Le(_) => "<=",
        BinOp::Gt(_) => ">",
        BinOp::Ge(_) => ">=",
        BinOp::AddAssign(_) => "+=",
        BinOp::SubAssign(_) => "-=",
        BinOp::MulAssign(_) => "*=",
        BinOp::DivAssign(_) => "/=",
        BinOp::RemAssign(_) => "%=",
        BinOp::ShlAssign(_) => "<<=",
        BinOp::ShrAssign(_) => ">>=",
        BinOp::BitAndAssign(_) => "&=",
        BinOp::BitOrAssign(_) => "|=",
        BinOp::BitXorAssign(_) => "^=",
        _ => "?op?",
    }
}

const fn un_op_str(op: &UnOp) -> &'static str {
    match op {
        UnOp::Deref(_) => "*",
        UnOp::Not(_) => "!",
        UnOp::Neg(_) => "-",
        _ => "?op?",
    }
}

fn lit_text(lit: &Lit) -> Option<String> {
    match lit {
        Lit::Str(l) => Some(l.token().to_string()),
        Lit::CStr(l) => Some(l.token().to_string()),
        Lit::ByteStr(l) => Some(l.token().to_string()),
        Lit::Byte(l) => Some(l.token().to_string()),
        Lit::Char(l) => Some(l.token().to_string()),
        Lit::Int(l) => Some(l.token().to_string()),
        Lit::Float(l) => Some(l.token().to_string()),
        Lit::Bool(l) => Some(l.value.to_string()),
        _ => None,
    }
}

impl<'ast> Visit<'ast> for FnVisitor {
    fn visit_type_param_bound(&mut self, b: &'ast syn::TypeParamBound) {
        if let syn::TypeParamBound::Trait(tb) = b {
            self.visit_path_segments(&tb.path);
        }
    }

    fn visit_block(&mut self, i: &'ast syn::Block) {
        for stmt in &i.stmts {
            self.visit_stmt(stmt);
        }
    }

    fn visit_stmt(&mut self, i: &'ast Stmt) {
        match i {
            Stmt::Local(l) => {
                self.op("let");
                self.loc += 1;
                self.visit_pat(&l.pat);
                if let Some(init) = &l.init {
                    self.op("=");
                    self.visit_expr(&init.expr);
                    if let Some((_, diverge)) = &init.diverge {
                        self.visit_expr(diverge);
                    }
                }
            }
            Stmt::Expr(e, _) => {
                self.loc += 1;
                self.visit_expr(e);
            }
            Stmt::Macro(m) => {
                self.loc += 1;
                self.macro_op(&m.mac);
            }

            Stmt::Item(_) => {}
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one match over every syn expression kind; the arms are the Halstead operator table and the recursion, kept together so each kind is counted exactly once"
    )]
    fn visit_expr(&mut self, e: &'ast Expr) {
        match e {
            Expr::Binary(x) => {
                self.op(bin_op_str(&x.op));
                if matches!(x.op, BinOp::And(_) | BinOp::Or(_)) {
                    self.cc += 1;
                }
                self.visit_expr(&x.left);
                self.visit_expr(&x.right);
            }
            Expr::Unary(ExprUnary { op, expr, .. }) => {
                self.op(un_op_str(op));
                self.visit_expr(expr);
            }
            Expr::Assign(x) => {
                self.op("=");
                self.visit_expr(&x.left);
                self.visit_expr(&x.right);
            }
            Expr::Call(ExprCall { func, args, .. }) => {
                self.op("call");
                self.visit_expr(func);
                for a in args {
                    self.visit_expr(a);
                }
            }
            Expr::MethodCall(ExprMethodCall {
                receiver,
                method,
                turbofish,
                args,
                ..
            }) => {
                self.op("method_call");

                let before = self.cur_chain;
                self.cur_chain += 1;
                if self.cur_chain > self.max_chain {
                    self.max_chain = self.cur_chain;
                }
                self.visit_expr(receiver);
                self.cur_chain = 0;
                self.operand(&method.to_string());
                if let Some(tf) = turbofish {
                    for arg in &tf.args {
                        match arg {
                            GenericArgument::Type(t) => self.visit_type(t),
                            GenericArgument::Const(e) => self.visit_expr(e),
                            GenericArgument::AssocType(a) => self.visit_type(&a.ty),
                            GenericArgument::AssocConst(a) => self.visit_expr(&a.value),
                            _ => {}
                        }
                    }
                }
                for a in args {
                    self.visit_expr(a);
                }
                self.cur_chain = before;
            }
            Expr::Field(ExprField { base, member, .. }) => {
                self.op("field");
                self.visit_expr(base);
                self.operand_member(member);
            }
            Expr::Index(ExprIndex { expr, index, .. }) => {
                self.op("index");
                self.visit_expr(expr);
                self.visit_expr(index);
            }
            Expr::Lit(x) => {
                if let Some(text) = lit_text(&x.lit) {
                    self.operand(&text);
                }
            }
            Expr::Path(x) => self.visit_path_expr(&x.qself, &x.path),
            Expr::If(ExprIf {
                cond,
                then_branch,
                else_branch,
                ..
            }) => {
                self.op("if");
                self.cc += 1;
                self.visit_expr(cond);
                self.with_nesting(|v| v.visit_block(then_branch));
                if let Some((_, else_expr)) = else_branch {
                    self.visit_expr(else_expr);
                }
            }
            Expr::Match(ExprMatch { expr, arms, .. }) => {
                self.op("match");
                self.visit_expr(expr);
                for arm in arms {
                    self.cc += 1;
                    self.loc += 1;
                    self.with_nesting(|v| {
                        v.visit_pat(&arm.pat);
                        if let Some((_, guard)) = &arm.guard {
                            v.visit_expr(guard);
                        }
                        v.visit_expr(&arm.body);
                    });
                }
            }
            Expr::While(ExprWhile { cond, body, .. }) => {
                self.op("while");
                self.cc += 1;
                self.visit_expr(cond);
                self.with_nesting(|v| v.visit_block(body));
            }
            Expr::ForLoop(ExprForLoop {
                pat, expr, body, ..
            }) => {
                self.op("for");
                self.cc += 1;
                self.visit_pat(pat);
                self.visit_expr(expr);
                self.with_nesting(|v| v.visit_block(body));
            }
            Expr::Loop(ExprLoop { body, .. }) => {
                self.op("loop");
                self.cc += 1;
                self.with_nesting(|v| v.visit_block(body));
            }
            Expr::Closure(x) => {
                self.op("closure");
                for p in &x.inputs {
                    self.visit_pat(p);
                }
                self.visit_expr(&x.body);
            }
            Expr::Return(ExprReturn { expr, .. }) => {
                self.op("return");
                if let Some(e) = expr {
                    self.visit_expr(e);
                }
            }
            Expr::Break(ExprBreak { expr, .. }) => {
                self.op("break");
                if let Some(e) = expr {
                    self.visit_expr(e);
                }
            }
            Expr::Continue(ExprContinue { .. }) => self.op("continue"),
            Expr::Await(ExprAwait { base, .. }) => {
                self.op("await");
                self.visit_expr(base);
            }
            Expr::Try(ExprTry { expr, .. }) => {
                self.op("?");
                self.cc += 1;
                self.visit_expr(expr);
            }
            Expr::Cast(ExprCast { expr, ty, .. }) => {
                self.op("as");
                self.visit_expr(expr);
                self.visit_type(ty);
            }
            Expr::Reference(ExprReference { expr, .. }) => {
                self.op("&");
                self.visit_expr(expr);
            }
            Expr::Paren(x) => self.visit_expr(&x.expr),
            Expr::Group(x) => self.visit_expr(&x.expr),
            Expr::Range(ExprRange {
                start, limits, end, ..
            }) => {
                self.op(match limits {
                    RangeLimits::HalfOpen(_) => "..",
                    RangeLimits::Closed(_) => "..=",
                });
                if let Some(s) = start {
                    self.visit_expr(s);
                }
                if let Some(e) = end {
                    self.visit_expr(e);
                }
            }
            Expr::Macro(ExprMacro { mac, .. }) => self.macro_op(mac),
            Expr::Struct(ExprStruct {
                qself,
                path,
                fields,
                ..
            }) => {
                self.op("struct_lit");
                self.visit_path_expr(qself, path);
                for f in fields {
                    self.operand_member(&f.member);
                    self.visit_expr(&f.expr);
                }
            }
            Expr::Array(x) => {
                self.op("array");
                for e in &x.elems {
                    self.visit_expr(e);
                }
            }
            Expr::Repeat(x) => {
                self.op("repeat");
                self.visit_expr(&x.expr);
                self.visit_expr(&x.len);
            }
            Expr::Tuple(x) => {
                for e in &x.elems {
                    self.visit_expr(e);
                }
            }
            Expr::Block(x) => self.visit_block(&x.block),
            Expr::Async(x) => {
                self.op("async");
                self.visit_block(&x.block);
            }
            Expr::Unsafe(x) => {
                self.op("unsafe");
                self.visit_block(&x.block);
            }
            Expr::Const(x) => self.visit_block(&x.block),
            Expr::TryBlock(x) => self.visit_block(&x.block),
            Expr::Let(ExprLet { expr, pat, .. }) => {
                self.op("let");
                self.visit_expr(expr);
                self.visit_pat(pat);
            }
            Expr::Yield(x) => {
                self.op("yield");
                if let Some(e) = &x.expr {
                    self.visit_expr(e);
                }
            }
            _ => {}
        }
    }

    fn visit_pat(&mut self, p: &'ast Pat) {
        match p {
            Pat::Ident(x) => {
                self.operand(&x.ident.to_string());
                if let Some((_, sub)) = &x.subpat {
                    self.visit_pat(sub);
                }
            }
            Pat::Lit(x) => {
                if let Some(text) = lit_text(&x.lit) {
                    self.operand(&text);
                }
            }
            Pat::Or(x) => {
                for case in &x.cases {
                    self.visit_pat(case);
                }
            }
            Pat::Paren(x) => self.visit_pat(&x.pat),
            Pat::Reference(x) => {
                self.op("&");
                self.visit_pat(&x.pat);
            }
            Pat::Range(x) => {
                if let Some(s) = &x.start {
                    self.visit_expr(s);
                }
                if let Some(e) = &x.end {
                    self.visit_expr(e);
                }
            }
            Pat::Slice(x) => {
                for e in &x.elems {
                    self.visit_pat(e);
                }
            }
            Pat::Struct(x) => {
                self.visit_path_expr(&None, &x.path);
                for f in &x.fields {
                    self.operand_member(&f.member);
                    self.visit_pat(&f.pat);
                }
            }
            Pat::Tuple(x) => {
                for e in &x.elems {
                    self.visit_pat(e);
                }
            }
            Pat::TupleStruct(x) => {
                self.visit_path_expr(&x.qself, &x.path);
                for e in &x.elems {
                    self.visit_pat(e);
                }
            }
            Pat::Type(x) => {
                self.visit_pat(&x.pat);
                self.visit_type(&x.ty);
            }
            Pat::Macro(x) => self.macro_op(&x.mac),
            Pat::Path(x) => self.visit_path_expr(&x.qself, &x.path),

            _ => {}
        }
    }

    fn visit_type(&mut self, t: &'ast Type) {
        match t {
            Type::Path(x) => self.visit_path_expr(&x.qself, &x.path),
            Type::Reference(x) => {
                self.op("&");
                self.visit_type(&x.elem);
            }
            Type::Ptr(x) => {
                self.op("*");
                self.visit_type(&x.elem);
            }
            Type::Array(x) => {
                self.visit_type(&x.elem);
                self.visit_expr(&x.len);
            }
            Type::Slice(x) => self.visit_type(&x.elem),
            Type::Paren(x) => self.visit_type(&x.elem),
            Type::Group(x) => self.visit_type(&x.elem),
            Type::Tuple(x) => {
                for e in &x.elems {
                    self.visit_type(e);
                }
            }
            Type::TraitObject(x) => {
                for b in &x.bounds {
                    self.visit_type_param_bound(b);
                }
            }
            Type::ImplTrait(x) => {
                for b in &x.bounds {
                    self.visit_type_param_bound(b);
                }
            }
            Type::BareFn(x) => {
                for arg in &x.inputs {
                    self.visit_type(&arg.ty);
                }
                if let syn::ReturnType::Type(_, t) = &x.output {
                    self.visit_type(t);
                }
            }
            Type::Macro(x) => self.macro_op(&x.mac),
            _ => {}
        }
    }
}

/// Walk one function body and produce the raw metric counts.
pub fn walk_block(block: &syn::Block) -> WalkResult {
    FnVisitor::run(block)
}
