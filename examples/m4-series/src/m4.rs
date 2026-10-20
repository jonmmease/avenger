//! The line's job: M4 decimation, which keeps each pixel column's first, last, lowest, and
//! highest samples, so within the view the line draws as the whole series would at the view's
//! resolution.

use arrow::array::Array;
use avenger_app::task::{BoxError, Cancel};
use avenger_datafusion_rasterize::Rasterize;
use avenger_scales::scales::linear::LinearScale;
use datafusion::functions_aggregate::expr_fn::{first_value, last_value, max, min};
use datafusion::logical_expr::{col, Expr, SortExpr};
use datafusion::prelude::DataFrame;

/// What a line was decimated for: the view's x domain and the plot's width in device pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct LineKey {
    pub x: (f64, f64),
    pub width: u32,
}

/// The samples that draw a view's line, in x order.
#[derive(Debug)]
pub struct Line {
    pub points: Vec<[f64; 2]>,
    /// The lowest and highest values in the view, or `None` when no sample lies in it.
    pub y: Option<(f64, f64)>,
}

fn by(column: &str, ascending: bool) -> Vec<SortExpr> {
    vec![col(column).sort(ascending, false)]
}

/// Each pixel column's aggregates: its first and last samples' x and y, ordered by x, and its
/// lowest and highest values with their x.
fn aggregates() -> [Expr; 8] {
    [
        first_value(col("x"), by("x", true)),
        first_value(col("y"), by("x", true)),
        last_value(col("x"), by("x", true)),
        last_value(col("y"), by("x", true)),
        min(col("y")),
        first_value(col("x"), by("y", true)),
        max(col("y")),
        first_value(col("x"), by("y", false)),
    ]
}

/// Decimate `series` for `key`'s view, with a column of aggregates for each pixel of the plot's
/// width.
pub async fn line(series: DataFrame, key: LineKey, cancel: Cancel) -> Result<Line, BoxError> {
    let scale = LinearScale::configured(key.x, (0.0, 1.0));
    let query = aggregates().into_iter().fold(
        Rasterize::new(col("x"), &scale, key.width as usize),
        Rasterize::aggregate,
    );
    let batches = query.plan(series)?.collect().await?;
    cancel.yield_now().await?;
    let planes = query.pack(&batches)?.remove(0).planes;
    let mut points = Vec::new();
    let mut y: Option<(f64, f64)> = None;
    for column in 0..planes[0].len() {
        if planes[0].is_null(column) {
            continue;
        }
        let value = |plane: usize| planes[plane].value(column);
        let (low, high) = (value(4), value(6));
        y = Some(y.map_or((low, high), |(min, max)| (min.min(low), max.max(high))));
        points.extend(column_points([
            [value(0), value(1)],
            [value(2), value(3)],
            [value(5), value(4)],
            [value(7), value(6)],
        ]));
    }
    Ok(Line { points, y })
}

/// A pixel column's first, last, lowest, and highest samples as `[x, y]`, in x order, each
/// sample once.
pub fn column_points(mut samples: [[f64; 2]; 4]) -> Vec<[f64; 2]> {
    samples.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut points = samples.to_vec();
    points.dedup();
    points
}
