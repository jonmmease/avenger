//! # avenger-chart-app
//!
//! An avenger app that shows an avenger [`Chart`](avenger_chart::Chart) and serves the tools
//! that its plots declare. [`ChartApp`] builds an `AvengerApp<ChartAppState>`, which
//! evaluates the chart at the size of the app, pans and zooms the scales of the plots that declare
//! a `Navigate`, and shows the tooltips of the plots that declare a `Tooltip`. A host sends new
//! charts through a [`ChartFeed`], and the app keeps the views of the scales that a new chart
//! still has. [`ChartChange`]s report the views that navigations settle, so a host can load the
//! data of the next chart.

mod app;
mod feed;
mod scene;
mod state;
mod tools;

pub use app::ChartApp;
pub use feed::ChartFeed;
pub use state::{ChartAppState, ChartChange};

/// The scene of the chart.
pub const CHART_SCENE: &str = "chart";

/// The scene of the tooltips, above the chart.
pub const TOOLTIP_SCENE: &str = "tooltips";
