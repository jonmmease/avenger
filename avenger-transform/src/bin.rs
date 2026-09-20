use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, Float64Array, StructArray},
        compute::cast,
        datatypes::{DataType, Field, Fields, Float64Type},
    },
    common::{exec_err, plan_err, Column, Result, ScalarValue},
    functions::core::expr_fn::get_field,
    logical_expr::{lit, ColumnarValue, Expr, LogicalPlan, LogicalPlanBuilder},
};

use crate::{
    formula::{internal_name, replace_fields},
    udf::{uniform, Function},
};

/// Expressions that configure binning. Omitted options use Vega defaults.
/// Numeric and Boolean options are scalar. `divide` and `steps` are lists.
#[derive(Clone, Debug, Default)]
pub struct BinOptions {
    /// Desired maximum bin count, default 20.
    pub maxbins: Option<Expr>,
    /// Number base for automatic steps, default 10.
    pub base: Option<Expr>,
    /// Allowed step divisors, default [5, 2].
    pub divide: Option<Expr>,
    /// Span used for step selection, independent of the extent endpoints.
    pub span: Option<Expr>,
    /// Explicit step, taking precedence over automatic selection.
    pub step: Option<Expr>,
    /// Positive increasing list of allowed steps.
    pub steps: Option<Expr>,
    /// Minimum automatically selected step, default zero.
    pub minstep: Option<Expr>,
    /// Round extent boundaries to the selected step, default true.
    pub nice: Option<Expr>,
    /// Shift the bins to align a boundary with this value.
    pub anchor: Option<Expr>,
}

/// Calculate a scalar `{start, stop, step, upper_bound}` from an extent struct.
/// Configuration remains in expression arguments and is evaluated at query time.
pub fn bin_parameters(extent: Expr, options: BinOptions) -> Result<Expr> {
    let args = [
        ("maxbins", options.maxbins),
        ("base", options.base),
        ("divide", options.divide),
        ("span", options.span),
        ("step", options.step),
        ("steps", options.steps),
        ("minstep", options.minstep),
        ("nice", options.nice),
        ("anchor", options.anchor),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.map(|value| [lit(name), value]))
    .flatten()
    .collect::<Vec<_>>();
    let options = if args.is_empty() {
        lit(ScalarValue::Struct(Arc::new(
            StructArray::new_empty_fields(1, None),
        )))
    } else {
        datafusion::functions::core::named_struct().call(args)
    };
    Ok(Function::BinParameters.call(vec![extent, options]))
}

/// Add or replace bin-start and bin-end fields while retaining the input fields.
/// Both boundaries read the original input value, even when replacing its field.
pub fn bin(
    input: LogicalPlan,
    value: Expr,
    parameters: Expr,
    output_names: [&str; 2],
) -> Result<LogicalPlan> {
    let mut name = internal_name(&input, "__avenger_bin");
    while output_names.contains(&name.as_str()) {
        name = internal_name(&input, &format!("{name}_"));
    }
    let computed = crate::formula(
        input,
        Function::BinBounds.call(vec![value, parameters]),
        &name,
    )?;
    let reference = Expr::Column(Column::from_name(&name));
    let expressions = replace_fields(
        &computed,
        vec![
            (output_names[0], get_field(reference.clone(), "start")),
            (output_names[1], get_field(reference, "end")),
        ],
    )?;
    let expressions = expressions
        .into_iter()
        .filter(|e| !matches!(e, Expr::Column(c) if c.name == name))
        .collect::<Vec<_>>();
    LogicalPlanBuilder::from(computed)
        .project(expressions)?
        .build()
}

pub(crate) fn fields(names: &[&str], nullable: bool) -> Fields {
    names
        .iter()
        .map(|name| Field::new(*name, DataType::Float64, nullable))
        .collect()
}

pub(crate) fn parameter_type() -> DataType {
    DataType::Struct(fields(&["start", "stop", "step", "upper_bound"], false))
}

pub(crate) fn validate_struct(t: &DataType, names: &[&str]) -> Result<()> {
    match t {
        DataType::Struct(fields)
            if fields.len() == names.len()
                && fields
                    .iter()
                    .zip(names)
                    .all(|(f, n)| f.name() == n && f.data_type() == &DataType::Float64) =>
        {
            Ok(())
        }
        _ => plan_err!("Binning expects Float64 struct fields {names:?}, received {t}"),
    }
}

pub(crate) fn validate_options(t: &DataType) -> Result<()> {
    let DataType::Struct(fields) = t else {
        return plan_err!("Bin options must be a struct");
    };
    for f in fields {
        let valid = match f.name().as_str() {
            "nice" => f.data_type() == &DataType::Boolean,
            "steps" | "divide" => {
                matches!(f.data_type(), DataType::List(item) if crate::values::numeric(item.data_type()) && item.data_type() != &DataType::Null)
            }
            "maxbins" | "base" | "span" | "step" | "minstep" | "anchor" => {
                crate::values::numeric(f.data_type()) && f.data_type() != &DataType::Null
            }
            _ => false,
        };
        if !valid {
            return plan_err!("Invalid bin option {} of type {}", f.name(), f.data_type());
        }
    }
    Ok(())
}

pub(crate) fn calculate(extent: &ColumnarValue, options: &ColumnarValue) -> Result<ScalarValue> {
    let ScalarValue::Struct(extent) = uniform(extent, "extent")? else {
        unreachable!()
    };
    let ScalarValue::Struct(options) = uniform(options, "options")? else {
        unreachable!()
    };
    if options.is_null(0) {
        return exec_err!("Bin options must be non-null");
    }
    let number = |name: &str| -> Result<Option<f64>> {
        options
            .column_by_name(name)
            .map(|a| scalar_number(a, name))
            .transpose()
    };
    let list = |name: &str| -> Result<Option<Vec<f64>>> {
        options
            .column_by_name(name)
            .map(|a| {
                let a = a.as_list::<i32>();
                if a.is_null(0) {
                    return exec_err!("Bin option {name} must be non-null");
                }
                let a = cast(&a.value(0), &DataType::Float64)?;
                a.as_primitive::<Float64Type>()
                    .iter()
                    .map(|v| {
                        v.filter(|v| v.is_finite()).ok_or_else(|| {
                            datafusion::common::exec_datafusion_err!(
                                "Bin option {name} requires finite non-null elements"
                            )
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()
    };
    let maxbins = number("maxbins")?.unwrap_or(20.0);
    let base = number("base")?.unwrap_or(10.0);
    let minstep = number("minstep")?.unwrap_or(0.0);
    let step = number("step")?;
    let span = number("span")?;
    let anchor = number("anchor")?;
    let divide = list("divide")?.unwrap_or_else(|| vec![5.0, 2.0]);
    let steps = list("steps")?;
    let nice = match options.column_by_name("nice") {
        Some(a) if !a.is_null(0) => a.as_boolean().value(0),
        Some(_) => return exec_err!("Bin option nice must be non-null"),
        None => true,
    };
    if maxbins < 1.0
        || base <= 1.0
        || minstep < 0.0
        || step.is_some_and(|x| x <= 0.0)
        || span.is_some_and(|x| x <= 0.0)
        || divide.is_empty()
        || divide.iter().any(|x| *x <= 1.0)
        || steps.as_ref().is_some_and(|s| {
            s.is_empty() || s.iter().any(|x| *x <= 0.0) || s.windows(2).any(|w| w[0] >= w[1])
        })
    {
        return exec_err!("Invalid bin options: maxbins >= 1, base/divide > 1, minstep >= 0, and positive step/span/increasing steps are required");
    }
    if extent.is_null(0) || extent.columns().iter().any(|a| a.is_null(0)) {
        return ScalarValue::try_from(&parameter_type());
    }
    let min = scalar_number(&extent.columns()[0], "extent.min")?;
    let max = scalar_number(&extent.columns()[1], "extent.max")?;
    if min > max {
        return exec_err!("Bin extent min must not exceed max");
    }
    let span = span.unwrap_or_else(|| {
        if max != min {
            max - min
        } else if min != 0.0 {
            min.abs()
        } else {
            1.0
        }
    });
    if !span.is_finite() {
        return exec_err!("Bin extent span is not finite");
    }
    let parameters = resolve(
        min,
        max,
        maxbins,
        base,
        minstep,
        step,
        steps.as_deref(),
        &divide,
        span,
        nice,
        anchor,
    )?;
    let DataType::Struct(fields) = parameter_type() else {
        unreachable!()
    };
    let arrays = parameters
        .into_iter()
        .map(|v| Arc::new(Float64Array::from(vec![v])) as ArrayRef)
        .collect();
    Ok(ScalarValue::Struct(Arc::new(StructArray::new(
        fields, arrays, None,
    ))))
}

fn scalar_number(array: &ArrayRef, name: &str) -> Result<f64> {
    let array = cast(array, &DataType::Float64)?;
    let array = array.as_primitive::<Float64Type>();
    if array.is_null(0) || !array.value(0).is_finite() {
        return exec_err!("Bin {name} must be finite and non-null");
    }
    Ok(array.value(0))
}

// Algorithm adapted from Vega v6.2.0. See LICENSE.vega and README references.
#[allow(clippy::too_many_arguments)]
fn resolve(
    mut min: f64,
    mut max: f64,
    maxbins: f64,
    base: f64,
    minstep: f64,
    step: Option<f64>,
    steps: Option<&[f64]>,
    divide: &[f64],
    span: f64,
    nice: bool,
    anchor: Option<f64>,
) -> Result<[f64; 4]> {
    let logb = base.ln();
    let step = if let Some(step) = step {
        step
    } else if let Some(steps) = steps {
        let i = steps.partition_point(|v| *v < span / maxbins);
        steps[i.saturating_sub(1)]
    } else {
        let level = (maxbins.ln() / logb).ceil();
        // Math.round ties toward positive infinity, including negative values.
        let rounded = (span.ln() / logb + 0.5).floor();
        let mut step = minstep.max(base.powf(rounded - level));
        if step <= 0.0 || !step.is_finite() {
            return exec_err!("Bin step is not representable");
        }
        let mut refinements = 0;
        while (span / step).ceil() > maxbins {
            let next = step * base;
            refinements += 1;
            if next <= step || !next.is_finite() || refinements > 2048 {
                return exec_err!("Bin step refinement exceeds finite arithmetic limits");
            }
            step = next;
        }
        for div in divide {
            let v = step / div;
            if v >= minstep && span / v <= maxbins {
                step = v;
            }
        }
        step
    };
    let log = step.ln();
    let precision = if log >= 0.0 {
        0.0
    } else {
        (-log / logb).trunc() + 1.0
    };
    let epsilon = base.powf(-precision - 1.0);
    if nice {
        let v = (min / step + epsilon).floor() * step;
        min = if min < v { v - step } else { v };
        max = (max / step).ceil() * step;
    }
    let stop = if max == min { min + step } else { max };
    let mut upper = min + ((stop - min) / step).ceil() * step;
    if let Some(anchor) = anchor {
        let delta = anchor - (min + step * ((anchor - min) / step).floor());
        min += delta;
        upper += delta;
    }
    if ![min, stop, step, upper].iter().all(|x| x.is_finite()) || step <= 0.0 || upper <= min {
        return exec_err!("Bin boundaries or step are not representable");
    }
    Ok([min, stop, step, upper])
}

pub(crate) fn apply(values: &ArrayRef, parameters: &ColumnarValue) -> Result<ArrayRef> {
    let ScalarValue::Struct(parameters) = uniform(parameters, "parameters")? else {
        unreachable!()
    };
    let (mut lo, mut hi) = (
        Vec::with_capacity(values.len()),
        Vec::with_capacity(values.len()),
    );
    let values = cast(values, &DataType::Float64)?;
    let present = !parameters.is_null(0);
    let p = parameters
        .columns()
        .iter()
        .map(|a| a.as_primitive::<Float64Type>().value(0))
        .collect::<Vec<_>>();
    if present
        && (parameters.columns().iter().any(|a| a.is_null(0))
            || !p.iter().all(|v| v.is_finite())
            || p[2] <= 0.0
            || p[3] <= p[0])
    {
        return exec_err!("Invalid resolved bin parameters");
    }
    for value in values.as_primitive::<Float64Type>().iter() {
        let start = value.filter(|_| present).map(|v| {
            if v.is_nan() {
                f64::NAN
            } else if v < p[0] {
                f64::NEG_INFINITY
            } else if v > p[3] {
                f64::INFINITY
            } else {
                let v = v.min(p[3] - p[2]).max(p[0]);
                p[0] + p[2] * (1e-14 + (v - p[0]) / p[2]).floor()
            }
        });
        lo.push(start);
        hi.push(start.map(|v| p[0] + p[2] * (1.0 + (v - p[0]) / p[2])));
    }
    Ok(Arc::new(StructArray::new(
        fields(&["start", "end"], true),
        vec![
            Arc::new(Float64Array::from(lo)),
            Arc::new(Float64Array::from(hi)),
        ],
        None,
    )))
}
