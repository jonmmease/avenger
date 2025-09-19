//! Title and subtitle configuration for plots

/// Alignment options for title and subtitle
#[derive(Clone, Debug, Copy, PartialEq, Default)]
pub enum TitleAlign {
    /// Title/subtitle spans entire width minus padding columns
    #[default]
    FullWidth,
    /// Title/subtitle only spans the plot area column
    PlotAreaOnly,
}

/// Minimal plot title configuration
#[derive(Clone, Debug)]
pub struct PlotTitle {
    pub text: String,
    pub font_size: Option<f32>,
    pub font_family: Option<String>,
    pub align: TitleAlign,
}

/// Minimal plot subtitle configuration
#[derive(Clone, Debug)]
pub struct PlotSubtitle {
    pub text: String,
    pub font_size: Option<f32>,
    pub font_family: Option<String>,
    pub align: TitleAlign,
}
