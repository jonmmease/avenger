use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, BooleanArray, UInt64Array},
        compute::filter,
        datatypes::{DataType, Field, FieldRef},
    },
    common::{plan_err, Result, ScalarValue},
    logical_expr::{
        function::{AccumulatorArgs, AggregateFunctionSimplification, StateFieldsArgs},
        Accumulator, AggregateUDFImpl, EmitTo, Expr, GroupsAccumulator, Signature, Volatility,
    },
};

use crate::{families::Family, state::StateType};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Operation {
    State,
    Merge,
    MergeState,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct StateAggregate {
    pub family: Family,
    pub operation: Operation,
    name: String,
    aliases: Vec<String>,
    signature: Signature,
}

impl StateAggregate {
    pub fn new(family: Family, operation: Operation) -> Self {
        let suffix = match operation {
            Operation::State => "State",
            Operation::Merge => "Merge",
            Operation::MergeState => "MergeState",
        };
        let name = format!("{}{suffix}", family.prefix());
        Self {
            aliases: vec![name.to_lowercase()],
            name,
            family,
            operation,
            signature: Signature::user_defined(Volatility::Immutable),
        }
    }

    fn state_type(&self, types: &[DataType]) -> Result<StateType> {
        if self.operation == Operation::State {
            StateType::new(self.family, types)
        } else {
            StateType::args_type(self.family, types)
        }
    }

    fn check_args(&self, args: &AccumulatorArgs) -> Result<()> {
        if args.is_distinct || !args.order_bys.is_empty() || args.ignore_nulls {
            return plan_err!(
                "{} does not support DISTINCT, ORDER BY, or null-treatment modifiers",
                self.name
            );
        }
        Ok(())
    }
}

impl AggregateUDFImpl for StateAggregate {
    fn name(&self) -> &str {
        &self.name
    }
    fn aliases(&self) -> &[String] {
        &self.aliases
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        if self.operation == Operation::State {
            self.family.coerce(types)
        } else {
            StateType::args_type(self.family, types)?;
            Ok(types.to_vec())
        }
    }
    fn return_type(&self, types: &[DataType]) -> Result<DataType> {
        let state = self.state_type(types)?;
        Ok(if self.operation == Operation::Merge {
            state.result.data_type().clone()
        } else {
            state.data_type
        })
    }
    fn is_nullable(&self) -> bool {
        self.operation == Operation::Merge && self.family != Family::Count
    }
    fn return_field(&self, inputs: &[FieldRef]) -> Result<FieldRef> {
        Ok(Arc::new(Field::new(
            &self.name,
            self.return_type(
                &inputs
                    .iter()
                    .map(|f| f.data_type().clone())
                    .collect::<Vec<_>>(),
            )?,
            self.is_nullable(),
        )))
    }
    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        if args.is_distinct || !args.ordering_fields.is_empty() {
            return plan_err!("{} does not support DISTINCT or ORDER BY", self.name);
        }
        let state = self.state_type(
            &args
                .input_fields
                .iter()
                .map(|f| f.data_type().clone())
                .collect::<Vec<_>>(),
        )?;
        // Physical partial states have native columns. Only logical results are wrapped.
        Ok(state
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| Arc::new(f.as_ref().clone().with_name(format!("{}[{i}]", args.name))))
            .collect())
    }
    fn accumulator(&self, args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        self.check_args(&args)?;
        let state = self.state_type(
            &args
                .expr_fields
                .iter()
                .map(|f| f.data_type().clone())
                .collect::<Vec<_>>(),
        )?;
        Ok(Box::new(StateAccumulator {
            inner: state.accumulator()?,
            state,
            operation: self.operation,
        }))
    }
    fn groups_accumulator_supported(&self, args: AccumulatorArgs) -> bool {
        self.check_args(&args).is_ok()
            && self
                .state_type(
                    &args
                        .expr_fields
                        .iter()
                        .map(|f| f.data_type().clone())
                        .collect::<Vec<_>>(),
                )
                .is_ok_and(|s| s.groups_supported())
    }
    fn create_groups_accumulator(
        &self,
        args: AccumulatorArgs,
    ) -> Result<Box<dyn GroupsAccumulator>> {
        self.check_args(&args)?;
        let state = self.state_type(
            &args
                .expr_fields
                .iter()
                .map(|f| f.data_type().clone())
                .collect::<Vec<_>>(),
        )?;
        Ok(Box::new(StateGroups {
            inner: state.groups()?,
            state,
            operation: self.operation,
        }))
    }
    fn default_value(&self, data_type: &DataType) -> Result<ScalarValue> {
        if self.operation == Operation::Merge {
            return self.family.native().default_value(data_type);
        }
        let state = StateType::from_type(self.family, data_type)?;
        state.pack_scalar(state.accumulator()?.state()?)
    }
    fn simplify(&self) -> Option<AggregateFunctionSimplification> {
        let count = self.family == Family::Count && self.operation == Operation::State;
        Some(Box::new(move |mut expr, _| {
            if expr.params.distinct
                || !expr.params.order_by.is_empty()
                || expr.params.null_treatment.is_some()
            {
                return plan_err!("Aggregate-state functions do not support DISTINCT, ORDER BY, or null-treatment modifiers");
            }
            if count && expr.params.args.is_empty() {
                expr.params.args.push(datafusion::logical_expr::lit(1_i64));
            }
            Ok(Expr::AggregateFunction(expr))
        }))
    }
}

#[derive(Debug)]
struct StateAccumulator {
    inner: Box<dyn Accumulator>,
    state: StateType,
    operation: Operation,
}

impl Accumulator for StateAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        if self.operation == Operation::State {
            self.inner.update_batch(values)
        } else {
            let (columns, mask) = self.state.unpack(values, None)?;
            self.inner.merge_batch(&filtered(&columns, mask.as_ref())?)
        }
    }
    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        self.inner.merge_batch(states)
    }
    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        self.inner.state()
    }
    fn evaluate(&mut self) -> Result<ScalarValue> {
        if self.operation == Operation::Merge {
            self.inner.evaluate()
        } else {
            self.state.pack_scalar(self.inner.state()?)
        }
    }
    fn size(&self) -> usize {
        size_of::<Self>() + self.inner.size() + self.state.heap_size()
    }
}

pub(crate) struct StateGroups {
    pub inner: Box<dyn GroupsAccumulator>,
    pub state: StateType,
    pub operation: Operation,
}

/// Native merge implementations may ignore filters intended for raw update calls.
pub(crate) fn merge_filtered(
    family: Family,
    inner: &mut dyn GroupsAccumulator,
    columns: &[ArrayRef],
    indices: &[usize],
    mask: Option<&BooleanArray>,
    total: usize,
) -> Result<()> {
    // Native grouped AVG treats a non-null zero count as an observed value.
    // Scalar empty states export precisely that count; skip them before merge
    // to avoid 0/0 (or a decimal division-by-zero error).
    let average_mask = if family == Family::Avg {
        let counts = columns[0]
            .as_any()
            .downcast_ref::<UInt64Array>()
            .expect("checked average layout");
        counts.iter().any(|c| c == Some(0)).then(|| {
            BooleanArray::from_iter((0..counts.len()).map(|i| {
                Some(
                    counts.is_valid(i)
                        && counts.value(i) != 0
                        && mask.is_none_or(|m| m.is_valid(i) && m.value(i)),
                )
            }))
        })
    } else {
        None
    };
    let mask = average_mask.as_ref().or(mask);
    if !family.moments() && family != Family::Count {
        // These native families honor masks. Keeping filtered group indices
        // lets their validity tracking retain groups with no contributing rows.
        return inner.merge_batch(columns, indices, mask, total);
    }
    if let Some(mask) = mask.filter(|m| m.true_count() != m.len()) {
        let indices: Vec<usize> = indices
            .iter()
            .zip(mask.iter())
            .filter_map(|(&i, v)| (v == Some(true)).then_some(i))
            .collect();
        inner.merge_batch(&filtered(columns, Some(mask))?, &indices, None, total)
    } else {
        inner.merge_batch(columns, indices, None, total)
    }
}

fn filtered(columns: &[ArrayRef], mask: Option<&BooleanArray>) -> Result<Vec<ArrayRef>> {
    if let Some(mask) = mask.filter(|m| m.true_count() != m.len()) {
        columns
            .iter()
            .map(|c| Ok(filter(c.as_ref(), mask)?))
            .collect()
    } else {
        Ok(columns.to_vec())
    }
}

impl GroupsAccumulator for StateGroups {
    fn update_batch(
        &mut self,
        values: &[ArrayRef],
        indices: &[usize],
        filter: Option<&BooleanArray>,
        total: usize,
    ) -> Result<()> {
        if self.operation == Operation::State {
            self.inner.update_batch(values, indices, filter, total)
        } else {
            let (columns, mask) = self.state.unpack(values, filter)?;
            merge_filtered(
                self.state.family,
                self.inner.as_mut(),
                &columns,
                indices,
                mask.as_ref(),
                total,
            )
        }
    }
    fn merge_batch(
        &mut self,
        states: &[ArrayRef],
        indices: &[usize],
        filter: Option<&BooleanArray>,
        total: usize,
    ) -> Result<()> {
        merge_filtered(
            self.state.family,
            self.inner.as_mut(),
            states,
            indices,
            filter,
            total,
        )
    }
    fn state(&mut self, emit: EmitTo) -> Result<Vec<ArrayRef>> {
        self.inner.state(emit)
    }
    fn evaluate(&mut self, emit: EmitTo) -> Result<ArrayRef> {
        if self.operation == Operation::Merge {
            self.inner.evaluate(emit)
        } else {
            self.state.pack(self.inner.state(emit)?)
        }
    }
    fn supports_convert_to_state(&self) -> bool {
        self.operation == Operation::State && self.inner.supports_convert_to_state()
    }
    fn convert_to_state(
        &self,
        values: &[ArrayRef],
        filter: Option<&BooleanArray>,
    ) -> Result<Vec<ArrayRef>> {
        if self.operation != Operation::State {
            return datafusion::common::not_impl_err!(
                "Input-to-state conversion is only supported for State aggregates"
            );
        }
        // This optimization feeds a physical final stage, which expects native
        // state columns rather than the exported logical struct.
        self.inner.convert_to_state(values, filter)
    }
    fn size(&self) -> usize {
        size_of::<Self>() + self.inner.size() + self.state.heap_size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::Float64Array;

    #[test]
    fn input_conversion_produces_native_partial_columns() -> Result<()> {
        let state = StateType::new(Family::Avg, &[DataType::Float64])?;
        let groups = StateGroups {
            inner: state.groups()?,
            state: state.clone(),
            operation: Operation::State,
        };
        assert!(groups.supports_convert_to_state());
        let values: ArrayRef = Arc::new(Float64Array::from(vec![Some(1.), None, Some(3.)]));
        let mask = BooleanArray::from(vec![false, true, true]);
        let partial = groups.convert_to_state(&[values], Some(&mask))?;
        assert_eq!(partial.len(), state.fields.len());
        let mut final_group = StateGroups {
            inner: state.groups()?,
            state,
            operation: Operation::Merge,
        };
        assert!(!final_group.supports_convert_to_state());
        final_group.merge_batch(&partial, &[0, 1, 2], None, 3)?;
        let actual = final_group.evaluate(EmitTo::All)?;
        assert_eq!(
            actual.as_ref(),
            &Float64Array::from(vec![None, None, Some(3.)])
        );
        Ok(())
    }

    #[test]
    fn partial_emission_keeps_retained_states_independent() -> Result<()> {
        for family in Family::ALL {
            let state = StateType::new(family, &[DataType::Float64])?;
            let mut groups = StateGroups {
                inner: state.groups()?,
                state: state.clone(),
                operation: Operation::State,
            };
            let before = groups.size();
            let input: ArrayRef = Arc::new(Float64Array::from(vec![
                Some(2.),
                Some(4.),
                None,
                Some(6.),
                Some(10.),
            ]));
            groups.update_batch(&[input], &[0, 0, 1, 2, 2], None, 3)?;
            assert!(groups.size() >= before);
            let first = groups.evaluate(EmitTo::First(1))?;
            let saved = ScalarValue::try_from_array(&first, 0)?;
            groups.update_batch(
                &[Arc::new(Float64Array::from(vec![None, Some(12.)]))],
                &[0, 1],
                None,
                2,
            )?;
            let rest = groups.evaluate(EmitTo::All)?;
            assert_eq!(rest.len(), 2);
            assert_eq!(ScalarValue::try_from_array(&first, 0)?, saved);
            assert!(groups.size() <= before + 128);

            // Merge the emitted batches into one result, exercising both packed
            // external state and the native internal partial-state protocol.
            let mut merge = StateGroups {
                inner: state.groups()?,
                state: state.clone(),
                operation: Operation::MergeState,
            };
            merge.update_batch(&[first], &[0], None, 1)?;
            merge.update_batch(&[rest], &[0, 0], None, 1)?;
            let partial = merge.state(EmitTo::All)?;
            let mut final_group = StateGroups {
                inner: state.groups()?,
                state: state.clone(),
                operation: Operation::Merge,
            };
            final_group.merge_batch(&partial, &[0], None, 1)?;
            let actual = ScalarValue::try_from_array(&final_group.evaluate(EmitTo::All)?, 0)?;
            let mut native = state.accumulator()?;
            native.update_batch(&[Arc::new(Float64Array::from(vec![2., 4., 6., 10., 12.]))])?;
            let expected = native.evaluate()?;
            match (&actual, &expected) {
                (ScalarValue::Float64(Some(a)), ScalarValue::Float64(Some(b))) => {
                    assert!((a - b).abs() < 1e-10)
                }
                _ => assert_eq!(actual, expected),
            }
        }
        Ok(())
    }
}
