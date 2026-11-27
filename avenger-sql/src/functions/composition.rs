//! The composition functions in DataFusion: an aggregate function for each composition of
//! `avenger_chart::functions`, which composes the charts of the rows of its group.
//!
//! ```sql
//! FROM penguins
//! |> AGGREGATE mark_symbol(x => bill_length_mm, y => bill_depth_mm) AS chart GROUP BY island
//! |> AGGREGATE facet_column(chart, island) AS chart
//! ```
//!
//! The registered function of a composition is a declaration, as the registered function of a
//! mark is: DataFusion's expression simplifier binds each call to an aggregate of its own, with
//! its options checked and the name of its key, so the errors of the options are planning errors.

use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use avenger_chart::{chart_field, functions::Composition, EXTENSION_NAME};
use datafusion::{
    arrow::datatypes::{DataType, Field, FieldRef},
    common::{internal_err, plan_datafusion_err, plan_err, Result},
    logical_expr::{
        expr::AggregateFunction,
        function::{AccumulatorArgs, AggregateFunctionSimplification},
        simplify::SimplifyContext,
        utils::AggregateOrderSensitivity,
        Accumulator, AggregateUDF, AggregateUDFImpl, DocSection, Documentation,
        DocumentationBuilder, Expr, ExprSchemable, Signature, Volatility,
    },
};

use avenger_physical_graph::functions::CompositionInstance;

use super::declaration::column_name;

/// The documentation section of the compositions.
pub(super) const SECTION: DocSection = DocSection {
    include: true,
    label: "Chart Compositions",
    description: Some(
        "Functions that compose avenger charts: scalars of the charts of a row, and aggregates of \
         the charts of a group.",
    ),
};

/// The registered function of a composition: its signature, its parameter names, and its
/// documentation. Its `simplify` hook binds each call to a [`CompositionInstance`].
pub(crate) struct CompositionDeclaration {
    spec: &'static Composition,
    signature: Signature,
    documentation: Documentation,
}

impl CompositionDeclaration {
    pub(crate) fn new(spec: &'static Composition) -> Self {
        let names = names(spec);
        let signature = Signature::user_defined(Volatility::Immutable)
            .with_parameter_names(names.clone())
            .expect("the parameter names of a composition are distinct");
        let syntax: Vec<String> = (names.iter().skip(leading(spec).len()))
            .map(|name| format!("{name} => value"))
            .collect();
        let syntax = format!(
            "{}({}{}{} [ORDER BY ...])",
            aggregate_name(spec),
            leading(spec).join(", "),
            if syntax.is_empty() { "" } else { ", " },
            syntax.join(", ")
        );
        let mut documentation = DocumentationBuilder::new_with_details(SECTION, spec.doc, syntax)
            .with_argument(
                "chart",
                "A column of charts, one for each row of the group.",
            );
        if spec.kind.takes_key() {
            documentation = documentation.with_argument(
                "key",
                "The key of each chart, whose values the facet lays out and labels.",
            );
        }
        for param in spec.params {
            documentation = documentation.with_argument(param.name, param.doc);
        }
        Self {
            spec,
            signature,
            documentation: documentation.build(),
        }
    }
}

/// Whether `field` is a column of charts: its extension is a chart's.
pub(super) fn is_chart(field: &Field) -> bool {
    field
        .metadata()
        .get("ARROW:extension:name")
        .map(String::as_str)
        == Some(EXTENSION_NAME)
}

pub(crate) use avenger_physical_graph::functions::aggregate_name;

/// The arguments of a composition before its options: the chart, and the key of a facet.
fn leading(spec: &Composition) -> &'static [&'static str] {
    match spec.kind.takes_key() {
        true => &["chart", "key"],
        false => &["chart"],
    }
}

/// The parameter names of a composition.
fn names(spec: &Composition) -> Vec<&'static str> {
    (leading(spec).iter().copied())
        .chain(spec.params.iter().map(|param| param.name))
        .collect()
}

impl std::fmt::Debug for CompositionDeclaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositionDeclaration")
            .field("name", &self.spec.name)
            .finish_non_exhaustive()
    }
}

impl PartialEq for CompositionDeclaration {
    fn eq(&self, other: &Self) -> bool {
        self.spec.name == other.spec.name
    }
}

impl Eq for CompositionDeclaration {}

impl Hash for CompositionDeclaration {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.spec.name.hash(state);
    }
}

impl AggregateUDFImpl for CompositionDeclaration {
    fn name(&self) -> &str {
        aggregate_name(self.spec)
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, arg_types: &[DataType]) -> Result<Vec<DataType>> {
        let (name, count) = (aggregate_name(self.spec), names(self.spec).len());
        if arg_types.len() > count {
            return plan_err!(
                "{name}() takes {count} arguments at most, not {}",
                arg_types.len()
            );
        }
        Ok(arg_types.to_vec())
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::BinaryView)
    }

    fn return_field(&self, _arg_fields: &[FieldRef]) -> Result<FieldRef> {
        Ok(Arc::new(chart_field(aggregate_name(self.spec), true)))
    }

    fn accumulator(&self, _acc_args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        internal_err!(
            "{}() runs only after the simplifier binds its call, which a session that skips \
             failed optimizer rules can leave unbound",
            aggregate_name(self.spec)
        )
    }

    fn order_sensitivity(&self) -> AggregateOrderSensitivity {
        AggregateOrderSensitivity::Beneficial
    }

    fn simplify(&self) -> Option<AggregateFunctionSimplification> {
        let spec = self.spec;
        Some(Box::new(move |call, context| bind(spec, call, context)))
    }

    fn documentation(&self) -> Option<&Documentation> {
        Some(&self.documentation)
    }
}

/// The call `call` of the composition `spec`, bound to an instance with its checked options and
/// the name of its key.
fn bind(
    spec: &'static Composition,
    call: AggregateFunction,
    context: &SimplifyContext,
) -> Result<Expr> {
    let name = aggregate_name(spec);
    let params = call.params;
    if params.distinct {
        return plan_err!("{name}() takes no DISTINCT");
    }
    let schema = context.schema();
    let mut args = params.args.into_iter();
    let Some(chart) = args.next() else {
        return plan_err!("{name}() needs a column of charts");
    };
    let (_, field) = chart.to_field(schema)?;
    if !is_chart(&field) {
        return plan_err!(
            "{name}() takes a column of charts, and `{}` is not one",
            column_name(&chart)
        );
    }
    let mut data = vec![chart];
    let mut column = None;
    if spec.kind.takes_key() {
        let Some(key) = args
            .next()
            .filter(|key| !matches!(key, Expr::Literal(v, _) if v.is_null()))
        else {
            return plan_err!("{name}() needs the key of its facet");
        };
        column = Some(column_name(&key));
        data.push(key);
    }
    let mut options = Vec::with_capacity(spec.params.len());
    let mut checked = Vec::with_capacity(spec.params.len());
    for (param, expr) in spec.params.iter().zip(args) {
        let value = match expr {
            Expr::Literal(value, _) => value,
            other => {
                return plan_err!(
                    "{name}() needs a plan-time constant for `{}`, not `{}`",
                    param.name,
                    other.human_display()
                )
            }
        };
        checked.push(super::arg(&value)?);
        options.push((!value.is_null()).then_some(value));
    }
    spec.check(column.as_deref(), &checked)
        .map_err(|error| plan_datafusion_err!("{name}(): {error}"))?;
    let instance = CompositionInstance::new(spec, column, options, &params.order_by);
    Ok(Expr::AggregateFunction(AggregateFunction::new_udf(
        Arc::new(AggregateUDF::new_from_impl(instance)),
        data,
        false,
        params.filter,
        params.order_by,
        None,
    )))
}
