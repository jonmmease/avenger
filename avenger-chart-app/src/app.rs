//! `ChartApp`, which builds the app of a chart.

use std::sync::Arc;

use avenger_app::{app::AvengerApp, error::AvengerAppError};
use avenger_chart::Chart;
use avenger_chart_eval::Registry;
use avenger_image::ImageCache;
use avenger_style::Cascade;
use avenger_typst_label::LabelEngine;

use crate::{
    scene::{app_error, ChartBuild},
    selections,
    state::{ChartAppState, ChartChange, OnChange, OnFocus, Reports, SelectionFocus},
    tools,
    updates::ChartUpdates,
};

/// The app of a chart: the chart, the cascade of its style table, the label engine and the custom
/// scales that the app evaluates it with, the cache of the images that it draws, and the handle
/// and the callbacks that join the app to its host. [`ChartApp::build`] builds an `AvengerApp`
/// that any avenger host runs.
#[must_use]
pub struct ChartApp {
    chart: Chart,
    cascade: Arc<Cascade>,
    engine: LabelEngine,
    registry: Registry,
    images: Option<ImageCache>,
    updates: ChartUpdates,
    on_change: Option<OnChange>,
    reports: Reports,
    on_focus: Option<OnFocus>,
    size: [f32; 2],
}

impl ChartApp {
    /// The app of `chart`, styled by `cascade`, whose labels `engine` typesets.
    pub fn new(chart: Chart, cascade: Arc<Cascade>, engine: LabelEngine) -> Self {
        Self {
            chart,
            cascade,
            engine,
            registry: Registry::new(),
            images: None,
            updates: ChartUpdates::new(),
            on_change: None,
            reports: Reports::default(),
            on_focus: None,
            size: [800.0, 600.0],
        }
    }

    /// The custom scales of the charts.
    pub fn registry(mut self, registry: Registry) -> Self {
        self.registry = registry;
        self
    }

    /// The cache of the images that the charts draw: the images of marks, and with the `geo`
    /// feature, the raster tiles of map plots. The host's canvas draws the images through the
    /// cache, and each image that arrives or fails redraws the app. The host builds the cache with
    /// the fetch and disk-cache policy that the terms of its image servers require.
    pub fn images(mut self, images: ImageCache) -> Self {
        self.images = Some(images);
        self
    }

    /// The handle through which the host sends updates, the mode, and selections.
    pub fn updates(mut self, updates: &ChartUpdates) -> Self {
        self.updates = updates.clone();
        self
    }

    /// The callback that hears each change of the app. It runs on the thread of the app's events,
    /// inside the handler of a tool, so it returns quickly, such as by sending the change on a
    /// channel.
    pub fn on_change(mut self, on_change: impl Fn(&ChartChange) + Send + Sync + 'static) -> Self {
        self.on_change = Some(Arc::new(on_change));
        self
    }

    /// Which changes the app reports: every change, by default, or the changes when a gesture
    /// settles.
    pub fn reports(mut self, reports: Reports) -> Self {
        self.reports = reports;
        self
    }

    /// The callback that hears the focus of the selections that can pre-aggregate: a
    /// [`SelectionFocus`] when the pointer enters or presses on a plot whose point selection, or
    /// whose interval selection with a `cell`, can write a clause, again when the grid of the
    /// focused plot changes, and `None` when the pointer leaves the plot. It runs on the thread of
    /// the app's events, so it returns quickly.
    pub fn on_focus(
        mut self,
        on_focus: impl Fn(Option<&SelectionFocus>) + Send + Sync + 'static,
    ) -> Self {
        self.on_focus = Some(Arc::new(on_focus));
        self
    }

    /// The size of the app until its host sets one.
    pub fn size(mut self, size: [f32; 2]) -> Self {
        self.size = size;
        self
    }

    /// Builds the app, which shows the chart, pans and zooms the scales of the plots that declare
    /// a `Navigate`, writes the selections of the plots and the layers that declare a `Select`, and
    /// shows the tooltips of the plots and the layers that declare a `Tooltip`. On native targets,
    /// give the built app a multi-thread runtime with `AvengerApp::with_runtime`, which prepares
    /// the charts of updates.
    pub async fn build(self) -> Result<AvengerApp<ChartAppState>, AvengerAppError> {
        #[cfg_attr(not(feature = "geo"), allow(unused_mut))]
        let mut state = ChartAppState::new(
            self.chart,
            self.cascade,
            &self.engine,
            &self.registry,
            self.updates.clone(),
            self.on_change,
            self.reports,
            self.on_focus,
        )
        .map_err(app_error)?;
        // The map plots of every chart that the app installs load their tiles through the cache.
        #[cfg(feature = "geo")]
        if let Some(images) = &self.images {
            (state.evaluator).set_image_resolver(Arc::new(images.clone()));
        }
        let engine = self.engine;
        let builder = AvengerApp::builder(state, ChartBuild, engine.clone())
            .size(self.size)
            .tool(tools::navigation());
        #[cfg(feature = "geo")]
        let builder = builder.tool(tools::map_navigation());
        let mut app = builder
            .tool(selections::brushes())
            .tool(tools::tooltips())
            .tool(selections::Focus)
            .build()
            .await?;
        if let Some(images) = self.images {
            app = app.with_images(images);
        }
        app = app.with_task(self.updates.task());
        self.updates.bind(app.waker(), &engine, &self.registry);
        Ok(app)
    }
}
