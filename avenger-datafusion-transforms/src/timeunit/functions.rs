//! The calls of `timeunit`, its placeholder window, the internal scalar functions, and the
//! aggregate that chooses units.

use super::*;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use crate::utils::PartitionValue;
use crate::window::{Output, PartitionedCall, Transform, placeholder, plan_partitioned};
use datafusion::arrow::array::{Array, ArrayRef, AsArray, Int64Array, UInt8Array, UInt64Array};
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{
    DataType, Field, FieldRef, Int32Type, Int64Type, UInt8Type, UInt64Type,
};
use datafusion::common::{Column, Result, ScalarValue, internal_err};
use datafusion::logical_expr::function::{AccumulatorArgs, StateFieldsArgs};
use datafusion::logical_expr::{
    Accumulator, AggregateUDF, AggregateUDFImpl, ColumnarValue, Expr, LogicalPlan, ReturnFieldArgs,
    ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility, WindowUDF,
};
use datafusion::physical_expr::expressions::Literal;

/// A call of `timeunit`: the date or timestamp, the window partitions, and the options.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeUnitCall {
    pub value: Expr,
    pub partition: Vec<Expr>,
    pub options: TimeUnitOptions,
}

/// The placeholder window of a call of `timeunit`, whose arguments are the value and the
/// struct of its options, from [`options`], and whose `PARTITION BY` gives the window
/// partitions. `gives_unit` tells whether the call gives a unit, which plans the units without
/// an aggregate. [`crate::plan_windows`] plans it.
pub fn window(gives_unit: bool) -> Arc<WindowUDF> {
    placeholder("timeunit".to_string(), Transform::TimeUnit { gives_unit })
}

/// The plan of `input` with the column `output`, the time unit of each row by `call`, after the
/// columns of `input`.
pub fn plan(input: LogicalPlan, call: TimeUnitCall, output: Column) -> Result<LogicalPlan> {
    let TimeUnitCall {
        value,
        partition,
        options,
    } = call;
    options.check()?;
    let call = PartitionedCall::TimeUnit {
        value,
        gives_unit: options.unit.is_some(),
        options: options.expr(),
    };
    plan_partitioned(input, &partition, vec![(call, output)])
}

/// The field `name` of the time units of the values of `input`, which checks the options when
/// they are a constant of the plan, `options`.
pub(crate) fn window_field(
    name: &str,
    input: &Field,
    options: Option<&ScalarValue>,
) -> Result<FieldRef> {
    if let Some(options) = options {
        let options = TimeUnitOptions::of_scalar(options)?;
        Calendar::new(input.data_type(), &options)?;
    }
    Ok(result_field(name, input.data_type(), input.is_nullable()))
}

/// The time units of `value` by `options`, the struct of its options, the call at index `i` of
/// its partitions: with a `unit`, of each row, else of the unit that each partition chooses.
pub(crate) fn output(value: Expr, options: Expr, gives_unit: bool, i: usize) -> Output {
    if gives_unit {
        return Output::Row(TIMEUNIT.call(vec![value, options]));
    }
    let alias = format!("__timeunit_unit_{i}");
    let chosen = PartitionValue {
        aggregates: vec![
            CHOOSE
                .call(vec![value.clone(), options.clone()])
                .alias(&alias),
        ],
        value: Expr::Column(Column::from_name(alias)),
    };
    Output::Partition(
        chosen,
        Box::new(move |unit| APPLY.call(vec![value, unit, options])),
    )
}

/// The options of a call, the constant struct of the argument `arg`.
fn options_of(arg: &ColumnarValue) -> Result<TimeUnitOptions> {
    let array = arg.to_array(1)?;
    Ok(TimeUnitOptions::of_struct(array.as_struct(), 0)?)
}

pub(crate) static TIMEUNIT: LazyLock<ScalarUDF> = LazyLock::new(|| {
    ScalarUDF::new_from_impl(TimeUnitScalar {
        signature: Signature::any(2, Volatility::Immutable),
    })
});

/// `__timeunit(x, options)`: internal; the unit of each value by options with a `unit`.
#[derive(Debug, PartialEq, Eq, Hash)]
struct TimeUnitScalar {
    signature: Signature,
}

impl ScalarUDFImpl for TimeUnitScalar {
    fn name(&self) -> &str {
        "__timeunit"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        internal_err!(
            "{} computes its field in return_field_from_args",
            self.name()
        )
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let options = args.scalar_arguments.get(1).copied().flatten();
        window_field(self.name(), &args.arg_fields[0], options)
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let options = options_of(&args.args[1])?;
        let unit = options.unit;
        let x = args.args[0].to_array(args.number_rows)?;
        Ok(ColumnarValue::Array(apply(&x, &options, |_| unit)?))
    }
}

pub(crate) static APPLY: LazyLock<ScalarUDF> = LazyLock::new(|| {
    ScalarUDF::new_from_impl(TimeUnitApply {
        signature: Signature::any(3, Volatility::Immutable),
    })
});

/// `__timeunit_apply(x, unit, options)`: internal; the unit of each value, from `unit`, the
/// text of a unit for each row from `__timeunit_choose`, as a dictionary.
#[derive(Debug, PartialEq, Eq, Hash)]
struct TimeUnitApply {
    signature: Signature,
}

impl ScalarUDFImpl for TimeUnitApply {
    fn name(&self) -> &str {
        "__timeunit_apply"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        internal_err!(
            "{} computes its field in return_field_from_args",
            self.name()
        )
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let options = args.scalar_arguments.get(2).copied().flatten();
        window_field(self.name(), &args.arg_fields[0], options)
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let o = &options_of(&args.args[2])?;
        let x = args.args[0].to_array(args.number_rows)?;
        let units = match &args.args[1] {
            ColumnarValue::Scalar(unit) => {
                let unit = unit
                    .try_as_str()
                    .flatten()
                    .map(Unit::parse)
                    .transpose()?
                    .map(|(unit, _)| unit);
                return Ok(ColumnarValue::Array(apply(&x, o, |_| unit)?));
            }
            ColumnarValue::Array(units) => units,
        };
        // The units of a join of the chosen units to the rows share the values of the chosen
        // units, one value for each partition, so each row parses only a key it has not seen.
        let units = match units.data_type() == &unit_type() {
            true => units.clone(),
            false => cast(units, &unit_type())?,
        };
        let units = units.as_dictionary::<Int32Type>();
        let texts = units.values().as_string::<i32>();
        let mut parsed: HashMap<i32, Option<Unit>> = HashMap::new();
        let mut last: Option<(i32, Option<Unit>)> = None;
        let mut of_row = Vec::with_capacity(units.len());
        for key in units.keys() {
            let unit = match key {
                None => None,
                Some(k) if last.is_some_and(|(l, _)| l == k) => last.unwrap().1,
                Some(k) => {
                    let unit = match parsed.get(&k) {
                        Some(unit) => *unit,
                        None => {
                            let i = k as usize;
                            let unit = texts
                                .is_valid(i)
                                .then(|| Unit::parse(texts.value(i)))
                                .transpose()?
                                .map(|(unit, _)| unit);
                            parsed.insert(k, unit);
                            unit
                        }
                    };
                    last = Some((k, unit));
                    unit
                }
            };
            of_row.push(unit);
        }
        Ok(ColumnarValue::Array(apply(&x, o, |i| of_row[i])?))
    }
}

pub(crate) static CHOOSE: LazyLock<AggregateUDF> = LazyLock::new(|| {
    AggregateUDF::new_from_impl(TimeUnitChoose {
        signature: Signature::any(2, Volatility::Immutable),
    })
});

/// `__timeunit_choose(x, options)`: internal aggregate; the text of the unit that `timeunit`
/// chooses for the values by its options, a constant of the plan.
#[derive(Debug, PartialEq, Eq, Hash)]
struct TimeUnitChoose {
    signature: Signature,
}

impl AggregateUDFImpl for TimeUnitChoose {
    fn name(&self) -> &str {
        "__timeunit_choose"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(unit_type())
    }
    fn accumulator(&self, args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        // The options are a literal after the plan binds its parameters.
        let literal = (args.exprs.get(1)).and_then(|expr| expr.downcast_ref::<Literal>());
        let Some(literal) = literal else {
            return internal_err!("{} takes its options as a constant", self.name());
        };
        let options = TimeUnitOptions::of_scalar(literal.value())?;
        let cal = Calendar::new(args.expr_fields[0].data_type(), &options)?;
        Ok(Box::new(ChooseAccumulator {
            options,
            cal,
            stats: Stats::default(),
        }))
    }
    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        let f = |n: &str, t: DataType| Arc::new(Field::new(format!("{}[{n}]", args.name), t, true));
        Ok(vec![
            f("n", DataType::UInt64),
            f("min", DataType::Int64),
            f("max", DataType::Int64),
            f("rank", DataType::UInt8),
            f("gcd", DataType::UInt64),
            f("reference", DataType::Int64),
        ])
    }
}

#[derive(Debug)]
pub(super) struct ChooseAccumulator {
    pub(super) options: TimeUnitOptions,
    pub(super) cal: Calendar,
    pub(super) stats: Stats,
}

impl Accumulator for ChooseAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        self.stats.add_array(&values[0], &mut self.cal)
    }
    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Dictionary(
            Box::new(DataType::Int32),
            Box::new(ScalarValue::Utf8(
                choose(&self.stats, &self.options, self.cal.kind).map(|u| u.text()),
            )),
        ))
    }
    fn size(&self) -> usize {
        size_of_val(self)
    }
    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        let s = &self.stats;
        Ok(vec![
            ScalarValue::UInt64(Some(s.n)),
            ScalarValue::Int64(Some(s.min)),
            ScalarValue::Int64(Some(s.max)),
            ScalarValue::UInt8(Some(s.rank)),
            ScalarValue::UInt64(Some(s.gcd)),
            ScalarValue::Int64(Some(s.reference)),
        ])
    }
    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        let n: &UInt64Array = states[0].as_primitive::<UInt64Type>();
        let min: &Int64Array = states[1].as_primitive::<Int64Type>();
        let max: &Int64Array = states[2].as_primitive::<Int64Type>();
        let rank: &UInt8Array = states[3].as_primitive::<UInt8Type>();
        let g: &UInt64Array = states[4].as_primitive::<UInt64Type>();
        let r: &Int64Array = states[5].as_primitive::<Int64Type>();
        for i in 0..n.len() {
            self.stats.merge(&Stats {
                n: n.value(i),
                min: min.value(i),
                max: max.value(i),
                rank: rank.value(i),
                gcd: g.value(i),
                reference: r.value(i),
            });
        }
        Ok(())
    }
}
