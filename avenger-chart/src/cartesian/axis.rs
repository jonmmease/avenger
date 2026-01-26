use crate::axis::Axis;
use crate::error::AvengerChartError;
use crate::maybe::{Maybe, MaybeOptionalExpr};
use crate::theme::Theme;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::any::Any;

/// Position for Cartesian axes
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AxisPosition {
    Top,
    Right,
    Bottom,
    Left,
}

/// Concrete struct for Cartesian axes
/// Using a struct instead of a trait enables type inference in closure parameters
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CartesianAxis {
    #[serde_as(as = "MaybeOptionalExpr")]
    pub visible: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub position: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub grid: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub tick_count: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_angle: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub format_number: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title_font_family: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_font_family: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub show_title: Maybe<Option<LogicalExprNode>>,
}

impl CartesianAxis {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(mut self, visible: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = visible.into_expr();
        self.visible = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize visible expr"),
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

    pub fn title(mut self, title: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = title.into_expr();
        self.title = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title expr"),
        ));
        self
    }

    pub fn grid(mut self, grid: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = grid.into_expr();
        self.grid = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize grid expr"),
        ));
        self
    }

    pub fn tick_count(mut self, count: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = count.into_expr();
        self.tick_count = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize tick_count expr"),
        ));
        self
    }

    pub fn label_angle(mut self, angle: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = angle.into_expr();
        self.label_angle = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_angle expr"),
        ));
        self
    }

    pub fn format(mut self, format: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = format.into_expr();
        self.format_number = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize format expr"),
        ));
        self
    }

    pub fn title_font_family(mut self, font: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = font.into_expr();
        self.title_font_family = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize title_font_family expr"),
        ));
        self
    }

    pub fn label_font_family(mut self, font: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = font.into_expr();
        self.label_font_family = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize label_font_family expr"),
        ));
        self
    }

    /// Show or hide the axis title only (labels unaffected)
    pub fn show_title(mut self, show: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = show.into_expr();
        self.show_title = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize show_title expr"),
        ));
        self
    }

    /// Update this axis configuration with another, applying all set fields
    pub fn update(mut self, other: CartesianAxis) -> Self {
        if other.visible.is_set() {
            self.visible = other.visible;
        }
        if other.position.is_set() {
            self.position = other.position;
        }
        if other.title.is_set() {
            self.title = other.title;
        }
        if other.grid.is_set() {
            self.grid = other.grid;
        }
        if other.tick_count.is_set() {
            self.tick_count = other.tick_count;
        }
        if other.label_angle.is_set() {
            self.label_angle = other.label_angle;
        }
        if other.format_number.is_set() {
            self.format_number = other.format_number;
        }
        if other.title_font_family.is_set() {
            self.title_font_family = other.title_font_family;
        }
        if other.label_font_family.is_set() {
            self.label_font_family = other.label_font_family;
        }
        self
    }

    /// Evaluate this axis to scene marks
    pub async fn evaluate(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &crate::layout::LayoutBounds,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &datafusion::prelude::SessionContext,
        facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        facet_position: Option<&[usize]>,
    ) -> Result<SceneMark, AvengerChartError> {
        use crate::plot::compiled::expr_eval::{
            evaluate_axis_position_expr, evaluate_bool_expr, evaluate_string_expr,
        };
        use crate::serialization::LogicalExprNodeExt;
        use avenger_guides::axis::{
            band::make_band_axis_marks,
            numeric::make_numeric_axis_marks,
            opts::{AxisConfig, AxisOrientation},
            point::make_point_axis_marks,
        };

        // Evaluate visible expression (default to true if not set)
        let visible = if let Some(visible_node) = self.visible.as_option().and_then(|o| o.as_ref())
        {
            let visible_expr = visible_node.to_expr(ctx)?;
            evaluate_bool_expr(&visible_expr, ctx, params).await?
        } else {
            true
        };

        // Skip if invisible
        if !visible {
            return Ok(SceneMark::Group(
                avenger_scenegraph::marks::group::SceneGroup {
                    marks: vec![],
                    ..Default::default()
                },
            ));
        }

        // Evaluate position expression
        let position =
            if let Some(position_node) = self.position.as_option().and_then(|o| o.as_ref()) {
                let position_expr = position_node.to_expr(ctx)?;
                evaluate_axis_position_expr(&position_expr, ctx, params).await?
            } else {
                // Default positions based on channel name
                match channel {
                    "x" => AxisPosition::Bottom,
                    "y" => AxisPosition::Left,
                    _ => AxisPosition::Bottom,
                }
            };

        // Convert position to orientation
        let orientation = match position {
            AxisPosition::Top => AxisOrientation::Top,
            AxisPosition::Bottom => AxisOrientation::Bottom,
            AxisPosition::Left => AxisOrientation::Left,
            AxisPosition::Right => AxisOrientation::Right,
        };

        // Axis origin is always the top-left corner of the plot area
        let axis_origin = [plot_bounds.x, plot_bounds.y];

        // Use coordinate system type and channel for CSS selector support
        // e.g., guide[type="cartesian"] axis[type="x"]
        let coord_type = Some("cartesian");
        let axis_type = Some(channel);

        // Create context for theme queries with params
        let axis_ctx = theme.axis_context_with_params(coord_type, axis_type, params.clone());
        let label_ctx = axis_ctx.child("label");
        let title_ctx = axis_ctx.child("title");

        // Get theme colors (already in normalized [f32; 4] format)
        let label_color = theme.text_color(&label_ctx);
        let title_color = theme.text_color(&title_ctx);
        let domain_color = theme.stroke_color(&axis_ctx.child("domain"));
        let tick_color = theme.stroke_color(&axis_ctx.child("tick"));

        // Evaluate grid expression (default to false)
        let grid = if let Some(grid_node) = self.grid.as_option().and_then(|o| o.as_ref()) {
            let grid_expr = grid_node.to_expr(ctx)?;
            evaluate_bool_expr(&grid_expr, ctx, params).await?
        } else {
            false
        };

        // Evaluate format_number expression
        let format_number =
            if let Some(format_node) = self.format_number.as_option().and_then(|o| o.as_ref()) {
                let format_expr = format_node.to_expr(ctx)?;
                Some(evaluate_string_expr(&format_expr, ctx, params).await?)
            } else {
                None
            };

        // Evaluate label_font_family expression
        let label_font_family = if let Some(label_font_node) =
            self.label_font_family.as_option().and_then(|o| o.as_ref())
        {
            let label_font_expr = label_font_node.to_expr(ctx)?;
            Some(evaluate_string_expr(&label_font_expr, ctx, params).await?)
        } else {
            theme.font_family(&label_ctx)
        };

        // Evaluate title_font_family expression
        let title_font_family = if let Some(title_font_node) =
            self.title_font_family.as_option().and_then(|o| o.as_ref())
        {
            let title_font_expr = title_font_node.to_expr(ctx)?;
            Some(evaluate_string_expr(&title_font_expr, ctx, params).await?)
        } else {
            theme.font_family(&title_ctx)
        };

        // Evaluate show_title (default true)
        let show_title_expr =
            if let Some(node) = self.show_title.as_option().and_then(|o| o.as_ref()) {
                let expr = node.to_expr(ctx)?;
                evaluate_bool_expr(&expr, ctx, params).await.unwrap_or(true)
            } else {
                true
            };

        // Query facet-aware visibility based on cell position and axis position
        let facet_visibility = if let Some(pos) = facet_position {
            facet_tree.axis_visibility(pos, position)
        } else {
            crate::facet::evaluated_facet_tree::AxisVisibility::visible()
        };

        // Combine user-specified show_title with facet visibility
        let show_title = show_title_expr && facet_visibility.show_title;
        let labels_visible = Some(facet_visibility.show_labels);

        // Create axis config with plot dimensions and theme
        let axis_config = AxisConfig {
            orientation,
            dimensions: [plot_width, plot_height],
            grid,
            format_number,
            title_font_size: theme.font_size(&title_ctx),
            // Pass colors (potentially overridden for dark backgrounds)
            domain_color,
            tick_color,
            grid_color: theme
                .stroke_color(&axis_ctx.child("grid"))
                .map(|mut color| {
                    if let Some(opacity) = theme.opacity(&axis_ctx.child("grid")) {
                        color[3] = opacity; // Apply opacity to alpha channel
                    }
                    color
                }),
            grid_width: theme.axis_grid_width(&axis_ctx),
            label_color,
            title_color,
            tick_length: theme.axis_tick_length(&axis_ctx),
            label_font_size: theme.font_size(&label_ctx),
            label_font_weight: theme.font_weight(&label_ctx),
            title_font_weight: theme.font_weight(&title_ctx),
            label_font_family,
            title_font_family,
            title_visible: Some(show_title),
            labels_visible,
        };

        // Evaluate title expression if present
        let title = if let Some(title_node) = self.title.as_option().and_then(|o| o.as_ref()) {
            let title_expr = title_node.to_expr(ctx)?;
            evaluate_string_expr(&title_expr, ctx, params).await?
        } else {
            String::new()
        };
        let title = title.as_str();

        // Generate axis marks based on scale characteristics
        // Use domain and range kinds to determine which axis maker to use
        use avenger_scales::scales::DomainKind;

        let domain_kind = scale.scale_impl.domain_kind();

        // For categorical domains, check the scale type
        let axis_group = match domain_kind {
            DomainKind::Categorical => {
                // Use scale_type to distinguish band/point/ordinal
                let scale_type = scale.scale_impl.scale_type();
                match scale_type {
                    "band" => make_band_axis_marks(scale, title, axis_origin, &axis_config)?,
                    "point" => {
                        make_point_axis_marks(scale.clone(), title, axis_origin, &axis_config)?
                    }
                    "ordinal" => {
                        // Ordinal scales with discrete ranges need band-like rendering
                        // For ordinal scales, convert to band scale for axis rendering
                        use avenger_scales::scales::band::BandScale;
                        let band_scale = BandScale::from_point_scale(scale);
                        make_band_axis_marks(&band_scale, title, axis_origin, &axis_config)?
                    }
                    _ => {
                        return Err(AvengerChartError::InternalError(format!(
                            "Unsupported scale type '{}' for categorical domain on axis '{}'",
                            scale_type, channel
                        )));
                    }
                }
            }
            _ => {
                // All continuous domain scales use numeric axis
                make_numeric_axis_marks(scale, title, axis_origin, &axis_config)?
            }
        };

        Ok(SceneMark::Group(axis_group))
    }
}

#[typetag::serde]
impl Axis for CartesianAxis {
    fn update(&mut self, other: &dyn Axis) {
        if let Some(o) = other.as_any().downcast_ref::<CartesianAxis>() {
            *self = self.clone().update(o.clone());
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn box_clone(&self) -> Box<dyn Axis> {
        Box::new(self.clone())
    }
}
