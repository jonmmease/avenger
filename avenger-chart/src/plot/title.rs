//! Title and subtitle configuration for plots.

use avenger_chart_core::IntoExpr;

use crate::{coords::CoordinateSystem, plot::Plot};

pub use avenger_chart_core::TitleSpec as PlotSubtitle;
pub use avenger_chart_core::TitleSpec as PlotTitle;

/// Transitional title/subtitle methods on Plot.
///
/// These move to Chart when the root furnishing fields relocate in phase 2b.1.
impl<C: CoordinateSystem> Plot<C> {
    pub fn title(mut self, text: impl IntoExpr) -> Self {
        self.title = Some(PlotTitle::new(text));
        self
    }

    pub fn configure_title<F>(mut self, text: impl IntoExpr, f: F) -> Self
    where
        F: FnOnce(PlotTitle) -> PlotTitle,
    {
        self.title = Some(f(PlotTitle::new(text)));
        self
    }

    pub fn subtitle(mut self, text: impl IntoExpr) -> Self {
        self.subtitle = Some(PlotSubtitle::new(text));
        self
    }

    pub fn configure_subtitle<F>(mut self, text: impl IntoExpr, f: F) -> Self
    where
        F: FnOnce(PlotSubtitle) -> PlotSubtitle,
    {
        self.subtitle = Some(f(PlotSubtitle::new(text)));
        self
    }

    pub fn get_title(&self) -> Option<&PlotTitle> {
        self.title.as_ref()
    }

    pub fn get_subtitle(&self) -> Option<&PlotSubtitle> {
        self.subtitle.as_ref()
    }
}
