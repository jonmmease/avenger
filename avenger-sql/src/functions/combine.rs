//! The scalar compositions in DataFusion: `layer`, `hconcat`, and `vconcat`, which compose the
//! charts of their arguments, one composed chart for each row.
//!
//! ```sql
//! FROM penguins
//! |> AGGREGATE bar(x => species, y => avg(body_mass_g)) AS bars, rule(y => avg(body_mass_g)) AS mean
//!    GROUP BY island
//! |> SELECT island, layer(bars, mean) AS chart
//! ```
//!
//! A scalar composes the charts of a row, and the aggregates `layer_agg`, `hconcat_agg`, and
//! `vconcat_agg` compose the charts of a group. A call takes up to [`CHARTS`] charts, its
//! children in the order of the arguments, then the options of its composition by name. The
//! syntax pass passes `NULL` for each chart that a call with options omits, and a `NULL` chart is
//! no child, as for the aggregates.

use std::sync::Arc;

use avenger_chart::{
    chart_field, charts_from_arrow,
    functions::{Arg, Child, ChildOrder, Composition},
    Decode, EXTENSION_NAME,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, BinaryViewBuilder},
        datatypes::{DataType, FieldRef},
    },
    common::{exec_datafusion_err, plan_datafusion_err, plan_err, Result, ScalarValue},
    logical_expr::{
        simplify::{ExprSimplifyResult, SimplifyContext},
        ColumnarValue, Documentation, DocumentationBuilder, Expr, ReturnFieldArgs,
        ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::composition::SECTION;

/// The number of charts that a scalar composition takes at most.
pub(crate) const CHARTS: usize = 16;

/// The scalar function of a layer or a concat.
#[derive(Debug)]
pub(crate) struct CombineFunction {
    spec: &'static Composition,
    signature: Signature,
    documentation: Documentation,
}

impl CombineFunction {
    pub(crate) fn new(spec: &'static Composition) -> Self {
        let charts: Vec<String> = (1..=CHARTS).map(|n| format!("chart_{n}")).collect();
        let names: Vec<String> = (charts.iter().cloned())
            .chain(spec.params.iter().map(|param| param.name.to_string()))
            .collect();
        let signature = Signature::user_defined(Volatility::Immutable)
            .with_parameter_names(names)
            .expect("the parameter names of a composition are distinct");
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
                format!(
                    "The charts to compose, up to {CHARTS}, in the order of their composition. A \
                     `NULL` chart is no child."
                ),
            );
        for param in spec.params {
            documentation = documentation.with_argument(param.name, param.doc);
        }
        Self {
            spec,
            signature,
            documentation: documentation.build(),
        }
    }

    /// The options of a call, from the constants of its arguments after the charts.
    fn options(&self, values: &[Option<&ScalarValue>]) -> Result<Vec<Option<Arg>>> {
        let name = self.spec.name;
        let mut options = Vec::with_capacity(self.spec.params.len());
        for (param, value) in self.spec.params.iter().zip(values) {
            let Some(value) = value else {
                return plan_err!("{name}() needs a plan-time constant for `{}`", param.name);
            };
            options.push(super::arg(value)?);
        }
        Ok(options)
    }
}

impl CombineFunction {
    /// Each option is a plan-time constant that the composition accepts.
    fn check(&self, values: &[Option<&ScalarValue>]) -> Result<()> {
        let name = self.spec.name;
        let options = self.options(values)?;
        self.spec
            .check(None, &options)
            .map_err(|error| plan_datafusion_err!("{name}(): {error}"))
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
        let (name, count) = (self.spec.name, CHARTS + self.spec.params.len());
        if arg_types.len() > count {
            return plan_err!(
                "{name}() takes {CHARTS} charts and {} options at most, not {} arguments",
                self.spec.params.len(),
                arg_types.len()
            );
        }
        Ok(arg_types.to_vec())
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::BinaryView)
    }

    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let name = self.spec.name;
        for (index, field) in args.arg_fields.iter().take(CHARTS).enumerate() {
            let chart = field
                .metadata()
                .get("ARROW:extension:name")
                .map(String::as_str)
                == Some(EXTENSION_NAME);
            if !chart && field.data_type() != &DataType::Null {
                return plan_err!(
                    "{name}() composes charts, and its argument {} is not a chart",
                    index + 1
                );
            }
        }
        // An option that is a call of constants, such as a constructor, folds later, so the
        // options check here when they are all constants, and in `simplify` after folding.
        let values: Vec<Option<&ScalarValue>> = (args.scalar_arguments.iter().skip(CHARTS))
            .copied()
            .collect();
        if values.iter().all(Option::is_some) {
            self.check(&values)?;
        }
        Ok(Arc::new(chart_field(name, true)))
    }

    fn simplify(&self, args: Vec<Expr>, _info: &SimplifyContext) -> Result<ExprSimplifyResult> {
        let values: Vec<Option<&ScalarValue>> = (args.iter().skip(CHARTS))
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
        let mut arrays: Vec<ArrayRef> = Vec::new();
        let mut options = Vec::new();
        for (index, arg) in args.args.into_iter().enumerate() {
            match index < CHARTS {
                true => arrays.push(arg.into_array(rows)?),
                false => match arg {
                    ColumnarValue::Scalar(value) => options.push(value),
                    ColumnarValue::Array(_) => {
                        return plan_err!("{name}() takes plan-time constants as its options")
                    }
                },
            }
        }
        let options = self.options(&options.iter().map(Some).collect::<Vec<_>>())?;
        let mut charts = Vec::with_capacity(arrays.len());
        for array in &arrays {
            match array.data_type() {
                DataType::Null => charts.push(vec![None; rows]),
                _ => charts.push(
                    charts_from_arrow(array.as_ref(), Decode::Auto)
                        .map_err(|error| exec_datafusion_err!("{name}(): {error}"))?,
                ),
            }
        }
        let mut builder = BinaryViewBuilder::with_capacity(rows);
        for row in 0..rows {
            let children: Vec<Child> = (charts.iter())
                .map(|column| Child {
                    chart: column[row].clone(),
                    key: None,
                })
                .collect();
            let chart = (self.spec)
                .compose(None, children, ChildOrder::AsGiven, &options)
                .map_err(|error| exec_datafusion_err!("{name}(): {error}"))?;
            let bytes = chart
                .to_bytes()
                .map_err(|error| exec_datafusion_err!("{name}(): {error}"))?;
            builder.append_value(bytes);
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish())))
    }

    fn documentation(&self) -> Option<&Documentation> {
        Some(&self.documentation)
    }
}
