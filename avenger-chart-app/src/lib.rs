//! # avenger-chart-app
//!
//! An avenger app that shows an avenger [`Chart`](avenger_chart::Chart) and serves the tools
//! that its plots declare. [`ChartApp`] builds an `AvengerApp<ChartAppState>`, which
//! evaluates the chart at the size of the app, pans and zooms the scales of the plots that declare
//! a `Navigate`, writes the selections of the plots that declare a `Select` with boxes, lassos,
//! and clicks, and shows the tooltips of the plots that declare a `Tooltip`. A host sends new
//! charts, the [`Mode`], and selections through a [`ChartFeed`], and the app keeps the views and
//! the selections that a new chart still has. [`ChartChange`]s report the views and the
//! selections that gestures change, so a host can load the data of the next chart.

mod app;
mod feed;
mod scene;
mod selections;
mod state;
mod tools;

pub use app::ChartApp;
pub use feed::ChartFeed;
pub use state::{ChartAppState, ChartChange, Mode};

/// The scene of the chart.
pub const CHART_SCENE: &str = "chart";

/// The scene of the boxes and the lassos of selections, above the chart.
pub const BRUSH_SCENE: &str = "brushes";

/// The scene of the tooltips, above the brushes.
pub const TOOLTIP_SCENE: &str = "tooltips";
