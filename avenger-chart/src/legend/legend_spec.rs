use super::LegendRenderer;
use crate::maybe::{Maybe, MaybeOptionalExpr};
use crate::serialization::SerializableNestedScalarMap;
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::sync::Arc;

/// Legend configuration for visualizations
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct Legend {
    #[serde_as(as = "MaybeOptionalExpr")]
    pub visible: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub position: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub orientation: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub symbol_size: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub gradient_thickness: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub columns: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_limit: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub format_number: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub background_fill: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub background_stroke: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub background_corner_radius: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub background_padding: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub order: Maybe<Option<LogicalExprNode>>,
    pub contributing_marks: Vec<String>,
    /// Optional custom renderer override
    pub renderer: Option<Arc<dyn LegendRenderer>>,
    /// Channels that have been merged into this legend (for layout width calculation)
    pub merged_channels: Vec<String>,
    /// Text colors from theme
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title_color: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_color: Maybe<Option<LogicalExprNode>>,
    /// Theme mark defaults (for legend symbol rendering)
    #[serde_as(as = "Option<FromInto<SerializableNestedScalarMap>>")]
    pub theme_mark_defaults: Option<
        indexmap::IndexMap<String, indexmap::IndexMap<String, datafusion_common::ScalarValue>>,
    >,
    /// Typography from theme
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title_font_family: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title_font_size: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title_font_weight: Maybe<Option<LogicalExprNode>>,
    /// Item label typography (for discrete legends: symbol, line, rect)
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_font_family: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_font_size: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_font_weight: Maybe<Option<LogicalExprNode>>,
    /// Tick label typography (for continuous legends: colorbar)
    #[serde_as(as = "MaybeOptionalExpr")]
    pub tick_font_family: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub tick_font_size: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub tick_font_weight: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub tick_color: Maybe<Option<LogicalExprNode>>,
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
        use crate::serialization::LogicalExprNodeExt;
        use datafusion::prelude::lit;

        Self {
            visible: Maybe::Set(Some(
                LogicalExprNode::from_expr(lit(true)).expect("Failed to serialize visible expr"),
            )),
            position: Maybe::Unset,
            title: Maybe::Unset,
            orientation: Maybe::Unset,
            symbol_size: Maybe::Unset,
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

    pub fn visible(mut self, visible: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = visible.into_expr();
        self.visible = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize visible expr"),
        ));
        self
    }

    pub fn title(mut self, title: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = title.into_expr();
        self.title = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title expr"),
        ));
        self
    }

    pub fn position(mut self, position: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = position.into_expr();
        self.position = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize position expr"),
        ));
        self
    }

    pub fn orientation(mut self, orientation: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = orientation.into_expr();
        self.orientation = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize orientation expr"),
        ));
        self
    }

    pub fn symbol_size(mut self, size: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = size.into_expr();
        self.symbol_size = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize symbol_size expr"),
        ));
        self
    }

    pub fn gradient_thickness(mut self, thickness: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = thickness.into_expr();
        self.gradient_thickness = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize gradient_thickness expr"),
        ));
        self
    }

    pub fn columns(mut self, columns: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = columns.into_expr();
        self.columns = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize columns expr"),
        ));
        self
    }

    pub fn label_limit(mut self, limit: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = limit.into_expr();
        self.label_limit = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_limit expr"),
        ));
        self
    }

    /// Set a numeric formatting string for legend labels.
    pub fn format_number(mut self, pattern: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = pattern.into_expr();
        self.format_number = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize format_number expr"),
        ));
        self
    }

    pub fn background_fill(mut self, color: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = color.into_expr();
        self.background_fill = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize background_fill expr"),
        ));
        self
    }

    pub fn background_stroke(mut self, color: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = color.into_expr();
        self.background_stroke = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize background_stroke expr"),
        ));
        self
    }

    pub fn background_corner_radius(mut self, r: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = r.into_expr();
        self.background_corner_radius = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr)
                .expect("Failed to serialize background_corner_radius expr"),
        ));
        self
    }

    pub fn background_padding(mut self, pad: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = pad.into_expr();
        self.background_padding = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize background_padding expr"),
        ));
        self
    }

    pub fn order(mut self, order: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = order.into_expr();
        self.order = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize order expr"),
        ));
        self
    }

    pub fn add_contributing_mark(mut self, mark_id: impl Into<String>) -> Self {
        self.contributing_marks.push(mark_id.into());
        self
    }

    /// Set title color
    pub fn title_color(mut self, color: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = color.into_expr();
        self.title_color = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title_color expr"),
        ));
        self
    }

    /// Set label color
    pub fn label_color(mut self, color: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = color.into_expr();
        self.label_color = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_color expr"),
        ));
        self
    }

    /// Set tick color
    pub fn tick_color(mut self, color: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = color.into_expr();
        self.tick_color = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize tick_color expr"),
        ));
        self
    }

    /// Set title font family
    pub fn title_font_family(mut self, family: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = family.into_expr();
        self.title_font_family = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title_font_family expr"),
        ));
        self
    }

    /// Set title font size
    pub fn title_font_size(mut self, size: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = size.into_expr();
        self.title_font_size = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title_font_size expr"),
        ));
        self
    }

    /// Set title font weight
    pub fn title_font_weight(mut self, weight: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = weight.into_expr();
        self.title_font_weight = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title_font_weight expr"),
        ));
        self
    }

    /// Set label font family
    pub fn label_font_family(mut self, family: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = family.into_expr();
        self.label_font_family = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_font_family expr"),
        ));
        self
    }

    /// Set label font size
    pub fn label_font_size(mut self, size: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = size.into_expr();
        self.label_font_size = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_font_size expr"),
        ));
        self
    }

    /// Set label font weight
    pub fn label_font_weight(mut self, weight: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = weight.into_expr();
        self.label_font_weight = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_font_weight expr"),
        ));
        self
    }

    /// Set tick font family
    pub fn tick_font_family(mut self, family: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = family.into_expr();
        self.tick_font_family = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize tick_font_family expr"),
        ));
        self
    }

    /// Set tick font size
    pub fn tick_font_size(mut self, size: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = size.into_expr();
        self.tick_font_size = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize tick_font_size expr"),
        ));
        self
    }

    /// Set tick font weight
    pub fn tick_font_weight(mut self, weight: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = weight.into_expr();
        self.tick_font_weight = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize tick_font_weight expr"),
        ));
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
        let plot = Plot::<ZeroDCoord>::new().legend("fill", |legend| {
            legend.title("Temperature").position(LegendPosition::Right)
        });

        // Should have legend configured
        assert!(plot.legends.contains_key("fill"));
        let legend = &plot.legends["fill"];
        // Check that fields are set (not Unset) - values are now LogicalExprNode
        assert!(legend.title.is_set());
        assert!(legend.position.is_set());
    }

    #[test]
    fn test_legend_fill_with_visible_false() {
        let plot = Plot::<ZeroDCoord>::new().legend("fill", |legend| legend.visible(false));

        // Legend exists but is marked invisible
        assert!(plot.legends.contains_key("fill"));
        let legend = &plot.legends["fill"];
        // Check that visible is set
        assert!(legend.visible.is_set());
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
        // Check that fields are set
        assert!(legend.title.is_set());
        assert!(legend.orientation.is_set());
        assert!(legend.columns.is_set());
    }

    #[test]
    fn test_legend_size_with_symbol_size() {
        let plot = Plot::<ZeroDCoord>::new().legend("size", |legend| {
            legend.title("Population").symbol_size(20.0)
        });

        assert!(plot.legends.contains_key("size"));
        let legend = &plot.legends["size"];
        // Check that fields are set
        assert!(legend.title.is_set());
        assert!(legend.symbol_size.is_set());
    }

    #[test]
    fn test_legend_opacity_with_gradient() {
        let plot = Plot::<ZeroDCoord>::new().legend("opacity", |legend| {
            legend.title("Confidence").gradient_thickness(15.0)
        });

        assert!(plot.legends.contains_key("opacity"));
        let legend = &plot.legends["opacity"];
        // Check that fields are set
        assert!(legend.title.is_set());
        assert!(legend.gradient_thickness.is_set());
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
        // Check that title is set (the second value overwrote the first)
        assert!(legend.title.is_set());
    }
}
