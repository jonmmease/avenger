use avenger_datafusion_dataflow::{DataflowBuilder, ExprInput, InputsBuilder, TableOutput};
use datafusion::{arrow::datatypes::DataType, logical_expr::Expr};

use crate::{QueryDiagnostics, QueryFamily, QueryPolicy, Result, SelectionSet};

/// A family installed once into a root dataflow or prepared extension.
/// Result retention, execution sharing, and cancellation belong to dataflow.
#[derive(Clone, Debug)]
pub struct InstalledSelectionQuery {
    family: QueryFamily,
    full_predicate: ExprInput,
    direct_output: TableOutput,
}
impl QueryFamily {
    /// Install a parameterized direct output into the caller's builder.
    ///
    /// Generated names are `{name}__selection_full` for the input,
    /// `{name}__direct` for the plan, and `name` for the output. Collisions and
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
        Ok(InstalledSelectionQuery {
            family: self.clone(),
            full_predicate,
            direct_output,
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
        Ok(SelectionQueryBinding {
            input: self.full_predicate.clone(),
            predicate: self.family.query.predicate(selections)?,
            output: self.direct_output,
            diagnostics: QueryFamily::diagnostics(policy),
        })
    }
}

/// Resolved inputs and output choice for one immutable selection snapshot.
/// The binding owns its predicate and can outlive subsequent chart-state updates.
#[derive(Clone, Debug)]
pub struct SelectionQueryBinding {
    input: ExprInput,
    predicate: Expr,
    output: TableOutput,
    diagnostics: QueryDiagnostics,
}
impl SelectionQueryBinding {
    /// Bind all inputs owned by this installation, preserving other inputs.
    /// The dataflow validates the predicate at every usage site. A builder from
    /// another graph is an error. The caller still supplies unrelated inputs.
    pub fn apply(&self, inputs: InputsBuilder) -> Result<InputsBuilder> {
        Ok(inputs.expr(&self.input, self.predicate.clone())?)
    }
    /// Return the requested table output, independent of execution strategy.
    pub fn output(&self) -> TableOutput {
        self.output
    }
    /// Return the compatible materialization for optional warm-up.
    /// Direct execution has no materialization and always returns None.
    pub fn preaggregate_output(&self) -> Option<TableOutput> {
        None
    }
    /// Explain the strategy chosen for this request.
    pub fn explain(&self) -> QueryDiagnostics {
        self.diagnostics
    }
}
