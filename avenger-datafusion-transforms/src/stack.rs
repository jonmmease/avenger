//! `stack` is a Vega-style stack transform. `PARTITION BY` gives the stacks and `ORDER BY`
//! the order of the layers in each stack. The result is `struct<start, end>`, the bounds of
//! each layer. The one option is `offset`: `zero` (the default) stacks positive values up and
//! negative values down from zero, `normalize` stacks absolute values from 0 to 1, and
//! `center` centers each stack of absolute values on zero (d3's silhouette offset). A NULL or
//! NaN value has no layer: its result is NULL, and it adds nothing to its stack. [`window`] is
//! the window function of an offset.

use std::sync::Arc;

use datafusion::arrow::array::{ArrayRef, AsArray, Float64Array, StructArray};
use datafusion::arrow::buffer::NullBuffer;
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{DataType, Field, FieldRef, Float64Type};
use datafusion::common::{Result, plan_err};
use datafusion::logical_expr::function::{PartitionEvaluatorArgs, WindowUDFFieldArgs};
use datafusion::logical_expr::{
    PartitionEvaluator, Signature, Volatility, WindowUDF, WindowUDFImpl,
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

/// `stack(v, ..options) OVER (..)`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct StackWindow {
    offset: StackOffset,
    signature: Signature,
}

/// The window function `stack` with `offset`, whose argument is the value to stack, whose
/// `PARTITION BY` gives the stacks, and whose `ORDER BY` gives the order of the layers.
pub fn window(offset: StackOffset) -> Arc<WindowUDF> {
    Arc::new(WindowUDF::new_from_impl(StackWindow {
        offset,
        signature: Signature::any(1, Volatility::Immutable),
    }))
}

impl WindowUDFImpl for StackWindow {
    fn name(&self) -> &str {
        // The name tells apart the stacks of one value with different offsets, as a window node
        // names its columns by their calls.
        match self.offset {
            StackOffset::Zero => "stack",
            StackOffset::Center => "stack[offset => center]",
            StackOffset::Normalize => "stack[offset => normalize]",
        }
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn partition_evaluator(
        &self,
        _args: PartitionEvaluatorArgs,
    ) -> Result<Box<dyn PartitionEvaluator>> {
        Ok(Box::new(StackEvaluator {
            offset: self.offset,
        }))
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
struct StackEvaluator {
    offset: StackOffset,
}

impl PartitionEvaluator for StackEvaluator {
    fn evaluate_all(&mut self, values: &[ArrayRef], _num_rows: usize) -> Result<ArrayRef> {
        let v = cast(&values[0], &DataType::Float64)?;
        let v: Vec<Option<f64>> = v.as_primitive::<Float64Type>().iter().collect();
        let layers = stack_values(&v, self.offset);
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
