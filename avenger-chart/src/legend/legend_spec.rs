use super::LegendRenderer;
use crate::maybe::Maybe;
use crate::serialization::SerializableNestedScalarMap;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, FromInto};
use std::sync::Arc;

/// Legend configuration for visualizations
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct Legend {
    pub visible: Maybe<bool>,
    pub title: Maybe<String>,
    pub position: Maybe<LegendPosition>,
    pub orientation: Maybe<LegendOrientation>,
    pub symbol_size: Maybe<f64>,
    pub gradient_length: Maybe<f64>,
    pub gradient_thickness: Maybe<f64>,
    pub columns: Maybe<usize>,
    pub label_limit: Maybe<f64>,
    pub format_number: Maybe<String>,
    pub background_fill: Maybe<String>,
    pub background_stroke: Maybe<String>,
    pub background_corner_radius: Maybe<f32>,
    pub background_padding: Maybe<f32>,
    pub order: Maybe<i32>,
    pub contributing_marks: Vec<String>,
    /// Optional custom renderer override
    pub renderer: Option<Arc<dyn LegendRenderer>>,
    /// Channels that have been merged into this legend (for layout width calculation)
    pub merged_channels: Vec<String>,
    /// Text colors from theme
    pub title_color: Maybe<String>,
    pub label_color: Maybe<String>,
    /// Theme mark defaults (for legend symbol rendering)
    #[serde_as(as = "Option<FromInto<SerializableNestedScalarMap>>")]
    pub theme_mark_defaults: Option<
        indexmap::IndexMap<String, indexmap::IndexMap<String, datafusion_common::ScalarValue>>,
    >,
    /// Typography from theme
    pub title_font_family: Maybe<String>,
    pub title_font_size: Maybe<f32>,
    pub title_font_weight: Maybe<f32>,
    /// Item label typography (for discrete legends: symbol, line, rect)
    pub label_font_family: Maybe<String>,
    pub label_font_size: Maybe<f32>,
    pub label_font_weight: Maybe<f32>,
    /// Tick label typography (for continuous legends: colorbar)
    pub tick_font_family: Maybe<String>,
    pub tick_font_size: Maybe<f32>,
    pub tick_font_weight: Maybe<f32>,
    pub tick_color: Maybe<String>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LegendPosition {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum LegendOrientation {
    Horizontal,
    Vertical,
}

impl Legend {
    pub fn new() -> Self {
        Self {
            visible: Maybe::Set(true),
            position: Maybe::Unset,
            title: Maybe::Unset,
            orientation: Maybe::Unset,
            symbol_size: Maybe::Unset,
            gradient_length: Maybe::Unset,
            gradient_thickness: Maybe::Unset,
            columns: Maybe::Unset,
            label_limit: Maybe::Unset,
            format_number: Maybe::Unset,
            background_fill: Maybe::Unset,
            background_stroke: Maybe::Unset,
            background_corner_radius: Maybe::Unset,
            background_padding: Maybe::Unset,
            order: Maybe::Unset,
            contributing_marks: Vec::new(),
            renderer: None,
            merged_channels: Vec::new(),
            title_color: Maybe::Unset,
            label_color: Maybe::Unset,
            theme_mark_defaults: None,
            title_font_family: Maybe::Unset,
            title_font_size: Maybe::Unset,
            title_font_weight: Maybe::Unset,
            label_font_family: Maybe::Unset,
            label_font_size: Maybe::Unset,
            label_font_weight: Maybe::Unset,
            tick_font_family: Maybe::Unset,
            tick_font_size: Maybe::Unset,
            tick_font_weight: Maybe::Unset,
            tick_color: Maybe::Unset,
        }
    }

    /// Apply updates from another Legend, overriding only Set properties
    pub fn update(mut self, other: Legend) -> Self {
        if other.visible.is_set() {
            self.visible = other.visible;
        }
        if other.title.is_set() {
            self.title = other.title;
        }
        if other.position.is_set() {
            self.position = other.position;
        }
        if other.orientation.is_set() {
            self.orientation = other.orientation;
        }
        if other.symbol_size.is_set() {
            self.symbol_size = other.symbol_size;
        }
        if other.gradient_length.is_set() {
            self.gradient_length = other.gradient_length;
        }
        if other.gradient_thickness.is_set() {
            self.gradient_thickness = other.gradient_thickness;
        }
        if other.columns.is_set() {
            self.columns = other.columns;
        }
        if other.label_limit.is_set() {
            self.label_limit = other.label_limit;
        }
        if other.format_number.is_set() {
            self.format_number = other.format_number;
        }
        if other.background_fill.is_set() {
            self.background_fill = other.background_fill;
        }
        if other.background_stroke.is_set() {
            self.background_stroke = other.background_stroke;
        }
        if other.background_corner_radius.is_set() {
            self.background_corner_radius = other.background_corner_radius;
        }
        if other.background_padding.is_set() {
            self.background_padding = other.background_padding;
        }
        if other.order.is_set() {
            self.order = other.order;
        }
        // Merge contributing marks
        self.contributing_marks.extend(other.contributing_marks);
        // Override renderer if provided
        if other.renderer.is_some() {
            self.renderer = other.renderer;
        }
        // Merge merged_channels
        self.merged_channels.extend(other.merged_channels);
        // Apply theme properties
        if other.title_color.is_set() {
            self.title_color = other.title_color;
        }
        if other.label_color.is_set() {
            self.label_color = other.label_color;
        }
        if other.theme_mark_defaults.is_some() {
            self.theme_mark_defaults = other.theme_mark_defaults;
        }
        if other.title_font_family.is_set() {
            self.title_font_family = other.title_font_family;
        }
        if other.title_font_size.is_set() {
            self.title_font_size = other.title_font_size;
        }
        if other.title_font_weight.is_set() {
            self.title_font_weight = other.title_font_weight;
        }
        if other.label_font_family.is_set() {
            self.label_font_family = other.label_font_family;
        }
        if other.label_font_size.is_set() {
            self.label_font_size = other.label_font_size;
        }
        if other.label_font_weight.is_set() {
            self.label_font_weight = other.label_font_weight;
        }
        if other.tick_font_family.is_set() {
            self.tick_font_family = other.tick_font_family;
        }
        if other.tick_font_size.is_set() {
            self.tick_font_size = other.tick_font_size;
        }
        if other.tick_font_weight.is_set() {
            self.tick_font_weight = other.tick_font_weight;
        }
        if other.tick_color.is_set() {
            self.tick_color = other.tick_color;
        }
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = Maybe::Set(visible);
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Maybe::Set(title.into());
        self
    }

    pub fn position(mut self, position: LegendPosition) -> Self {
        self.position = Maybe::Set(position);
        self
    }

    pub fn orientation(mut self, orientation: LegendOrientation) -> Self {
        self.orientation = Maybe::Set(orientation);
        self
    }

    pub fn symbol_size(mut self, size: f64) -> Self {
        self.symbol_size = Maybe::Set(size);
        self
    }

    pub fn gradient_length(mut self, length: f64) -> Self {
        self.gradient_length = Maybe::Set(length);
        self
    }

    pub fn gradient_thickness(mut self, thickness: f64) -> Self {
        self.gradient_thickness = Maybe::Set(thickness);
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = Maybe::Set(columns);
        self
    }

    pub fn label_limit(mut self, limit: f64) -> Self {
        self.label_limit = Maybe::Set(limit);
        self
    }

    /// Set a numeric formatting string for legend labels.
    pub fn format_number(mut self, pattern: impl Into<String>) -> Self {
        self.format_number = Maybe::Set(pattern.into());
        self
    }

    pub fn background_fill(mut self, color: impl Into<String>) -> Self {
        self.background_fill = Maybe::Set(color.into());
        self
    }

    pub fn background_stroke(mut self, color: impl Into<String>) -> Self {
        self.background_stroke = Maybe::Set(color.into());
        self
    }

    pub fn background_corner_radius(mut self, r: f32) -> Self {
        self.background_corner_radius = Maybe::Set(r);
        self
    }

    pub fn background_padding(mut self, pad: f32) -> Self {
        self.background_padding = Maybe::Set(pad);
        self
    }

    pub fn order(mut self, order: i32) -> Self {
        self.order = Maybe::Set(order);
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
    use crate::maybe::Maybe;
    use crate::plot::Plot;
    use crate::zerod::ZeroDCoord;

    #[test]
    fn test_legend_fill_with_configuration() {
        let plot = Plot::<ZeroDCoord>::new().legend("fill", |legend| {
            legend.title("Temperature").position(LegendPosition::Right)
        });

        // Should have legend configured
        assert!(plot.legends.contains_key("fill"));
        let legend = &plot.legends["fill"];
        assert_eq!(legend.title, Maybe::Set("Temperature".to_string()));
        assert_eq!(legend.position, Maybe::Set(LegendPosition::Right));
    }

    #[test]
    fn test_legend_fill_with_visible_false() {
        let plot = Plot::<ZeroDCoord>::new().legend("fill", |legend| legend.visible(false));

        // Legend exists but is marked invisible
        assert!(plot.legends.contains_key("fill"));
        let legend = &plot.legends["fill"];
        assert_eq!(legend.visible, Maybe::Set(false));
    }

    #[test]
    fn test_legend_stroke_with_orientation() {
        let plot = Plot::<ZeroDCoord>::new().legend("stroke", |legend| {
            legend
                .title("Category")
                .orientation(LegendOrientation::Horizontal)
                .columns(3)
        });

        assert!(plot.legends.contains_key("stroke"));
        let legend = &plot.legends["stroke"];
        assert_eq!(legend.title, Maybe::Set("Category".to_string()));
        assert_eq!(
            legend.orientation,
            Maybe::Set(LegendOrientation::Horizontal)
        );
        assert_eq!(legend.columns, Maybe::Set(3));
    }

    #[test]
    fn test_legend_size_with_symbol_size() {
        let plot = Plot::<ZeroDCoord>::new().legend("size", |legend| {
            legend.title("Population").symbol_size(20.0)
        });

        assert!(plot.legends.contains_key("size"));
        let legend = &plot.legends["size"];
        assert_eq!(legend.title, Maybe::Set("Population".to_string()));
        assert_eq!(legend.symbol_size, Maybe::Set(20.0));
    }

    #[test]
    fn test_legend_opacity_with_gradient() {
        let plot = Plot::<ZeroDCoord>::new().legend("opacity", |legend| {
            legend
                .title("Confidence")
                .gradient_length(150.0)
                .gradient_thickness(15.0)
        });

        assert!(plot.legends.contains_key("opacity"));
        let legend = &plot.legends["opacity"];
        assert_eq!(legend.title, Maybe::Set("Confidence".to_string()));
        assert_eq!(legend.gradient_length, Maybe::Set(150.0));
        assert_eq!(legend.gradient_thickness, Maybe::Set(15.0));
    }

    #[test]
    fn test_multiple_legends() {
        let plot = Plot::<ZeroDCoord>::new()
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
            .legend("fill", |legend| legend.title("First Title"))
            .legend("fill", |legend| legend.title("Updated Title"));

        // Second call should update the existing legend
        assert_eq!(plot.legends.len(), 1);
        let legend = &plot.legends["fill"];
        assert_eq!(legend.title, Maybe::Set("Updated Title".to_string()));
    }
}
