//! Cyclomatic complexity of the functions and closures in one Rust source file.
//!
//! Complexity is `1` plus the number of decision points in a function's own
//! body. A closure is its own space: its decisions count towards the closure,
//! not the function around it, because `cargo llvm-cov` reports closures as
//! separate methods and OtterWise sums every method it is given.
//!
//! The [`Rules::Rca`] rule set follows `rust-code-analysis`, the established
//! Rust implementation, so scores are comparable with other tools: one for
//! each `if`, `for`, `while`, `loop`, match arm, match-arm guard, `?`, `&&`
//! and `||`. Inside a macro's arguments, which are unparsed tokens, only the
//! keywords `if`, `for`, `while` and `loop` count, as in `rust-code-analysis`.
//!
//! Known differences from `rust-code-analysis`, from counting syntax rather
//! than tokens: `&&` or `for` in type position (`&&T`, `for<'a>`) is not a
//! decision here, and `let ... else` is not counted by either.

use anyhow::{Context, Result};
use proc_macro2::{TokenStream, TokenTree};
use syn::{spanned::Spanned, visit::Visit};

/// Which decision points count towards a function's complexity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Rules {
    /// Matches `rust-code-analysis`: every match arm and every `?` counts.
    Rca,
    /// Textbook McCabe: a `match` adds one less than its arms, and `?` is
    /// not a decision.
    Mccabe,
}

/// Decision points found in one function or closure body.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Counts {
    /// `if`, `for`, `while` and `loop`, including those in macro arguments.
    decisions: u32,
    /// Match arms, and the number of `match` expressions they belong to.
    arms: u32,
    matches: u32,
    /// Match-arm guards (`Some(x) if x > 0 =>`).
    guards: u32,
    /// `&&` and `||`.
    bool_ops: u32,
    /// `?`.
    tries: u32,
}

impl Counts {
    pub(crate) fn complexity(&self, rules: Rules) -> u32 {
        let base = 1 + self.decisions + self.guards + self.bool_ops;
        match rules {
            Rules::Rca => base + self.arms + self.tries,
            Rules::Mccabe => base + self.arms.saturating_sub(self.matches),
        }
    }
}

/// One function or closure, identified the way `cargo llvm-cov` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// 1-based line the function (its `fn` keyword) or closure starts on.
    pub(crate) line: usize,
    /// Line it ends on. `cargo llvm-cov` reports a closure at the first line
    /// that carries a count, which can be later than the line it starts on, so
    /// a closure is also found by the lines it covers.
    pub(crate) end_line: usize,
    /// For a closure, its `{closure#N}` indexes from the enclosing function
    /// outwards (`[0, 1]` is `f::{closure#0}::{closure#1}`); empty for a
    /// function. Tells apart closures that start on the same line.
    pub(crate) path: Vec<u32>,
    pub(crate) counts: Counts,
    /// The function's own identifier; empty for a closure. Tells apart
    /// functions that start on the same line.
    pub(crate) name: String,
    /// Inside `#[cfg(test)]` code, or a `#[test]` function.
    pub(crate) is_test: bool,
}

struct Frame {
    entry: Entry,
    /// Next `{closure#N}` index to hand to a closure directly inside this one.
    next_closure: u32,
}

#[derive(Default)]
struct Walker {
    stack: Vec<Frame>,
    entries: Vec<Entry>,
    /// Closures outside any function (`static F: fn() = || ..`).
    root_closures: u32,
    test_depth: u32,
}

impl Walker {
    fn enter(
        &mut self,
        (line, end_line): (usize, usize),
        path: Vec<u32>,
        name: String,
        is_test: bool,
    ) {
        self.stack.push(Frame {
            entry: Entry {
                line,
                end_line,
                path,
                counts: Counts::default(),
                name,
                is_test: is_test || self.test_depth > 0,
            },
            next_closure: 0,
        });
    }

    fn leave(&mut self) {
        let frame = self.stack.pop().expect("balanced enter/leave");
        self.entries.push(frame.entry);
    }

    fn counts(&mut self) -> Option<&mut Counts> {
        self.stack.last_mut().map(|f| &mut f.entry.counts)
    }

    /// Run `walk` with a `#[cfg(test)]` region open when `attrs` say so.
    fn in_cfg_test<R>(&mut self, attrs: &[syn::Attribute], walk: impl FnOnce(&mut Self) -> R) -> R {
        let test = attrs.iter().any(is_cfg_test);
        self.test_depth += u32::from(test);
        let out = walk(self);
        self.test_depth -= u32::from(test);
        out
    }

    fn function(
        &mut self,
        attrs: &[syn::Attribute],
        name: &syn::Ident,
        lines: (usize, usize),
        walk: impl FnOnce(&mut Self),
    ) {
        self.in_cfg_test(attrs, |w| {
            w.enter(
                lines,
                Vec::new(),
                name.to_string(),
                attrs.iter().any(is_test_attr),
            );
            walk(w);
            w.leave();
        });
    }
}

/// `#[cfg(test)]`, including `cfg(all(test, ..))` and `cfg(any(test, ..))`, but
/// not `#[cfg(not(test))]`.
fn is_cfg_test(attr: &syn::Attribute) -> bool {
    if !attr.path().is_ident("cfg") {
        return false;
    }
    let Ok(list) = attr.meta.require_list() else {
        return false;
    };
    mentions_test(list.tokens.clone())
}

/// Whether `test` appears in a cfg predicate other than under a `not(..)`.
fn mentions_test(tokens: TokenStream) -> bool {
    let mut negated = false;
    for tree in tokens {
        match tree {
            TokenTree::Ident(i) if i == "test" => return true,
            TokenTree::Ident(i) => negated = i == "not",
            TokenTree::Group(g) => {
                if !negated && mentions_test(g.stream()) {
                    return true;
                }
                negated = false;
            }
            _ => negated = false,
        }
    }
    false
}

/// `#[test]`, or a runtime's variant such as `#[tokio::test]`.
fn is_test_attr(attr: &syn::Attribute) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|s| s.ident == "test")
}

/// Count the decision keywords in a macro's unparsed arguments.
fn macro_decisions(tokens: TokenStream) -> u32 {
    tokens
        .into_iter()
        .map(|tree| match tree {
            TokenTree::Ident(i) => u32::from(matches!(
                i.to_string().as_str(),
                "if" | "for" | "while" | "loop"
            )),
            TokenTree::Group(g) => macro_decisions(g.stream()),
            _ => 0,
        })
        .sum()
}

impl<'ast> Visit<'ast> for Walker {
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        self.in_cfg_test(&i.attrs, |w| syn::visit::visit_item_mod(w, i));
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        self.in_cfg_test(&i.attrs, |w| syn::visit::visit_item_impl(w, i));
    }

    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let lines = (i.sig.fn_token.span().start().line, i.span().end().line);
        self.function(&i.attrs, &i.sig.ident, lines, |w| {
            syn::visit::visit_item_fn(w, i)
        });
    }

    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        let lines = (i.sig.fn_token.span().start().line, i.span().end().line);
        self.function(&i.attrs, &i.sig.ident, lines, |w| {
            syn::visit::visit_impl_item_fn(w, i)
        });
    }

    fn visit_trait_item_fn(&mut self, i: &'ast syn::TraitItemFn) {
        // A declaration without a body has nothing to count.
        if i.default.is_some() {
            let lines = (i.sig.fn_token.span().start().line, i.span().end().line);
            self.function(&i.attrs, &i.sig.ident, lines, |w| {
                syn::visit::visit_trait_item_fn(w, i)
            });
        } else {
            syn::visit::visit_trait_item_fn(self, i);
        }
    }

    fn visit_expr_closure(&mut self, c: &'ast syn::ExprClosure) {
        let path = match self.stack.last_mut() {
            Some(parent) => {
                let index = parent.next_closure;
                parent.next_closure += 1;
                let mut path = parent.entry.path.clone();
                path.push(index);
                path
            }
            None => {
                let index = self.root_closures;
                self.root_closures += 1;
                vec![index]
            }
        };
        let is_test = self.stack.last().is_some_and(|f| f.entry.is_test);
        let lines = (c.span().start().line, c.span().end().line);
        self.enter(lines, path, String::new(), is_test);
        syn::visit::visit_expr_closure(self, c);
        self.leave();
    }

    fn visit_expr_if(&mut self, e: &'ast syn::ExprIf) {
        if let Some(c) = self.counts() {
            c.decisions += 1;
        }
        syn::visit::visit_expr_if(self, e);
    }

    fn visit_expr_while(&mut self, e: &'ast syn::ExprWhile) {
        if let Some(c) = self.counts() {
            c.decisions += 1;
        }
        syn::visit::visit_expr_while(self, e);
    }

    fn visit_expr_for_loop(&mut self, e: &'ast syn::ExprForLoop) {
        if let Some(c) = self.counts() {
            c.decisions += 1;
        }
        syn::visit::visit_expr_for_loop(self, e);
    }

    fn visit_expr_loop(&mut self, e: &'ast syn::ExprLoop) {
        if let Some(c) = self.counts() {
            c.decisions += 1;
        }
        syn::visit::visit_expr_loop(self, e);
    }

    fn visit_expr_match(&mut self, e: &'ast syn::ExprMatch) {
        if let Some(c) = self.counts() {
            c.matches += 1;
            c.arms += u32::try_from(e.arms.len()).unwrap_or(u32::MAX);
        }
        syn::visit::visit_expr_match(self, e);
    }

    fn visit_pat_guard(&mut self, g: &'ast syn::PatGuard) {
        if let Some(c) = self.counts() {
            c.guards += 1;
        }
        syn::visit::visit_pat_guard(self, g);
    }

    fn visit_expr_binary(&mut self, e: &'ast syn::ExprBinary) {
        if matches!(e.op, syn::BinOp::And(_) | syn::BinOp::Or(_))
            && let Some(c) = self.counts()
        {
            c.bool_ops += 1;
        }
        syn::visit::visit_expr_binary(self, e);
    }

    fn visit_expr_try(&mut self, e: &'ast syn::ExprTry) {
        if let Some(c) = self.counts() {
            c.tries += 1;
        }
        syn::visit::visit_expr_try(self, e);
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        let n = macro_decisions(m.tokens.clone());
        if let Some(c) = self.counts() {
            c.decisions += n;
        }
    }
}

/// Every function and closure in `source`, in the order they finish (a
/// closure before the function around it).
pub(crate) fn analyze(source: &str) -> Result<Vec<Entry>> {
    let file = syn::parse_file(source).context("failed to parse Rust source")?;
    let mut walker = Walker::default();
    walker.visit_file(&file);
    Ok(walker.entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE: &str = include_str!("../../tests/fixtures/complexity_reference.rs.txt");
    const MACROS: &str = include_str!("../../tests/fixtures/complexity_macros.rs.txt");

    /// (start line, closure path, complexity) for every function and closure.
    fn table(source: &str, rules: Rules) -> Vec<(usize, Vec<u32>, u32)> {
        analyze(source)
            .expect("parses")
            .into_iter()
            .map(|e| (e.line, e.path.clone(), e.counts.complexity(rules)))
            .collect()
    }

    fn value(source: &str, rules: Rules, line: usize, path: &[u32]) -> u32 {
        let t = table(source, rules);
        let hits: Vec<_> = t
            .iter()
            .filter(|(l, p, _)| *l == line && p.as_slice() == path)
            .collect();
        assert_eq!(hits.len(), 1, "line {line} path {path:?} in {t:?}");
        hits[0].2
    }

    /// Values produced by `rust-code-analysis` 0.0.25 (`rust-code-analysis-cli
    /// -m`) on `complexity_reference.rs.txt`: a function's own value is its
    /// `sum` minus its children's, with closures as separate spaces.
    #[test]
    fn rca_rules_match_rust_code_analysis_on_the_reference_fixture() {
        let expected: &[(usize, &[u32], u32)] = &[
            (1, &[], 1),      // plain
            (5, &[], 3),      // if / else if
            (15, &[], 2),     // if let
            (19, &[], 5),     // while, for, loop, if
            (38, &[], 2),     // while let
            (46, &[], 5),     // three arms and a guard
            (54, &[], 3),     // && and ||
            (58, &[], 3),     // two `?`
            (64, &[], 1),     // with_closure
            (65, &[0], 2),    //   its closure
            (68, &[], 1),     // nested_closure
            (70, &[0], 1),    //   outer closure
            (71, &[0, 0], 2), //   inner closure
            (77, &[], 1),     // same_line_closures
            (78, &[0], 1),    //   outer closure, same line as ...
            (78, &[0, 0], 2), //  ... the inner one
            (81, &[], 1),     // two_closures
            (82, &[0], 2),
            (83, &[1], 2),
            (87, &[], 1),   // let-else is not counted
            (94, &[], 2),   // `&&` in assert! is not counted; the guard in matches! is
            (99, &[], 1),   // nested_fn
            (100, &[], 2),  //   inner fn
            (109, &[], 3),  // impl method, two arms
            (118, &[], 2),  // trait default method
            (124, &[], 2),  // async fn
            (128, &[], 1),  // empty_closure
            (129, &[0], 1), //   `|| 1`
        ];
        let got = table(REFERENCE, Rules::Rca);
        for (line, path, want) in expected {
            assert_eq!(
                value(REFERENCE, Rules::Rca, *line, path),
                *want,
                "line {line} path {path:?}"
            );
        }
        assert_eq!(
            got.len(),
            expected.len(),
            "every space is accounted for: {got:?}"
        );
    }

    /// Inside a macro's arguments `rust-code-analysis` counts only the keywords
    /// `if`, `for`, `while` and `loop`: not `&&`, `||`, `?` or match arms.
    #[test]
    fn only_keywords_count_inside_macro_arguments() {
        let expected: &[(usize, u32)] = &[
            (5, 1),  // m!(a)
            (9, 1),  // &&
            (13, 1), // ||
            (17, 1), // ?
            (21, 2), // if
            (25, 3), // if ... else if
            (29, 2), // for
            (33, 2), // while
            (37, 2), // loop
            (41, 1), // match arms
            (45, 2), // match with a guard
            (49, 2), // a closure body is part of the function
            (53, 1), // `|| 1`
            (57, 2), // nested macros
            (61, 1), // assert!(a && b)
            (65, 2), // format! argument
            (69, 2), // matches! guard
            (73, 1), // matches! with `|`
            (77, 1), // dbg!(x?)
            (82, 2), // macro statement with braces
            (86, 2), // vec![if ...]
        ];
        let got = table(MACROS, Rules::Rca);
        for (line, want) in expected {
            assert_eq!(value(MACROS, Rules::Rca, *line, &[]), *want, "line {line}");
        }
        assert_eq!(
            got.len(),
            expected.len(),
            "closures in macros are not entries: {got:?}"
        );
    }

    #[test]
    fn mccabe_counts_one_less_per_match_and_ignores_try() {
        // matcher: 1 + (3 arms - 1) + guard = 4 (rca: 5)
        assert_eq!(value(REFERENCE, Rules::Mccabe, 46, &[]), 4);
        // method: 1 + (2 arms - 1) = 2 (rca: 3)
        assert_eq!(value(REFERENCE, Rules::Mccabe, 109, &[]), 2);
        // tries: `?` is not counted (rca: 3)
        assert_eq!(value(REFERENCE, Rules::Mccabe, 58, &[]), 1);
        // everything without a match or `?` is the same under both rule sets
        for line in [1, 5, 15, 19, 38, 54, 94, 118, 124] {
            assert_eq!(
                value(REFERENCE, Rules::Mccabe, line, &[]),
                value(REFERENCE, Rules::Rca, line, &[]),
                "line {line}"
            );
        }
    }

    #[test]
    fn entries_record_the_line_they_end_on() {
        let src = "\
fn f(v: &[i32]) -> Vec<i32> {
    v.iter().map(|x| Wrapper {
        a: *x,
    }).collect()
}
";
        let spans: Vec<(usize, usize, Vec<u32>)> = analyze(src)
            .expect("parses")
            .into_iter()
            .map(|e| (e.line, e.end_line, e.path))
            .collect();
        // the closure first (it finishes first), then the function
        assert_eq!(spans, vec![(2, 4, vec![0]), (1, 5, vec![])]);
    }

    #[test]
    fn test_code_is_flagged() {
        let src = "\
fn prod() {}

#[cfg(test)]
mod tests {
    #[test]
    fn a() {}
    fn helper() { let _ = || 1; }
}

#[test]
fn top_level_test() {}

#[cfg(not(test))]
fn not_a_test() {}
";
        let flags: Vec<(usize, bool)> = analyze(src)
            .expect("parses")
            .into_iter()
            .map(|e| (e.line, e.is_test))
            .collect();
        assert_eq!(
            flags,
            vec![
                (1, false),
                (6, true),
                (7, true),
                (7, true),
                (11, true),
                (14, false)
            ]
        );
    }

    #[test]
    fn unparseable_source_is_an_error() {
        assert!(analyze("fn broken( {").is_err());
    }

    #[test]
    fn cfg_all_test_and_not_something_else_is_still_test_code() {
        let src = "#[cfg(all(test, not(windows)))]\nfn a() { if true {} }\n\
                   #[cfg(not(test))]\nfn b() {}\n\
                   #[cfg(any(test, feature = \"x\"))]\nfn c() {}\n";
        let tests: Vec<_> = analyze(src)
            .expect("parses")
            .iter()
            .map(|e| (e.line, e.is_test))
            .collect();
        assert_eq!(tests, vec![(2, true), (4, false), (6, true)]);
    }
}
