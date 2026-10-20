//! Pixel rasters of a DataFrame's rows, aggregated in DataFusion.
//!
//! [`Rasterize`] maps each row's x, and optionally its y, to pixels through avenger-scales'
//! scales, and aggregates the rows of each pixel cell with any DataFusion aggregate, such as
//! `count`, `avg`, or a `first_value` ordered by another column. Each partition aggregates its
//! rows into a dense grid of the aggregate's own groups state, with no hash table, and the
//! final aggregation merges the partitions' grids. [`Rasterize::pack`] turns the result into
//! dense planes, one for each aggregate.

mod udaf;

use std::cmp::Ordering;
use std::sync::Arc;

use arrow::array::{Array, AsArray, Float64Array, RecordBatch};
use arrow::compute::{cast, SortOptions};
use arrow::datatypes::{DataType, Float64Type};
use avenger_scales::datafusion::{config_expr, scale_expr};
use avenger_scales::scales::ConfiguredScale;
use datafusion::common::{internal_err, plan_err, Result, ScalarValue};
use datafusion::logical_expr::expr::{AggregateFunction, NullTreatment};
use datafusion::logical_expr::{AggregateUDF, Expr};
use datafusion::prelude::DataFrame;

use crate::udaf::{Grid, RasterizeUdaf};

/// Rasterizes a DataFrame's rows: bins them into a grid of pixel cells and aggregates the rows
/// of each cell.
///
/// The query sets each scale's range to the grid's pixels: x from 0 at the left to the grid's
/// width, and y from the grid's height at the bottom to 0 at the top, so row 0 of a raster is
/// its top. It also turns each scale's `clamp` option off, so rows outside the scales' domains
/// count nowhere. A row's cell is its floored pixels, and a row at the grid's far edge counts in
/// the last cell.
#[derive(Debug, Clone)]
pub struct Rasterize {
    x: Binned,
    y: Option<Binned>,
    width: usize,
    height: usize,
    aggregates: Vec<Expr>,
    group_by: Vec<Expr>,
}

/// An expression with the scale that maps it to pixels.
#[derive(Debug, Clone)]
struct Binned {
    values: Expr,
    scale: ConfiguredScale,
}

impl Binned {
    /// The values' pixels through the scale, with the range from `start` to `end` and no clamp.
    fn pixels(&self, start: f64, end: f64) -> Result<Expr> {
        let mut scale = self.scale.clone().with_range_interval((start, end));
        let definitions = scale.scale_impl.option_definitions();
        if definitions.iter().any(|option| option.name == "clamp") {
            scale
                .config
                .options
                .insert("clamp".to_string(), false.into());
        }
        scale_expr(
            scale.scale_impl.as_ref(),
            self.values.clone(),
            config_expr(&scale.config)?,
        )
    }
}

impl Rasterize {
    /// A query that bins rows into `width` columns by `x` through `scale`, in a raster one row
    /// high.
    pub fn new(x: Expr, scale: &ConfiguredScale, width: usize) -> Self {
        Self {
            x: Binned {
                values: x,
                scale: scale.clone(),
            },
            y: None,
            width,
            height: 1,
            aggregates: Vec::new(),
            group_by: Vec::new(),
        }
    }

    /// Bin rows into `height` rows by `y` through `scale` as well.
    pub fn with_y(mut self, y: Expr, scale: &ConfiguredScale, height: usize) -> Self {
        self.y = Some(Binned {
            values: y,
            scale: scale.clone(),
        });
        self.height = height;
        self
    }

    /// Aggregate each cell's rows with `aggregate`, an aggregate expression, into the next plane.
    /// Its `FILTER`, `ORDER BY`, and null treatment apply within each cell. DISTINCT aggregates
    /// and aggregates that need their rows in order, other than WITHIN GROUP aggregates such as
    /// percentiles, aren't supported.
    pub fn aggregate(mut self, aggregate: Expr) -> Self {
        self.aggregates.push(aggregate);
        self
    }

    /// Make a raster for each value of `expr`, as well as each other group-by expression. Each
    /// group holds a grid in each partition, so group-by expressions take few values, such as
    /// facets. A query whose groups are nearly as many as its rows fails, since DataFusion then
    /// passes rows to the final aggregation as their own groups, which a raster can't do.
    pub fn group_by(mut self, expr: Expr) -> Self {
        self.group_by.push(expr);
        self
    }

    /// `df`'s rows aggregated by the group-by expressions, with one column for each aggregate
    /// that holds a list of the aggregate's values over the grid's cells, for
    /// [`Rasterize::pack`].
    pub fn plan(&self, df: DataFrame) -> Result<DataFrame> {
        if self.width == 0 || self.height == 0 {
            return plan_err!("a raster has at least one column and one row");
        }
        let grid = Grid {
            width: self.width,
            height: self.height,
            binned_y: self.y.is_some(),
        };
        let mut pixels = vec![self.x.pixels(0.0, self.width as f64)?];
        if let Some(y) = &self.y {
            pixels.push(y.pixels(self.height as f64, 0.0)?);
        }
        let aggregates = self
            .aggregates
            .iter()
            .enumerate()
            .map(|(index, aggregate)| {
                Ok(raster_aggregate(aggregate, grid, &pixels)?.alias(format!("__raster_{index}")))
            })
            .collect::<Result<Vec<_>>>()?;
        df.aggregate(self.group_by.clone(), aggregates)
    }

    /// The rasters in `batches`, the collected rows of [`Rasterize::plan`]: one for each
    /// group, ordered by the groups' values, or one when the query has no group-by expressions.
    pub fn pack(&self, batches: &[RecordBatch]) -> Result<Vec<Raster>> {
        let groups = self.group_by.len();
        let cells = self.width * self.height;
        let mut rasters = Vec::new();
        for batch in batches {
            for row in 0..batch.num_rows() {
                let group = (0..groups)
                    .map(|group| ScalarValue::try_from_array(batch.column(group), row))
                    .collect::<Result<Vec<_>>>()?;
                let planes = (0..self.aggregates.len())
                    .map(|plane| {
                        let Some(lists) = batch.column(groups + plane).as_list_opt::<i32>() else {
                            return internal_err!("a raster plan's planes are lists");
                        };
                        let values = cast(&lists.value(row), &DataType::Float64)?;
                        if values.len() != cells {
                            return internal_err!("a raster plane has a value for each cell");
                        }
                        Ok(values.as_primitive::<Float64Type>().clone())
                    })
                    .collect::<Result<Vec<_>>>()?;
                rasters.push(Raster {
                    group,
                    width: self.width,
                    height: self.height,
                    planes,
                });
            }
        }
        rasters.sort_by(|a, b| a.group.partial_cmp(&b.group).unwrap_or(Ordering::Equal));
        Ok(rasters)
    }
}

/// `aggregate` over the cells of `grid`, which the rows' `pixels` give.
fn raster_aggregate(aggregate: &Expr, grid: Grid, pixels: &[Expr]) -> Result<Expr> {
    // Helpers such as `count_all` alias their aggregate, and callers can alias it again.
    let mut unaliased = aggregate;
    while let Expr::Alias(alias) = unaliased {
        unaliased = alias.expr.as_ref();
    }
    let Expr::AggregateFunction(AggregateFunction { func, params }) = unaliased else {
        return plan_err!("a raster aggregate is an aggregate function, not {aggregate}");
    };
    if params.distinct {
        return plan_err!("raster aggregates can't be DISTINCT, as {aggregate} is");
    }
    // An aggregate that ignores order takes no ordering values, as DataFusion drops its ORDER BY.
    let sensitivity = func.order_sensitivity();
    let order_by = match sensitivity.is_insensitive() {
        true => &[][..],
        false => params.order_by.as_slice(),
    };
    // A WITHIN GROUP aggregate, such as a percentile, takes its ORDER BY's direction and orders
    // its values itself, so it needs no ordered rows.
    if !order_by.is_empty() && sensitivity.hard_requires() && !func.supports_within_group_clause() {
        return plan_err!("{aggregate} needs its rows in order, which raster cells don't keep");
    }
    let ordering = order_by
        .iter()
        .map(|sort| SortOptions {
            descending: !sort.asc,
            nulls_first: sort.nulls_first,
        })
        .collect();
    let ignore_nulls = params.null_treatment == Some(NullTreatment::IgnoreNulls);
    let udaf = RasterizeUdaf::new(func.clone(), grid, ordering, ignore_nulls);
    let mut args = pixels.to_vec();
    args.extend(params.args.iter().cloned());
    args.extend(order_by.iter().map(|sort| sort.expr.clone()));
    Ok(Expr::AggregateFunction(AggregateFunction::new_udf(
        Arc::new(AggregateUDF::new_from_impl(udaf)),
        args,
        false,
        params.filter.clone(),
        Vec::new(),
        None,
    )))
}

/// One group's aggregates over the grid's cells.
#[derive(Debug, Clone)]
pub struct Raster {
    /// The values of the query's group-by expressions, in their order.
    pub group: Vec<ScalarValue>,
    pub width: usize,
    pub height: usize,
    /// One plane for each of the query's aggregates, in their order, with the cell at column `c`
    /// and row `r` at `r * width + c`. A cell that no row reached holds the aggregate's value
    /// over no rows: 0 for `count`, and no value for aggregates such as `avg` and `first_value`.
    pub planes: Vec<Float64Array>,
}
