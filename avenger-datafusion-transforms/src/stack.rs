//! `stack` is a Vega-style stack transform. `PARTITION BY` gives the stacks and `ORDER BY`
//! the order of the layers in each stack. The result is `struct<start, end>`, the bounds of
//! each layer. The one option is `offset`, the second argument, a `STRING` of the plan, such as
//! a literal or a placeholder: `zero` (the default) stacks positive values up and negative
//! values down from zero, `normalize` stacks absolute values from 0 to 1, and `center` centers
//! each stack of absolute values on zero (d3's silhouette offset). A NULL or NaN value has no
//! layer: its result is NULL, and it adds nothing to its stack. [`window`] is the window
//! function.

use std::sync::Arc;

use datafusion::arrow::array::{Array, ArrayRef, AsArray, Float64Array, StructArray};
use datafusion::arrow::buffer::NullBuffer;
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{DataType, Field, FieldRef, Float64Type};
use datafusion::common::{Result, exec_err, plan_err};
use datafusion::logical_expr::function::{PartitionEvaluatorArgs, WindowUDFFieldArgs};
use datafusion::logical_expr::{
    PartitionEvaluator, Signature, TypeSignature, Volatility, WindowUDF, WindowUDFImpl,
};

use crate::utils::interval_fields;

// ---------------------------------------------------------------------------------------
// stack(v, ..options) OVER (PARTITION BY .. ORDER BY ..)
// ---------------------------------------------------------------------------------------

/// The stack offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StackOffset {
    /// Positive values stack up from 0, negative values down from 0.
    Zero,
    /// A stack of absolute values centered on zero.
    Center,
    /// A stack of absolute values from 0 to 1.
    Normalize,
}

impl StackOffset {
    /// The offset of the name `zero`, `center`, or `normalize`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "zero" => Some(Self::Zero),
            "center" => Some(Self::Center),
            "normalize" => Some(Self::Normalize),
            _ => None,
        }
    }

    /// The offset of the row `i` of `names`, the texts of an offset, or `zero` for NULL.
    fn of_row(names: &ArrayRef, i: usize) -> Result<Self> {
        let names = cast(names, &DataType::Utf8)?;
        let names = names.as_string::<i32>();
        if names.is_null(i) {
            return Ok(Self::Zero);
        }
        let name = names.value(i);
        match Self::from_name(name) {
            Some(offset) => Ok(offset),
            None => {
                exec_err!("The offset of stack is 'zero', 'normalize', or 'center', not '{name}'")
            }
        }
    }
}

/// The start and end of each value of one stack, in the order of its layers. A NULL or NaN
/// value has no layer: its bounds are None, and it adds nothing to the stack.
pub fn stack_values(values: &[Option<f64>], offset: StackOffset) -> Vec<Option<(f64, f64)>> {
    let value = |x: &Option<f64>| x.filter(|x| !x.is_nan());
    let total: f64 = values.iter().filter_map(value).map(f64::abs).sum();
    let (mut pos, mut neg) = (0.0, 0.0);
    let mut last = match offset {
        StackOffset::Center => -total / 2.0,
        _ => 0.0,
    };
    let mut acc = 0.0;
    values
        .iter()
        .map(|x| {
            let x = value(x)?;
            Some(match offset {
                StackOffset::Zero if x < 0.0 => {
                    let start = neg;
                    neg += x;
                    (start, neg)
                }
                StackOffset::Zero => {
                    let start = pos;
                    pos += x;
                    (start, pos)
                }
                StackOffset::Center => {
                    let start = last;
                    last += x.abs();
                    (start, last)
                }
                StackOffset::Normalize => {
                    let start = last;
                    acc += x.abs();
                    last = acc / total;
                    (start, last)
                }
            })
        })
        .collect()
}

/// `stack(v[, offset]) OVER (..)`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct StackWindow {
    signature: Signature,
}

/// The window function `stack`, whose arguments are the value to stack and the offset, which
/// the call may leave out, whose `PARTITION BY` gives the stacks, and whose `ORDER BY` gives the
/// order of the layers.
pub fn window() -> Arc<WindowUDF> {
    Arc::new(WindowUDF::new_from_impl(StackWindow {
        signature: Signature::one_of(
            vec![TypeSignature::Any(1), TypeSignature::Any(2)],
            Volatility::Immutable,
        ),
    }))
}

impl WindowUDFImpl for StackWindow {
    fn name(&self) -> &str {
        "stack"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn partition_evaluator(
        &self,
        _args: PartitionEvaluatorArgs,
    ) -> Result<Box<dyn PartitionEvaluator>> {
        Ok(Box::new(StackEvaluator))
    }
    fn field(&self, args: WindowUDFFieldArgs) -> Result<FieldRef> {
        let Some(v) = args.input_fields().first() else {
            return plan_err!("stack needs a value to stack");
        };
        if !v.data_type().is_numeric() && !v.data_type().is_null() {
            return plan_err!("stack stacks numbers, not {}", v.data_type());
        }
        // A NULL or NaN value has no layer.
        Ok(Arc::new(Field::new(
            args.name(),
            DataType::Struct(interval_fields()),
            true,
        )))
    }
}

/// Stacks a whole partition at once. `zero` could stream, since each row needs only the rows
/// before it, but the input of the window is sorted by the partition and the order, and the
/// sort holds the whole input anyway.
#[derive(Debug)]
struct StackEvaluator;

impl PartitionEvaluator for StackEvaluator {
    fn evaluate_all(&mut self, values: &[ArrayRef], num_rows: usize) -> Result<ArrayRef> {
        // The offset is a constant of the plan, one value for every row.
        let offset = match values.get(1) {
            Some(names) if num_rows > 0 => StackOffset::of_row(names, 0)?,
            _ => StackOffset::Zero,
        };
        let v = cast(&values[0], &DataType::Float64)?;
        let v: Vec<Option<f64>> = v.as_primitive::<Float64Type>().iter().collect();
        let layers = stack_values(&v, offset);
        let nulls = NullBuffer::from_iter(layers.iter().map(Option::is_some));
        let (starts, ends): (Vec<f64>, Vec<f64>) =
            layers.into_iter().map(|l| l.unwrap_or_default()).unzip();
        Ok(Arc::new(StructArray::new(
            interval_fields(),
            vec![
                Arc::new(Float64Array::from(starts)) as ArrayRef,
                Arc::new(Float64Array::from(ends)),
            ],
            (nulls.null_count() > 0).then_some(nulls),
        )))
    }
}
