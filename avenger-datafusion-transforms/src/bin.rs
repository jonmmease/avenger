//! `bin` is a Vega-style bin transform: bins of a nice step, for at most `maxbins` bins. Its
//! options, [`BinOptions`], with their defaults, are `maxbins` (20), `base` (10), `divide`
//! ([5, 2]), `extent`, `span`, `step`, `steps`, `minstep` (0), `nice` (true), and `anchor`.
//! Without `extent`, the extent is that of the finite values in each window partition. The
//! result is `struct<start, end>`, the bounds of the bin of each value, or NULL for a value
//! outside the bins: NULL, NaN, an infinite value, a value outside an explicit `extent`, or a
//! value before the first bin, where an `anchor` moves the bins past the least value.
//!
//! A call is a [`BinCall`], which [`plan`] adds to its input, or among the windows of a
//! projection, the placeholder window of [`window`], which [`crate::plan_windows`] plans. The
//! options are values of the plan: one struct of [`options`], whose fields an expression gives,
//! such as a placeholder of a parameter, and which the internal function `__bin_params` reads
//! and checks when the query runs. With `extent`, the bins need no aggregate. Without it, the
//! plan reads the `MIN` and the `MAX` of the finite values of each partition, from a join of an
//! aggregate by the partition, or without a partition, a scalar subquery. [`crate::StructGroupSplit`] groups by the fields of a non-null
//! struct key, such as a bin, then makes the struct again above the aggregate.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use datafusion::arrow::array::{Array, ArrayRef, AsArray, Float64Array, StructArray};
use datafusion::arrow::buffer::NullBuffer;
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{DataType, Field, FieldRef, Fields, Float64Type};
use datafusion::common::{Column, Result, ScalarValue, plan_err};
use datafusion::functions_aggregate::expr_fn::{max as max_expr, min as min_expr};
use datafusion::logical_expr::{
    ColumnarValue, Expr, LogicalPlan, ReturnFieldArgs, ScalarFunctionArgs, ScalarUDF,
    ScalarUDFImpl, Signature, Volatility, WindowUDF, lit,
};

use crate::OptionError;
use crate::utils::{PartitionValue, interval_fields};
use crate::window::{Output, PartitionedCall, Transform, placeholder, plan_partitioned};

// ---------------------------------------------------------------------------------------
// Calls and options
// ---------------------------------------------------------------------------------------

/// A call of `bin`: the value to bin, the window partitions, and the options.
#[derive(Debug, Clone, PartialEq)]
pub struct BinCall {
    pub value: Expr,
    pub partition: Vec<Expr>,
    pub options: BinOptions,
}

/// The options of `bin`.
#[derive(Debug, Clone, PartialEq)]
pub struct BinOptions {
    pub maxbins: f64,
    pub base: f64,
    pub divide: Vec<f64>,
    pub extent: Option<[f64; 2]>,
    pub span: Option<f64>,
    pub step: Option<f64>,
    pub steps: Option<Vec<f64>>,
    pub minstep: f64,
    pub nice: bool,
    pub anchor: Option<f64>,
}

impl Default for BinOptions {
    fn default() -> Self {
        Self {
            maxbins: 20.0,
            base: 10.0,
            divide: vec![5.0, 2.0],
            extent: None,
            span: None,
            step: None,
            steps: None,
            minstep: 0.0,
            nice: true,
            anchor: None,
        }
    }
}

// Checked options hold no NaN, so the equality of their numbers is an equivalence.
impl Eq for BinOptions {}

impl Hash for BinOptions {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let bits = |v: &f64| v.to_bits();
        bits(&self.maxbins).hash(state);
        bits(&self.base).hash(state);
        self.divide.iter().map(bits).for_each(|b| b.hash(state));
        self.extent.map(|e| e.map(|v| v.to_bits())).hash(state);
        self.span.as_ref().map(bits).hash(state);
        self.step.as_ref().map(bits).hash(state);
        (self.steps.iter().flatten()).for_each(|v| v.to_bits().hash(state));
        bits(&self.minstep).hash(state);
        self.nice.hash(state);
        self.anchor.as_ref().map(bits).hash(state);
    }
}

impl BinOptions {
    /// Rejects the options with which the algorithm does not end or gives no bins.
    pub fn check(&self) -> std::result::Result<(), OptionError> {
        let positive = |name: &'static str, v: f64| match v.is_finite() && v > 0.0 {
            true => Ok(()),
            false => Err(OptionError::new(
                name,
                format!("The {name} of bin must be a positive number, not {v}"),
            )),
        };
        positive("maxbins", self.maxbins)?;
        if !(self.base.is_finite() && self.base > 1.0) {
            return Err(OptionError::new(
                "base",
                format!(
                    "The base of bin must be a number greater than 1, not {}",
                    self.base
                ),
            ));
        }
        for d in &self.divide {
            positive("divide", *d)?;
        }
        if let Some(v) = self.span {
            positive("span", v)?;
        }
        if let Some(v) = self.step {
            positive("step", v)?;
        }
        if let Some(steps) = &self.steps {
            if steps.is_empty() {
                return Err(OptionError::new(
                    "steps",
                    "The steps of bin must have at least one step",
                ));
            }
            for v in steps {
                positive("steps", *v)?;
            }
            if steps.windows(2).any(|w| w[0] >= w[1]) {
                return Err(OptionError::new(
                    "steps",
                    "The steps of bin must be in increasing order",
                ));
            }
        }
        if !(self.minstep.is_finite() && self.minstep >= 0.0) {
            return Err(OptionError::new(
                "minstep",
                format!(
                    "The minstep of bin must be a number of at least 0, not {}",
                    self.minstep
                ),
            ));
        }
        if let Some([lo, hi]) = self.extent
            && !(lo.is_finite() && hi.is_finite())
        {
            return Err(OptionError::new(
                "extent",
                format!("The extent of bin must be two finite numbers, not [{lo}, {hi}]"),
            ));
        }
        if let Some(a) = self.anchor.filter(|a| !a.is_finite()) {
            return Err(OptionError::new(
                "anchor",
                format!("The anchor of bin must be a finite number, not {a}"),
            ));
        }
        Ok(())
    }
}

impl BinOptions {
    /// The options as a constant struct of [`options_fields`].
    pub fn expr(&self) -> Expr {
        let number = |v: f64| lit(v);
        let optional = |v: Option<f64>| lit(ScalarValue::Float64(v));
        let list = |v: Option<&[f64]>| match v {
            Some(v) => {
                let values: Vec<ScalarValue> = v.iter().map(|v| ScalarValue::from(*v)).collect();
                lit(ScalarValue::List(ScalarValue::new_list_nullable(
                    &values,
                    &DataType::Float64,
                )))
            }
            None => lit(ScalarValue::Null),
        };
        let fields = [
            number(self.maxbins),
            number(self.base),
            list(Some(&self.divide)),
            list(self.extent.as_ref().map(|e| &e[..])),
            optional(self.span),
            optional(self.step),
            list(self.steps.as_deref()),
            number(self.minstep),
            lit(self.nice),
            optional(self.anchor),
        ];
        let given = (options_fields().iter().zip(fields))
            .map(|(field, value)| (field.name().clone(), value))
            .collect();
        options(given).expect("the options of bin")
    }

    /// The options of the row `i` of `array`, a struct of [`options_fields`], with the default
    /// of each NULL field, checked.
    pub fn of_struct(array: &StructArray, i: usize) -> std::result::Result<Self, OptionError> {
        let default = Self::default();
        let number = |j: usize| {
            let column = array.column(j).as_primitive::<Float64Type>();
            column.is_valid(i).then(|| column.value(i))
        };
        let list = |j: usize| {
            let column = array.column(j).as_list::<i32>();
            column.is_valid(i).then(|| {
                let values = column.value(i);
                let values = values.as_primitive::<Float64Type>();
                values
                    .iter()
                    .map(|v| v.unwrap_or(f64::NAN))
                    .collect::<Vec<f64>>()
            })
        };
        let extent = match list(3) {
            Some(extent) => match extent[..] {
                [lo, hi] => Some([lo, hi]),
                _ => {
                    return Err(OptionError::new(
                        "extent",
                        "The extent of bin takes two numbers, as `[low, high]`",
                    ));
                }
            },
            None => None,
        };
        let nice = array.column(8).as_boolean();
        let options = Self {
            maxbins: number(0).unwrap_or(default.maxbins),
            base: number(1).unwrap_or(default.base),
            divide: list(2).unwrap_or(default.divide),
            extent,
            span: number(4),
            step: number(5),
            steps: list(6),
            minstep: number(7).unwrap_or(default.minstep),
            nice: match nice.is_valid(i) {
                true => nice.value(i),
                false => default.nice,
            },
            anchor: number(9),
        };
        options.check()?;
        Ok(options)
    }
}

/// The names and the types of the options of `bin`, the fields of the struct of [`options`].
pub fn options_fields() -> Fields {
    let list = || DataType::List(Arc::new(Field::new_list_field(DataType::Float64, true)));
    Fields::from(vec![
        Field::new("maxbins", DataType::Float64, true),
        Field::new("base", DataType::Float64, true),
        Field::new("divide", list(), true),
        Field::new("extent", list(), true),
        Field::new("span", DataType::Float64, true),
        Field::new("step", DataType::Float64, true),
        Field::new("steps", list(), true),
        Field::new("minstep", DataType::Float64, true),
        Field::new("nice", DataType::Boolean, true),
        Field::new("anchor", DataType::Float64, true),
    ])
}

/// The struct of the options `given` of a call, by their names, as expressions of the plan,
/// such as literals or placeholders: each cast to the type of its field, and NULL for an option
/// that the call omits, which takes its default.
pub fn options(given: Vec<(String, Expr)>) -> Result<Expr> {
    crate::utils::options_struct("bin", &options_fields(), given)
}

/// The placeholder window of a call of `bin`, whose arguments are the value and the struct of
/// [`options`], and whose `PARTITION BY` gives the window partitions. `gives_extent` tells
/// whether the call gives an extent, which plans the bins without an aggregate.
/// [`crate::plan_windows`] plans it.
pub fn window(gives_extent: bool) -> Arc<WindowUDF> {
    placeholder("bin".to_string(), Transform::Bin { gives_extent })
}

/// The plan of `input` with the column `output`, the bin of each row by `call`, after the
/// columns of `input`.
pub fn plan(input: LogicalPlan, call: BinCall, output: Column) -> Result<LogicalPlan> {
    let BinCall {
        value,
        partition,
        options,
    } = call;
    options.check()?;
    let call = PartitionedCall::Bin {
        value,
        gives_extent: options.extent.is_some(),
        options: options.expr(),
    };
    plan_partitioned(input, &partition, vec![(call, output)])
}

/// The field `name` of the bins of the values of `input`.
pub(crate) fn result_field(name: &str, input: &Field) -> Result<FieldRef> {
    if !input.data_type().is_numeric() && !input.data_type().is_null() {
        return plan_err!("bin bins numbers, not {}", input.data_type());
    }
    // NULL, NaN, infinite values, and values outside an explicit extent have no bin.
    Ok(Arc::new(Field::new(
        name,
        DataType::Struct(interval_fields()),
        true,
    )))
}

/// The bins of `value` by `options`, the call at index `i` of its partitions.
pub(crate) fn output(value: Expr, options: Expr, gives_extent: bool, i: usize) -> Result<Output> {
    // With an extent, the bins are those of the extent of the options.
    if gives_extent {
        let none = lit(ScalarValue::Float64(None));
        let params = BIN_PARAMS.call(vec![none.clone(), none, options]);
        return Ok(Output::Row(BIN_APPLY.call(vec![value, params])));
    }
    let (lo, hi) = (format!("__bin_lo_{i}"), format!("__bin_hi_{i}"));
    let params = BIN_PARAMS.call(vec![
        Expr::Column(Column::from_name(&lo)),
        Expr::Column(Column::from_name(&hi)),
        options,
    ]);
    let extent = PartitionValue {
        aggregates: vec![
            min_expr(finite(&value)).alias(lo),
            max_expr(finite(&value)).alias(hi),
        ],
        value: params,
    };
    Ok(Output::Partition(
        extent,
        Box::new(move |params| BIN_APPLY.call(vec![value, params])),
    ))
}

// ---------------------------------------------------------------------------------------
// The bin algorithm
// ---------------------------------------------------------------------------------------

/// The bins of an extent: values in [start, end] go to bins of size `step` from `start`. As
/// a struct, it is also a sequence for the `fill_gaps` argument of `impute`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BinParams {
    pub start: f64,
    pub end: f64,
    pub step: f64,
}

/// The bins of an extent: a nice step for at most `maxbins` bins, the start and the end
/// rounded to whole steps, and both moved to the anchor. The algorithm is adapted from `bin`
/// of vega-statistics and the bin transform of vega-transforms. `log` and `pow` come from the
/// `libm` crate, so that the bins are the same on every platform.
pub fn bin_params(lo: f64, hi: f64, o: &BinOptions) -> BinParams {
    let maxb = o.maxbins;
    let base = o.base;
    let logb = libm::log(base);
    let (mut min, mut max) = (lo, hi);
    // The width of the extent, or for a single value its magnitude, or 1 for zero.
    let span = o.span.unwrap_or(if max != min {
        max - min
    } else if min != 0.0 {
        min.abs()
    } else {
        1.0
    });

    let step = if let Some(step) = o.step {
        step
    } else if let Some(steps) = &o.steps {
        let v = span / maxb;
        let i = steps.iter().take_while(|s| **s < v).count();
        steps[i.saturating_sub(1)]
    } else {
        let level = (libm::log(maxb) / logb).ceil();
        let minstep = o.minstep;
        let mut step = minstep.max(libm::pow(base, (libm::log(span) / logb).round() - level));
        while (span / step).ceil() > maxb {
            step *= base;
        }
        for d in &o.divide {
            let v = step / d;
            if v >= minstep && span / v <= maxb {
                step = v;
            }
        }
        step
    };

    let v = libm::log(step);
    let precision = if v >= 0.0 {
        0.0
    } else {
        (-v / logb).trunc() + 1.0
    };
    let eps = libm::pow(base, -precision - 1.0);
    if o.nice {
        let v = (min / step + eps).floor() * step;
        min = if min < v { v - step } else { v };
        max = (max / step).ceil() * step;
    }
    let stop = if max == min { min + step } else { max };

    // The end, rounded to whole steps from the start, and both moved to the anchor.
    let mut start = min;
    let mut stop = start + ((stop - start) / step).ceil() * step;
    if let Some(a) = o.anchor {
        let d = a - (start + step * ((a - start) / step).floor());
        start += d;
        stop += d;
    }
    BinParams {
        start,
        end: stop,
        step,
    }
}

/// A tolerance for a value on the edge of a bin, where `(v - start) / step` can fall just
/// short of a whole number.
const EPSILON: f64 = 1e-14;

/// The bin of a value: its start and its end, or None for a value outside the bins: NaN, an
/// infinite value, or a value outside an explicit extent.
pub fn apply(v: f64, p: &BinParams) -> Option<(f64, f64)> {
    let BinParams {
        start,
        end: stop,
        step,
    } = *p;
    if !(start <= v && v <= stop) {
        return None;
    }
    let v = start.max(v.min(stop - step));
    let b = start + step * (EPSILON + (v - start) / step).floor();
    // The end in the arithmetic of the start of the next bin, so that the two agree in
    // floating point.
    Some((b, start + step * (1.0 + (b - start) / step)))
}

// ---------------------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------------------

pub fn params_fields() -> Fields {
    Fields::from(vec![
        Field::new("start", DataType::Float64, false),
        Field::new("end", DataType::Float64, false),
        Field::new("step", DataType::Float64, false),
    ])
}

fn to_f64(v: &ColumnarValue, n: usize) -> Result<ArrayRef> {
    Ok(cast(&v.to_array(n)?, &DataType::Float64)?)
}

/// The bins of `xs`, with the parameters of each row (None for a null result).
fn bin_array(xs: &Float64Array, mut params: impl FnMut(usize) -> Option<BinParams>) -> ArrayRef {
    let n = xs.len();
    let mut starts = Vec::with_capacity(n);
    let mut ends = Vec::with_capacity(n);
    let mut valid = Vec::with_capacity(n);
    for i in 0..n {
        let p = if xs.is_null(i) { None } else { params(i) };
        let bin = p.and_then(|p| apply(xs.value(i), &p));
        let (s, e) = bin.unwrap_or_default();
        starts.push(s);
        ends.push(e);
        valid.push(bin.is_some());
    }
    let nulls = NullBuffer::from(valid);
    Arc::new(StructArray::new(
        interval_fields(),
        vec![
            Arc::new(Float64Array::from(starts)),
            Arc::new(Float64Array::from(ends)),
        ],
        (nulls.null_count() > 0).then_some(nulls),
    ))
}

/// `x` as a double, where it is finite, for the extent of a partition: NaN and the infinities
/// take no part, and have no bin.
fn finite(x: &Expr) -> Expr {
    let x = Expr::Cast(datafusion::logical_expr::Cast::new(
        Box::new(x.clone()),
        DataType::Float64,
    ));
    datafusion::logical_expr::when(
        x.clone()
            .gt(lit(f64::NEG_INFINITY))
            .and(x.clone().lt(lit(f64::INFINITY))),
        x,
    )
    .end()
    .expect("a CASE with one branch")
}

pub(crate) static BIN_PARAMS: LazyLock<ScalarUDF> = LazyLock::new(|| {
    ScalarUDF::new_from_impl(BinParamsUdf {
        signature: Signature::any(3, Volatility::Immutable),
    })
});

/// `__bin_params(lo, hi, options)`: internal; the bins of the extent from `lo` to `hi`, or of
/// the extent of the options, by the options, a struct of [`options_fields`], which it checks.
#[derive(Debug, PartialEq, Eq, Hash)]
struct BinParamsUdf {
    signature: Signature,
}

impl ScalarUDFImpl for BinParamsUdf {
    fn name(&self) -> &str {
        "__bin_params"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Struct(params_fields()))
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let nullable = args.arg_fields.iter().any(|f| f.is_nullable());
        Ok(Arc::new(Field::new(
            self.name(),
            DataType::Struct(params_fields()),
            nullable,
        )))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let n = args.number_rows;
        let lo = to_f64(&args.args[0], n)?;
        let hi = to_f64(&args.args[1], n)?;
        let (lo, hi) = (
            lo.as_primitive::<Float64Type>(),
            hi.as_primitive::<Float64Type>(),
        );
        // The options are a constant of the plan, one value for every row.
        let options = args.args[2].to_array(1)?;
        let options = BinOptions::of_struct(options.as_struct(), 0)?;
        let mut cols = [
            Vec::with_capacity(n),
            Vec::with_capacity(n),
            Vec::with_capacity(n),
        ];
        let mut valid = Vec::with_capacity(n);
        for i in 0..n {
            let extent = match options.extent {
                Some([lo, hi]) => Some((lo, hi)),
                None => (lo.is_valid(i) && hi.is_valid(i)).then(|| (lo.value(i), hi.value(i))),
            };
            let ok = extent.is_some();
            let p = match extent {
                Some((lo, hi)) => bin_params(lo, hi, &options),
                None => BinParams {
                    start: 0.0,
                    end: 0.0,
                    step: 0.0,
                },
            };
            cols[0].push(p.start);
            cols[1].push(p.end);
            cols[2].push(p.step);
            valid.push(ok);
        }
        let nulls = NullBuffer::from(valid);
        let [a, b, c] = cols;
        Ok(ColumnarValue::Array(Arc::new(StructArray::new(
            params_fields(),
            vec![
                Arc::new(Float64Array::from(a)) as ArrayRef,
                Arc::new(Float64Array::from(b)),
                Arc::new(Float64Array::from(c)),
            ],
            (nulls.null_count() > 0).then_some(nulls),
        ))))
    }
}

pub(crate) static BIN_APPLY: LazyLock<ScalarUDF> = LazyLock::new(|| {
    ScalarUDF::new_from_impl(BinApply {
        signature: Signature::any(2, Volatility::Immutable),
    })
});

/// `__bin_apply(x, params)`: internal; the bin of each value by the bins `params`, a result of
/// `__bin_params` or a constant.
#[derive(Debug, PartialEq, Eq, Hash)]
struct BinApply {
    signature: Signature,
}

fn params_of(p: &StructArray, i: usize) -> Option<BinParams> {
    if p.is_null(i) {
        return None;
    }
    let f = |j: usize| p.column(j).as_primitive::<Float64Type>().value(i);
    Some(BinParams {
        start: f(0),
        end: f(1),
        step: f(2),
    })
}

impl ScalarUDFImpl for BinApply {
    fn name(&self) -> &str {
        "__bin_apply"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Struct(interval_fields()))
    }
    fn return_field_from_args(&self, _args: ReturnFieldArgs) -> Result<FieldRef> {
        // NULL, NaN, infinite values, and values outside an explicit extent have no bin.
        Ok(Arc::new(Field::new(
            self.name(),
            DataType::Struct(interval_fields()),
            true,
        )))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let n = args.number_rows;
        let x = to_f64(&args.args[0], n)?;
        let x = x.as_primitive::<Float64Type>();
        let array = match &args.args[1] {
            ColumnarValue::Scalar(ScalarValue::Struct(p)) => {
                let p = params_of(p, 0);
                bin_array(x, |_| p)
            }
            ColumnarValue::Array(p) => {
                let p = p.as_struct();
                bin_array(x, |i| params_of(p, i))
            }
            other => return plan_err!("__bin_apply needs bin parameters, not {other:?}"),
        };
        Ok(ColumnarValue::Array(array))
    }
}
