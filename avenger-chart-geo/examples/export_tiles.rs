//! Render a Geo (mercator) tile plot to SVG and PDF.
//!
//! The tile layer uses an inline data URI so the example is deterministic and
//! does not require network access. Pass an output directory as the first
//! argument, or use `target/examples/webmercator-export` by default.
//!
//! ```bash
//! cargo run -p avenger-chart-geo --example export_tiles --release
//! ```

use std::{error::Error, fs, path::PathBuf};

use avenger_chart::{
    prelude::*,
    render::{PdfRenderer, SvgRenderer},
};
use avenger_chart_geo::{Geo, GeoPositionChannels, RasterTileLayer, Symbol};
use datafusion::prelude::SessionContext;

const TINY_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAG0lEQVR4nGO4o6b2XzX59X8GscVe/3+dEf0PAE8fCXZKLiUkAAAAAElFTkSuQmCC";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/examples/geo-export"));
    fs::create_dir_all(&out_dir)?;

    let ctx = SessionContext::new();
    let data = ctx
        .sql(
            "SELECT * FROM (VALUES
                (-73.9857, 40.7484, '#ef4444'),
                (-0.1276, 51.5072, '#2563eb'),
                (139.6917, 35.6895, '#16a34a')
            ) AS t(lon, lat, color)",
        )
        .await?;

    let coord = Geo::mercator().center_lon_lat(0.0, 20.0).zoom(0.0).tiles(
        RasterTileLayer::xyz(TINY_PNG_DATA_URI)
            .id("inline")
            .max_zoom(0)
            .attribution("Example inline tile"),
    );
    let plot = Chart::with_coord(coord.clone())
        .canvas_size(640.0, 420.0)
        .data(data)
        .mark(
            Symbol::new()
                .lon_lat(&coord, col("lon"), col("lat"))
                .fill_with(col("color"), |fill| {
                    fill.no_scale().legend(|legend| legend.visible(false))
                })
                .stroke("#111827")
                .stroke_width(1.5)
                .size(180.0),
        );

    let evaluated = plot.compile(&ctx).await?.evaluate(&ctx, None).await?;
    let svg = SvgRenderer::new().render_evaluated_plot(&evaluated)?;
    let pdf = PdfRenderer::new().render_evaluated_plot(&evaluated)?;

    let svg_path = out_dir.join("geo-export.svg");
    let pdf_path = out_dir.join("geo-export.pdf");
    fs::write(&svg_path, svg)?;
    fs::write(&pdf_path, pdf)?;

    println!("wrote {}", svg_path.display());
    println!("wrote {}", pdf_path.display());
    Ok(())
}
