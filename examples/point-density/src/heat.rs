//! The heatmap's job: count the points in each cell of a grid over a view, and shade the counts.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array};
use avenger_app::task::{BoxError, Cancel};
use avenger_datafusion_rasterize::Rasterize;
use avenger_image::RgbaImage;
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::log::LogScale;
use avenger_scales::scales::ConfiguredScale;
use datafusion::functions_aggregate::expr_fn::count;
use datafusion::logical_expr::{col, lit};
use datafusion::prelude::DataFrame;

/// Viridis, in nine stops from dark to light.
const VIRIDIS: [[f32; 4]; 9] = [
    [0.267, 0.004, 0.329, 1.0],
    [0.282, 0.157, 0.471, 1.0],
    [0.243, 0.286, 0.537, 1.0],
    [0.192, 0.408, 0.557, 1.0],
    [0.149, 0.510, 0.557, 1.0],
    [0.122, 0.620, 0.537, 1.0],
    [0.208, 0.718, 0.475, 1.0],
    [0.431, 0.808, 0.345, 1.0],
    [0.992, 0.906, 0.145, 1.0],
];

/// The shades that a heatmap's counts take, from one count to the largest.
const SHADES: usize = 256;

/// The heatmap's cells along each side, whatever the plot's size.
const CELLS: usize = 400;

/// What a heatmap was counted for: the view's domains.
#[derive(Debug, Clone, PartialEq)]
pub struct HeatKey {
    pub x: (f64, f64),
    pub y: (f64, f64),
}

/// A heatmap of the points in a view, one pixel of the image for each cell.
#[derive(Debug)]
pub struct Heat {
    pub image: Arc<RgbaImage>,
    /// The largest count of a cell.
    pub max: f64,
}

/// The color scale of a heatmap whose largest count is `max`: a log scale from one point.
pub fn color_scale(max: f64) -> ConfiguredScale {
    LogScale::configured((1.0, max.max(2.0)), (0.0, 1.0))
        .with_range_colors(VIRIDIS.to_vec())
        .expect("the colors form a range")
}

/// Count `points` in each cell of a grid over `key`'s view, and shade the counts through
/// [`color_scale`], with no color where no point lies.
pub async fn heat(points: DataFrame, key: HeatKey, cancel: Cancel) -> Result<Heat, BoxError> {
    let query = Rasterize::new(col("x"), &LinearScale::configured(key.x, (0.0, 1.0)), CELLS)
        .with_y(col("y"), &LinearScale::configured(key.y, (0.0, 1.0)), CELLS)
        .aggregate(count(lit(1)));
    let batches = query.plan(points)?.collect().await?;
    cancel.yield_now().await?;
    let counts = query.pack(&batches)?.remove(0).planes.remove(0);
    cancel.yield_now().await?;
    let max = counts.iter().flatten().fold(1.0, f64::max);
    let image = shade(&counts, &color_scale(max), [CELLS as u32; 2])?;
    Ok(Heat {
        image: Arc::new(image),
        max,
    })
}

/// The counts' pixels through `scale`'s colors, which a lookup table samples at even steps of
/// the scale's log domain.
fn shade(
    counts: &Float64Array,
    scale: &ConfiguredScale,
    size: [u32; 2],
) -> Result<RgbaImage, BoxError> {
    let (_, top) = scale.numeric_interval_domain()?;
    let steps: ArrayRef = Arc::new(Float64Array::from_iter_values(
        (0..SHADES).map(|step| top.powf(step as f64 / (SHADES - 1) as f64)),
    ));
    let table: Vec<[u8; 4]> = scale
        .scale_to_color(&steps)?
        .as_vec(SHADES, None)
        .iter()
        .map(|color| {
            color
                .color_or_transparent()
                .map(|c| (c * 255.0).round() as u8)
        })
        .collect();
    let log_top = top.ln();
    let mut data = Vec::with_capacity(counts.len() * 4);
    for count in counts.iter() {
        let pixel = match count {
            Some(count) if count > 0.0 => {
                let step = (count.ln() / log_top * (SHADES - 1) as f64).round();
                table[(step as usize).min(SHADES - 1)]
            }
            _ => [0; 4],
        };
        data.extend_from_slice(&pixel);
    }
    Ok(RgbaImage {
        width: size[0],
        height: size[1],
        data,
    })
}
