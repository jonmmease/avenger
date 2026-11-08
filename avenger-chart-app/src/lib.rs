//! # avenger-chart-app
//!
//! An avenger app that shows an avenger [`Chart`](avenger_chart::Chart) and serves the tools that
//! its plots and layers declare. [`ChartApp`] builds an `AvengerApp<ChartAppState>`, which
//! evaluates the chart at the size of the app, pans and zooms the scales of the plots that declare
//! a `Navigate`, writes the selections of the plots and layers that declare a `Select` with boxes,
//! lassos, and clicks, and shows the tooltips of the plots and layers that declare a `Tooltip`.
//! A layer's tools act on its own marks, and its clauses are its own. A tool that names modes of
//! the chart is active only while one of them is on, and a mode widget or the host turns the
//! modes on and off.
//!
//! Boxes and lassos work through the coordinates of each plot, so every coordinate system takes
//! them: a box is a rectangle in a Cartesian plot and an annular sector in a polar plot. The
//! `geo` feature serves the tools of geo plots: a navigation pans and zooms the projection, a box
//! selects ranges of longitude and latitude, and a lasso selects the places inside it.
//!
//! A host changes the app only through one [`ChartUpdates`] handle: a [`ChartUpdate`] with a new
//! chart, new style inputs, or both, the chart's modes, and the states of selections. Each scale of a
//! new chart fits the new rows, keeps its old domain, or extends it ([`Fit`]), the views of pans
//! and zooms stay or go ([`Views`]), and each selection keeps its clauses or takes the new chart's
//! ([`Carry`]). A [`ChartChange`] reports the domains, the projections, the selections, and the
//! modes that one gesture changed, so a host can load the data of the next chart.

mod app;
mod scene;
mod selections;
mod state;
mod tools;
mod updates;
mod widgets;

pub use app::ChartApp;
pub use avenger_chart_eval::{
    Carry, EvaluatedWidget, Fit, Fitted, ProjectionIdentity, ProjectionView, ScaleIdentity, Views,
    WidgetSpec, WidgetTarget, WidgetValue,
};
pub use state::{ChartAppState, ChartChange, FocusDimension, Reports, SelectionFocus};
pub use updates::{ChartUpdate, ChartUpdates};

/// The scene of the chart.
pub const CHART_SCENE: &str = "chart";

/// The scene of the boxes and the lassos of selections, above the chart.
pub const BRUSH_SCENE: &str = "brushes";

/// The scene of the tooltips, above the brushes.
pub const TOOLTIP_SCENE: &str = "tooltips";

/// The scene of the widgets, above the tooltips.
pub const WIDGET_SCENE: &str = "widgets";

pub use widgets::SLOTS;
