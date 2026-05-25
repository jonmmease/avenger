use std::any::Any;

pub use avenger_chart_core::AxisPosition;
use avenger_chart_core::{
    AvengerChartError, Axis, AxisVisibility, CoordinationAxis, GuideSharingContext,
    INVALID_FACET_PATH_AXIS_FALLBACK_HIDDEN_PARAM, IntoExpr, LayoutBounds, Maybe,
    MaybeOptionalExpr, SharingGroupEdge, SharingLevel, Theme, axis_ownership_mode_from_params,
    evaluate_axis_position_expr, evaluate_bool_expr, evaluate_f32_expr, evaluate_string_expr,
    owner_for_edge, project_container_edge_levels, serialization::DefaultLogicalExprNodeExt,
};
use avenger_guides::axis::{
    band::make_band_axis_marks,
    numeric::make_numeric_axis_marks,
    opts::{AxisConfig, AxisOrientation},
    point::make_point_axis_marks,
};
use avenger_scales::scales::{DomainKind, band::BandScale};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use datafusion::{common::ScalarValue, prelude::SessionContext};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::marks::subplot::{CARTESIAN_SUBPLOT_X_CHANNEL, CARTESIAN_SUBPLOT_Y_CHANNEL};

/// Concrete struct for Cartesian axes.
///
/// Using a struct instead of a trait enables type inference in closure
/// parameters.
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

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        let expr = visible.into_expr();
        self.visible = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize visible expr"),
        ));
        self
    }

    pub fn position(mut self, position: impl IntoExpr) -> Self {
        let expr = position.into_expr();
        self.position = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize position expr"),
        ));
        self
    }

    pub fn title(mut self, title: impl IntoExpr) -> Self {
        let expr = title.into_expr();
        self.title = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize title expr"),
        ));
        self
    }

    pub fn grid(mut self, grid: impl IntoExpr) -> Self {
        let expr = grid.into_expr();
        self.grid = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize grid expr"),
        ));
        self
    }

    pub fn tick_count(mut self, count: impl IntoExpr) -> Self {
        let expr = count.into_expr();
        self.tick_count = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize tick_count expr"),
        ));
        self
    }

    pub fn label_angle(mut self, angle: impl IntoExpr) -> Self {
        let expr = angle.into_expr();
        self.label_angle = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize label_angle expr"),
        ));
        self
    }

    pub fn format(mut self, format: impl IntoExpr) -> Self {
        let expr = format.into_expr();
        self.format_number = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize format expr"),
        ));
        self
    }

    pub fn title_font_family(mut self, font: impl IntoExpr) -> Self {
        let expr = font.into_expr();
        self.title_font_family = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr)
                .expect("Failed to serialize title_font_family expr"),
        ));
        self
    }

    pub fn label_font_family(mut self, font: impl IntoExpr) -> Self {
        let expr = font.into_expr();
        self.label_font_family = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr)
                .expect("Failed to serialize label_font_family expr"),
        ));
        self
    }

    /// Show or hide the axis title only (labels unaffected).
    pub fn show_title(mut self, show: impl IntoExpr) -> Self {
        let expr = show.into_expr();
        self.show_title = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize show_title expr"),
        ));
        self
    }

    /// Update this axis configuration with another, applying all set fields.
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
        if other.show_title.is_set() {
            self.show_title = other.show_title;
        }
        self
    }
}

fn default_axis_position_for_channel(channel: &str) -> AxisPosition {
    match channel {
        "y" | CARTESIAN_SUBPLOT_Y_CHANNEL => AxisPosition::Left,
        "x" | CARTESIAN_SUBPLOT_X_CHANNEL => AxisPosition::Bottom,
        _ => AxisPosition::Bottom,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChildFrameAxisOwnershipRole {
    Labels,
    Title,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CartesianAxisOwnershipScope {
    edge: SharingGroupEdge,
    position_indices: Vec<usize>,
    level_counts: Vec<usize>,
    boundary: usize,
}

impl CartesianAxisOwnershipScope {
    fn current_position_owns(&self) -> bool {
        owner_for_edge(
            self.edge,
            &self.position_indices,
            &self.level_counts,
            self.boundary,
        )
    }
}

fn axis_owner_for_scope(scope: Option<CartesianAxisOwnershipScope>) -> bool {
    scope
        .as_ref()
        .map(CartesianAxisOwnershipScope::current_position_owns)
        .unwrap_or(true)
}

#[inline]
fn child_frame_axis_for_position(axis_position: AxisPosition) -> CoordinationAxis {
    match axis_position {
        AxisPosition::Top | AxisPosition::Bottom => CoordinationAxis::Vertical,
        AxisPosition::Left | AxisPosition::Right => CoordinationAxis::Horizontal,
    }
}

#[inline]
fn child_frame_edge_for_axis_position(axis_position: AxisPosition) -> SharingGroupEdge {
    match axis_position {
        AxisPosition::Top | AxisPosition::Left => SharingGroupEdge::Start,
        AxisPosition::Bottom | AxisPosition::Right => SharingGroupEdge::End,
    }
}

fn child_frame_axis_ownership_scope(
    _role: ChildFrameAxisOwnershipRole,
    _channel: &str,
    sharing_context: GuideSharingContext<'_>,
    axis_position: AxisPosition,
    sharing_level: SharingLevel,
) -> Option<CartesianAxisOwnershipScope> {
    if sharing_level.is_free() {
        return None;
    }

    let position_indices = sharing_context.child_frame_position_indices();
    let level_counts = sharing_context.child_frame_level_counts();
    let level_axes = sharing_context.child_frame_level_axes();
    let projection = project_container_edge_levels(
        &position_indices,
        &level_counts,
        &level_axes,
        child_frame_axis_for_position(axis_position),
        child_frame_edge_for_axis_position(axis_position),
    )?;

    if projection.relevant_depth == 0 {
        return None;
    }

    let sharing_level = sharing_level.clamp_to_depth(projection.relevant_depth as u8);
    let boundary = sharing_level.group_boundary(projection.path_depth());

    Some(CartesianAxisOwnershipScope {
        edge: projection.edge,
        position_indices: projection.position_indices,
        level_counts: projection.level_counts,
        boundary,
    })
}

fn child_frame_axis_title_scope(
    channel: &str,
    sharing_context: GuideSharingContext<'_>,
    axis_position: AxisPosition,
    sharing_level: SharingLevel,
) -> Option<CartesianAxisOwnershipScope> {
    if sharing_level.is_free() {
        return None;
    }

    let relevant_axis = child_frame_axis_for_position(axis_position);
    let relevant_depth = sharing_context.child_frame_relevant_depth(relevant_axis);
    if relevant_depth == 0 {
        return None;
    }

    child_frame_axis_ownership_scope(
        ChildFrameAxisOwnershipRole::Title,
        channel,
        sharing_context,
        axis_position,
        SharingLevel::from_raw(relevant_depth as u8),
    )
}

/// Evaluate a Cartesian axis to scene marks.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub async fn evaluate_cartesian_axis(
    axis: &CartesianAxis,
    channel: &str,
    scale: &avenger_scales::scales::ConfiguredScale,
    plot_width: f32,
    plot_height: f32,
    plot_bounds: &LayoutBounds,
    theme: &Theme,
    params: &indexmap::IndexMap<String, ScalarValue>,
    ctx: &SessionContext,
    sharing_context: GuideSharingContext<'_>,
    facet_sharing_level: SharingLevel,
    child_frame_sharing_level: SharingLevel,
) -> Result<SceneMark, AvengerChartError> {
    let visible = if let Some(visible_node) = axis.visible.as_option().and_then(|o| o.as_ref()) {
        let visible_expr = visible_node.to_default_expr(ctx)?;
        evaluate_bool_expr(&visible_expr, ctx, params).await?
    } else {
        true
    };

    if !visible {
        return Ok(SceneMark::Group(SceneGroup {
            marks: vec![],
            ..Default::default()
        }));
    }

    let position = if let Some(position_node) = axis.position.as_option().and_then(|o| o.as_ref()) {
        let position_expr = position_node.to_default_expr(ctx)?;
        evaluate_axis_position_expr(&position_expr, ctx, params).await?
    } else {
        default_axis_position_for_channel(channel)
    };

    let orientation = match position {
        AxisPosition::Top => AxisOrientation::Top,
        AxisPosition::Bottom => AxisOrientation::Bottom,
        AxisPosition::Left => AxisOrientation::Left,
        AxisPosition::Right => AxisOrientation::Right,
    };

    let axis_origin = [plot_bounds.x, plot_bounds.y];

    let coord_type = Some("cartesian");
    let axis_type = Some(channel);

    let axis_ctx = theme.axis_context_with_params(coord_type, axis_type, params.clone());
    let label_ctx = axis_ctx.child("label");
    let title_ctx = axis_ctx.child("title");

    let label_color = theme.text_color(&label_ctx);
    let title_color = theme.text_color(&title_ctx);
    let domain_color = theme.stroke_color(&axis_ctx.child("domain"));
    let tick_color = theme.stroke_color(&axis_ctx.child("tick"));

    let grid = if let Some(grid_node) = axis.grid.as_option().and_then(|o| o.as_ref()) {
        let grid_expr = grid_node.to_default_expr(ctx)?;
        evaluate_bool_expr(&grid_expr, ctx, params).await?
    } else {
        false
    };

    let format_number =
        if let Some(format_node) = axis.format_number.as_option().and_then(|o| o.as_ref()) {
            let format_expr = format_node.to_default_expr(ctx)?;
            Some(evaluate_string_expr(&format_expr, ctx, params).await?)
        } else {
            None
        };

    let label_font_family = if let Some(label_font_node) =
        axis.label_font_family.as_option().and_then(|o| o.as_ref())
    {
        let label_font_expr = label_font_node.to_default_expr(ctx)?;
        Some(evaluate_string_expr(&label_font_expr, ctx, params).await?)
    } else {
        theme.font_family(&label_ctx)
    };

    let title_font_family = if let Some(title_font_node) =
        axis.title_font_family.as_option().and_then(|o| o.as_ref())
    {
        let title_font_expr = title_font_node.to_default_expr(ctx)?;
        Some(evaluate_string_expr(&title_font_expr, ctx, params).await?)
    } else {
        theme.font_family(&title_ctx)
    };

    let show_title_expr = if let Some(node) = axis.show_title.as_option().and_then(|o| o.as_ref()) {
        let expr = node.to_default_expr(ctx)?;
        evaluate_bool_expr(&expr, ctx, params).await.unwrap_or(true)
    } else {
        true
    };

    let hide_invalid_facet_path_axes = params
        .get(INVALID_FACET_PATH_AXIS_FALLBACK_HIDDEN_PARAM)
        .and_then(|value| match value {
            ScalarValue::Boolean(Some(value)) => Some(*value),
            _ => None,
        })
        .unwrap_or(false);
    let ownership_mode = axis_ownership_mode_from_params(params);
    let facet_visibility = if sharing_context.is_root_facet_path() {
        AxisVisibility::visible()
    } else {
        sharing_context
            .channel_axis_visibility_for_path_checked_with_mode(
                position,
                facet_sharing_level.raw(),
                ownership_mode,
            )
            .unwrap_or_else(|| {
                if hide_invalid_facet_path_axes {
                    AxisVisibility::hidden()
                } else {
                    AxisVisibility::visible()
                }
            })
    };

    let jagged_labels_override = !sharing_context.is_root_facet_path()
        && !facet_sharing_level.is_free()
        && sharing_context.facet_is_jagged_for_axis(position);

    let child_frame_labels_visible = axis_owner_for_scope(child_frame_axis_ownership_scope(
        ChildFrameAxisOwnershipRole::Labels,
        channel,
        sharing_context,
        position,
        child_frame_sharing_level,
    ));
    let child_frame_title_visible = axis_owner_for_scope(child_frame_axis_title_scope(
        channel,
        sharing_context,
        position,
        child_frame_sharing_level,
    ));

    let show_title = show_title_expr && facet_visibility.show_title && child_frame_title_visible;
    let labels_visible = Some(
        (facet_visibility.show_labels || jagged_labels_override) && child_frame_labels_visible,
    );

    let tick_count = if let Some(tc_node) = axis.tick_count.as_option().and_then(|o| o.as_ref()) {
        let tc_expr = tc_node.to_default_expr(ctx)?;
        Some(evaluate_f32_expr(&tc_expr, ctx, params).await?)
    } else {
        None
    };

    let axis_config = AxisConfig {
        orientation,
        dimensions: [plot_width, plot_height],
        grid,
        format_number,
        title_font_size: theme.font_size(&title_ctx),
        domain_color,
        tick_color,
        grid_color: theme
            .stroke_color(&axis_ctx.child("grid"))
            .map(|mut color| {
                if let Some(opacity) = theme.opacity(&axis_ctx.child("grid")) {
                    color[3] = opacity;
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
        tick_count,
    };

    let title = if let Some(title_node) = axis.title.as_option().and_then(|o| o.as_ref()) {
        let title_expr = title_node.to_default_expr(ctx)?;
        evaluate_string_expr(&title_expr, ctx, params).await?
    } else {
        String::new()
    };
    let title = title.as_str();

    let domain_kind = scale.scale_impl.domain_kind();
    let axis_group = match domain_kind {
        DomainKind::Categorical => {
            let scale_type = scale.scale_impl.scale_type();
            match scale_type {
                "band" => make_band_axis_marks(scale, title, axis_origin, &axis_config)?,
                "point" => make_point_axis_marks(scale.clone(), title, axis_origin, &axis_config)?,
                "ordinal" => {
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
        _ => make_numeric_axis_marks(scale, title, axis_origin, &axis_config)?,
    };

    Ok(SceneMark::Group(axis_group))
}

#[cfg(test)]
mod tests {
    use super::{
        AxisPosition, ChildFrameAxisOwnershipRole, child_frame_axis_ownership_scope,
        child_frame_axis_title_scope, default_axis_position_for_channel,
    };
    use avenger_chart_core::{
        AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM, AxisOwnershipMode, AxisVisibility,
        ChildFrameGuideSharingView, CoordinationAxis, FacetGuideSharingView, GuideSharingContext,
        SharingLevel, axis_owner_ignore_empty_cells_from_params, axis_ownership_mode_from_params,
    };
    use datafusion::common::ScalarValue;
    use indexmap::IndexMap;

    use crate::marks::subplot::{CARTESIAN_SUBPLOT_X_CHANNEL, CARTESIAN_SUBPLOT_Y_CHANNEL};

    #[derive(Debug, Default)]
    struct EmptyFacetView;

    impl FacetGuideSharingView for EmptyFacetView {
        fn channel_axis_visibility_for_path_checked(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
        ) -> Option<AxisVisibility> {
            None
        }

        fn channel_axis_visibility_for_path_checked_with_mode(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
            _ownership_mode: AxisOwnershipMode,
        ) -> Option<AxisVisibility> {
            None
        }

        fn is_jagged_for_axis(&self, _axis_position: AxisPosition) -> bool {
            false
        }

        fn channel_domain_sharing_level(&self, _channel: &str) -> SharingLevel {
            SharingLevel::FREE
        }

        fn effective_edge_indices_for_values_at_path(
            &self,
            _facet_path: &[ScalarValue],
            _values: &[ScalarValue],
        ) -> Option<(usize, usize)> {
            None
        }
    }

    #[derive(Debug)]
    struct TestChildFrameView {
        position_indices: Vec<usize>,
        level_counts: Vec<usize>,
        level_axes: Vec<CoordinationAxis>,
    }

    impl TestChildFrameView {
        fn new(index: usize, count: usize, axis: CoordinationAxis) -> Self {
            Self {
                position_indices: vec![index],
                level_counts: vec![count],
                level_axes: vec![axis],
            }
        }
    }

    impl ChildFrameGuideSharingView for TestChildFrameView {
        fn position_indices(&self) -> Vec<usize> {
            self.position_indices.clone()
        }

        fn level_counts(&self) -> Vec<usize> {
            self.level_counts.clone()
        }

        fn level_axes(&self) -> Vec<CoordinationAxis> {
            self.level_axes.clone()
        }
    }

    fn sharing_context<'a>(
        facet_view: &'a EmptyFacetView,
        child_view: &'a TestChildFrameView,
    ) -> GuideSharingContext<'a> {
        GuideSharingContext::new(facet_view, &[], child_view)
    }

    #[test]
    fn ownership_mode_from_params_matches_existing_behavior() {
        let mut params = IndexMap::new();
        assert!(!axis_owner_ignore_empty_cells_from_params(&params));
        assert_eq!(
            axis_ownership_mode_from_params(&params),
            AxisOwnershipMode::DomainSlots
        );

        params.insert(
            AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM.to_string(),
            ScalarValue::Boolean(Some(true)),
        );
        assert!(axis_owner_ignore_empty_cells_from_params(&params));
        assert_eq!(
            axis_ownership_mode_from_params(&params),
            AxisOwnershipMode::NonEmptySlots
        );
    }

    #[test]
    fn subplot_position_channels_default_to_cartesian_axis_edges() {
        assert_eq!(
            default_axis_position_for_channel(CARTESIAN_SUBPLOT_X_CHANNEL),
            AxisPosition::Bottom
        );
        assert_eq!(
            default_axis_position_for_channel(CARTESIAN_SUBPLOT_Y_CHANNEL),
            AxisPosition::Left
        );
        assert_eq!(default_axis_position_for_channel("x"), AxisPosition::Bottom);
        assert_eq!(default_axis_position_for_channel("y"), AxisPosition::Left);
    }

    #[test]
    fn hconcat_shared_y_axis_owned_by_left_child() {
        let facet_view = EmptyFacetView;
        let left_path = TestChildFrameView::new(0, 2, CoordinationAxis::Horizontal);
        let context = sharing_context(&facet_view, &left_path);
        let scope = child_frame_axis_ownership_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "y",
            context,
            AxisPosition::Left,
            SharingLevel::GLOBAL,
        )
        .expect("hconcat should project y-axis ownership");
        assert!(scope.current_position_owns());

        let right_path = TestChildFrameView::new(1, 2, CoordinationAxis::Horizontal);
        let right_context = sharing_context(&facet_view, &right_path);
        let right_scope = child_frame_axis_ownership_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "y",
            right_context,
            AxisPosition::Left,
            SharingLevel::GLOBAL,
        )
        .expect("hconcat should project y-axis ownership");
        assert!(!right_scope.current_position_owns());
    }

    #[test]
    fn hconcat_free_y_axis_title_not_owned_by_child_frame() {
        let facet_view = EmptyFacetView;
        let path = TestChildFrameView::new(1, 2, CoordinationAxis::Horizontal);
        let context = sharing_context(&facet_view, &path);
        assert!(
            child_frame_axis_title_scope("y", context, AxisPosition::Left, SharingLevel::FREE)
                .is_none()
        );
    }

    #[test]
    fn vconcat_shared_x_axis_owned_by_bottom_child() {
        let facet_view = EmptyFacetView;
        let top_path = TestChildFrameView::new(0, 2, CoordinationAxis::Vertical);
        let top_context = sharing_context(&facet_view, &top_path);
        let top_scope = child_frame_axis_ownership_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "x",
            top_context,
            AxisPosition::Bottom,
            SharingLevel::GLOBAL,
        )
        .expect("vconcat should project x-axis ownership");
        assert!(!top_scope.current_position_owns());

        let bottom_path = TestChildFrameView::new(1, 2, CoordinationAxis::Vertical);
        let bottom_context = sharing_context(&facet_view, &bottom_path);
        let bottom_scope = child_frame_axis_ownership_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "x",
            bottom_context,
            AxisPosition::Bottom,
            SharingLevel::GLOBAL,
        )
        .expect("vconcat should project x-axis ownership");
        assert!(bottom_scope.current_position_owns());
    }
}
