use crate::legend_renderer::LegendRenderer;
use std::sync::Arc;

/// Legend configuration for visualizations
#[derive(Clone)]
pub struct Legend {
    pub visible: bool,
    pub title: Option<String>,
    pub position: Option<LegendPosition>,
    pub orientation: Option<LegendOrientation>,
    pub symbol_size: Option<f64>,
    pub gradient_length: Option<f64>,
    pub gradient_thickness: Option<f64>,
    pub columns: Option<usize>,
    pub label_limit: Option<f64>,
    pub format_number: Option<String>,
    pub background_fill: Option<String>,
    pub background_stroke: Option<String>,
    pub background_corner_radius: Option<f32>,
    pub background_padding: Option<f32>,
    pub order: Option<i32>,
    pub contributing_marks: Vec<String>,
    /// Optional custom renderer override
    pub renderer: Option<Arc<dyn LegendRenderer>>,
    /// Channels that have been merged into this legend (for layout width calculation)
    pub merged_channels: Vec<String>,
    /// Text colors from theme
    pub title_color: Option<String>,
    pub label_color: Option<String>,
    /// Theme mark defaults (for legend symbol rendering)
    pub theme_mark_defaults: Option<
        indexmap::IndexMap<String, indexmap::IndexMap<String, datafusion_common::ScalarValue>>,
    >,
    /// Typography from theme
    pub title_font_family: Option<String>,
    pub title_font_size: Option<f32>,
    pub title_font_weight: Option<f32>,
    /// Item label typography (for discrete legends: symbol, line, rect)
    pub label_font_family: Option<String>,
    pub label_font_size: Option<f32>,
    pub label_font_weight: Option<f32>,
    /// Tick label typography (for continuous legends: colorbar)
    pub tick_font_family: Option<String>,
    pub tick_font_size: Option<f32>,
    pub tick_font_weight: Option<f32>,
    pub tick_color: Option<String>,
}

// Custom Debug implementation since LegendRenderer doesn't implement Debug
impl std::fmt::Debug for Legend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Legend")
            .field("visible", &self.visible)
            .field("title", &self.title)
            .field("position", &self.position)
            .field("orientation", &self.orientation)
            .field("symbol_size", &self.symbol_size)
            .field("gradient_length", &self.gradient_length)
            .field("gradient_thickness", &self.gradient_thickness)
            .field("columns", &self.columns)
            .field("label_limit", &self.label_limit)
            .field("format_number", &self.format_number)
            .field("background_fill", &self.background_fill)
            .field("background_stroke", &self.background_stroke)
            .field("background_corner_radius", &self.background_corner_radius)
            .field("background_padding", &self.background_padding)
            .field("order", &self.order)
            .field("contributing_marks", &self.contributing_marks)
            .field("renderer", &self.renderer.is_some())
            .field("merged_channels", &self.merged_channels)
            .field("title_color", &self.title_color)
            .field("label_color", &self.label_color)
            .field("theme_mark_defaults", &self.theme_mark_defaults.is_some())
            .field("title_font_family", &self.title_font_family)
            .field("title_font_size", &self.title_font_size)
            .field("title_font_weight", &self.title_font_weight)
            .field("label_font_family", &self.label_font_family)
            .field("label_font_size", &self.label_font_size)
            .field("label_font_weight", &self.label_font_weight)
            .field("tick_font_family", &self.tick_font_family)
            .field("tick_font_size", &self.tick_font_size)
            .field("tick_font_weight", &self.tick_font_weight)
            .field("tick_color", &self.tick_color)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LegendPosition {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LegendOrientation {
    Horizontal,
    Vertical,
}

impl Legend {
    pub fn new() -> Self {
        Self {
            visible: true,
            position: None,
            title: None,
            orientation: None,
            symbol_size: None,
            gradient_length: None,
            gradient_thickness: None,
            columns: None,
            label_limit: None,
            format_number: None,
            background_fill: None,
            background_stroke: None,
            background_corner_radius: None,
            background_padding: None,
            order: None,
            contributing_marks: Vec::new(),
            renderer: None,
            merged_channels: Vec::new(),
            title_color: None,
            label_color: None,
            theme_mark_defaults: None,
            title_font_family: None,
            title_font_size: None,
            title_font_weight: None,
            label_font_family: None,
            label_font_size: None,
            label_font_weight: None,
            tick_font_family: None,
            tick_font_size: None,
            tick_font_weight: None,
            tick_color: None,
        }
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn position(mut self, position: LegendPosition) -> Self {
        self.position = Some(position);
        self
    }

    pub fn orientation(mut self, orientation: LegendOrientation) -> Self {
        self.orientation = Some(orientation);
        self
    }

    pub fn symbol_size(mut self, size: f64) -> Self {
        self.symbol_size = Some(size);
        self
    }

    pub fn gradient_length(mut self, length: f64) -> Self {
        self.gradient_length = Some(length);
        self
    }

    pub fn gradient_thickness(mut self, thickness: f64) -> Self {
        self.gradient_thickness = Some(thickness);
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = Some(columns);
        self
    }

    pub fn label_limit(mut self, limit: f64) -> Self {
        self.label_limit = Some(limit);
        self
    }

    /// Set a numeric formatting string for legend labels.
    pub fn format_number(mut self, pattern: impl Into<String>) -> Self {
        self.format_number = Some(pattern.into());
        self
    }

    pub fn background_fill(mut self, color: impl Into<String>) -> Self {
        self.background_fill = Some(color.into());
        self
    }

    pub fn background_stroke(mut self, color: impl Into<String>) -> Self {
        self.background_stroke = Some(color.into());
        self
    }

    pub fn background_corner_radius(mut self, r: f32) -> Self {
        self.background_corner_radius = Some(r);
        self
    }

    pub fn background_padding(mut self, pad: f32) -> Self {
        self.background_padding = Some(pad);
        self
    }

    pub fn order(mut self, order: i32) -> Self {
        self.order = Some(order);
        self
    }

    pub fn add_contributing_mark(mut self, mark_id: impl Into<String>) -> Self {
        self.contributing_marks.push(mark_id.into());
        self
    }

    /// Set a custom renderer for this legend
    pub fn renderer(mut self, renderer: impl LegendRenderer + 'static) -> Self {
        self.renderer = Some(Arc::new(renderer));
        self
    }
}

impl Default for Legend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::legend::{LegendOrientation, LegendPosition};
    use crate::plot::Plot;
    use crate::zerod::ZeroDCoord;

    #[test]
    fn test_legend_fill_with_configuration() {
        let plot = Plot::<ZeroDCoord>::new()
            ._scale("fill", |scale| scale)
            .legend("fill", |legend| {
                legend.title("Temperature").position(LegendPosition::Right)
            });

        // Should have legend configured
        assert!(plot.legends.contains_key("fill"));
        let legend = &plot.legends["fill"];
        assert_eq!(legend.title, Some("Temperature".to_string()));
        assert_eq!(legend.position, Some(LegendPosition::Right));
    }

    #[test]
    fn test_legend_fill_with_visible_false() {
        let plot = Plot::<ZeroDCoord>::new()
            ._scale("fill", |scale| scale)
            .legend("fill", |legend| legend.visible(false));

        // Legend exists but is marked invisible
        assert!(plot.legends.contains_key("fill"));
        let legend = &plot.legends["fill"];
        assert!(!legend.visible);
    }

    #[test]
    fn test_legend_stroke_with_orientation() {
        let plot = Plot::<ZeroDCoord>::new()
            ._scale("stroke", |scale| scale)
            .legend("stroke", |legend| {
                legend
                    .title("Category")
                    .orientation(LegendOrientation::Horizontal)
                    .columns(3)
            });

        assert!(plot.legends.contains_key("stroke"));
        let legend = &plot.legends["stroke"];
        assert_eq!(legend.title, Some("Category".to_string()));
        assert_eq!(legend.orientation, Some(LegendOrientation::Horizontal));
        assert_eq!(legend.columns, Some(3));
    }

    #[test]
    fn test_legend_size_with_symbol_size() {
        let plot = Plot::<ZeroDCoord>::new()
            ._scale("size", |scale| scale)
            .legend("size", |legend| {
                legend.title("Population").symbol_size(20.0)
            });

        assert!(plot.legends.contains_key("size"));
        let legend = &plot.legends["size"];
        assert_eq!(legend.title, Some("Population".to_string()));
        assert_eq!(legend.symbol_size, Some(20.0));
    }

    #[test]
    fn test_legend_opacity_with_gradient() {
        let plot = Plot::<ZeroDCoord>::new()
            ._scale("opacity", |scale| scale)
            .legend("opacity", |legend| {
                legend
                    .title("Confidence")
                    .gradient_length(150.0)
                    .gradient_thickness(15.0)
            });

        assert!(plot.legends.contains_key("opacity"));
        let legend = &plot.legends["opacity"];
        assert_eq!(legend.title, Some("Confidence".to_string()));
        assert_eq!(legend.gradient_length, Some(150.0));
        assert_eq!(legend.gradient_thickness, Some(15.0));
    }

    #[test]
    fn test_multiple_legends() {
        let plot = Plot::<ZeroDCoord>::new()
            ._scale("fill", |scale| scale)
            ._scale("size", |scale| scale)
            .legend("fill", |legend| {
                legend.title("Temperature").position(LegendPosition::Right)
            })
            .legend("size", |legend| {
                legend.title("Population").position(LegendPosition::Left)
            });

        assert_eq!(plot.legends.len(), 2);
        assert!(plot.legends.contains_key("fill"));
        assert!(plot.legends.contains_key("size"));
    }

    #[test]
    fn test_legend_modification() {
        let plot = Plot::<ZeroDCoord>::new()
            ._scale("fill", |scale| scale)
            .legend("fill", |legend| legend.title("First Title"))
            .legend("fill", |legend| legend.title("Updated Title"));

        // Second call should update the existing legend
        assert_eq!(plot.legends.len(), 1);
        let legend = &plot.legends["fill"];
        assert_eq!(legend.title, Some("Updated Title".to_string()));
    }
}
