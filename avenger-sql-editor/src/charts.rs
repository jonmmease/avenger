//! What every chart of the editor shares, the stylesheets, their theme, the label engine, and how
//! its images load, and the static chart of a table cell, which evaluates at the size that the
//! editor gives it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use avenger_chart::Chart;
use avenger_chart_eval::{ChartEvaluator, EvalError, Registry};
use avenger_format_datetime_googlesql::GoogleSqlDateTimeFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;
use avenger_image::{ImageCache, ImageCacheOptions, ImageResolver};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_style::{
    Cascade, ColorScheme, Env, HostInputs, ParseOptions, Preferences, Sheet, SourceId, StyleTable,
};
use avenger_typst_label::LabelEngine;
use eframe::egui;

/// The label engine of the charts: the bundled fonts, with GoogleSQL's number and date formats,
/// whose syntaxes the charts of avenger-sql name.
pub fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(GoogleSqlNumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(GoogleSqlDateTimeFormatProvider::new()))
}

/// How the charts load their images, such as the tiles of a map. On native targets, the editor
/// fetches them with its own User-Agent and keeps them on disk beside its settings, as
/// OpenStreetMap's tile usage policy asks. The browser fetches with its own `fetch` and keeps its
/// own HTTP cache.
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
    /// The images of the charts, whose arrivals for the static charts repaint `ctx`.
    pub fn new(ctx: &egui::Context) -> Result<Self, String> {
        let options = Self::options()?;
        let statics = ImageCache::with_options(options.clone());
        let arrivals = Arc::new(AtomicU64::new(0));
        {
            let (arrivals, ctx) = (arrivals.clone(), ctx.clone());
            statics.set_on_change(move || {
                arrivals.fetch_add(1, Ordering::SeqCst);
                ctx.request_repaint();
            });
        }
        Ok(Self {
            options,
            statics,
            arrivals,
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn options() -> Result<ImageCacheOptions, String> {
        const USER_AGENT: &str = concat!(
            "avenger-sql-editor/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/jonmmease/avenger)"
        );
        let fetcher = avenger_image::reqwest_fetcher::ReqwestImageFetcher::new(
            USER_AGENT,
            std::time::Duration::from_secs(30),
        )
        .map_err(|error| error.to_string())?;
        Ok(ImageCacheOptions {
            fetcher: Arc::new(fetcher),
            disk: eframe::storage_dir(crate::APP_ID)
                .map(|dir| avenger_image::DiskCacheOptions::new(dir.join("images"))),
            ..Default::default()
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn options() -> Result<ImageCacheOptions, String> {
        Ok(ImageCacheOptions::default())
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

/// The theme of the charts, from the editor's visuals: the background of the panels, so that a
/// chart and the panel around it are one surface, and whether the theme is dark. The charts' text
/// takes the contrast of the background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChartTheme {
    pub background: egui::Color32,
    pub dark: bool,
}

impl ChartTheme {
    pub fn of(visuals: &egui::Visuals) -> Self {
        Self {
            background: visuals.panel_fill,
            dark: visuals.dark_mode,
        }
    }

    /// The preferences of the charts' reader: the color scheme of the theme.
    pub fn preferences(&self) -> Preferences {
        let color_scheme = match self.dark {
            true => ColorScheme::Dark,
            false => ColorScheme::Light,
        };
        Preferences {
            color_scheme,
            ..Preferences::default()
        }
    }

    /// The cascade of the base stylesheet, `std:base.css`, and of the entry stylesheet of a
    /// project, `project`, as an author stylesheet after it, both on the theme's background.
    /// Without a project, the base stylesheet styles the charts alone.
    pub fn cascade(&self, project: Option<&Sheet>) -> Arc<Cascade> {
        let [r, g, b, _] = self.background.to_array();
        let background = format!("#{r:02x}{g:02x}{b:02x}");
        cascade_on(Env::from([("background".to_string(), background)]), project)
    }
}

impl Default for ChartTheme {
    /// The theme of egui's light visuals.
    fn default() -> Self {
        Self::of(&egui::Visuals::light())
    }
}

/// The cascade of the base stylesheet, `std:base.css`, and the author stylesheet `project`, each
/// with the `env()` values `env`.
fn cascade_on(env: Env, project: Option<&Sheet>) -> Arc<Cascade> {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, diagnostics) = Sheet::load(
        "@import 'std:base.css';",
        None,
        SourceId(1),
        options,
        &mut no_files,
    );
    debug_assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let table = StyleTable::new().with_author(sheet, env.clone());
    let table = match project {
        Some(project) => table.with_author(project.clone(), env),
        None => table,
    };
    Arc::new(Cascade::new(&table))
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
