//! The scalar compositions in DataFusion: `layer`, `hconcat`, and `vconcat`, which compose the
//! charts of their arguments, one composed chart for each row.
//!
//! ```sql
//! FROM penguins
//! |> AGGREGATE layer(
//!      mark_symbol(x => bill_length_mm, y => bill_depth_mm),
//!      mark_symbol(x => flipper_length_mm, y => body_mass_g, size => 20),
//!    ) AS chart
//!    GROUP BY island
//! ```
//!
//! A scalar composes the charts of a row, and the aggregates `layer_agg`, `hconcat_agg`, and
//! `vconcat_agg` compose the charts of a group. A call's children are its charts in the order of
//! its arguments, and a `NULL` chart is no child, as for the aggregates. A call takes one chart or
//! more. The library passes the options of the composition first, one for each parameter, with
//! `NULL` for an omitted one, then the charts, so a call takes any number of charts.

use std::sync::Arc;

use avenger_chart::{
    chart_field,
    functions::{Arg, Composition},
};
use datafusion::{
    arrow::datatypes::{DataType, FieldRef},
    common::{plan_datafusion_err, plan_err, Result, ScalarValue},
    logical_expr::{
        simplify::{ExprSimplifyResult, SimplifyContext},
        ColumnarValue, Documentation, DocumentationBuilder, Expr, ReturnFieldArgs,
        ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::composition::{is_chart, SECTION};

/// The scalar function of a layer or a concat.
#[derive(Debug)]
pub(crate) struct CombineFunction {
    spec: &'static Composition,
    signature: Signature,
    documentation: Documentation,
}

impl CombineFunction {
    pub(crate) fn new(spec: &'static Composition) -> Self {
        let options: Vec<String> = (spec.params.iter())
            .map(|param| format!("{} => value", param.name))
            .collect();
        let syntax = format!(
            "{}(chart, ...{}{})",
            spec.name,
            if options.is_empty() { "" } else { ", " },
            options.join(", ")
        );
        let mut documentation = DocumentationBuilder::new_with_details(SECTION, spec.doc, syntax)
            .with_argument(
                "chart",
                "The charts to compose, in the order of their composition. A `NULL` chart is no \
                 child.",
            );
        for param in spec.params {
            documentation = documentation.with_argument(param.name, param.doc);
        }
        Self {
            spec,
            signature: Signature::user_defined(Volatility::Immutable),
            documentation: documentation.build(),
        }
    }

    /// The number of the options of a call, which come before its charts.
    fn options(&self) -> usize {
        self.spec.params.len()
    }

    /// The options of a call, from the constants of its first arguments, or the error of an
    /// option that is not a constant.
    fn arguments(&self, values: &[Option<&ScalarValue>]) -> Result<Vec<Option<Arg>>> {
        let name = self.spec.name;
        let mut options = Vec::with_capacity(self.options());
        for (param, value) in self.spec.params.iter().zip(values) {
            let Some(value) = value else {
                return plan_err!("{name}() needs a plan-time constant for `{}`", param.name);
            };
            options.push(super::arg(value)?);
        }
        Ok(options)
    }

    /// Checks that each option is a constant that the composition accepts.
    fn check(&self, values: &[Option<&ScalarValue>]) -> Result<()> {
        let name = self.spec.name;
        let options = self.arguments(values)?;
        (self.spec.check(None, &options)).map_err(|error| plan_datafusion_err!("{name}(): {error}"))
    }
}

impl PartialEq for CombineFunction {
    fn eq(&self, other: &Self) -> bool {
        self.spec.name == other.spec.name
    }
}

impl Eq for CombineFunction {}

impl std::hash::Hash for CombineFunction {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.spec.name.hash(state);
    }
}

impl ScalarUDFImpl for CombineFunction {
    fn name(&self) -> &str {
        self.spec.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, arg_types: &[DataType]) -> Result<Vec<DataType>> {
        Ok(arg_types.to_vec())
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::BinaryView)
    }

    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let name = self.spec.name;
        let charts = &args.arg_fields[self.options().min(args.arg_fields.len())..];
        if charts.is_empty() {
            return plan_err!("{name}() takes at least one chart");
        }
        for (index, field) in charts.iter().enumerate() {
            if !is_chart(field) && field.data_type() != &DataType::Null {
                return plan_err!(
                    "{name}() composes charts, and its argument {} is not a chart",
                    index + 1
                );
            }
        }
        // An option that is a call of constants, such as a constructor, folds later, so the
        // options check here when they are all constants, and in `simplify` after folding.
        let values: Vec<Option<&ScalarValue>> = (args.scalar_arguments.iter().take(self.options()))
            .copied()
            .collect();
        if values.iter().all(Option::is_some) {
            self.check(&values)?;
        }
        Ok(Arc::new(chart_field(name, true)))
    }

    fn simplify(&self, args: Vec<Expr>, _info: &SimplifyContext) -> Result<ExprSimplifyResult> {
        let values: Vec<Option<&ScalarValue>> = (args.iter().take(self.options()))
            .map(|arg| match arg {
                Expr::Literal(value, _) => Some(value),
                _ => None,
            })
            .collect();
        self.check(&values)?;
        Ok(ExprSimplifyResult::Original(args))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let name = self.spec.name;
        let rows = args.number_rows;
        let mut values = args.args.into_iter();
        let mut options = Vec::with_capacity(self.options());
        for value in values.by_ref().take(self.options()) {
            match value {
                ColumnarValue::Scalar(value) => options.push(value),
                ColumnarValue::Array(_) => {
                    return plan_err!("{name}() takes plan-time constants as its options")
                }
            }
        }
        let options = self.arguments(&options.iter().map(Some).collect::<Vec<_>>())?;
        avenger_physical_graph::functions::combine(self.spec, &options, values, rows)
    }

    fn documentation(&self) -> Option<&Documentation> {
        Some(&self.documentation)
    }
}
