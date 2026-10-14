use std::sync::Arc;

use arrow::array::{Array, AsArray};
use datafusion::common::{exec_err, Result, ScalarValue};
use datafusion::logical_expr::{lit, AggregateUDF, Expr, ScalarUDF};

use super::config::{config_at, config_struct, domain_options_struct, options_struct};
use super::fit::FitUdf;
use super::scale::ScaleUdf;
use super::summary::SummaryUdaf;
use crate::scales::domain_summary::DomainOptions;
use crate::scales::{ConfiguredScale, ScaleConfig, ScaleImpl};

/// A scale configuration as an expression: a struct of its `domain` and `range`, each a list, and
/// its `options`. [`scale_expr`] takes configurations in this form, and [`fit_expr`] returns them.
pub fn config_expr(config: &ScaleConfig) -> Result<Expr> {
    Ok(lit(ScalarValue::Struct(Arc::new(config_struct(config)?))))
}

/// The configuration at `row` of an array of configurations in the form of [`config_expr`],
/// such as a column that [`fit_expr`] computes.
pub fn config_from_arrow(array: &dyn Array, row: usize) -> Result<ScaleConfig> {
    let Some(configs) = array.as_struct_opt() else {
        return exec_err!("a scale configuration is a struct");
    };
    if row >= configs.len() || configs.is_null(row) {
        return exec_err!("no scale configuration at row {row}");
    }
    config_at(configs, row)
}

/// `scale` applied to `values`, with its configuration from `config`, an expression of the form of
/// [`config_expr`]. Rows with equal configurations are scaled together, so the configuration
/// can be a literal, a scalar subquery of a [`fit_expr`], or a column of fitted configurations,
/// such as one for each facet. A row with a null configuration has a null result. The result has
/// the scale's [`output_type`](ScaleImpl::output_type).
pub fn scale_expr(scale: &dyn ScaleImpl, values: Expr, config: Expr) -> Result<Expr> {
    let udf = ScalarUDF::new_from_impl(ScaleUdf::new(scale)?);
    Ok(udf.call(vec![values, config]))
}

/// An aggregate that summarizes `values` for fitting the domain of `scale`, with the scale's
/// options. `extents` are the lower and upper pixel extents of the marks drawn at the values.
/// Missing values are skipped with their extents, and a missing extent of a value is an error. The
/// result is the summary in its Arrow form, and no values give the summary of no data.
pub fn summary_expr(
    scale: &ConfiguredScale,
    values: Expr,
    extents: Option<(Expr, Expr)>,
) -> Result<Expr> {
    let udaf = AggregateUDF::new_from_impl(SummaryUdaf::new(scale.scale_impl.as_ref())?);
    let options = lit(ScalarValue::Struct(Arc::new(options_struct(
        &scale.config.options,
    )?)));
    let mut args = vec![values, options];
    if let Some((lower, upper)) = extents {
        args.extend([lower, upper]);
    }
    Ok(udaf.call(args))
}

/// `scale`'s configuration with its domain fitted to `summary`, a summary in its Arrow form such
/// as the result of [`summary_expr`], following `domain_options`. The result has the form of
/// [`config_expr`], for [`scale_expr`] or [`config_from_arrow`]. An empty domain or explicit
/// categories in `domain_options` are cast to the type of the fitted domain.
pub fn fit_expr(
    scale: &ConfiguredScale,
    summary: Expr,
    domain_options: &DomainOptions,
) -> Result<Expr> {
    let udf = ScalarUDF::new_from_impl(FitUdf::new(scale.scale_impl.as_ref())?);
    let domain_options = lit(ScalarValue::Struct(Arc::new(domain_options_struct(
        domain_options,
    )?)));
    Ok(udf.call(vec![summary, config_expr(&scale.config)?, domain_options]))
}
