//! Ported from crates/typst-eval/src/flow.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: only conditionals. A label has no loops or functions of its own, so it has no flow
//! events to break, continue or return with.

use crate::typst_library::diag::{At, SourceResult};
use crate::typst_library::foundations::Value;
use typst_syntax::ast::{self, AstNode};

use super::{Eval, Vm};

impl Eval for ast::Conditional<'_> {
    type Output = Value;

    // avenger: no return to mark as conditional, since a label has no functions to return from.
    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let condition = self.condition();
        let output = if condition.eval(vm)?.cast::<bool>().at(condition.span())? {
            self.if_body().eval(vm)?
        } else if let Some(else_body) = self.else_body() {
            else_body.eval(vm)?
        } else {
            Value::None
        };

        Ok(output)
    }
}
