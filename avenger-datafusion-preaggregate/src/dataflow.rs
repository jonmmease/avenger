//! Install prepared query families in dataflow and bind their predicate inputs.
//!
//! Requires the `dataflow` feature. The adapter owns handles and checked bindings.
//! Query execution, caching, and cancellation use the ordinary dataflow API.

use crate::{
    runtime::ParameterExpressions, PreparationReport, PreparedQuery, QueryDiagnostics, QueryPolicy,
};
use avenger_datafusion_dataflow::{
    DataflowBuilder, ExprInput, InputsBuilder, PlanNode, Result as FlowResult, ScopeBuilder,
    ScopedBindingsBuilder, TableOutput,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::Result,
    logical_expr::{lit, Expr, LogicalPlan},
};
use std::sync::Arc;

/// An installed query with direct and optional materialization/rollup outputs.
/// Clones share its preparation and graph handles, not a separate cache.
#[derive(Clone, Debug)]
pub struct Query {
    prepared: Arc<PreparedQuery>,
    source: ExprInput,
    direct: TableOutput,
    optimized: Option<Optimized>,
}

#[derive(Clone, Debug)]
struct Optimized {
    retained: ExprInput,
    materialization: TableOutput,
    rollup: TableOutput,
}

impl Query {
    /// Register a prepared query in a root or additional dataflow.
    ///
    /// Uses `{name}_source`, `{name}_retained`, `{name}_direct`, `{name}_states`,
    /// and `{name}_rollup`. Ineligible preparations register only source/direct.
    /// Existing query filters remain intact. No source is evaluated.
    /// Discard the builder on failure because some names may be registered.
    pub fn install(
        builder: &mut DataflowBuilder,
        name: impl Into<String>,
        prepared: PreparedQuery,
    ) -> FlowResult<Self> {
        Self::register(builder, &name.into(), prepared)
    }

    /// Register the query in the current scope, using ordinary capture rules.
    /// The naming and failure contracts match [`Self::install`].
    pub fn install_scoped(
        builder: &mut ScopeBuilder<'_>,
        name: impl Into<String>,
        prepared: PreparedQuery,
    ) -> FlowResult<Self> {
        Self::register(builder, &name.into(), prepared)
    }

    fn register(
        builder: &mut impl Register,
        name: &str,
        prepared: PreparedQuery,
    ) -> FlowResult<Self> {
        let source = builder.predicate(format!("{name}_source"))?;
        let retained = prepared
            .materialization_plan()
            .map(|_| builder.predicate(format!("{name}_retained")))
            .transpose()?;
        let templates = prepared.parameterize(ParameterExpressions {
            source: source.expr_ref(),
            retained: retained
                .as_ref()
                .map_or_else(|| lit(true), ExprInput::expr_ref),
        })?;
        let (_, direct) = builder.output(format!("{name}_direct"), templates.direct)?;
        let optimized = match templates.preaggregated {
            Some(plans) => {
                let (states, materialization) =
                    builder.output(format!("{name}_states"), plans.materialization)?;
                let (_, rollup) = builder.output(
                    format!("{name}_rollup"),
                    plans.rollup.with_materialization(states.plan_ref())?,
                )?;
                Some(Optimized {
                    retained: retained.expect("eligible preparation has a retained input"),
                    materialization,
                    rollup,
                })
            }
            None => None,
        };
        Ok(Self {
            prepared: Arc::new(prepared),
            source,
            direct,
            optimized,
        })
    }

    /// Validate a changing predicate and automatically select direct or rollup.
    pub fn bind(&self, changing: Expr) -> Result<Binding> {
        self.bind_with_policy(changing, QueryPolicy::Auto)
    }

    /// Bind with an explicit strategy policy. Invalid predicates remain errors.
    /// This constructs values and handles, not new concrete logical plans.
    pub fn bind_with_policy(&self, changing: Expr, policy: QueryPolicy) -> Result<Binding> {
        let (diagnostics, values) = self.prepared.bind_predicates(changing, policy)?;
        values.check_owner(self.prepared.id)?;
        let materialization = values.retained.as_ref().and(self.optimized.as_ref());
        Ok(Binding {
            output: materialization.map_or(self.direct, |o| o.rollup),
            materialization: materialization.map(|o| o.materialization),
            source: (self.source.clone(), values.source),
            retained: self.optimized.as_ref().map(|o| {
                (
                    o.retained.clone(),
                    values.retained.unwrap_or_else(|| lit(true)),
                )
            }),
            diagnostics,
        })
    }

    /// Return preparation-level warm-up availability, before a predicate exists.
    /// For policy-aware availability, use [`Binding::materialization_output`].
    pub fn materialization_output(&self) -> Option<TableOutput> {
        self.optimized.as_ref().map(|o| o.materialization)
    }

    /// Inspect planner eligibility, stored dimensions, and aggregate states.
    pub fn explain(&self) -> &PreparationReport {
        self.prepared.explain()
    }
}

/// Checked values and selected output for one installed query's predicate.
/// Apply to the owning graph's inputs, then request [`Self::output`].
#[derive(Clone, Debug)]
pub struct Binding {
    source: (ExprInput, Expr),
    retained: Option<(ExprInput, Expr)>,
    output: TableOutput,
    materialization: Option<TableOutput>,
    diagnostics: QueryDiagnostics,
}

impl Binding {
    /// Set this query's root inputs and preserve all other bindings.
    /// Direct bindings reset any unused retained predicate to true.
    /// Apply a `query.bind(lit(true))?` binding to initialize inactive queries.
    pub fn apply(&self, inputs: InputsBuilder) -> FlowResult<InputsBuilder> {
        let inputs = inputs.expr(&self.source.0, self.source.1.clone())?;
        match &self.retained {
            Some((input, value)) => inputs.expr(input, value.clone()),
            None => Ok(inputs),
        }
    }

    /// Set scoped inputs inside a `scope_defaults` or `at` closure.
    /// Dataflow validates the defining scope and graph ownership.
    pub fn apply_scoped(&self, inputs: ScopedBindingsBuilder) -> FlowResult<ScopedBindingsBuilder> {
        let inputs = inputs.expr(&self.source.0, self.source.1.clone())?;
        match &self.retained {
            Some((input, value)) => inputs.expr(input, value.clone()),
            None => Ok(inputs),
        }
    }

    /// Return the direct or rollup output selected for this binding.
    pub fn output(&self) -> TableOutput {
        self.output
    }

    /// Return a warm-up output only when this binding uses preaggregation.
    pub fn materialization_output(&self) -> Option<TableOutput> {
        self.materialization
    }

    /// Report the selected strategy and direct fallback reason.
    /// Execution and cache statistics remain on the dataflow result report.
    pub fn diagnostics(&self) -> &QueryDiagnostics {
        &self.diagnostics
    }
}

// Root and scoped builders expose the same registration operations.
trait Register {
    fn predicate(&mut self, name: String) -> FlowResult<ExprInput>;
    fn output(&mut self, name: String, plan: LogicalPlan) -> FlowResult<(PlanNode, TableOutput)>;
}

macro_rules! register {
    ($builder:ty) => {
        impl Register for $builder {
            fn predicate(&mut self, name: String) -> FlowResult<ExprInput> {
                self.expr_input(name, DataType::Boolean)
            }
            fn output(
                &mut self,
                name: String,
                plan: LogicalPlan,
            ) -> FlowResult<(PlanNode, TableOutput)> {
                let node = self.add_plan(&name, plan)?;
                let output = self.table_output(name, &node)?;
                Ok((node, output))
            }
        }
    };
}
register!(DataflowBuilder);
register!(ScopeBuilder<'_>);
