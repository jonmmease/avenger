//! `ChartApp`, which builds the app of a chart.

use std::sync::Arc;

use avenger_app::{app::AvengerApp, error::AvengerAppError};
use avenger_chart::Chart;
use avenger_chart_eval::Theme;
use avenger_typst_label::LabelEngine;

use crate::{
    feed::ChartFeed,
    scene::{app_error, ChartBuild},
    selections,
    state::{ChartAppState, ChartChange, OnChange},
    tools,
};

/// The app of a chart: the chart, the label engine and the theme that the app evaluates it with,
/// and the feed and the callback that join the app to its host. [`ChartApp::build`] builds an
/// `AvengerApp` that any avenger host runs.
#[must_use]
pub struct ChartApp {
    chart: Chart,
    engine: LabelEngine,
    theme: Theme,
    feed: ChartFeed,
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
            feed: ChartFeed::new(),
            on_change: None,
            size: [800.0, 600.0],
        }
    }

    /// The theme of the chart.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// The feed through which the host sends new charts.
    pub fn feed(mut self, feed: &ChartFeed) -> Self {
        self.feed = feed.clone();
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
        let state = ChartAppState::new(
            &self.chart,
            &self.engine,
            &self.theme,
            self.feed.clone(),
            self.on_change,
        )
        .map_err(app_error)?;
        let app = AvengerApp::builder(state, ChartBuild, self.engine)
            .size(self.size)
            .tool(tools::navigation())
            .tool(selections::brushes())
            .tool(tools::tooltips())
            .build()
            .await?;
        self.feed.bind(app.waker());
        Ok(app)
    }
}
