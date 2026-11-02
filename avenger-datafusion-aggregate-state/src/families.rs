use std::sync::Arc;

use datafusion::{
    arrow::datatypes::DataType,
    common::{plan_err, Result},
    functions_aggregate::{average, count, min_max, stddev, sum, variance},
    logical_expr::{type_coercion::functions::fields_with_udf, AggregateUDF},
};

use crate::state::input_fields;

/// An aggregate whose state the crate exposes: one of DataFusion's native aggregates of the same
/// name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Count,
    Sum,
    Min,
    Max,
    Avg,
    VarSamp,
    VarPop,
    StddevSamp,
    StddevPop,
}

impl Family {
    /// Every family.
    pub const ALL: [Self; 9] = [
        Self::Count,
        Self::Sum,
        Self::Min,
        Self::Max,
        Self::Avg,
        Self::VarSamp,
        Self::VarPop,
        Self::StddevSamp,
        Self::StddevPop,
    ];

    /// The name of the native aggregate, which prefixes the names of the family's functions, such
    /// as `var_samp` in `var_samp_state`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Min => "min",
            Self::Max => "max",
            Self::Avg => "avg",
            Self::VarSamp => "var_samp",
            Self::VarPop => "var_pop",
            Self::StddevSamp => "stddev_samp",
            Self::StddevPop => "stddev_pop",
        }
    }

    pub(crate) fn native(self) -> Arc<AggregateUDF> {
        match self {
            Self::Count => count::count_udaf(),
            Self::Sum => sum::sum_udaf(),
            Self::Min => min_max::min_udaf(),
            Self::Max => min_max::max_udaf(),
            Self::Avg => average::avg_udaf(),
            Self::VarSamp => variance::var_samp_udaf(),
            Self::VarPop => variance::var_pop_udaf(),
            Self::StddevSamp => stddev::stddev_udaf(),
            Self::StddevPop => stddev::stddev_pop_udaf(),
        }
    }

    pub(crate) fn coerce(self, types: &[DataType]) -> Result<Vec<DataType>> {
        if types.is_empty() && self == Self::Count {
            return Ok(vec![]);
        }
        if types.len() != 1 {
            return plan_err!("{}_state expects one argument", self.name());
        }
        if self != Self::Count
            && matches!(
                types[0],
                DataType::List(_)
                    | DataType::LargeList(_)
                    | DataType::FixedSizeList(_, _)
                    | DataType::ListView(_)
                    | DataType::LargeListView(_)
                    | DataType::Struct(_)
                    | DataType::Map(_, _)
                    | DataType::Union(_, _)
                    | DataType::RunEndEncoded(_, _)
            )
        {
            return plan_err!("{}_state does not support nested arguments", self.name());
        }
        Ok(
            fields_with_udf(&input_fields(types), self.native().as_ref())?
                .iter()
                .map(|f| f.data_type().clone())
                .collect(),
        )
    }

    pub(crate) fn moments(self) -> bool {
        matches!(
            self,
            Self::VarSamp | Self::VarPop | Self::StddevSamp | Self::StddevPop
        )
    }
}
