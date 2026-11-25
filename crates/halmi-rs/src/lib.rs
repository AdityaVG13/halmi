//! halmi-rs: the Rust language adapter for halmi.
//!
//! Walks a Rust AST once (via `syn`) and produces one
//! [`halmi_core::FunctionReport`] per function with the full Halstead suite,
//! all MI variants, spans, and raw counts. Call
//! [`halmi_core::ranking::finalize`] afterwards to attach priority, reasons,
//! and repository ranking.
//!
//! Classification rules and metric definitions are documented in
//! [`classify`](crate::classify) and in the project README.

#![deny(unsafe_code)]

mod classify;
mod comments;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::{Block, ImplItem, Item, Signature, TraitItem};

pub use halmi_core::adapter::SourceAnalysis;

use halmi_core::adapter::LanguageAdapter;
use halmi_core::report::{FunctionReport, RawFunction, count_u32};

/// The Rust language adapter.
///
/// The reference implementation of [`halmi_core::adapter::LanguageAdapter`]. A new language plugs into halmi
/// by implementing the same trait with the same contract; everything
/// downstream (metrics, ranking, delta, gates) is language-neutral.
#[derive(Debug, Clone, Copy, Default)]
pub struct RustAdapter;

impl LanguageAdapter for RustAdapter {
    fn language(&self) -> &'static str {
        "Rust"
    }

    /// Analyze Rust source text. Never panics on syntax errors; degraded
    /// files return `functions: []` and an `error` message.
    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        let file = match syn::parse_file(src) {
            Ok(f) => f,
            Err(e) => {
                return SourceAnalysis {
                    path: path.to_string(),
                    functions: Vec::new(),
                    error: Some(e.to_string()),
                };
            }
        };
        let map = comments::comment_line_map(src);
        let mut analyzer = Analyzer {
            path,
            comment_map: &map,
            out: Vec::new(),
        };
        analyzer.walk_items(&file.items);
        SourceAnalysis {
            path: path.to_string(),
            functions: analyzer.out,
            error: None,
        }
    }
}

/// Analyze Rust source text (delegates to [`RustAdapter`]).
#[must_use]
pub fn analyze_source(path: &str, src: &str) -> SourceAnalysis {
    RustAdapter.analyze(path, src)
}

struct Analyzer<'a> {
    path: &'a str,
    comment_map: &'a [bool],
    out: Vec<FunctionReport>,
}

impl Analyzer<'_> {
    fn walk_items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Fn(f) => self.add_function(f.sig.ident.to_string(), &f.sig, &f.block),
                Item::Impl(i) => {
                    let owner = type_name(i.self_ty.as_ref()).unwrap_or_default();
                    for impl_item in &i.items {
                        if let ImplItem::Fn(f) = impl_item {
                            let name = qualified(&owner, &f.sig.ident.to_string());
                            self.add_function(name, &f.sig, &f.block);
                        }
                    }
                }
                Item::Trait(t) => {
                    for trait_item in &t.items {
                        if let TraitItem::Fn(f) = trait_item
                            && let Some(block) = &f.default
                        {
                            let name = qualified(&t.ident.to_string(), &f.sig.ident.to_string());
                            self.add_function(name, &f.sig, block);
                        }
                    }
                }
                Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        self.walk_items(inner);
                    }
                }
                _ => {}
            }
        }
    }

    fn add_function(&mut self, name: String, sig: &Signature, block: &Block) {
        let (start_line, end_line) = fn_span(sig, block);
        let mut walked = classify::walk_block(block);
        walked.counts.io_params = unique_io_params(sig);
        let raw = RawFunction {
            path: self.path.to_string(),
            language: halmi_core::report::Language::Rust,
            name,
            start_line,
            end_line,
            counts: walked.counts,
            cyclomatic: walked.cyclomatic,
            loc: walked.loc,
            nesting: walked.nesting,
            comment_density: comments::density(self.comment_map, start_line, end_line),
            method_chain: walked.method_chain,
        };
        self.out.push(FunctionReport::from_raw(&raw));
    }
}

/// eta2*: distinct parameter names plus one for a non-unit return type.
/// `self` counts as a parameter.
fn unique_io_params(sig: &Signature) -> usize {
    let mut names: Vec<String> = Vec::new();
    for arg in &sig.inputs {
        match arg {
            syn::FnArg::Receiver(_) => {
                if !names.iter().any(|n| n == "self") {
                    names.push("self".to_string());
                }
            }
            syn::FnArg::Typed(pat) => collect_pat_idents(&pat.pat, &mut names),
        }
    }
    let returns = !matches!(sig.output, syn::ReturnType::Default);
    names.len() + usize::from(returns)
}

fn collect_pat_idents(pat: &syn::Pat, out: &mut Vec<String>) {
    match pat {
        syn::Pat::Ident(p) => {
            if !out.contains(&p.ident.to_string()) {
                out.push(p.ident.to_string());
            }
        }
        syn::Pat::Tuple(p) => p.elems.iter().for_each(|e| collect_pat_idents(e, out)),
        syn::Pat::Or(p) => p.cases.iter().for_each(|e| collect_pat_idents(e, out)),
        syn::Pat::Reference(p) => collect_pat_idents(&p.pat, out),
        syn::Pat::Slice(p) => p.elems.iter().for_each(|e| collect_pat_idents(e, out)),
        syn::Pat::Struct(p) => p
            .fields
            .iter()
            .for_each(|f| collect_pat_idents(&f.pat, out)),
        syn::Pat::TupleStruct(p) => p.elems.iter().for_each(|e| collect_pat_idents(e, out)),
        syn::Pat::Type(p) => collect_pat_idents(&p.pat, out),
        syn::Pat::Paren(p) => collect_pat_idents(&p.pat, out),
        _ => {}
    }
}

/// `Type::method` when there is an owner, else just `method`.
fn qualified(owner: &str, method: &str) -> String {
    if owner.is_empty() {
        method.to_string()
    } else {
        format!("{owner}::{method}")
    }
}

/// Last path segment of a self type (`impl crate::foo::Bar` -> `Bar`).
fn type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        syn::Type::Reference(r) => type_name(&r.elem),
        _ => None,
    }
}

/// 1-based inclusive line span from the first token of the signature through
/// the last token of the body (attributes and visibility are excluded).
fn fn_span(sig: &Signature, block: &Block) -> (u32, u32) {
    let mut start = u32::MAX;
    let mut end = 0;
    span_bounds(&sig.to_token_stream(), &mut start, &mut end);
    span_bounds(&block.to_token_stream(), &mut start, &mut end);
    (start, end)
}

fn span_bounds(ts: &TokenStream, start: &mut u32, end: &mut u32) {
    for tt in ts.clone() {
        if let TokenTree::Group(g) = tt {
            let sp = g.span();
            *start = (*start).min(count_u32(sp.start().line));
            *end = (*end).max(count_u32(sp.end().line));
            span_bounds(&g.stream(), start, end);
        } else {
            let sp = tt.span();
            *start = (*start).min(count_u32(sp.start().line));
            *end = (*end).max(count_u32(sp.end().line));
        }
    }
}
