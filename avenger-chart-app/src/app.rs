//! `ChartApp`, which builds the app of a chart.

use std::sync::Arc;

use avenger_app::{app::AvengerApp, error::AvengerAppError};
use avenger_chart::Chart;
use avenger_chart_eval::Theme;
use avenger_image::ImageCache;
use avenger_typst_label::LabelEngine;

use crate::{
    feed::ChartFeed,
    scene::{app_error, ChartBuild},
    selections,
    state::{ChartAppState, ChartChange, OnChange},
    tools,
    update::{ChartTask, TaskSource},
};

/// The app of a chart: the chart, the label engine and the theme that the app evaluates it with,
/// the cache of the images that it draws, and the feed, the task of charts, and the callback that
/// join the app to its host. [`ChartApp::build`] builds an `AvengerApp` that any avenger host runs.
#[must_use]
pub struct ChartApp {
    chart: Chart,
    engine: LabelEngine,
    theme: Theme,
    images: Option<ImageCache>,
    feed: ChartFeed,
    charts: Option<TaskSource>,
    on_change: Option<OnChange>,
    size: [f32; 2],
}

impl ChartApp {
    /// The app of `chart`, whose labels `engine` typesets.
    pub fn new(chart: Chart, engine: LabelEngine) -> Self {
        Self {
            chart,
            engine,
            theme: Theme::default(),
            images: None,
            feed: ChartFeed::new(),
            charts: None,
            on_change: None,
            size: [800.0, 600.0],
        }
    }

    /// The theme of the chart.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// The cache of the images that the charts draw: the images of marks, and with the `geo`
    /// feature, the raster tiles of map plots, which a chart with tiles needs. The host's canvas
    /// draws the images through the cache, and each image that arrives or fails redraws the app,
    /// so a tile draws on the frame after it arrives, over the nearest coarser tile that the cache
    /// holds until then. The host builds the cache with the fetch and disk-cache policy that the
    /// terms of its image servers require, such as a disk cache that honors their cache headers
    /// and a user agent that names the app.
    pub fn images(mut self, images: ImageCache) -> Self {
        self.images = Some(images);
        self
    }

    /// The feed through which the host sends new charts.
    pub fn feed(mut self, feed: &ChartFeed) -> Self {
        self.feed = feed.clone();
        self
    }

    /// The task whose updates the app installs. After each request finishes, the next build
    /// installs its update, which the task prepared on the app's runtime. A failed request leaves
    /// the current chart, and the task keeps its error.
    ///
    /// On native targets, give the built app a multi-thread runtime with
    /// `AvengerApp::with_runtime`, which runs the task's jobs.
    pub fn charts<K>(mut self, task: &ChartTask<K>) -> Self
    where
        K: Clone + PartialEq + Send + Sync + 'static,
    {
        self.charts = Some(task.source());
        self
    }

    /// The callback that hears each change of the app. It runs on the thread of the app's events,
    /// inside the handler of a tool, so it returns quickly, such as by sending the change on a
    /// channel.
    pub fn on_change(mut self, on_change: impl Fn(&ChartChange) + Send + Sync + 'static) -> Self {
        self.on_change = Some(Arc::new(on_change));
        self
    }

    /// The size of the app until its host sets one.
    pub fn size(mut self, size: [f32; 2]) -> Self {
        self.size = size;
        self
    }

    /// Builds the app, which shows the chart, pans and zooms the scales of the plots that declare
    /// a `Navigate`, writes the selections of the plots that declare a `Select`, and shows the
    /// tooltips of the plots that declare a `Tooltip`.
    pub async fn build(self) -> Result<AvengerApp<ChartAppState>, AvengerAppError> {
        let (charts, attach) = match self.charts {
            Some(source) => (Some(source.charts), Some(source.attach)),
            None => (None, None),
        };
        #[cfg_attr(not(feature = "geo"), allow(unused_mut))]
        let mut state = ChartAppState::new(
            &self.chart,
            &self.engine,
            &self.theme,
            self.feed.clone(),
            charts,
            self.on_change,
        )
        .map_err(app_error)?;
        // The map plots of every chart that the app installs load their tiles through the cache.
        #[cfg(feature = "geo")]
        if let Some(images) = &self.images {
            (state.evaluator)
                .set_image_resolver(std::sync::Arc::new(images.clone()))
                .map_err(app_error)?;
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
            .build()
            .await?;
        if let Some(images) = self.images {
            app = app.with_images(images);
        }
        self.feed.bind(app.waker(), &engine, &self.theme);
        if let Some(attach) = attach {
            app = attach(app, &engine, &self.theme);
        }
        Ok(app)
    }
}
