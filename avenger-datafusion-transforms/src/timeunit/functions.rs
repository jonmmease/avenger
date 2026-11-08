//! The window function, the internal scalar functions, and the aggregate that chooses units.

use super::*;

use std::collections::HashMap;
use std::sync::Arc;

use crate::utils::{physical_literals, scalar_args};
use datafusion::arrow::array::{Array, ArrayRef, AsArray, Int64Array, UInt8Array, UInt64Array};
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{
    DataType, Field, FieldRef, Int32Type, Int64Type, UInt8Type, UInt64Type,
};
use datafusion::common::{Result, ScalarValue, internal_err, plan_err};
use datafusion::logical_expr::function::{
    AccumulatorArgs, PartitionEvaluatorArgs, StateFieldsArgs, WindowUDFFieldArgs,
};
use datafusion::logical_expr::{
    Accumulator, AggregateUDFImpl, ColumnarValue, PartitionEvaluator, ReturnFieldArgs,
    ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility, WindowUDFImpl,
};

/// `timeunit(x, ..options) OVER (..)`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct TimeUnitWindow {
    pub(super) signature: Signature,
}

impl Default for TimeUnitWindow {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

impl WindowUDFImpl for TimeUnitWindow {
    fn name(&self) -> &str {
        "timeunit"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn partition_evaluator(
        &self,
        args: PartitionEvaluatorArgs,
    ) -> Result<Box<dyn PartitionEvaluator>> {
        let options =
            TimeUnitOptions::parse(&physical_literals("timeunit", &args.input_exprs()[1..])?)?;
        options.check(args.input_fields()[0].data_type())?;
        Ok(Box::new(TimeUnitEvaluator { options }))
    }
    fn field(&self, args: WindowUDFFieldArgs) -> Result<FieldRef> {
        let Some(x) = args.input_fields().first() else {
            return plan_err!("timeunit needs a value");
        };
        Calendar::new(x.data_type(), &TimeUnitOptions::default())?;
        // A window function does not see its constant arguments here, so the metadata
        // cannot hold the options. The rule plans the call as `__timeunit_apply`, which
        // sets them.
        Ok(result_field(
            args.name(),
            x.data_type(),
            x.is_nullable(),
            "{}".to_string(),
        ))
    }
}

#[derive(Debug)]
pub(super) struct TimeUnitEvaluator {
    pub(super) options: TimeUnitOptions,
}

impl PartitionEvaluator for TimeUnitEvaluator {
    fn evaluate_all(&mut self, values: &[ArrayRef], _num_rows: usize) -> Result<ArrayRef> {
        let x = &values[0];
        let unit = match self.options.unit {
            Some(u) => Some(u),
            None => {
                let mut cal = Calendar::new(x.data_type(), &self.options)?;
                let stats = Stats::of(x, &mut cal)?;
                choose(&stats, &self.options, cal.kind)
            }
        };
        apply(x, &self.options, |_| unit)
    }
}

/// `__timeunit(x, ..options)`: the scalar form, which needs a `unit`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct TimeUnitScalar {
    pub(super) signature: Signature,
}

impl Default for TimeUnitScalar {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

pub(super) fn scalar_options(
    args: &ReturnFieldArgs,
    from: usize,
) -> Result<Option<TimeUnitOptions>> {
    let options: Option<Vec<ScalarValue>> = args.scalar_arguments[from..]
        .iter()
        .map(|v| v.cloned())
        .collect();
    options.map(|o| TimeUnitOptions::parse(&o)).transpose()
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
        let Some(x) = args.arg_fields.first() else {
            return plan_err!("timeunit needs a value");
        };
        let o = scalar_options(&args, 1)?.unwrap_or_default();
        if o.unit.is_none() && args.scalar_arguments[1..].iter().all(|v| v.is_some()) {
            return plan_err!(
                "timeunit without 'unit' chooses a unit from other rows, so it needs an OVER clause, for example timeunit(t) OVER ()"
            );
        }
        Calendar::new(x.data_type(), &o)?;
        Ok(result_field(
            self.name(),
            x.data_type(),
            x.is_nullable(),
            o.tag(),
        ))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let o = TimeUnitOptions::parse(&scalar_args("timeunit", &args.args[1..])?)?;
        let Some(unit) = o.unit else {
            return plan_err!("timeunit without 'unit' needs an OVER clause");
        };
        let x = args.args[0].to_array(args.number_rows)?;
        Ok(ColumnarValue::Array(apply(&x, &o, |_| Some(unit))?))
    }
}

/// `__timeunit_apply(x, unit, ..options)`: internal; `unit` is the text of a unit for each
/// row, from `__timeunit_choose`, as a dictionary.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct TimeUnitApply {
    pub(super) signature: Signature,
}

impl Default for TimeUnitApply {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
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
        let x = &args.arg_fields[0];
        let o = scalar_options(&args, 2)?.unwrap_or_default();
        Ok(result_field(
            self.name(),
            x.data_type(),
            x.is_nullable(),
            o.tag(),
        ))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let o = TimeUnitOptions::parse(&scalar_args("timeunit", &args.args[2..])?)?;
        let x = args.args[0].to_array(args.number_rows)?;
        let units = match &args.args[1] {
            ColumnarValue::Scalar(unit) => {
                let unit = unit.try_as_str().flatten().map(Unit::parse).transpose()?;
                return Ok(ColumnarValue::Array(apply(&x, &o, |_| unit)?));
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
                                .transpose()?;
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
        Ok(ColumnarValue::Array(apply(&x, &o, |i| of_row[i])?))
    }
}

/// `__timeunit_choose(x, ..options)`: internal aggregate; the text of the unit that
/// `timeunit` chooses for the values.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct TimeUnitChoose {
    pub(super) signature: Signature,
}

impl Default for TimeUnitChoose {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
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
        let options = TimeUnitOptions::parse(&physical_literals("timeunit", &args.exprs[1..])?)?;
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
