//! Vega's data transforms as DataFusion functions: `bin`, `stack`, `timeunit`, `lump`, and
//! `impute`. [`session`] makes a session with all of them and their optimizer rules.
//!
//! ```sql
//! SELECT *, bin(x, 'maxbins', 30) OVER (PARTITION BY g) AS binned FROM t
//! SELECT *, stack(v, 'offset', 'center') OVER (PARTITION BY x ORDER BY series) AS y FROM t
//! SELECT *, timeunit(t, 'maxbins', 20) OVER (PARTITION BY g) AS u FROM events
//! SELECT *, lump(port, 'n', 3, 'by', 'sum(tons)') OVER (PARTITION BY region) AS l FROM catches
//! SELECT * FROM impute((SELECT month, region, units FROM sales), month, [region], units, value => 0)
//! ```
//!
//! The options of the window functions are pairs of a name and a constant after the first
//! argument, in any order, because DataFusion's named arguments can omit only the last
//! parameters. A function of a whole window partition (`bin`, `timeunit`, and `lump`) takes
//! no ORDER BY or frame in its OVER clause.
//!
//! # bin
//!
//! `bin` is the Vega bin transform. Its options and their defaults are Vega's: `maxbins`
//! (20), `base` (10), `divide` ([5, 2]), `extent`, `span`, `step`, `steps`, `minstep` (0),
//! `nice` (true), and `anchor`. Without `extent`, the extent is that of the finite values of
//! `x` in each window partition. The result is `struct<start, end>`, Vega's `bin0` and
//! `bin1`.
//!
//! - `bin(x, ..options) OVER (..)`: the window function that users call.
//! - `bin_params(lo, hi, ..options)`: the `start`, `end`, and `step` of the bins of an extent,
//!   for axes, scale domains, and empty bins.
//! - `__bin_apply(x, params)`: internal; bins each value by the result of `bin_params`.
//!
//! [`BinWindowRewrite`] replaces `bin(..) OVER (..)` with `__bin_apply`, which gets its
//! parameters from constants (with `extent`), from a scalar subquery (no `PARTITION BY`), or
//! from a join with the parameters of each partition. [`StructGroupSplit`] groups by the
//! fields of a non-null struct key, such as a bin, then makes the struct again above the
//! aggregate.
//!
//! # stack
//!
//! `stack` is the Vega stack transform. `PARTITION BY` gives the stacks (Vega's `groupby`)
//! and `ORDER BY` the order of the layers in each stack (Vega's `sort`). The result is
//! `struct<start, end>`, Vega's `y0` and `y1`. The one option is `offset`: `zero` (the
//! default), `center`, or `normalize`. `zero` and `normalize` are Vega's. `center` centers
//! each stack on zero (d3's silhouette offset): a stack starts at minus half its total.
//! Vega's `center` starts a stack at half the difference between the largest total of all
//! stacks and its own, which a window function cannot see; the two differ by the same
//! constant for all stacks.
//!
//! # timeunit, lump, and impute
//!
//! The modules [`timeunit`], [`lump`], and [`impute`] describe the other transforms.

pub mod impute;
pub mod lump;
pub mod timeunit;

use std::sync::Arc;

use datafusion::arrow::array::{Array, ArrayRef, AsArray, Float64Array, StructArray};
use datafusion::arrow::buffer::NullBuffer;
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{DataType, Field, FieldRef, Fields, Float64Type};
use datafusion::common::scalar::ScalarStructBuilder;
use datafusion::common::tree_node::{Transformed, TreeNode};
use datafusion::common::{
    Column, JoinType, NullEquality, Result, ScalarValue, Spans, plan_datafusion_err, plan_err,
};
use datafusion::execution::SessionStateBuilder;
use datafusion::functions::core::expr_fn::get_field;
use datafusion::functions_aggregate::expr_fn::{max as max_expr, min as min_expr};
use datafusion::logical_expr::expr::{WindowFunction, WindowFunctionParams};
use datafusion::logical_expr::function::{PartitionEvaluatorArgs, WindowUDFFieldArgs};
use datafusion::logical_expr::{
    Aggregate, ColumnarValue, Expr, ExprSchemable, LogicalPlan, LogicalPlanBuilder,
    PartitionEvaluator, ReturnFieldArgs, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature,
    Subquery, Volatility, Window, WindowFunctionDefinition, WindowUDF, WindowUDFImpl, lit,
};
use datafusion::optimizer::{ApplyOrder, Optimizer, OptimizerConfig, OptimizerRule};
use datafusion::physical_expr::expressions::Literal;
use datafusion::prelude::SessionContext;

// ---------------------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------------------

/// The options of the Vega bin transform.
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

impl BinOptions {
    /// Reads options from pairs of a name and a value. `allow_extent` is false for
    /// `bin_params`, which takes the extent as arguments.
    pub fn parse(pairs: &[ScalarValue], allow_extent: bool) -> Result<Self> {
        if !pairs.len().is_multiple_of(2) {
            return plan_err!(
                "bin options must be pairs of a name and a value, but the last name has no value"
            );
        }
        let mut o = Self::default();
        let mut seen: Vec<&str> = Vec::new();
        for pair in pairs.chunks(2) {
            let Some(Some(name)) = pair[0].try_as_str() else {
                return plan_err!("a bin option name must be a string, not {}", pair[0]);
            };
            if seen.contains(&name) {
                return plan_err!("the bin option '{name}' is given twice");
            }
            seen.push(name);
            let v = &pair[1];
            // As in Vega, a zero maxbins, base, step, span, or minstep means the default.
            match name {
                "maxbins" => o.maxbins = or(number(name, v)?, 20.0),
                "base" => o.base = or(number(name, v)?, 10.0),
                "divide" => o.divide = numbers(name, v)?,
                "extent" if allow_extent => {
                    let e = numbers(name, v)?;
                    let [lo, hi] = e[..] else {
                        return plan_err!("the bin option 'extent' must have 2 numbers");
                    };
                    o.extent = Some([lo, hi]);
                }
                "span" => o.span = Some(number(name, v)?).filter(|s| truthy(*s)),
                "step" => o.step = Some(number(name, v)?).filter(|s| truthy(*s)),
                "steps" => {
                    let s = numbers(name, v)?;
                    if s.is_empty() {
                        return plan_err!("the bin option 'steps' must have at least one step");
                    }
                    o.steps = Some(s);
                }
                "minstep" => o.minstep = number(name, v)?,
                "nice" => {
                    let ScalarValue::Boolean(Some(b)) = v else {
                        return plan_err!("the bin option 'nice' must be true or false, not {v}");
                    };
                    o.nice = *b;
                }
                "anchor" => o.anchor = Some(number(name, v)?),
                _ => {
                    let names = "maxbins, base, divide, span, step, steps, minstep, nice, anchor";
                    let names = if allow_extent {
                        format!("{names}, extent")
                    } else {
                        names.to_string()
                    };
                    if name == "extent" {
                        return plan_err!(
                            "bin_params takes the extent as its first two arguments, not as the option 'extent'"
                        );
                    }
                    return plan_err!("unknown bin option '{name}': the options are {names}");
                }
            }
        }
        o.check()?;
        Ok(o)
    }

    /// Rejects the options with which Vega's algorithm does not end or gives no bins.
    fn check(&self) -> Result<()> {
        let positive = |name: &str, v: f64| -> Result<()> {
            if v.is_finite() && v > 0.0 {
                Ok(())
            } else {
                plan_err!("the bin option '{name}' must be a positive number, not {v}")
            }
        };
        positive("maxbins", self.maxbins)?;
        if !(self.base.is_finite() && self.base > 1.0) {
            return plan_err!(
                "the bin option 'base' must be a number greater than 1, not {}",
                self.base
            );
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
            for v in steps {
                positive("steps", *v)?;
            }
            if steps.windows(2).any(|w| w[0] >= w[1]) {
                return plan_err!("the bin option 'steps' must be in increasing order");
            }
        }
        if !(self.minstep.is_finite() && self.minstep >= 0.0) {
            return plan_err!(
                "the bin option 'minstep' must be a number of at least 0, not {}",
                self.minstep
            );
        }
        if let Some([lo, hi]) = self.extent
            && !(lo.is_finite() && hi.is_finite())
        {
            return plan_err!(
                "the bin option 'extent' must be two finite numbers, not [{lo}, {hi}]"
            );
        }
        if let Some(a) = self.anchor.filter(|a| !a.is_finite()) {
            return plan_err!("the bin option 'anchor' must be a finite number, not {a}");
        }
        Ok(())
    }
}

/// JavaScript truthiness of a number: 0 and NaN are false.
fn truthy(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}

/// JavaScript `a || b` for numbers.
fn or(a: f64, b: f64) -> f64 {
    if truthy(a) { a } else { b }
}

fn number(name: &str, v: &ScalarValue) -> Result<f64> {
    if v.is_null() || !v.data_type().is_numeric() {
        return plan_err!("the bin option '{name}' must be a number, not {v}");
    }
    match v.cast_to(&DataType::Float64)? {
        ScalarValue::Float64(Some(f)) => Ok(f),
        _ => plan_err!("the bin option '{name}' must be a number, not {v}"),
    }
}

fn numbers(name: &str, v: &ScalarValue) -> Result<Vec<f64>> {
    let array = match v {
        ScalarValue::List(a) if !a.is_null(0) => a.value(0),
        ScalarValue::LargeList(a) if !a.is_null(0) => a.value(0),
        ScalarValue::FixedSizeList(a) if !a.is_null(0) => a.value(0),
        _ => return plan_err!("the bin option '{name}' must be a list of numbers, not {v}"),
    };
    if !array.data_type().is_numeric() || array.null_count() > 0 {
        return plan_err!("the bin option '{name}' must be a list of numbers, not {v}");
    }
    let array = cast(&array, &DataType::Float64)?;
    Ok(array.as_primitive::<Float64Type>().values().to_vec())
}

/// The constant values of the expressions of a call's options.
pub(crate) fn literals(exprs: &[Expr]) -> Result<Vec<ScalarValue>> {
    exprs
        .iter()
        .map(|e| match e {
            Expr::Literal(v, _) => Ok(v.clone()),
            _ => plan_err!("bin options must be constants, not {e}"),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// The Vega algorithm
// ---------------------------------------------------------------------------------------

/// The bins that the Vega bin transform uses: values in [start, end] go to bins of size
/// `step` from `start`. Vega calls `end` the stop. As a struct, it is also a sequence for
/// the `fill_gaps` argument of `impute`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BinParams {
    pub start: f64,
    pub end: f64,
    pub step: f64,
}

/// `bin` of vega-statistics (packages/vega-statistics/src/bin.js), then the start and the
/// stop that the bin transform of vega-transforms uses (`_bins` in Bin.js): the stop rounded
/// to whole steps from the start, and both moved to the anchor.
pub fn bin_params(lo: f64, hi: f64, o: &BinOptions) -> BinParams {
    let maxb = o.maxbins;
    let base = o.base;
    let logb = js_log(base);
    let (mut min, mut max) = (lo, hi);
    let span = o.span.unwrap_or_else(|| or(or(max - min, min.abs()), 1.0));

    let step = if let Some(step) = o.step {
        step
    } else if let Some(steps) = &o.steps {
        let v = span / maxb;
        let i = steps.iter().take_while(|s| **s < v).count();
        steps[i.saturating_sub(1)]
    } else {
        let level = (js_log(maxb) / logb).ceil();
        let minstep = o.minstep;
        let mut step = js_max(minstep, js_pow(base, js_round(js_log(span) / logb) - level));
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

    let v = js_log(step);
    let precision = if v >= 0.0 {
        0.0
    } else {
        (-v / logb).trunc() + 1.0
    };
    let eps = js_pow(base, -precision - 1.0);
    if o.nice {
        let v = (min / step + eps).floor() * step;
        min = if min < v { v - step } else { v };
        max = (max / step).ceil() * step;
    }
    let stop = if max == min { min + step } else { max };

    // vega-transforms Bin._bins
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

/// `Math.log` and `Math.pow` of the Vega algorithm, from the `libm` crate, so that the bins
/// are the same on every platform (the platform `powf` differs between platforms, and wasm
/// uses `libm` anyway). V8 computes them with its own fdlibm code, whose `Math.pow` rounds
/// some powers of ten differently (`Math.pow(10, -4)` is 9.999999999999999e-5), so for
/// some tiny extents (a span near 1e-3 or 1e-9) Vega picks a different step. Only a port
/// of V8's `pow` would match those too.
fn js_log(v: f64) -> f64 {
    libm::log(v)
}

fn js_pow(base: f64, exp: f64) -> f64 {
    libm::pow(base, exp)
}

/// JavaScript `Math.round`: halves round up.
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// JavaScript `Math.max` of two numbers: NaN if either is NaN.
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

const EPSILON: f64 = 1e-14;

/// The bin of a value, as `bin0` and `bin1` of the Vega bin transform.
pub fn apply(v: f64, p: &BinParams) -> (f64, f64) {
    let BinParams {
        start,
        end: stop,
        step,
    } = *p;
    let b = if v.is_nan() {
        f64::NAN
    } else if v < start {
        f64::NEG_INFINITY
    } else if v > stop {
        f64::INFINITY
    } else {
        let v = start.max(v.min(stop - step));
        start + step * (EPSILON + (v - start) / step).floor()
    };
    // Vega's formula for the end, for its floating point agreement (vega/vega#830).
    (b, start + step * (1.0 + (b - start) / step))
}

// ---------------------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------------------

pub fn bin_fields() -> Fields {
    Fields::from(vec![
        Field::new("start", DataType::Float64, false),
        Field::new("end", DataType::Float64, false),
    ])
}

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
        let (s, e) = p.map_or((0.0, 0.0), |p| apply(xs.value(i), &p));
        starts.push(s);
        ends.push(e);
        valid.push(p.is_some());
    }
    let nulls = NullBuffer::from(valid);
    Arc::new(StructArray::new(
        bin_fields(),
        vec![
            Arc::new(Float64Array::from(starts)),
            Arc::new(Float64Array::from(ends)),
        ],
        (nulls.null_count() > 0).then_some(nulls),
    ))
}

/// `bin(x, ..options) OVER (..)`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct BinWindow {
    signature: Signature,
}

impl Default for BinWindow {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

impl WindowUDFImpl for BinWindow {
    fn name(&self) -> &str {
        "bin"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn partition_evaluator(
        &self,
        args: PartitionEvaluatorArgs,
    ) -> Result<Box<dyn PartitionEvaluator>> {
        let options = args.input_exprs()[1..]
            .iter()
            .map(|e| {
                e.downcast_ref::<Literal>()
                    .map(|l| l.value().clone())
                    .ok_or_else(|| plan_datafusion_err!("bin options must be constants, not {e}"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Box::new(BinEvaluator {
            options: BinOptions::parse(&options, true)?,
        }))
    }
    fn field(&self, args: WindowUDFFieldArgs) -> Result<FieldRef> {
        let Some(x) = args.input_fields().first() else {
            return plan_err!("bin needs a value to bin");
        };
        if !x.data_type().is_numeric() {
            return plan_err!("bin bins numbers, not {}", x.data_type());
        }
        Ok(Arc::new(Field::new(
            args.name(),
            DataType::Struct(bin_fields()),
            x.is_nullable(),
        )))
    }
}

#[derive(Debug)]
struct BinEvaluator {
    options: BinOptions,
}

impl PartitionEvaluator for BinEvaluator {
    fn evaluate_all(&mut self, values: &[ArrayRef], _num_rows: usize) -> Result<ArrayRef> {
        let x = cast(&values[0], &DataType::Float64)?;
        let x = x.as_primitive::<Float64Type>();
        let extent = match self.options.extent {
            Some([lo, hi]) => Some((lo, hi)),
            None => finite_extent(x),
        };
        let params = extent.map(|(lo, hi)| bin_params(lo, hi, &self.options));
        Ok(bin_array(x, |_| params))
    }
}

/// The least and the greatest finite value. Vega's extent skips NaN, but not the
/// infinities, which make every bin NaN; here an infinite value bins to an infinite bin.
fn finite_extent(x: &Float64Array) -> Option<(f64, f64)> {
    x.iter()
        .flatten()
        .filter(|v| v.is_finite())
        .fold(None, |e, v| match e {
            None => Some((v, v)),
            Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
        })
}

/// `x` as a double, where it is finite, for the extents of the rule, which must agree with
/// [`finite_extent`].
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

/// `bin_params(lo, hi, ..options)`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct BinParamsUdf {
    signature: Signature,
}

impl Default for BinParamsUdf {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

impl ScalarUDFImpl for BinParamsUdf {
    fn name(&self) -> &str {
        "bin_params"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Struct(params_fields()))
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        if args.arg_fields.len() < 2 {
            return plan_err!("bin_params needs the low and the high ends of an extent");
        }
        // Options such as `[1, 5]` are not folded to constants until the plan is simplified,
        // so check them here only when they are all constants already. The execution
        // checks them again.
        let options: Option<Vec<ScalarValue>> = args.scalar_arguments[2..]
            .iter()
            .map(|v| v.cloned())
            .collect();
        if let Some(options) = options {
            BinOptions::parse(&options, false)?;
        }
        let nullable = args.arg_fields[..2].iter().any(|f| f.is_nullable());
        Ok(Arc::new(Field::new(
            self.name(),
            DataType::Struct(params_fields()),
            nullable,
        )))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let options = args.args[2..]
            .iter()
            .map(|a| match a {
                ColumnarValue::Scalar(v) => Ok(v.clone()),
                ColumnarValue::Array(_) => plan_err!("bin_params options must be constants"),
            })
            .collect::<Result<Vec<_>>>()?;
        let o = BinOptions::parse(&options, false)?;
        let n = args.number_rows;
        let lo = to_f64(&args.args[0], n)?;
        let hi = to_f64(&args.args[1], n)?;
        let (lo, hi) = (
            lo.as_primitive::<Float64Type>(),
            hi.as_primitive::<Float64Type>(),
        );
        let mut cols = [
            Vec::with_capacity(n),
            Vec::with_capacity(n),
            Vec::with_capacity(n),
        ];
        let mut valid = Vec::with_capacity(n);
        for i in 0..n {
            let ok = lo.is_valid(i) && hi.is_valid(i);
            let p = if ok {
                bin_params(lo.value(i), hi.value(i), &o)
            } else {
                BinParams {
                    start: 0.0,
                    end: 0.0,
                    step: 0.0,
                }
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

/// `__bin_apply(x, params)`: internal; `params` is the result of `bin_params`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct BinApply {
    signature: Signature,
}

impl Default for BinApply {
    fn default() -> Self {
        Self {
            signature: Signature::any(2, Volatility::Immutable),
        }
    }
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
        Ok(DataType::Struct(bin_fields()))
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        // The parameters are null only for an extent with no values, so a non-null `x`
        // always has parameters.
        Ok(Arc::new(Field::new(
            self.name(),
            DataType::Struct(bin_fields()),
            args.arg_fields[0].is_nullable(),
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

// ---------------------------------------------------------------------------------------
// Rule 1: bin(x, ..options) OVER (..) -> __bin_apply(x, params)
// ---------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct BinWindowRewrite {
    params: Arc<ScalarUDF>,
    apply: Arc<ScalarUDF>,
}

impl Default for BinWindowRewrite {
    fn default() -> Self {
        Self {
            params: Arc::new(ScalarUDF::new_from_impl(BinParamsUdf::default())),
            apply: Arc::new(ScalarUDF::new_from_impl(BinApply::default())),
        }
    }
}

const EXTENT: &str = "__bin_extent";

impl OptimizerRule for BinWindowRewrite {
    fn name(&self) -> &str {
        "bin_window_rewrite"
    }

    fn apply_order(&self) -> Option<ApplyOrder> {
        Some(ApplyOrder::BottomUp)
    }

    fn rewrite(
        &self,
        plan: LogicalPlan,
        _config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        let LogicalPlan::Window(window) = &plan else {
            return Ok(Transformed::no(plan));
        };
        if let Some(split) = split_window(window, |_, e| is_call::<BinWindow>(e))? {
            let lower = self.rewrite(split.lower.clone(), _config)?.data;
            return Ok(Transformed::yes(split.rebuild(lower, Ok)?));
        }
        let mut calls: Vec<&WindowFunction> = Vec::new();
        for expr in &window.window_expr {
            let Some(wf) = window_call(expr).filter(|_| is_call::<BinWindow>(expr)) else {
                return Ok(Transformed::no(plan));
            };
            whole_partition("bin", &wf.params)?;
            if wf.params.args.is_empty() {
                return Ok(Transformed::no(plan));
            }
            calls.push(wf);
        }
        // The rewrite reads the input twice: for the extents, and for the rows.
        if is_volatile(&window.input)? || calls.iter().any(|c| c.params.args[0].is_volatile()) {
            return Ok(Transformed::no(plan));
        }
        let partition_by = &calls[0].params.partition_by;
        if calls.iter().any(|c| &c.params.partition_by != partition_by) {
            return Ok(Transformed::no(plan));
        }
        let Some(keys) = partition_by
            .iter()
            .map(|e| match e {
                Expr::Column(c) => Some(c.clone()),
                _ => None,
            })
            .collect::<Option<Vec<Column>>>()
        else {
            return Ok(Transformed::no(plan));
        };
        // The options, checked here so that a bad option fails when the query is planned.
        let mut options = Vec::new();
        for call in &calls {
            let exprs = &call.params.args[1..];
            options.push((exprs.to_vec(), BinOptions::parse(&literals(exprs)?, true)?));
        }

        let input = window.input.as_ref().clone();
        let n_in = input.schema().fields().len();
        let mut exprs: Vec<Expr> = input
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect();
        let out = |i: usize| window.schema.qualified_field(n_in + i);
        // The options of bin_params: all but `extent`.
        let params_options = |exprs: &[Expr]| -> Vec<Expr> {
            exprs
                .chunks(2)
                .filter(|p| !matches!(&p[0], Expr::Literal(v, _) if v.try_as_str() == Some(Some("extent"))))
                .flatten()
                .cloned()
                .collect()
        };

        // Calls that need the extent of the data, and the expression of their parameters.
        let mut params: Vec<Option<Expr>> = vec![None; calls.len()];
        let mut need_extent = Vec::new();
        for (i, (_, o)) in options.iter().enumerate() {
            match o.extent {
                // The parameters are constants.
                Some([lo, hi]) => {
                    let p = bin_params(lo, hi, o);
                    let fields = params_fields();
                    let value = ScalarStructBuilder::new()
                        .with_scalar(fields[0].clone(), ScalarValue::from(p.start))
                        .with_scalar(fields[1].clone(), ScalarValue::from(p.end))
                        .with_scalar(fields[2].clone(), ScalarValue::from(p.step))
                        .build()?;
                    params[i] = Some(lit(value));
                }
                None => need_extent.push(i),
            }
        }

        let builder = if need_extent.is_empty() {
            LogicalPlanBuilder::from(input)
        } else if keys.is_empty() {
            for &i in &need_extent {
                let x = calls[i].params.args[0].clone();
                let mut args = vec![
                    Expr::Column(Column::from_name("lo")),
                    Expr::Column(Column::from_name("hi")),
                ];
                args.extend(params_options(&options[i].0));
                let subquery = LogicalPlanBuilder::from(input.clone())
                    .aggregate(
                        Vec::<Expr>::new(),
                        vec![
                            min_expr(finite(&x)).alias("lo"),
                            max_expr(finite(&x)).alias("hi"),
                        ],
                    )?
                    .project(vec![self.params.call(args)])?
                    .build()?;
                params[i] = Some(Expr::ScalarSubquery(Subquery {
                    subquery: Arc::new(subquery),
                    outer_ref_columns: vec![],
                    spans: Spans::new(),
                }));
            }
            LogicalPlanBuilder::from(input)
        } else {
            let group: Vec<Expr> = keys
                .iter()
                .enumerate()
                .map(|(k, c)| Expr::Column(c.clone()).alias(format!("__bin_key_{k}")))
                .collect();
            let mut aggs = Vec::new();
            for &i in &need_extent {
                let x = &calls[i].params.args[0];
                aggs.push(min_expr(finite(x)).alias(format!("__bin_lo_{i}")));
                aggs.push(max_expr(finite(x)).alias(format!("__bin_hi_{i}")));
            }
            let mut projection: Vec<Expr> = (0..keys.len())
                .map(|k| Expr::Column(Column::from_name(format!("__bin_key_{k}"))))
                .collect();
            for &i in &need_extent {
                let mut args = vec![
                    Expr::Column(Column::from_name(format!("__bin_lo_{i}"))),
                    Expr::Column(Column::from_name(format!("__bin_hi_{i}"))),
                ];
                args.extend(params_options(&options[i].0));
                projection.push(self.params.call(args).alias(format!("__bin_params_{i}")));
                params[i] = Some(Expr::Column(Column::new(
                    Some(EXTENT),
                    format!("__bin_params_{i}"),
                )));
            }
            let extent = LogicalPlanBuilder::from(input.clone())
                .aggregate(group, aggs)?
                .project(projection)?
                .alias(EXTENT)?
                .build()?;
            let right: Vec<Column> = (0..keys.len())
                .map(|k| Column::new(Some(EXTENT), format!("__bin_key_{k}")))
                .collect();
            // PARTITION BY puts NULL keys in one partition, so NULL keys must match.
            LogicalPlanBuilder::from(input).join_detailed(
                extent,
                JoinType::Inner,
                (keys, right),
                None,
                NullEquality::NullEqualsNull,
            )?
        };

        for (i, call) in calls.iter().enumerate() {
            let (q, f) = out(i);
            let p = params[i].take().expect("parameters of each call");
            exprs.push(
                self.apply
                    .call(vec![call.params.args[0].clone(), p])
                    .alias_qualified(q.cloned(), f.name()),
            );
        }
        Ok(Transformed::yes(builder.project(exprs)?.build()?))
    }
}

// ---------------------------------------------------------------------------------------
// Window nodes, for the rules
// ---------------------------------------------------------------------------------------

/// The call of a window expression. The simplifier keeps the name of a call whose options
/// it folds (`[0, 100]` becomes a list constant) by wrapping the call in an alias.
pub(crate) fn window_call(expr: &Expr) -> Option<&WindowFunction> {
    let expr = match expr {
        Expr::Alias(a) => a.expr.as_ref(),
        e => e,
    };
    match expr {
        Expr::WindowFunction(wf) => Some(wf.as_ref()),
        _ => None,
    }
}

/// Whether a window expression calls the window function `T`.
pub(crate) fn is_call<T: WindowUDFImpl>(expr: &Expr) -> bool {
    window_call(expr).is_some_and(
        |wf| matches!(&wf.fun, WindowFunctionDefinition::WindowUDF(u) if u.inner().is::<T>()),
    )
}

/// Checks the OVER clause of a function of whole partitions: no ORDER BY, no frame that is
/// not the whole partition, and no DISTINCT or null treatment, which it would ignore.
pub(crate) fn whole_partition(name: &str, params: &WindowFunctionParams) -> Result<()> {
    if !params.order_by.is_empty() {
        return plan_err!("the window of {name} cannot have an ORDER BY");
    }
    let frame = &params.window_frame;
    if !(frame.start_bound.is_unbounded() && frame.end_bound.is_unbounded()) {
        return plan_err!("the window of {name} is its whole partition, so it cannot have a frame");
    }
    if params.distinct || params.null_treatment.is_some() || params.filter.is_some() {
        return plan_err!("{name} takes no DISTINCT, IGNORE NULLS, or FILTER");
    }
    Ok(())
}

/// Whether a plan computes anything volatile, such as `random()`. A rule that reads the
/// input of a window two times would read different values.
pub(crate) fn is_volatile(plan: &LogicalPlan) -> Result<bool> {
    plan.exists(|p| Ok(p.expressions().iter().any(Expr::is_volatile)))
}

/// A window node whose calls a rule takes only some of: a window node of those calls over
/// the input, under a window node of the others.
#[derive(Debug, Clone)]
pub(crate) struct Split {
    /// The window node of the calls that the rule takes.
    pub lower: LogicalPlan,
    others: Vec<Expr>,
    schema: datafusion::common::DFSchemaRef,
}

/// Splits a window node into the calls that `take` takes and the others, or None when it
/// takes all or none of them.
pub(crate) fn split_window(
    window: &Window,
    take: impl Fn(usize, &Expr) -> bool,
) -> Result<Option<Split>> {
    let (taken, others): (Vec<_>, Vec<_>) = window
        .window_expr
        .iter()
        .enumerate()
        .partition(|(i, e)| take(*i, e));
    if taken.is_empty() || others.is_empty() {
        return Ok(None);
    }
    let taken = taken.into_iter().map(|(_, e)| e.clone()).collect();
    let lower = LogicalPlan::Window(Window::try_new(taken, window.input.clone())?);
    Ok(Some(Split {
        lower,
        others: others.into_iter().map(|(_, e)| e.clone()).collect(),
        schema: window.schema.clone(),
    }))
}

impl Split {
    /// The window node of the other calls over `lower`, which has the schema of the lower
    /// window node, rewritten by `upper`, then the columns in the order of the window node
    /// that was split.
    pub fn rebuild(
        self,
        lower: LogicalPlan,
        upper: impl FnOnce(LogicalPlan) -> Result<LogicalPlan>,
    ) -> Result<LogicalPlan> {
        let upper = upper(LogicalPlan::Window(Window::try_new(
            self.others,
            Arc::new(lower),
        )?))?;
        let columns: Vec<Expr> = self
            .schema
            .iter()
            .map(|(q, f)| Expr::Column(Column::new(q.cloned(), f.name())))
            .collect();
        LogicalPlanBuilder::from(upper).project(columns)?.build()
    }
}

// ---------------------------------------------------------------------------------------
// Rule 2: GROUP BY struct -> GROUP BY its fields, then the struct again
// ---------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct StructGroupSplit;

/// A UDF that makes a non-null struct with exactly these fields, from one array for each
/// field. Unlike `named_struct`, the fields keep their nullability, so a struct made again
/// has the type that it had.
pub(crate) fn make_struct_udf(fields: Fields) -> ScalarUDF {
    let signature = Signature::any(fields.len(), Volatility::Immutable);
    ScalarUDF::new_from_impl(MakeStruct { fields, signature })
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct MakeStruct {
    fields: Fields,
    signature: Signature,
}

impl ScalarUDFImpl for MakeStruct {
    fn name(&self) -> &str {
        "__make_struct"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Struct(self.fields.clone()))
    }
    fn return_field_from_args(&self, _: ReturnFieldArgs) -> Result<FieldRef> {
        Ok(Arc::new(Field::new(
            self.name(),
            DataType::Struct(self.fields.clone()),
            false,
        )))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let n = args.number_rows;
        let arrays = args
            .args
            .iter()
            .map(|a| a.to_array(n))
            .collect::<Result<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(StructArray::try_new(
            self.fields.clone(),
            arrays,
            None,
        )?)))
    }
}

/// Whether the fields of a struct key can be group keys of their own: at least one, each with
/// a name that is not empty and no other field has, and none a struct.
fn splittable(fields: &Fields) -> bool {
    let mut names = std::collections::HashSet::new();
    !fields.is_empty()
        && fields.iter().all(|f| {
            !f.name().is_empty()
                && names.insert(f.name())
                && !matches!(f.data_type(), DataType::Struct(_))
        })
}

impl OptimizerRule for StructGroupSplit {
    fn name(&self) -> &str {
        "struct_group_split"
    }

    fn apply_order(&self) -> Option<ApplyOrder> {
        Some(ApplyOrder::BottomUp)
    }

    fn rewrite(
        &self,
        plan: LogicalPlan,
        _config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        let LogicalPlan::Aggregate(agg) = &plan else {
            return Ok(Transformed::no(plan));
        };
        if agg
            .group_expr
            .iter()
            .any(|e| matches!(e, Expr::GroupingSet(_)))
        {
            return Ok(Transformed::no(plan));
        }
        let input_schema = agg.input.schema();
        let mut group = Vec::new();
        // For each group key: the fields that replace it, or None if it stays.
        let mut split: Vec<Option<Fields>> = Vec::new();
        for (i, e) in agg.group_expr.iter().enumerate() {
            let (_, field) = e.to_field(input_schema)?;
            match field.data_type().clone() {
                // A NULL struct and a struct of NULL fields are different keys, so the rule
                // splits only keys that cannot be NULL. It reads the fields by name, and does
                // not split nested structs.
                DataType::Struct(fields) if !field.is_nullable() && splittable(&fields) => {
                    let inner = e.clone().unalias();
                    for (j, f) in fields.iter().enumerate() {
                        group.push(
                            get_field(inner.clone(), f.name().as_str())
                                .alias(format!("__group_{i}_{j}")),
                        );
                    }
                    split.push(Some(fields));
                }
                _ => {
                    group.push(e.clone());
                    split.push(None);
                }
            }
        }
        if split.iter().all(Option::is_none) {
            return Ok(Transformed::no(plan));
        }

        let n_group = group.len();
        let new_agg = LogicalPlan::Aggregate(Aggregate::try_new(
            agg.input.clone(),
            group,
            agg.aggr_expr.clone(),
        )?);
        let new_schema = new_agg.schema().clone();
        let column = |k: usize| Expr::Column(Column::from(new_schema.qualified_field(k)));

        // The projection gives each output column the name, qualifier and type it had.
        let mut exprs = Vec::new();
        let mut k = 0;
        for (i, fields) in split.iter().enumerate() {
            let (q, f) = agg.schema.qualified_field(i);
            let expr = match fields {
                None => {
                    k += 1;
                    column(k - 1)
                }
                Some(fields) => {
                    let args = (k..k + fields.len()).map(column).collect();
                    k += fields.len();
                    make_struct_udf(fields.clone()).call(args)
                }
            };
            // An alias to the same name confuses push_down_leaf_projections (see below).
            let same =
                matches!(&expr, Expr::Column(c) if c.relation.as_ref() == q && &c.name == f.name());
            exprs.push(if same {
                expr
            } else {
                expr.alias_qualified(q.cloned(), f.name())
            });
        }
        for a in 0..agg.aggr_expr.len() {
            let (q, f) = agg.schema.qualified_field(split.len() + a);
            let (nq, nf) = new_schema.qualified_field(n_group + a);
            // An alias to the same name confuses DataFusion's push_down_leaf_projections,
            // which then keeps both the column and the alias, two fields of one name.
            if nq == q && nf.name() == f.name() {
                exprs.push(column(n_group + a));
            } else {
                exprs.push(column(n_group + a).alias_qualified(q.cloned(), f.name()));
            }
        }
        Ok(Transformed::yes(
            LogicalPlanBuilder::from(new_agg).project(exprs)?.build()?,
        ))
    }
}

// ---------------------------------------------------------------------------------------
// stack(v, ..options) OVER (PARTITION BY .. ORDER BY ..)
// ---------------------------------------------------------------------------------------

/// The stack offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackOffset {
    /// Vega's `zero`: positive values stack up from 0, negative values down from 0.
    Zero,
    /// A stack of absolute values centered on zero.
    Center,
    /// Vega's `normalize`: a stack of absolute values from 0 to 1.
    Normalize,
}

impl StackOffset {
    pub fn parse(pairs: &[ScalarValue]) -> Result<Self> {
        if !pairs.len().is_multiple_of(2) {
            return plan_err!(
                "stack options must be pairs of a name and a value, but the last name has no value"
            );
        }
        let mut offset = None;
        for pair in pairs.chunks(2) {
            let Some(Some(name)) = pair[0].try_as_str() else {
                return plan_err!("a stack option name must be a string, not {}", pair[0]);
            };
            if name != "offset" {
                return plan_err!("unknown stack option '{name}': the option is offset");
            }
            if offset.is_some() {
                return plan_err!("the stack option 'offset' is given twice");
            }
            offset = Some(match pair[1].try_as_str() {
                Some(Some("zero")) => Self::Zero,
                Some(Some("center")) => Self::Center,
                Some(Some("normalize")) => Self::Normalize,
                _ => {
                    return plan_err!(
                        "the stack option 'offset' must be 'zero', 'center', or 'normalize', not {}",
                        pair[1]
                    );
                }
            });
        }
        Ok(offset.unwrap_or(Self::Zero))
    }
}

/// The start and end of each value of one stack, in the order of its layers. As in Vega, a
/// null value counts as 0 (`+null` and `Math.abs(null)` are 0 in JavaScript).
pub fn stack_values(values: &[Option<f64>], offset: StackOffset) -> Vec<(f64, f64)> {
    let v = |x: &Option<f64>| x.unwrap_or(0.0);
    match offset {
        StackOffset::Zero => {
            let (mut pos, mut neg) = (0.0, 0.0);
            values
                .iter()
                .map(|x| {
                    let x = v(x);
                    if x < 0.0 {
                        let start = neg;
                        neg += x;
                        (start, neg)
                    } else {
                        let start = pos;
                        pos += x;
                        (start, pos)
                    }
                })
                .collect()
        }
        StackOffset::Center => {
            let sum: f64 = values.iter().map(|x| v(x).abs()).sum();
            let mut last = -sum / 2.0;
            values
                .iter()
                .map(|x| {
                    let start = last;
                    last += v(x).abs();
                    (start, last)
                })
                .collect()
        }
        StackOffset::Normalize => {
            let sum: f64 = values.iter().map(|x| v(x).abs()).sum();
            let scale = 1.0 / sum;
            let (mut last, mut acc) = (0.0, 0.0);
            values
                .iter()
                .map(|x| {
                    let start = last;
                    acc += v(x).abs();
                    last = scale * acc;
                    (start, last)
                })
                .collect()
        }
    }
}

/// `stack(v, ..options) OVER (..)`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct StackWindow {
    signature: Signature,
}

impl Default for StackWindow {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
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
        args: PartitionEvaluatorArgs,
    ) -> Result<Box<dyn PartitionEvaluator>> {
        let options = args.input_exprs()[1..]
            .iter()
            .map(|e| {
                e.downcast_ref::<Literal>()
                    .map(|l| l.value().clone())
                    .ok_or_else(|| plan_datafusion_err!("stack options must be constants, not {e}"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Box::new(StackEvaluator {
            offset: StackOffset::parse(&options)?,
        }))
    }
    fn field(&self, args: WindowUDFFieldArgs) -> Result<FieldRef> {
        let Some(v) = args.input_fields().first() else {
            return plan_err!("stack needs a value to stack");
        };
        if !v.data_type().is_numeric() && !v.data_type().is_null() {
            return plan_err!("stack stacks numbers, not {}", v.data_type());
        }
        // A null value stacks as 0, so the result is never null.
        Ok(Arc::new(Field::new(
            args.name(),
            DataType::Struct(bin_fields()),
            false,
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
        let (starts, ends): (Vec<f64>, Vec<f64>) =
            stack_values(&v, self.offset).into_iter().unzip();
        Ok(Arc::new(StructArray::new(
            bin_fields(),
            vec![
                Arc::new(Float64Array::from(starts)) as ArrayRef,
                Arc::new(Float64Array::from(ends)),
            ],
            None,
        )))
    }
}

// ---------------------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------------------

/// A session with the transforms: the functions `bin`, `bin_params`, `stack`, `lump`, and
/// `timeunit`, the table function `impute`, and the aggregate states that the fusion of lumps
/// uses. With `rules`, it also has the optimizer rules: without them, the window functions run
/// through their own evaluators, which are slower and do not take a `lump` with `'keep'` or
/// `'by'`.
pub fn session(rules: bool) -> SessionContext {
    session_with(rules, true)
}

/// [`session`], with or without [`lump::LumpChartFusion`].
pub fn session_with(rules: bool, lump_fusion: bool) -> SessionContext {
    let mut list = Optimizer::new().rules;
    if rules {
        // Right after the first simplification, which folds the options to constants (`[0,
        // 100]` becomes a list constant), so that every later rule sees the rewritten plan.
        let at = list
            .iter()
            .position(|r| r.name() == "simplify_expressions")
            .expect("DataFusion's optimizer has the rule simplify_expressions")
            + 1;
        let mut ours: Vec<Arc<dyn OptimizerRule + Send + Sync>> = vec![
            Arc::new(timeunit::TimeUnitWindowRewrite::default()),
            Arc::new(BinWindowRewrite::default()),
        ];
        if lump_fusion {
            ours.push(Arc::new(lump::LumpChartFusion));
        }
        ours.push(Arc::new(lump::LumpRewrite));
        ours.push(Arc::new(StructGroupSplit));
        list.splice(at..at, ours);
    }
    let state = SessionStateBuilder::new()
        .with_default_features()
        .with_optimizer_rules(list)
        .with_query_planner(Arc::new(impute::TransformsQueryPlanner))
        .build();
    let mut ctx = SessionContext::new_with_state(state);
    ctx.register_udwf(WindowUDF::new_from_impl(BinWindow::default()));
    ctx.register_udf(ScalarUDF::new_from_impl(BinParamsUdf::default()));
    ctx.register_udwf(WindowUDF::new_from_impl(StackWindow::default()));
    ctx.register_udwf(lump::lump_udwf());
    ctx.register_udwf(WindowUDF::new_from_impl(timeunit::TimeUnitWindow::default()));
    ctx.register_udf(timeunit::scalar_udf());
    avenger_datafusion_aggregate_state::register_all(&mut ctx)
        .expect("register the aggregate states");
    ctx.register_relation_planner(Arc::new(impute::ImputePlanner))
        .expect("register the impute planner");
    ctx
}
