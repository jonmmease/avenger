//! Deferred graph integration. Install templates once, then obtain both predicate
//! forms from the same preparation's concrete binder for each request. Ordinary
//! inputs already in the query retain their own validation and invalidation contract.
//! In particular, a trusted expression wrapper must enforce its scalar properties
//! on every supplied value before warm-up or reuse.

use crate::{expressions, markers, PreparedQuery, RollupQuery};
use datafusion::{
    common::Result,
    logical_expr::{Expr, LogicalPlan},
};

/// Caller-owned Boolean expressions used as deferred predicate parameters.
#[derive(Clone, Debug)]
pub struct ParameterExpressions {
    /// Changing predicate in source-row space.
    pub source: Expr,
    /// Validated changing predicate in stored-cell space.
    pub retained: Expr,
}
/// Persistent plans tied to one preparation identity.
#[derive(Clone, Debug)]
pub struct ParameterizedFamily {
    /// Original query using the source-space predicate parameter.
    pub direct: LogicalPlan,
    /// State and rollup templates when preparation is eligible.
    pub preaggregated: Option<PreaggregateTemplates>,
    id: u64,
}
impl ParameterizedFamily {
    /// Reject predicate bindings produced by a different preparation.
    /// Callers must also select the output indicated by the binding diagnostics.
    pub fn check_binding(&self, values: &BoundPredicates) -> Result<()> {
        if self.id != values.id {
            return datafusion::common::plan_err!(
                "predicate binding belongs to a different prepared query"
            );
        }
        Ok(())
    }
}
/// State construction and deferred finishing plans for runtime installation.
#[derive(Clone, Debug)]
pub struct PreaggregateTemplates {
    /// State construction with the query's existing input dependencies.
    pub materialization: LogicalPlan,
    /// Finishing query using the retained-space predicate parameter.
    pub rollup: RollupQuery,
}
/// Checked predicate forms with private preparation ownership.
#[derive(Clone, Debug)]
pub struct BoundPredicates {
    pub(crate) id: u64,
    pub(crate) source: Expr,
    pub(crate) retained: Option<Expr>,
}
impl BoundPredicates {
    /// Return the concrete source-row predicate, including direct-only bindings.
    pub fn source(&self) -> &Expr {
        &self.source
    }
    /// Return the checked cell predicate. Direct-only bindings return None.
    pub fn retained(&self) -> Option<&Expr> {
        self.retained.as_ref()
    }
}
impl PreparedQuery {
    /// Create deferred templates without claiming eligibility for future values.
    /// Bind those values through this preparation, verify ownership, and enforce
    /// property contracts for ordinary deferred inputs before requesting execution.
    pub fn parameterize(&self, parameters: ParameterExpressions) -> Result<ParameterizedFamily> {
        let source = expressions::boolean(parameters.source, &self.query.source_schema, false)?;
        let direct = markers::substitute(&self.query.plan, self.query.site, Some(source))?;
        let preaggregated = self
            .analyzed
            .as_ref()
            .ok()
            .map(|p| {
                let retained =
                    expressions::boolean(parameters.retained, p.materialization.schema(), false)?;
                Ok::<_, datafusion::common::DataFusionError>(PreaggregateTemplates {
                    materialization: p.materialization.clone(),
                    rollup: p.rollup(retained)?,
                })
            })
            .transpose()?;
        Ok(ParameterizedFamily {
            direct,
            preaggregated,
            id: self.id,
        })
    }
}
