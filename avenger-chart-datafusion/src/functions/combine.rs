//! The scalar compositions `layer`, `hconcat`, and `vconcat`, which compose the charts of their
//! arguments, one composed chart for each row: [`combine`], which a library's declarations of the
//! functions call, and [`Combine`], the function of one composition in the registry of a session
//! that runs a host's plans.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, BinaryViewBuilder},
    datatypes::{DataType, FieldRef},
};
use avenger_chart::{
    chart_field, charts_from_arrow,
    functions::{compositions, Arg, Child, ChildOrder, Composition, CompositionKind},
    Decode,
};
use datafusion_common::{exec_datafusion_err, plan_err, Result};
use datafusion_expr::{
    ColumnarValue, ReturnFieldArgs, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature,
    Volatility,
};

/// The charts of each row of `values`, the chart arguments of a call of the scalar composition
/// `spec`, composed with its `options`, as a column of `rows` charts.
pub fn combine(
    spec: &Composition,
    options: &[Option<Arg>],
    values: impl IntoIterator<Item = ColumnarValue>,
    rows: usize,
) -> Result<ColumnarValue> {
    let name = spec.name;
    let mut charts = Vec::new();
    for value in values {
        let array: ArrayRef = value.into_array(rows)?;
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
        let chart = spec
            .compose(
                None,
                children,
                ChildOrder::AsGiven,
                options,
                super::FORMAT_SYNTAXES,
            )
            .map_err(|error| exec_datafusion_err!("{name}(): {error}"))?;
        let bytes = chart
            .to_bytes()
            .map_err(|error| exec_datafusion_err!("{name}(): {error}"))?;
        builder.append_value(bytes);
    }
    Ok(ColumnarValue::Array(Arc::new(builder.finish())))
}

/// The function of the scalar composition `spec` in the registry of a session that runs a host's
/// plans, which takes its options, one for each parameter, then its charts, as the library's
/// declaration of it does.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Combine {
    name: &'static str,
    signature: Signature,
}

impl Combine {
    /// The functions of the scalar compositions: `layer`, `hconcat`, and `vconcat`.
    pub fn all() -> Vec<Arc<ScalarUDF>> {
        (compositions().iter())
            .filter(|spec| {
                matches!(
                    spec.kind,
                    CompositionKind::Layer | CompositionKind::HConcat | CompositionKind::VConcat
                )
            })
            .map(|spec| {
                Arc::new(ScalarUDF::from(Combine {
                    name: spec.name,
                    signature: Signature::user_defined(Volatility::Immutable),
                }))
            })
            .collect()
    }

    fn spec(&self) -> &'static Composition {
        avenger_chart::functions::composition(self.name).expect("a composition of the table")
    }
}

impl ScalarUDFImpl for Combine {
    fn name(&self) -> &str {
        self.name
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

    fn return_field_from_args(&self, _args: ReturnFieldArgs) -> Result<FieldRef> {
        Ok(Arc::new(chart_field(self.name, true)))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let spec = self.spec();
        let rows = args.number_rows;
        let mut values = args.args.into_iter();
        let mut options = Vec::with_capacity(spec.params.len());
        for value in values.by_ref().take(spec.params.len()) {
            match value {
                ColumnarValue::Scalar(value) => options.push(super::arg(&value)?),
                ColumnarValue::Array(_) => {
                    return plan_err!("{}() takes constants as its options", self.name)
                }
            }
        }
        combine(spec, &options, values, rows)
    }
}
