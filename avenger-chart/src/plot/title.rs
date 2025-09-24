//! Title and subtitle configuration for plots

use serde::{Deserialize, Serialize};
use crate::coords::CoordinateSystem;
use crate::plot::Plot;

/// Alignment options for title and subtitle
#[derive(Clone, Debug, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TitleAlign {
    /// Title/subtitle spans entire width minus padding columns
    #[default]
    FullWidth,
    /// Title/subtitle only spans the plot area column
    PlotAreaOnly,
}

/// Minimal plot title configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlotTitle {
    pub text: String,
    pub font_size: Option<f32>,
    pub font_family: Option<String>,
    pub align: TitleAlign,
}

/// Minimal plot subtitle configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlotSubtitle {
    pub text: String,
    pub font_size: Option<f32>,
    pub font_family: Option<String>,
    pub align: TitleAlign,
}

/// Title and subtitle configuration methods for Plot
impl<C: CoordinateSystem> Plot<C> {
    /// Set a simple plot title. For advanced styling, a richer API can be added later.
    pub fn title(mut self, text: impl Into<String>) -> Self {
        self.title = Some(PlotTitle {
            text: text.into(),
            font_size: None,
            font_family: None,
            align: TitleAlign::default(),
        });
        self
    }

    /// Configure the title with a closure for advanced options
    pub fn configure_title<F>(mut self, text: impl Into<String>, f: F) -> Self
    where
        F: FnOnce(PlotTitle) -> PlotTitle,
    {
        let title = PlotTitle {
            text: text.into(),
            font_size: None,
            font_family: None,
            align: TitleAlign::default(),
        };
        self.title = Some(f(title));
        self
    }

    /// Set a simple plot subtitle. For advanced styling, a richer API can be added later.
    pub fn subtitle(mut self, text: impl Into<String>) -> Self {
        self.subtitle = Some(PlotSubtitle {
            text: text.into(),
            font_size: None,
            font_family: None,
            align: TitleAlign::default(),
        });
        self
    }

    /// Configure the subtitle with a closure for advanced options
    pub fn configure_subtitle<F>(mut self, text: impl Into<String>, f: F) -> Self
    where
        F: FnOnce(PlotSubtitle) -> PlotSubtitle,
    {
        let subtitle = PlotSubtitle {
            text: text.into(),
            font_size: None,
            font_family: None,
            align: TitleAlign::default(),
        };
        self.subtitle = Some(f(subtitle));
        self
    }

    /// Access the configured title
    pub fn get_title(&self) -> Option<&PlotTitle> {
        self.title.as_ref()
    }

    /// Access the configured subtitle
    pub fn get_subtitle(&self) -> Option<&PlotSubtitle> {
        self.subtitle.as_ref()
    }
}
