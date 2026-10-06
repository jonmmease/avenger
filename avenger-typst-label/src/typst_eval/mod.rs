//! Ported from crates/typst-eval/src/lib.rs @ v0.15.1, modified for Avenger.
//!
//! Typst's code interpreter.
//!
//! avenger: a static evaluator for a label: markup, math and code expressions over the label
//! library and the label's parameters. A label has no set or show rules, bindings, closures,
//! conditionals, loops, imports, includes or context, so these are errors, and so is any
//! markup that doesn't fit on one line.

mod call;
mod code;
mod markup;
mod math;
mod ops;
mod vm;

#[cfg(test)]
mod tests;

pub use self::vm::Vm;

use crate::label::label_file;
use crate::typst_library::Library;
use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::{Content, Scope, Scopes};
use typst_syntax::ast;
use typst_syntax::{RangeMapper, SyntaxKind, SyntaxNode, parse};

/// Parses a label's source as markup, with each node spanning its range of the source.
// avenger: in place of `Source`, which numbers spans; a label is its own file.
pub fn parse_label(text: &str) -> SyntaxNode {
    let mut root = parse(text);
    let mapper = RangeMapper::new(std::iter::once(0..text.len()))
        .expect("one range is a valid mapping");
    root.synthesize_mapped(label_file(), &mapper)
        .expect("the mapping covers the source");
    root
}

/// Evaluates a label's parsed markup with the label's parameters in scope.
// avenger: in place of `eval` and `eval_string`; a label is markup, evaluated once.
pub fn eval_label(
    engine: &mut Engine,
    root: &SyntaxNode,
    params: Scope,
) -> SourceResult<Content> {
    // Check for errors or warnings in the syntax tree before evaluating it.
    let (errors, warnings) = root.errors_and_warnings();
    for warning in warnings {
        engine.sink.warn(warning.into());
    }
    if !errors.is_empty() {
        return Err(errors.into_iter().map(Into::into).collect());
    }

    // Prepare VM.
    let mut scopes = Scopes::new(Some(Library::get()));
    scopes.top = params;
    let engine = Engine { world: engine.world, sink: &mut *engine.sink };
    let mut vm = Vm::new(engine, scopes);

    let markup = root.cast::<ast::Markup>().expect("the root of a label is markup");
    markup.eval(&mut vm)
}

/// The deepest nesting of math constructs in a label's syntax tree: delimiters, attachments,
/// fractions, roots, calls, parentheses, arrays, dictionaries and blocks inside equations each
/// add a level. Evaluation, realization and layout recurse per level.
// avenger: labels bound math nesting to bound recursion.
pub fn math_nesting_depth(root: &SyntaxNode) -> usize {
    let mut deepest = 0;
    let mut pending = vec![(root, 0usize, root.kind() == SyntaxKind::Math)];
    while let Some((node, depth, in_math)) = pending.pop() {
        let in_math = in_math || node.kind() == SyntaxKind::Equation;
        let depth = depth + usize::from(in_math && is_math_nesting_kind(node.kind()));
        deepest = deepest.max(depth);
        pending.extend(node.children().map(|child| (child, depth, in_math)));
    }
    deepest
}

/// Syntax kinds that open a nesting level inside math.
fn is_math_nesting_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::MathDelimited
            | SyntaxKind::MathAttach
            | SyntaxKind::MathFrac
            | SyntaxKind::MathRoot
            | SyntaxKind::MathCall
            | SyntaxKind::FuncCall
            | SyntaxKind::Parenthesized
            | SyntaxKind::Array
            | SyntaxKind::Dict
            | SyntaxKind::CodeBlock
            | SyntaxKind::ContentBlock
    )
}

/// Evaluate an expression.
pub(crate) trait Eval {
    /// The output of evaluating the expression.
    type Output;

    /// Evaluate the expression to the output value.
    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output>;
}
