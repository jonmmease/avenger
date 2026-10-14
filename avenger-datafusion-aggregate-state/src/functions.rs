//! Function factories for selective registration or custom expression builders.
use crate::{
    aggregate::{Operation, StateAggregate},
    families::Family,
    finalize::Finalize,
};
use datafusion::logical_expr::{AggregateUDF, ScalarUDF};
use std::sync::{Arc, LazyLock};

macro_rules! aggregate_factory {
    ($name:ident, $family:ident, $operation:ident, $doc:expr) => {
        #[doc = $doc]
        pub fn $name() -> Arc<AggregateUDF> {
            static FUNCTION: LazyLock<Arc<AggregateUDF>> = LazyLock::new(|| {
                Arc::new(AggregateUDF::from(StateAggregate::new(
                    Family::$family,
                    Operation::$operation,
                )))
            });
            Arc::clone(&FUNCTION)
        }
    };
}

macro_rules! family_factories {
    ($family:ident, $description:literal, $state:ident, $merge:ident, $merge_state:ident, $finalize:ident) => {
        aggregate_factory!(
            $state,
            $family,
            State,
            concat!("Return the ", $description, " State aggregate function.")
        );
        aggregate_factory!(
            $merge,
            $family,
            Merge,
            concat!("Return the ", $description, " Merge aggregate function.")
        );
        aggregate_factory!(
            $merge_state,
            $family,
            MergeState,
            concat!(
                "Return the ",
                $description,
                " MergeState aggregate function."
            )
        );
        #[doc = concat!("Return the ", $description, " Finalize scalar function.")]
        pub fn $finalize() -> Arc<ScalarUDF> {
            static FUNCTION: LazyLock<Arc<ScalarUDF>> =
                LazyLock::new(|| Arc::new(ScalarUDF::from(Finalize::new(Family::$family))));
            Arc::clone(&FUNCTION)
        }
    };
}

family_factories!(
    Count,
    "count",
    count_state_udaf,
    count_merge_udaf,
    count_merge_state_udaf,
    count_finalize_udf
);
family_factories!(
    Sum,
    "sum",
    sum_state_udaf,
    sum_merge_udaf,
    sum_merge_state_udaf,
    sum_finalize_udf
);
family_factories!(
    Min,
    "minimum",
    min_state_udaf,
    min_merge_udaf,
    min_merge_state_udaf,
    min_finalize_udf
);
family_factories!(
    Max,
    "maximum",
    max_state_udaf,
    max_merge_udaf,
    max_merge_state_udaf,
    max_finalize_udf
);
family_factories!(
    Avg,
    "average",
    avg_state_udaf,
    avg_merge_udaf,
    avg_merge_state_udaf,
    avg_finalize_udf
);
family_factories!(
    VarSamp,
    "sample variance",
    var_samp_state_udaf,
    var_samp_merge_udaf,
    var_samp_merge_state_udaf,
    var_samp_finalize_udf
);
family_factories!(
    VarPop,
    "population variance",
    var_pop_state_udaf,
    var_pop_merge_udaf,
    var_pop_merge_state_udaf,
    var_pop_finalize_udf
);
family_factories!(
    StddevSamp,
    "sample standard deviation",
    stddev_samp_state_udaf,
    stddev_samp_merge_udaf,
    stddev_samp_merge_state_udaf,
    stddev_samp_finalize_udf
);
family_factories!(
    StddevPop,
    "population standard deviation",
    stddev_pop_state_udaf,
    stddev_pop_merge_udaf,
    stddev_pop_merge_state_udaf,
    stddev_pop_finalize_udf
);
