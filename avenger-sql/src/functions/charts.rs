//! `chart(value)`: the charts of a `BinaryView` column, in a column of the chart extension.
//!
//! DataFusion drops the extension of a field through many functions, such as `CASE`,
//! `coalesce`, `nullif`, `greatest`, a cast, and the element of an array, so their result is a
//! `BinaryView` column whose values are charts, but whose field does not say so. `chart` gives
//! the column the field of a column of charts, which a reader of the result, such as an app,
//! takes as charts.

use std::sync::Arc;

use avenger_chart::{chart_field, charts_from_arrow, Decode};
use datafusion::{
    arrow::{
        array::new_null_array,
        datatypes::{DataType, FieldRef},
    },
    common::{exec_datafusion_err, plan_err, Result},
    logical_expr::{
        ColumnarValue, Documentation, DocumentationBuilder, ReturnFieldArgs, ScalarFunctionArgs,
        ScalarUDFImpl, Signature, Volatility,
    },
};

use super::composition::SECTION;

/// `chart(value)`, whose result is `value` in a column of the chart extension. Each value that is
/// not `NULL` decodes as a chart, or the call fails.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChartFunction {
    signature: Signature,
    documentation: Documentation,
}

impl ChartFunction {
    pub(crate) fn new() -> Self {
        let documentation = DocumentationBuilder::new_with_details(
            SECTION,
            "The charts of a `BinaryView` column, in a column of charts. DataFusion drops the \
             extension of a column of charts through functions such as `CASE` and `coalesce`, \
             and `chart` restores it, so a reader of the result takes the values as charts.",
            "chart(value)",
        )
        .with_argument(
            "value",
            "A `BinaryView` expression whose values are charts. A value that is not a chart is \
             an error.",
        )
        .build();
        Self {
            signature: Signature::any(1, Volatility::Immutable),
            documentation,
        }
    }
}

impl ScalarUDFImpl for ChartFunction {
    fn name(&self) -> &str {
        "chart"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::BinaryView)
    }

    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let field = &args.arg_fields[0];
        if !super::holds_charts(field) && field.data_type() != &DataType::Null {
            return plan_err!(
                "chart() takes a BinaryView of charts, and its argument is a {}",
                field.data_type()
            );
        }
        Ok(Arc::new(chart_field(self.name(), field.is_nullable())))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let value = args
            .args
            .into_iter()
            .next()
            .expect("chart() takes one argument");
        let array = match value.into_array(rows)? {
            array if array.data_type() == &DataType::Null => {
                new_null_array(&DataType::BinaryView, array.len())
            }
            array => {
                charts_from_arrow(array.as_ref(), Decode::Auto)
                    .map_err(|error| exec_datafusion_err!("chart(): {error}"))?;
                array
            }
        };
        Ok(ColumnarValue::Array(array))
    }

    fn documentation(&self) -> Option<&Documentation> {
        Some(&self.documentation)
    }
}
