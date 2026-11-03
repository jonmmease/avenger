//! What a query reads of the states of selections, call by call, so a dataflow over the query's
//! plan can cut it where a part stops reading an input that changes, such as the clause of a plot
//! that a brush moves.

use std::collections::BTreeSet;

use datafusion::{
    common::{
        tree_node::{TreeNode, TreeNodeRecursion},
        DFSchema, Result,
    },
    logical_expr::{
        expr::{Placeholder, ScalarFunction},
        Expr,
    },
};

use crate::{
    bind::marker,
    lower::{fold_options, is_selected, Options},
};

/// The clauses of one selection that a call reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionRead {
    /// The placeholder of the selection's state, such as `$brush`.
    pub parameter: String,
    pub owners: Owners,
}

/// The owners whose clauses a call reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owners {
    /// The clauses of every owner but these.
    AllBut(BTreeSet<String>),
    /// The clauses of these owners alone.
    Only(BTreeSet<String>),
}

/// What `expr`, an expression over the rows of `schema` in an unbound plan, reads of the states of
/// selections: a read for each `selected` call and each `in_keys` call of a pre-aggregation's
/// rollup whose state is a placeholder.
///
/// - `selected` reads the clauses of every owner but those that it skips: the owners of
///   `without`, and the owners of `exclude_own` unless it is faceted. With `empty => 'none'`, it
///   also reads whether the selection has clauses, so it reads every owner.
/// - `in_keys($s, owner, keys)` reads the clauses of `owner` alone.
pub fn selection_reads(expr: &Expr, schema: &DFSchema) -> Result<Vec<SelectionRead>> {
    let mut reads = Vec::new();
    expr.apply(|expr| {
        let Expr::ScalarFunction(ScalarFunction { func, args }) = expr else {
            return Ok(TreeNodeRecursion::Continue);
        };
        let Some(Expr::Placeholder(Placeholder { id, .. })) = args.first() else {
            return Ok(TreeNodeRecursion::Continue);
        };
        if is_selected(func) {
            let options = Options::parse(&fold_options(args, schema)?)?.selected;
            let mut skipped: BTreeSet<String> = options.without.iter().cloned().collect();
            if !options.faceted {
                skipped.extend(options.exclude_own.iter().cloned());
            }
            if options.empty_selects_none {
                skipped.clear();
            }
            reads.push(SelectionRead {
                parameter: id.clone(),
                owners: Owners::AllBut(skipped),
            });
        } else if let (Some(_), [_, Expr::Literal(owner, _), _]) = (marker(func), args.as_slice()) {
            let owner = owner.try_as_str().flatten().unwrap_or_default().to_string();
            reads.push(SelectionRead {
                parameter: id.clone(),
                owners: Owners::Only(BTreeSet::from([owner])),
            });
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(reads)
}
