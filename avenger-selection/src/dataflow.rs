use avenger_datafusion_dataflow::{DataflowBuilder, ExprInput, InputsBuilder, TableOutput};
use datafusion::{
    arrow::datatypes::DataType,
    logical_expr::{lit, Expr},
};

use crate::{QueryDiagnostics, QueryFamily, QueryPolicy, Result, SelectionSet};

/// A family installed once into a root dataflow or prepared extension.
/// Result retention, execution sharing, and cancellation belong to dataflow.
#[derive(Clone, Debug)]
pub struct InstalledSelectionQuery {
    family: QueryFamily,
    full_predicate: ExprInput,
    direct_output: TableOutput,
    preaggregation: Option<InstalledPreaggregation>,
}

#[derive(Clone, Debug)]
struct InstalledPreaggregation {
    fixed: ExprInput,
    changing: ExprInput,
    materialization: TableOutput,
    output: TableOutput,
}
impl QueryFamily {
    /// Install direct and, when supported, pre-aggregated outputs into the builder.
    ///
    /// Generated names are `{name}__selection_full` for the input,
    /// `{name}__direct` for the plan, and `name` for the output.
    /// Supported families also own `__selection_fixed`, `__selection_changing`,
    /// `__materialization`, and `__aggregate` names. Collisions and
    /// invalid graph references are errors. Discard the builder after an error
    /// because registration can have partially succeeded. No data is read.
    pub fn install(
        &self,
        builder: &mut DataflowBuilder,
        name: impl AsRef<str>,
    ) -> Result<InstalledSelectionQuery> {
        let name = name.as_ref();
        let full_predicate =
            builder.expr_input(format!("{name}__selection_full"), DataType::Boolean)?;
        let direct = builder.add_plan(
            format!("{name}__direct"),
            self.query.with_predicate(full_predicate.expr_ref())?,
        )?;
        let direct_output = builder.table_output(name, &direct)?;
        let preaggregation = match &self.preaggregation {
            Ok(prepared) => {
                let fixed =
                    builder.expr_input(format!("{name}__selection_fixed"), DataType::Boolean)?;
                let changing =
                    builder.expr_input(format!("{name}__selection_changing"), DataType::Boolean)?;
                let materialized = builder.add_plan(
                    format!("{name}__materialization"),
                    prepared.materialization(fixed.expr_ref())?,
                )?;
                let materialization =
                    builder.table_output(format!("{name}__materialization"), &materialized)?;
                let aggregate = builder.add_plan(
                    format!("{name}__aggregate"),
                    prepared
                        .aggregate(changing.expr_ref())?
                        .over(materialized.plan_ref())?,
                )?;
                let output = builder.table_output(format!("{name}__aggregate"), &aggregate)?;
                Some(InstalledPreaggregation {
                    fixed,
                    changing,
                    materialization,
                    output,
                })
            }
            Err(_) => None,
        };
        Ok(InstalledSelectionQuery {
            family: self.clone(),
            full_predicate,
            direct_output,
            preaggregation,
        })
    }
}
impl InstalledSelectionQuery {
    /// Describe the family's default strategy without executing the dataflow.
    pub fn explain(&self) -> QueryDiagnostics {
        self.family.explain()
    }
    /// Resolve a snapshot using the default policy without rebuilding plans.
    pub fn bind(&self, selections: &SelectionSet) -> Result<SelectionQueryBinding> {
        self.bind_with_policy(selections, self.family.policy())
    }
    /// Select a per-request policy on the same installed preparation.
    /// ForceDirect does not disable the runtime's ordinary result cache.
    pub fn bind_with_policy(
        &self,
        selections: &SelectionSet,
        policy: QueryPolicy,
    ) -> Result<SelectionQueryBinding> {
        let full = self.family.query.predicate(selections)?;
        let split = self.family.split(selections, policy)?;
        let mut predicates = vec![(self.full_predicate.clone(), full)];
        let (output, materialization, diagnostics) = match (&self.preaggregation, split) {
            (Some(prepared), Ok(split)) => {
                predicates.push((prepared.fixed.clone(), split.fixed));
                predicates.push((prepared.changing.clone(), split.changing));
                (
                    prepared.output,
                    Some(prepared.materialization),
                    QueryDiagnostics::preaggregated(),
                )
            }
            (prepared, Err(reason)) => {
                if let Some(prepared) = prepared {
                    // Dataflow requires every declared input, even on unused branches.
                    predicates.push((prepared.fixed.clone(), lit(true)));
                    predicates.push((prepared.changing.clone(), lit(true)));
                }
                (self.direct_output, None, QueryDiagnostics::direct(reason))
            }
            (None, Ok(_)) => unreachable!("eligible family was installed without its template"),
        };
        Ok(SelectionQueryBinding {
            predicates,
            output,
            materialization,
            diagnostics,
        })
    }
}

/// Resolved inputs and output choice for one immutable selection snapshot.
/// The binding owns its predicate and can outlive subsequent chart-state updates.
#[derive(Clone, Debug)]
pub struct SelectionQueryBinding {
    predicates: Vec<(ExprInput, Expr)>,
    output: TableOutput,
    materialization: Option<TableOutput>,
    diagnostics: QueryDiagnostics,
}
impl SelectionQueryBinding {
    /// Bind all inputs owned by this installation, preserving other inputs.
    /// The dataflow validates the predicate at every usage site. A builder from
    /// another graph is an error. The caller still supplies unrelated inputs.
    pub fn apply(&self, mut inputs: InputsBuilder) -> Result<InputsBuilder> {
        for (input, predicate) in &self.predicates {
            inputs = inputs.expr(input, predicate.clone())?;
        }
        Ok(inputs)
    }
    /// Return the requested table output, independent of execution strategy.
    pub fn output(&self) -> TableOutput {
        self.output
    }
    /// Return the compatible materialization for optional warm-up.
    /// Direct execution has no materialization and always returns None.
    pub fn preaggregate_output(&self) -> Option<TableOutput> {
        self.materialization
    }
    /// Explain the strategy chosen for this request.
    pub fn explain(&self) -> QueryDiagnostics {
        self.diagnostics
    }
}
