//! # avenger-chart-app
//!
//! An avenger app that shows an avenger [`Chart`](avenger_chart::Chart) and serves the tools
//! that its plots declare. [`ChartApp`] builds an `AvengerApp<ChartAppState>`, which
//! evaluates the chart at the size of the app, pans and zooms the scales of the plots that declare
//! a `Navigate`, writes the selections of the plots that declare a `Select` with boxes, lassos,
//! and clicks, and shows the tooltips of the plots that declare a `Tooltip`.
//!
//! Boxes and lassos work through the projection of each plot, so every plot with a projection takes
//! them: a box is a rectangle in a Cartesian plot and an annular sector in a polar plot.
//!
//! The `geo` feature serves the tools of map plots: a navigation pans and zooms the projection,
//! a box selects ranges of longitude and latitude, and a lasso selects the places inside it. The
//! map plots draw their raster tiles through the app's image cache, which [`ChartApp::images`]
//! sets.
//!
//! A host changes the data that the app shows only by a [`ChartUpdate`]: a new chart, with what
//! the app carries over from the chart that it replaces. Each scale fits the new rows, keeps its
//! old domain, or extends it ([`Fit`]), the views of pans and zooms stay or go ([`Views`]), and
//! each selection keeps its contributions or takes the new chart's ([`Carry`]). A host sends
//! updates, the [`Mode`], and selections through a [`ChartFeed`], or requests updates on a
//! [`ChartTask`], which loads and prepares them on the app's runtime. A [`ChartChange`] reports
//! the domains, the projections, and the selections that one gesture changed, so a host can load
//! the data of the next chart.

mod app;
mod feed;
mod scene;
mod selections;
mod state;
mod tools;
mod update;

pub use app::ChartApp;
pub use avenger_chart_eval::{
    Carry, Fit, Fitted, ProjectionIdentity, ProjectionView, ScaleIdentity, Views,
};
pub use feed::ChartFeed;
pub use state::{ChartAppState, ChartChange, Mode};
pub use update::{ChartTask, ChartUpdate};

/// The scene of the chart.
pub const CHART_SCENE: &str = "chart";

/// The scene of the boxes and the lassos of selections, above the chart.
pub const BRUSH_SCENE: &str = "brushes";

/// The scene of the tooltips, above the brushes.
pub const TOOLTIP_SCENE: &str = "tooltips";
