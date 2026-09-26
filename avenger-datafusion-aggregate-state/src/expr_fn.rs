//! Logical expression constructors. Validation runs during DataFusion planning.
use crate::functions;
use datafusion::logical_expr::Expr;

macro_rules! expression {
    ($name:ident, $factory:ident, $doc:literal) => {
        #[doc = $doc]
        pub fn $name(value: Expr) -> Expr {
            functions::$factory().call(vec![value])
        }
    };
}

expression!(
    count_state,
    count_state_udaf,
    "Count non-null values and return a count state."
);
expression!(
    count_merge,
    count_merge_udaf,
    "Merge count states and return the final result."
);
expression!(
    count_merge_state,
    count_merge_state_udaf,
    "Merge count states into another state."
);
expression!(
    count_finalize,
    count_finalize_udf,
    "Finalize each count state independently."
);

expression!(
    sum_state,
    sum_state_udaf,
    "Accumulate values into a sum state."
);
expression!(
    sum_merge,
    sum_merge_udaf,
    "Merge sum states and return the final result."
);
expression!(
    sum_merge_state,
    sum_merge_state_udaf,
    "Merge sum states into another state."
);
expression!(
    sum_finalize,
    sum_finalize_udf,
    "Finalize each sum state independently."
);

expression!(
    min_state,
    min_state_udaf,
    "Accumulate values into a minimum state."
);
expression!(
    min_merge,
    min_merge_udaf,
    "Merge minimum states and return the final result."
);
expression!(
    min_merge_state,
    min_merge_state_udaf,
    "Merge minimum states into another state."
);
expression!(
    min_finalize,
    min_finalize_udf,
    "Finalize each minimum state independently."
);

expression!(
    max_state,
    max_state_udaf,
    "Accumulate values into a maximum state."
);
expression!(
    max_merge,
    max_merge_udaf,
    "Merge maximum states and return the final result."
);
expression!(
    max_merge_state,
    max_merge_state_udaf,
    "Merge maximum states into another state."
);
expression!(
    max_finalize,
    max_finalize_udf,
    "Finalize each maximum state independently."
);

expression!(
    avg_state,
    avg_state_udaf,
    "Accumulate values into an average state."
);
expression!(
    avg_merge,
    avg_merge_udaf,
    "Merge average states and return the final result."
);
expression!(
    avg_merge_state,
    avg_merge_state_udaf,
    "Merge average states into another state."
);
expression!(
    avg_finalize,
    avg_finalize_udf,
    "Finalize each average state independently."
);

expression!(
    var_samp_state,
    var_samp_state_udaf,
    "Accumulate values into a sample variance state."
);
expression!(
    var_samp_merge,
    var_samp_merge_udaf,
    "Merge sample variance states and return the final result."
);
expression!(
    var_samp_merge_state,
    var_samp_merge_state_udaf,
    "Merge sample variance states into another state."
);
expression!(
    var_samp_finalize,
    var_samp_finalize_udf,
    "Finalize each sample variance state independently."
);

expression!(
    var_pop_state,
    var_pop_state_udaf,
    "Accumulate values into a population variance state."
);
expression!(
    var_pop_merge,
    var_pop_merge_udaf,
    "Merge population variance states and return the final result."
);
expression!(
    var_pop_merge_state,
    var_pop_merge_state_udaf,
    "Merge population variance states into another state."
);
expression!(
    var_pop_finalize,
    var_pop_finalize_udf,
    "Finalize each population variance state independently."
);

expression!(
    stddev_samp_state,
    stddev_samp_state_udaf,
    "Accumulate values into a sample standard deviation state."
);
expression!(
    stddev_samp_merge,
    stddev_samp_merge_udaf,
    "Merge sample standard deviation states and return the final result."
);
expression!(
    stddev_samp_merge_state,
    stddev_samp_merge_state_udaf,
    "Merge sample standard deviation states into another state."
);
expression!(
    stddev_samp_finalize,
    stddev_samp_finalize_udf,
    "Finalize each sample standard deviation state independently."
);

expression!(
    stddev_pop_state,
    stddev_pop_state_udaf,
    "Accumulate values into a population standard deviation state."
);
expression!(
    stddev_pop_merge,
    stddev_pop_merge_udaf,
    "Merge population standard deviation states and return the final result."
);
expression!(
    stddev_pop_merge_state,
    stddev_pop_merge_state_udaf,
    "Merge population standard deviation states into another state."
);
expression!(
    stddev_pop_finalize,
    stddev_pop_finalize_udf,
    "Finalize each population standard deviation state independently."
);

/// Count input rows, including rows with null fields.
pub fn count_star_state() -> Expr {
    count_state(datafusion::logical_expr::lit(1_i64))
}
