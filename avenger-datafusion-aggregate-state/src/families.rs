use std::sync::Arc;

use datafusion::{
    arrow::datatypes::DataType,
    common::{plan_err, Result},
    functions_aggregate::{average, count, min_max, stddev, sum, variance},
    logical_expr::{type_coercion::functions::fields_with_udf, AggregateUDF},
};

use crate::state::input_fields;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Family {
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

    pub fn prefix(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Min => "min",
            Self::Max => "max",
            Self::Avg => "avg",
            Self::VarSamp => "varSamp",
            Self::VarPop => "varPop",
            Self::StddevSamp => "stddevSamp",
            Self::StddevPop => "stddevPop",
        }
    }

    pub fn native(self) -> Arc<AggregateUDF> {
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

    pub fn coerce(self, types: &[DataType]) -> Result<Vec<DataType>> {
        if types.is_empty() && self == Self::Count {
            return Ok(vec![]);
        }
        if types.len() != 1 {
            return plan_err!("{}State expects one argument", self.prefix());
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
            return plan_err!("{}State does not support nested arguments", self.prefix());
        }
        Ok(
            fields_with_udf(&input_fields(types), self.native().as_ref())?
                .iter()
                .map(|f| f.data_type().clone())
                .collect(),
        )
    }

    pub fn moments(self) -> bool {
        matches!(
            self,
            Self::VarSamp | Self::VarPop | Self::StddevSamp | Self::StddevPop
        )
    }
}
