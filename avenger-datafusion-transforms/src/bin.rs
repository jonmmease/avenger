//! `bin` is a Vega-style bin transform: bins of a nice step, for at most `maxbins` bins. Its
//! options, with their defaults, are `maxbins` (20), `base` (10), `divide` ([5, 2]),
//! `extent`, `span`, `step`, `steps`, `minstep` (0), `nice` (true), and `anchor`. Without
//! `extent`, the extent is that of the finite values of `x` in each window partition. The
//! result is `struct<start, end>`, the bounds of the bin of each value, or NULL for a value
//! outside the bins: NULL, NaN, an infinite value, or a value outside an explicit `extent`.
//!
//! - `bin(x, ..options) OVER (..)`: the window function that users call.
//! - `bin_params(lo, hi, ..options)`: the `start`, `end`, and `step` of the bins of an extent,
//!   for axes, scale domains, and empty bins.
//! - `__bin_apply(x, params)`: internal; bins each value by the result of `bin_params`.
//!
//! [`BinWindowRewrite`] replaces `bin(..) OVER (..)` with `__bin_apply`, which gets its
//! parameters from constants (with `extent`), from a scalar subquery (no `PARTITION BY`), or
//! from a join with the parameters of each partition. [`crate::StructGroupSplit`] groups by the
//! fields of a non-null struct key, such as a bin, then makes the struct again above the
//! aggregate.

use std::sync::Arc;

use datafusion::arrow::array::{Array, ArrayRef, AsArray, Float64Array, StructArray};
use datafusion::arrow::buffer::NullBuffer;
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{DataType, Field, FieldRef, Fields, Float64Type};
use datafusion::common::scalar::ScalarStructBuilder;
use datafusion::common::tree_node::Transformed;
use datafusion::common::{Column, Result, ScalarValue, plan_err};
use datafusion::functions_aggregate::expr_fn::{max as max_expr, min as min_expr};
use datafusion::logical_expr::function::{PartitionEvaluatorArgs, WindowUDFFieldArgs};
use datafusion::logical_expr::{
    ColumnarValue, Expr, LogicalPlan, PartitionEvaluator, ReturnFieldArgs, ScalarFunctionArgs,
    ScalarUDF, ScalarUDFImpl, Signature, Volatility, WindowUDFImpl, lit,
};
use datafusion::optimizer::{ApplyOrder, OptimizerConfig, OptimizerRule};

use crate::utils::{
    PartitionCalls, PartitionValue, interval_fields, is_call, literals, partition_calls,
    partition_values, physical_literals, scalar_args, split_window,
};

// ---------------------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------------------

/// The options of `bin`, which its options parse, with their defaults, which
/// [`BinOptions::default`] gives.
pub const OPTIONS: &[crate::OptionSpec] = &[
    option(
        "maxbins",
        "FLOAT64",
        "The greatest number of bins. When omitted, 20.",
    ),
    option(
        "base",
        "FLOAT64",
        "The number base of the steps. When omitted, 10.",
    ),
    option(
        "divide",
        "ARRAY<FLOAT64>",
        "The factors by which a step can be divided to give more bins. When omitted, `[5, 2]`.",
    ),
    option(
        "extent",
        "ARRAY<FLOAT64>",
        "The range to bin, as `[low, high]`. A value outside it has no bin. When omitted, the range of the finite values of the partition.",
    ),
    option(
        "span",
        "FLOAT64",
        "The length of the range that chooses the step, in place of the length of the extent.",
    ),
    option(
        "step",
        "FLOAT64",
        "The step of the bins, in place of the step that `bin` chooses.",
    ),
    option(
        "steps",
        "ARRAY<FLOAT64>",
        "The steps that `bin` chooses from: the smallest that gives at most `maxbins` bins.",
    ),
    option("minstep", "FLOAT64", "The smallest step. When omitted, 0."),
    option(
        "nice",
        "BOOL",
        "Whether the first bin starts at a whole multiple of the step. When omitted, `TRUE`. With `FALSE`, the first bin starts at the low end of the range.",
    ),
    option("anchor", "FLOAT64", "A value at which a bin starts."),
];

const fn option(name: &'static str, ty: &'static str, doc: &'static str) -> crate::OptionSpec {
    crate::OptionSpec {
        name,
        ty,
        values: &[],
        doc,
    }
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
            // A zero maxbins, base, step, span, or minstep means the default.
            match name {
                "maxbins" => o.maxbins = number(name, v)?,
                "base" => o.base = number(name, v)?,
                "divide" => o.divide = numbers(name, v)?,
                "extent" if allow_extent => {
                    let e = numbers(name, v)?;
                    let [lo, hi] = e[..] else {
                        return plan_err!("the bin option 'extent' must have 2 numbers");
                    };
                    o.extent = Some([lo, hi]);
                }
                "span" => o.span = Some(number(name, v)?),
                "step" => o.step = Some(number(name, v)?),
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

    /// Rejects the options with which the algorithm does not end or gives no bins.
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
        let options = physical_literals("bin", &args.input_exprs()[1..])?;
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
            DataType::Struct(interval_fields()),
            // NULL, NaN, infinite values, and values outside an explicit extent have no bin.
            true,
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

/// The least and the greatest finite value. NaN and the infinities take no part, and have
/// no bin.
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
        let options = scalar_args("bin_params", &args.args[2..])?;
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
        let Some(PartitionCalls { calls, keys }) = partition_calls::<BinWindow>(window, "bin")?
        else {
            return Ok(Transformed::no(plan));
        };
        // The options, checked here so that a bad option fails when the query is planned.
        let mut options = Vec::new();
        for call in &calls {
            let exprs = &call.params.args[1..];
            options.push((
                exprs.to_vec(),
                BinOptions::parse(&literals("bin", exprs)?, true)?,
            ));
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

        // The parameters of each call: constants with `extent`, else from the extent of the
        // finite values of each partition.
        let mut params: Vec<Option<Expr>> = vec![None; calls.len()];
        let mut need_extent = Vec::new();
        let mut values = Vec::new();
        for (i, (exprs, o)) in options.iter().enumerate() {
            match o.extent {
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
                None => {
                    let x = &calls[i].params.args[0];
                    let (lo, hi) = (format!("__bin_lo_{i}"), format!("__bin_hi_{i}"));
                    let mut args = vec![
                        Expr::Column(Column::from_name(&lo)),
                        Expr::Column(Column::from_name(&hi)),
                    ];
                    args.extend(params_options(exprs));
                    values.push(PartitionValue {
                        aggregates: vec![
                            min_expr(finite(x)).alias(lo),
                            max_expr(finite(x)).alias(hi),
                        ],
                        value: self.params.call(args),
                    });
                    need_extent.push(i);
                }
            }
        }
        let (builder, extents) = partition_values(input, &keys, values, "bin")?;
        for (i, e) in need_extent.into_iter().zip(extents) {
            params[i] = Some(e);
        }

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
