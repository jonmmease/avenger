use std::{any::Any, sync::Arc};

use datafusion::{common::ScalarValue, logical_expr::Expr, prelude::lit};
use datafusion_proto::{
    logical_plan::{DefaultLogicalExtensionCodec, to_proto::serialize_expr},
    protobuf::LogicalExprNode,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    ChartEventBinding, IntoExpr, Maybe, MaybeOptionalExpr, SerializableNestedScalarMap,
    validate_structural_id,
};

mod serde_colorbar_overlays {
    use std::{any::Any, sync::Arc};

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S>(
        _value: &Vec<Arc<dyn Any + Send + Sync>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Vec::<()>::new().serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Vec<Arc<dyn Any + Send + Sync>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let _ = Vec::<()>::deserialize(deserializer)?;
        Ok(Vec::new())
    }
}

trait LogicalExprNodeExt {
    fn from_expr(expr: Expr) -> Result<LogicalExprNode, String>;
}

impl LogicalExprNodeExt for LogicalExprNode {
    fn from_expr(expr: Expr) -> Result<LogicalExprNode, String> {
        let codec = DefaultLogicalExtensionCodec {};
        serialize_expr(&expr, &codec).map_err(|err| err.to_string())
    }
}

/// Legend configuration for visualizations
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct Legend {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub explicit_ids: Vec<String>,
    #[serde(default)]
    pub event_bindings: Vec<ChartEventBinding>,
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
    pub background_stroke_width: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub background_corner_radius: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub background_padding: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub order: Maybe<Option<LogicalExprNode>>,
    pub contributing_marks: Vec<String>,
    /// Channels that have been merged into this legend (for layout width calculation)
    pub merged_channels: Vec<String>,
    /// Text colors from theme
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title_color: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_color: Maybe<Option<LogicalExprNode>>,
    /// Theme mark defaults (for legend symbol rendering)
    #[serde_as(as = "Option<FromInto<SerializableNestedScalarMap>>")]
    pub theme_mark_defaults: Option<IndexMap<String, IndexMap<String, ScalarValue>>>,
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
    #[serde(with = "serde_colorbar_overlays")]
    pub colorbar_overlays: Vec<Arc<dyn Any + Send + Sync>>,
}

impl std::fmt::Debug for Legend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Legend")
            .field("id", &self.id)
            .field("explicit_ids", &self.explicit_ids)
            .field("event_bindings", &self.event_bindings.len())
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
            .field("background_stroke_width", &self.background_stroke_width)
            .field("background_corner_radius", &self.background_corner_radius)
            .field("background_padding", &self.background_padding)
            .field("order", &self.order)
            .field("contributing_marks", &self.contributing_marks)
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

impl Legend {
    pub fn new() -> Self {
        Self {
            id: None,
            explicit_ids: Vec::new(),
            event_bindings: Vec::new(),
            colorbar_overlays: Vec::new(),
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
            background_stroke_width: Maybe::Unset,
            background_corner_radius: Maybe::Unset,
            background_padding: Maybe::Unset,
            order: Maybe::Unset,
            contributing_marks: Vec::new(),
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
        if self.id.is_none() {
            self.id = other.id.clone();
        }
        self.explicit_ids.extend(other.explicit_ids);
        self.event_bindings.extend(other.event_bindings);
        self.colorbar_overlays.extend(other.colorbar_overlays);
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
        if other.background_stroke_width.is_set() {
            self.background_stroke_width = other.background_stroke_width;
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

    pub fn id(mut self, id: impl Into<String>) -> Self {
        let id = id.into();
        self.id = Some(id.clone());
        self.explicit_ids.push(id);
        self
    }

    pub fn event_binding(mut self, binding: ChartEventBinding) -> Self {
        self.event_bindings.push(binding);
        self
    }

    pub fn event_bindings(mut self, bindings: impl IntoIterator<Item = ChartEventBinding>) -> Self {
        self.event_bindings.extend(bindings);
        self
    }

    #[doc(hidden)]
    pub fn colorbar_overlay_any(mut self, overlay: Arc<dyn Any + Send + Sync>) -> Self {
        self.colorbar_overlays.push(overlay);
        self
    }

    pub fn validate_event_surface(&self) -> Result<(), crate::AvengerChartError> {
        for id in &self.explicit_ids {
            validate_structural_id("legend id", id)?;
        }
        let distinct = self
            .explicit_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if distinct.len() > 1 {
            return Err(crate::AvengerChartError::InvalidArgument(format!(
                "Merged legend has conflicting explicit ids: {}",
                distinct.into_iter().cloned().collect::<Vec<_>>().join(", ")
            )));
        }
        for binding in &self.event_bindings {
            binding.validate()?;
        }
        Ok(())
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        let expr = visible.into_expr();
        self.visible = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize visible expr"),
        ));
        self
    }

    pub fn title(mut self, title: impl IntoExpr) -> Self {
        let expr = title.into_expr();
        self.title = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title expr"),
        ));
        self
    }

    pub fn position(mut self, position: impl IntoExpr) -> Self {
        let expr = position.into_expr();
        self.position = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize position expr"),
        ));
        self
    }

    pub fn orientation(mut self, orientation: impl IntoExpr) -> Self {
        let expr = orientation.into_expr();
        self.orientation = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize orientation expr"),
        ));
        self
    }

    pub fn symbol_size(mut self, size: impl IntoExpr) -> Self {
        let expr = size.into_expr();
        self.symbol_size = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize symbol_size expr"),
        ));
        self
    }

    pub fn gradient_thickness(mut self, thickness: impl IntoExpr) -> Self {
        let expr = thickness.into_expr();
        self.gradient_thickness = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize gradient_thickness expr"),
        ));
        self
    }

    pub fn columns(mut self, columns: impl IntoExpr) -> Self {
        let expr = columns.into_expr();
        self.columns = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize columns expr"),
        ));
        self
    }

    pub fn label_limit(mut self, limit: impl IntoExpr) -> Self {
        let expr = limit.into_expr();
        self.label_limit = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_limit expr"),
        ));
        self
    }

    /// Set a numeric formatting string for legend labels.
    pub fn format_number(mut self, pattern: impl IntoExpr) -> Self {
        let expr = pattern.into_expr();
        self.format_number = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize format_number expr"),
        ));
        self
    }

    pub fn background_fill(mut self, color: impl IntoExpr) -> Self {
        let expr = color.into_expr();
        self.background_fill = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize background_fill expr"),
        ));
        self
    }

    pub fn background_stroke(mut self, color: impl IntoExpr) -> Self {
        let expr = color.into_expr();
        self.background_stroke = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize background_stroke expr"),
        ));
        self
    }

    pub fn background_stroke_width(mut self, width: impl IntoExpr) -> Self {
        let expr = width.into_expr();
        self.background_stroke_width = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr)
                .expect("Failed to serialize background_stroke_width expr"),
        ));
        self
    }

    pub fn background_corner_radius(mut self, r: impl IntoExpr) -> Self {
        let expr = r.into_expr();
        self.background_corner_radius = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr)
                .expect("Failed to serialize background_corner_radius expr"),
        ));
        self
    }

    pub fn background_padding(mut self, pad: impl IntoExpr) -> Self {
        let expr = pad.into_expr();
        self.background_padding = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize background_padding expr"),
        ));
        self
    }

    pub fn order(mut self, order: impl IntoExpr) -> Self {
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
    pub fn title_color(mut self, color: impl IntoExpr) -> Self {
        let expr = color.into_expr();
        self.title_color = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title_color expr"),
        ));
        self
    }

    /// Set label color
    pub fn label_color(mut self, color: impl IntoExpr) -> Self {
        let expr = color.into_expr();
        self.label_color = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_color expr"),
        ));
        self
    }

    /// Set tick color
    pub fn tick_color(mut self, color: impl IntoExpr) -> Self {
        let expr = color.into_expr();
        self.tick_color = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize tick_color expr"),
        ));
        self
    }

    /// Set title font family
    pub fn title_font_family(mut self, family: impl IntoExpr) -> Self {
        let expr = family.into_expr();
        self.title_font_family = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title_font_family expr"),
        ));
        self
    }

    /// Set title font size
    pub fn title_font_size(mut self, size: impl IntoExpr) -> Self {
        let expr = size.into_expr();
        self.title_font_size = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title_font_size expr"),
        ));
        self
    }

    /// Set title font weight
    pub fn title_font_weight(mut self, weight: impl IntoExpr) -> Self {
        let expr = weight.into_expr();
        self.title_font_weight = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title_font_weight expr"),
        ));
        self
    }

    /// Set label font family
    pub fn label_font_family(mut self, family: impl IntoExpr) -> Self {
        let expr = family.into_expr();
        self.label_font_family = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_font_family expr"),
        ));
        self
    }

    /// Set label font size
    pub fn label_font_size(mut self, size: impl IntoExpr) -> Self {
        let expr = size.into_expr();
        self.label_font_size = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_font_size expr"),
        ));
        self
    }

    /// Set label font weight
    pub fn label_font_weight(mut self, weight: impl IntoExpr) -> Self {
        let expr = weight.into_expr();
        self.label_font_weight = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_font_weight expr"),
        ));
        self
    }

    /// Set tick font family
    pub fn tick_font_family(mut self, family: impl IntoExpr) -> Self {
        let expr = family.into_expr();
        self.tick_font_family = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize tick_font_family expr"),
        ));
        self
    }

    /// Set tick font size
    pub fn tick_font_size(mut self, size: impl IntoExpr) -> Self {
        let expr = size.into_expr();
        self.tick_font_size = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize tick_font_size expr"),
        ));
        self
    }

    /// Set tick font weight
    pub fn tick_font_weight(mut self, weight: impl IntoExpr) -> Self {
        let expr = weight.into_expr();
        self.tick_font_weight = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize tick_font_weight expr"),
        ));
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
    use datafusion::prelude::lit;

    use crate::{ChartEventBinding, ChartEventType};

    use super::*;

    #[test]
    fn legend_event_bindings_update_and_validate() {
        let binding = ChartEventBinding::on(ChartEventType::Click).filter(lit(true));
        let merged = Legend::new()
            .id("primary")
            .update(Legend::new().event_binding(binding));

        merged
            .validate_event_surface()
            .expect("merged legend binding validates");
        assert_eq!(merged.id.as_deref(), Some("primary"));
        assert_eq!(merged.explicit_ids, vec!["primary".to_string()]);
        assert_eq!(merged.event_bindings.len(), 1);
    }

    #[test]
    fn merged_legend_conflicting_ids_error() {
        let merged = Legend::new().id("left").update(Legend::new().id("right"));
        let err = merged
            .validate_event_surface()
            .expect_err("conflicting legend ids should fail");
        assert!(err.to_string().contains("conflicting explicit ids"));
    }
}
