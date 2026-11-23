//! What the charts of a view share in egui: the surface that they draw on, from egui's visuals,
//! how their images load, and the static chart of a table cell, which evaluates at the size that
//! its cell gives it. The label engine, the preferences of a color scheme, and the cascade come
//! from avenger-sql-project's `charts` module, so a window and an image agree.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use avenger_chart::Chart;
use avenger_chart_eval::{ChartEvaluator, EvalError, Registry};
use avenger_image::{ImageCache, ImageCacheOptions, ImageResolver};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_sql_project::charts::{cascade, preferences};
use avenger_style::{Cascade, ColorScheme, HostInputs, Preferences, Sheet};
use avenger_typst_label::LabelEngine;
use eframe::egui;

/// How the charts load their images, such as the tiles of a map, by the options that their host
/// gives: its own User-Agent, as tile servers' usage policies ask, and its own disk cache. The
/// browser fetches with its own `fetch` and keeps its own HTTP cache.
///
/// A cache wakes one app when an image arrives, so each live chart has a cache of its own, and
/// the caches share the fetcher and the disk. The static charts share one cache, whose arrivals
/// the cells count, so that a cell that drew before its images arrived draws again.
pub struct Images {
    options: ImageCacheOptions,
    statics: ImageCache,
    arrivals: Arc<AtomicU64>,
}

impl Images {
    /// The images of the charts, by `options`, whose arrivals for the static charts repaint
    /// `ctx`.
    pub fn new(ctx: &egui::Context, options: ImageCacheOptions) -> Self {
        let statics = ImageCache::with_options(options.clone());
        let arrivals = Arc::new(AtomicU64::new(0));
        {
            let (arrivals, ctx) = (arrivals.clone(), ctx.clone());
            statics.set_on_change(move || {
                arrivals.fetch_add(1, Ordering::SeqCst);
                ctx.request_repaint();
            });
        }
        Self {
            options,
            statics,
            arrivals,
        }
    }

    /// A cache for a live chart.
    pub fn live(&self) -> ImageCache {
        ImageCache::with_options(self.options.clone())
    }

    /// The cache of the static charts.
    pub fn statics(&self) -> Arc<dyn ImageResolver> {
        Arc::new(self.statics.clone())
    }

    /// The number of times that an image of the static charts arrived or failed.
    pub fn arrivals(&self) -> u64 {
        self.arrivals.load(Ordering::SeqCst)
    }
}

/// The surface that the charts draw on in egui: the background of the panels, so that a chart and
/// the panel around it are one surface, and the color scheme of egui's visuals, which the
/// stylesheets' `prefers-color-scheme` tests. The charts' text takes the contrast of the
/// background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub background: egui::Color32,
    pub color_scheme: ColorScheme,
}

impl Surface {
    /// The surface of egui's `visuals`.
    pub fn of(visuals: &egui::Visuals) -> Self {
        Self {
            background: visuals.panel_fill,
            color_scheme: match visuals.dark_mode {
                true => ColorScheme::Dark,
                false => ColorScheme::Light,
            },
        }
    }

    /// The preferences of the charts' reader: the color scheme.
    pub fn preferences(&self) -> Preferences {
        preferences(self.color_scheme)
    }

    /// The cascade of the base stylesheet, `std:base.css`, and the author stylesheets `sheets`
    /// after it, such as a project's entry stylesheet, on the surface's background.
    pub fn cascade<'a>(&self, sheets: impl IntoIterator<Item = &'a Sheet>) -> Arc<Cascade> {
        let [r, g, b, _] = self.background.to_array();
        cascade(&format!("#{r:02x}{g:02x}{b:02x}"), sheets)
    }
}

impl Default for Surface {
    /// The surface of egui's light visuals.
    fn default() -> Self {
        Self::of(&egui::Visuals::light())
    }
}

/// The chart of a table cell, which draws without an app, at any size.
pub struct StaticChart {
    evaluator: ChartEvaluator,
    preferences: Preferences,
}

impl StaticChart {
    /// Prepares `chart`, styled by `cascade` for a reader with `preferences`, with the labels of
    /// `engine`.
    pub fn new(
        chart: Chart,
        cascade: Arc<Cascade>,
        preferences: Preferences,
        engine: LabelEngine,
    ) -> Result<Self, EvalError> {
        let evaluator = ChartEvaluator::new(chart, cascade, engine, Registry::new())?;
        Ok(Self {
            evaluator,
            preferences,
        })
    }

    /// Loads the tiles of the chart's maps through `images`.
    pub fn with_images(mut self, images: Arc<dyn ImageResolver>) -> Self {
        self.evaluator.set_image_resolver(images);
        self
    }

    /// The scene of the chart at `size`, in logical pixels.
    pub fn scene(&mut self, size: [f32; 2]) -> Result<SceneGraph, EvalError> {
        let host = HostInputs {
            preferences: self.preferences.clone(),
            ..HostInputs::sized(size[0], size[1])
        };
        Ok(self.evaluator.evaluate(&host)?.scene)
    }
}
