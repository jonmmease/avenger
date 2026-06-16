use std::{any::Any, collections::BTreeMap};

pub use avenger_chart_core::AxisPosition;
use avenger_chart_core::{
    AvengerChartError, Axis, AxisGuideVisibilityPolicy, AxisVisibility, CoordinationAxis,
    GuideSharingContext, INVALID_FACET_PATH_AXIS_FALLBACK_HIDDEN_PARAM, IntoExpr, LayoutBounds,
    Maybe, MaybeOptionalExpr, ScalarValueHelpers, SharingGroupEdge, SharingLevel, Theme,
    axis_ownership_mode_from_params, collect_derived_scalar_ids, eval_to_scalars,
    evaluate_axis_position_expr, evaluate_bool_expr, evaluate_f32_expr, evaluate_string_expr,
    owner_for_edge, params_to_datafusion, project_container_edge_levels, resolve_derived_scalars,
    serialization::DefaultLogicalExprNodeExt,
};
use avenger_guides::axis::{
    band::make_band_axis_marks,
    numeric::make_numeric_axis_marks,
    opts::{AxisConfig, AxisOrientation, AxisTickSpacing},
    point::make_point_axis_marks,
};
use avenger_scales::scales::{DomainKind, band::BandScale};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use datafusion::{
    arrow::array::{Array, StructArray},
    common::ScalarValue,
    prelude::{Expr, SessionContext, lit, named_struct},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::marks::subplot::{CARTESIAN_SUBPLOT_X_CHANNEL, CARTESIAN_SUBPLOT_Y_CHANNEL};
use crate::nested_axis::{NestedAxisLevelGuideConfig, make_nested_axis_marks};

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
    pub tick_spacing: Maybe<Option<LogicalExprNode>>,
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

    /// Generate numeric axis ticks from a struct expression with `start` and `step` fields.
    ///
    /// The generated ticks are `start + n * step`, clipped to the scale domain.
    pub fn tick_spacing(mut self, spacing: impl IntoExpr) -> Self {
        self.tick_spacing = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(spacing.into_expr())
                .expect("Failed to serialize tick_spacing expr"),
        ));
        self
    }

    /// Generate numeric axis ticks from `start + n * step`, clipped to the scale domain.
    pub fn ticks_start_step(self, start: impl IntoExpr, step: impl IntoExpr) -> Self {
        self.tick_spacing(named_struct(vec![
            lit("start"),
            start.into_expr(),
            lit("step"),
            step.into_expr(),
        ]))
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
        if other.tick_spacing.is_set() {
            self.tick_spacing = other.tick_spacing;
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

fn resolve_axis_expr(
    expr: Expr,
    channel: &str,
    sharing_context: GuideSharingContext<'_>,
) -> Result<Expr, AvengerChartError> {
    if let Some(derived_scalars) = sharing_context.derived_scalars_for_channel(channel) {
        resolve_derived_scalars(expr, derived_scalars)
    } else {
        Ok(expr)
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

    fn set_default_title_expr(&mut self, title: Expr) -> Result<bool, AvengerChartError> {
        if self.title.is_set() {
            return Ok(false);
        }
        self.title = Maybe::Set(Some(LogicalExprNode::from_default_expr(title)?));
        Ok(true)
    }

    fn all_exprs(&self, ctx: &SessionContext) -> Vec<Expr> {
        [
            &self.visible,
            &self.position,
            &self.title,
            &self.grid,
            &self.tick_count,
            &self.tick_spacing,
            &self.label_angle,
            &self.format_number,
            &self.title_font_family,
            &self.label_font_family,
            &self.show_title,
        ]
        .into_iter()
        .filter_map(|maybe| maybe.as_option().and_then(|o| o.as_ref()))
        .filter_map(|node| node.to_default_expr(ctx).ok())
        .collect()
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn Axis>, AvengerChartError> {
        Ok(Box::new(CartesianAxis {
            visible: map_maybe_expr(self.visible.clone(), f)?,
            position: map_maybe_expr(self.position.clone(), f)?,
            title: map_maybe_expr(self.title.clone(), f)?,
            grid: map_maybe_expr(self.grid.clone(), f)?,
            tick_count: map_maybe_expr(self.tick_count.clone(), f)?,
            tick_spacing: map_maybe_expr(self.tick_spacing.clone(), f)?,
            label_angle: map_maybe_expr(self.label_angle.clone(), f)?,
            format_number: map_maybe_expr(self.format_number.clone(), f)?,
            title_font_family: map_maybe_expr(self.title_font_family.clone(), f)?,
            label_font_family: map_maybe_expr(self.label_font_family.clone(), f)?,
            show_title: map_maybe_expr(self.show_title.clone(), f)?,
        }))
    }
}

fn map_maybe_expr(
    value: Maybe<Option<LogicalExprNode>>,
    f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
) -> Result<Maybe<Option<LogicalExprNode>>, AvengerChartError> {
    let ctx = SessionContext::new();
    match value {
        Maybe::Unset => Ok(Maybe::Unset),
        Maybe::Set(None) => Ok(Maybe::Set(None)),
        Maybe::Set(Some(node)) => Ok(Maybe::Set(Some(LogicalExprNode::from_default_expr(f(
            node.to_default_expr(&ctx)?,
        )?)?))),
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

fn child_frame_axis_policy_scope(
    role: ChildFrameAxisOwnershipRole,
    channel: &str,
    sharing_context: GuideSharingContext<'_>,
    axis_position: AxisPosition,
    sharing_level: SharingLevel,
    policy: AxisGuideVisibilityPolicy,
) -> Option<CartesianAxisOwnershipScope> {
    match policy {
        AxisGuideVisibilityPolicy::Auto => match role {
            ChildFrameAxisOwnershipRole::Labels => child_frame_axis_ownership_scope(
                role,
                channel,
                sharing_context,
                axis_position,
                sharing_level,
            ),
            ChildFrameAxisOwnershipRole::Title => {
                child_frame_axis_title_scope(channel, sharing_context, axis_position, sharing_level)
            }
        },
        AxisGuideVisibilityPolicy::All => None,
        AxisGuideVisibilityPolicy::OuterEdges => child_frame_axis_ownership_scope(
            role,
            channel,
            sharing_context,
            axis_position,
            SharingLevel::GLOBAL,
        ),
        AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups => None,
    }
}

fn facet_axis_ownership_applies(sharing_context: GuideSharingContext<'_>) -> bool {
    if sharing_context.is_root_facet_path() {
        return false;
    }

    // Facet ownership can hide labels only when a peer facet axis is an exact
    // substitute. Axes inside positioned child frames are data-positioned inside
    // the facet cell, so the outer facet lane is not aligned with them.
    !sharing_context
        .child_frame_level_axes()
        .contains(&CoordinationAxis::Positioned)
}

fn facet_axis_visibility_for_cartesian_axis(
    sharing_context: GuideSharingContext<'_>,
    position: AxisPosition,
    facet_sharing_level: SharingLevel,
    ownership_mode: avenger_chart_core::AxisOwnershipMode,
    hide_invalid_facet_path_axes: bool,
) -> AxisVisibility {
    if !facet_axis_ownership_applies(sharing_context) {
        return AxisVisibility::visible();
    }

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
    nested_axis_levels: Option<&BTreeMap<usize, CartesianAxis>>,
) -> Result<SceneMark, AvengerChartError> {
    let visible = if let Some(visible_node) = axis.visible.as_option().and_then(|o| o.as_ref()) {
        let visible_expr =
            resolve_axis_expr(visible_node.to_default_expr(ctx)?, channel, sharing_context)?;
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
        let position_expr = resolve_axis_expr(
            position_node.to_default_expr(ctx)?,
            channel,
            sharing_context,
        )?;
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
        let grid_expr =
            resolve_axis_expr(grid_node.to_default_expr(ctx)?, channel, sharing_context)?;
        evaluate_bool_expr(&grid_expr, ctx, params).await?
    } else {
        false
    };

    let format_number =
        if let Some(format_node) = axis.format_number.as_option().and_then(|o| o.as_ref()) {
            let format_expr =
                resolve_axis_expr(format_node.to_default_expr(ctx)?, channel, sharing_context)?;
            Some(evaluate_string_expr(&format_expr, ctx, params).await?)
        } else {
            None
        };

    let label_angle =
        if let Some(angle_node) = axis.label_angle.as_option().and_then(|o| o.as_ref()) {
            let angle_expr =
                resolve_axis_expr(angle_node.to_default_expr(ctx)?, channel, sharing_context)?;
            Some(evaluate_f32_expr(&angle_expr, ctx, params).await?)
        } else {
            None
        };

    let label_font_family = if let Some(label_font_node) =
        axis.label_font_family.as_option().and_then(|o| o.as_ref())
    {
        let label_font_expr = resolve_axis_expr(
            label_font_node.to_default_expr(ctx)?,
            channel,
            sharing_context,
        )?;
        Some(evaluate_string_expr(&label_font_expr, ctx, params).await?)
    } else {
        theme.font_family(&label_ctx)
    };

    let title_font_family = if let Some(title_font_node) =
        axis.title_font_family.as_option().and_then(|o| o.as_ref())
    {
        let title_font_expr = resolve_axis_expr(
            title_font_node.to_default_expr(ctx)?,
            channel,
            sharing_context,
        )?;
        Some(evaluate_string_expr(&title_font_expr, ctx, params).await?)
    } else {
        theme.font_family(&title_ctx)
    };

    let show_title_expr = if let Some(node) = axis.show_title.as_option().and_then(|o| o.as_ref()) {
        let expr = resolve_axis_expr(node.to_default_expr(ctx)?, channel, sharing_context)?;
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
    let facet_visibility = facet_axis_visibility_for_cartesian_axis(
        sharing_context,
        position,
        facet_sharing_level,
        ownership_mode,
        hide_invalid_facet_path_axes,
    );

    let labels_policy_is_auto = sharing_context
        .axis_guide_visibility_config(position)
        .map(|config| config.labels == AxisGuideVisibilityPolicy::Auto)
        .unwrap_or(true);
    let jagged_labels_override = labels_policy_is_auto
        && facet_axis_ownership_applies(sharing_context)
        && !facet_sharing_level.is_free()
        && sharing_context.facet_is_jagged_for_axis(position);

    let child_frame_policy = sharing_context
        .child_frame_axis_guide_visibility_config(child_frame_axis_for_position(position));
    let child_frame_labels_visible = axis_owner_for_scope(child_frame_axis_policy_scope(
        ChildFrameAxisOwnershipRole::Labels,
        channel,
        sharing_context,
        position,
        child_frame_sharing_level,
        child_frame_policy.labels,
    ));
    let child_frame_title_visible = axis_owner_for_scope(child_frame_axis_policy_scope(
        ChildFrameAxisOwnershipRole::Title,
        channel,
        sharing_context,
        position,
        child_frame_sharing_level,
        child_frame_policy.title,
    ));

    let show_title = show_title_expr && facet_visibility.show_title && child_frame_title_visible;
    let labels_visible = Some(
        (facet_visibility.show_labels || jagged_labels_override) && child_frame_labels_visible,
    );

    let tick_count = if let Some(tc_node) = axis.tick_count.as_option().and_then(|o| o.as_ref()) {
        let tc_expr = resolve_axis_expr(tc_node.to_default_expr(ctx)?, channel, sharing_context)?;
        Some(evaluate_f32_expr(&tc_expr, ctx, params).await?)
    } else {
        None
    };
    let tick_start_step =
        evaluate_tick_spacing(axis, channel, ctx, params, sharing_context).await?;

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
        label_angle,
        title_font_weight: theme.font_weight(&title_ctx),
        label_font_family,
        title_font_family,
        title_visible: Some(show_title),
        labels_visible,
        tick_count,
        tick_start_step,
    };

    let title = if let Some(title_node) = axis.title.as_option().and_then(|o| o.as_ref()) {
        let title_expr =
            resolve_axis_expr(title_node.to_default_expr(ctx)?, channel, sharing_context)?;
        evaluate_string_expr(&title_expr, ctx, params).await?
    } else {
        String::new()
    };
    let title = title.as_str();

    let nested_axis_level_configs = match scale.scale_impl.domain_kind() {
        DomainKind::NestedCategorical => Some(
            evaluate_nested_axis_level_configs(
                nested_axis_levels,
                channel,
                ctx,
                params,
                sharing_context,
            )
            .await?,
        ),
        _ => None,
    };

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
        DomainKind::NestedCategorical => make_nested_axis_marks(
            scale,
            title,
            axis_origin,
            &axis_config,
            nested_axis_level_configs.as_ref(),
        )?,
        _ => make_numeric_axis_marks(scale, title, axis_origin, &axis_config)?,
    };

    Ok(SceneMark::Group(axis_group))
}

async fn evaluate_nested_axis_level_configs(
    nested_axis_levels: Option<&BTreeMap<usize, CartesianAxis>>,
    channel: &str,
    ctx: &SessionContext,
    params: &indexmap::IndexMap<String, ScalarValue>,
    sharing_context: GuideSharingContext<'_>,
) -> Result<BTreeMap<usize, NestedAxisLevelGuideConfig>, AvengerChartError> {
    let Some(nested_axis_levels) = nested_axis_levels else {
        return Ok(BTreeMap::new());
    };

    let mut configs = BTreeMap::new();
    for (level, axis) in nested_axis_levels {
        let visible = if let Some(visible_node) = axis.visible.as_option().and_then(|o| o.as_ref())
        {
            let visible_expr =
                resolve_axis_expr(visible_node.to_default_expr(ctx)?, channel, sharing_context)?;
            evaluate_bool_expr(&visible_expr, ctx, params).await?
        } else {
            true
        };
        configs.insert(*level, NestedAxisLevelGuideConfig { visible });
    }

    Ok(configs)
}

async fn evaluate_tick_spacing(
    axis: &CartesianAxis,
    channel: &str,
    ctx: &SessionContext,
    params: &indexmap::IndexMap<String, ScalarValue>,
    sharing_context: GuideSharingContext<'_>,
) -> Result<Option<AxisTickSpacing>, AvengerChartError> {
    let Some(spacing_node) = axis.tick_spacing.as_option().and_then(|o| o.as_ref()) else {
        return Ok(None);
    };
    let spacing_expr =
        resolve_axis_expr(spacing_node.to_default_expr(ctx)?, channel, sharing_context)?;
    if !collect_derived_scalar_ids(&spacing_expr)?.is_empty() {
        return Ok(None);
    }
    let spacing = evaluate_scalar_expr(&spacing_expr, ctx, params).await?;
    Ok(Some(extract_tick_spacing(spacing)?))
}

async fn evaluate_scalar_expr(
    expr: &Expr,
    ctx: &SessionContext,
    params: &indexmap::IndexMap<String, ScalarValue>,
) -> Result<ScalarValue, AvengerChartError> {
    let scalars = eval_to_scalars(
        vec![expr.clone()],
        Some(ctx),
        params_to_datafusion(params).as_ref(),
    )
    .await
    .map_err(|err| {
        AvengerChartError::InternalError(format!("Failed to evaluate scalar expression: {err}"))
    })?;

    scalars
        .into_iter()
        .next()
        .ok_or_else(|| AvengerChartError::InternalError("No value returned".to_string()))
}

fn extract_tick_spacing(spacing: ScalarValue) -> Result<AxisTickSpacing, AvengerChartError> {
    let ScalarValue::Struct(struct_array) = spacing else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Axis tick_spacing must evaluate to a struct with start and step fields, got {spacing}"
        )));
    };

    if struct_array.len() != 1 {
        return Err(AvengerChartError::InvalidArgument(
            "Axis tick_spacing struct must contain exactly one row".to_string(),
        ));
    }

    let start = tick_spacing_field(&struct_array, "start")?;
    let step = tick_spacing_field(&struct_array, "step")?;
    if let (Ok(start), Ok(step)) = (start.as_f32(), step.as_f32()) {
        return Ok(AxisTickSpacing::Numeric { start, step });
    }

    let start_millis = tick_spacing_start_millis(&start)?;
    let (months, days, nanos) = tick_spacing_interval_parts(&step)?;
    Ok(AxisTickSpacing::Temporal {
        start_millis,
        months,
        days,
        nanos,
    })
}

fn tick_spacing_field(
    struct_array: &StructArray,
    name: &str,
) -> Result<ScalarValue, AvengerChartError> {
    let (field_index, _) = struct_array
        .fields()
        .iter()
        .enumerate()
        .find(|(_, field)| field.name() == name)
        .ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "Axis tick_spacing struct is missing required field '{name}'"
            ))
        })?;
    let value =
        ScalarValue::try_from_array(struct_array.column(field_index), 0).map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Failed to read axis tick_spacing field '{name}': {err}"
            ))
        })?;
    if value.is_null() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Axis tick_spacing field '{name}' must not be null"
        )));
    }
    Ok(value)
}

fn tick_spacing_start_millis(start: &ScalarValue) -> Result<i64, AvengerChartError> {
    match start {
        ScalarValue::Date32(Some(days)) => Ok(i64::from(*days) * 86_400_000),
        ScalarValue::Date64(Some(millis)) => Ok(*millis),
        ScalarValue::TimestampSecond(Some(value), _) => Ok(*value * 1_000),
        ScalarValue::TimestampMillisecond(Some(value), _) => Ok(*value),
        ScalarValue::TimestampMicrosecond(Some(value), _) => Ok(*value / 1_000),
        ScalarValue::TimestampNanosecond(Some(value), _) => Ok(*value / 1_000_000),
        _ => Err(AvengerChartError::InvalidArgument(format!(
            "Axis temporal tick_spacing start must be a date or timestamp, got {start}"
        ))),
    }
}

fn tick_spacing_interval_parts(step: &ScalarValue) -> Result<(i32, i32, i64), AvengerChartError> {
    let ScalarValue::IntervalMonthDayNano(Some(value)) = step else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Axis tick_spacing step must be numeric or an IntervalMonthDayNano scalar, got {step}"
        )));
    };
    Ok(datafusion::arrow::array::types::IntervalMonthDayNanoType::to_parts(*value))
}

#[cfg(test)]
mod tests {
    use super::{
        AxisPosition, CartesianAxis, ChildFrameAxisOwnershipRole, child_frame_axis_ownership_scope,
        child_frame_axis_policy_scope, child_frame_axis_title_scope,
        default_axis_position_for_channel, extract_tick_spacing, facet_axis_ownership_applies,
        facet_axis_visibility_for_cartesian_axis,
    };
    use avenger_chart_core::{
        AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM, Axis, AxisGuideVisibilityPolicy, AxisOwnershipMode,
        AxisVisibility, ChildFrameGuideSharingView, CoordinationAxis, DefaultLogicalExprNodeExt,
        FacetGuideSharingView, GuideSharingContext, RepeatContext, ResolvedRepeatVariable,
        SharingLevel, axis_owner_ignore_empty_cells_from_params, axis_ownership_mode_from_params,
        repeat, simplify_to_scalar_sync,
    };
    use avenger_guides::axis::opts::AxisTickSpacing;
    use datafusion::arrow::{
        array::{
            ArrayRef, Float64Array, IntervalMonthDayNanoArray, StructArray,
            TimestampMillisecondArray,
        },
        datatypes::{DataType, Field},
    };
    use datafusion::common::ScalarValue;
    use datafusion::prelude::col;
    use indexmap::IndexMap;
    use std::sync::Arc;

    use crate::marks::subplot::{CARTESIAN_SUBPLOT_X_CHANNEL, CARTESIAN_SUBPLOT_Y_CHANNEL};

    fn resolved_repeat(
        id: &str,
        expr: datafusion::logical_expr::Expr,
        title: &str,
    ) -> ResolvedRepeatVariable {
        ResolvedRepeatVariable {
            id: id.to_string(),
            expr,
            title: title.to_string(),
            type_hint: None,
        }
    }

    #[test]
    fn map_exprs_resolves_repeat_placeholders_in_axis_config() {
        let repeat_context =
            RepeatContext::new().with_column(resolved_repeat("col_b", col("b"), "Column B"), 2, 4);
        let axis = CartesianAxis::new()
            .title(repeat::column_title())
            .tick_count(repeat::column_index());
        let mapped = axis
            .map_exprs(&mut |expr| repeat::resolve_repeat_placeholders(expr, &repeat_context))
            .expect("axis expressions resolve");
        let mapped = mapped
            .as_any()
            .downcast_ref::<CartesianAxis>()
            .expect("cartesian axis");

        let title = mapped
            .title
            .as_option()
            .and_then(|node| node.as_ref())
            .expect("title")
            .to_default_expr(&datafusion::prelude::SessionContext::new())
            .expect("title expr");
        assert_eq!(
            simplify_to_scalar_sync(title).expect("title scalar"),
            ScalarValue::Utf8(Some("Column B".to_string()))
        );

        let tick_count = mapped
            .tick_count
            .as_option()
            .and_then(|node| node.as_ref())
            .expect("tick count")
            .to_default_expr(&datafusion::prelude::SessionContext::new())
            .expect("tick count expr");
        assert_eq!(
            simplify_to_scalar_sync(tick_count).expect("tick count scalar"),
            ScalarValue::Int64(Some(2))
        );
    }

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
        fn root() -> Self {
            Self {
                position_indices: vec![],
                level_counts: vec![],
                level_axes: vec![],
            }
        }

        fn new(index: usize, count: usize, axis: CoordinationAxis) -> Self {
            Self {
                position_indices: vec![index],
                level_counts: vec![count],
                level_axes: vec![axis],
            }
        }

        fn grid(row: usize, rows: usize, column: usize, columns: usize) -> Self {
            Self {
                position_indices: vec![row, column],
                level_counts: vec![rows, columns],
                level_axes: vec![CoordinationAxis::Vertical, CoordinationAxis::Horizontal],
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

    #[derive(Debug, Default)]
    struct HiddenFacetView;

    impl FacetGuideSharingView for HiddenFacetView {
        fn channel_axis_visibility_for_path_checked(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
        ) -> Option<AxisVisibility> {
            Some(AxisVisibility::hidden())
        }

        fn channel_axis_visibility_for_path_checked_with_mode(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
            _ownership_mode: AxisOwnershipMode,
        ) -> Option<AxisVisibility> {
            Some(AxisVisibility::hidden())
        }

        fn is_jagged_for_axis(&self, _axis_position: AxisPosition) -> bool {
            false
        }

        fn channel_domain_sharing_level(&self, _channel: &str) -> SharingLevel {
            SharingLevel::GLOBAL
        }

        fn effective_edge_indices_for_values_at_path(
            &self,
            _facet_path: &[ScalarValue],
            _values: &[ScalarValue],
        ) -> Option<(usize, usize)> {
            None
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
    fn tick_spacing_extracts_start_step_struct() {
        let spacing = ScalarValue::Struct(Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("start", DataType::Float64, false)),
                Arc::new(Float64Array::from(vec![1.5])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("step", DataType::Float64, false)),
                Arc::new(Float64Array::from(vec![2.5])) as ArrayRef,
            ),
        ])));

        assert_eq!(
            extract_tick_spacing(spacing).expect("spacing"),
            AxisTickSpacing::Numeric {
                start: 1.5,
                step: 2.5
            }
        );
    }

    #[test]
    fn tick_spacing_extracts_temporal_start_interval_struct() {
        let step = datafusion::arrow::array::types::IntervalMonthDayNanoType::make_value(1, 0, 0);
        let spacing = ScalarValue::Struct(Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new(
                    "start",
                    DataType::Timestamp(datafusion::arrow::datatypes::TimeUnit::Millisecond, None),
                    false,
                )),
                Arc::new(TimestampMillisecondArray::from(vec![1_325_376_000_000])) as ArrayRef,
            ),
            (
                Arc::new(Field::new(
                    "step",
                    DataType::Interval(datafusion::arrow::datatypes::IntervalUnit::MonthDayNano),
                    false,
                )),
                Arc::new(IntervalMonthDayNanoArray::from(vec![step])) as ArrayRef,
            ),
        ])));

        assert_eq!(
            extract_tick_spacing(spacing).expect("spacing"),
            AxisTickSpacing::Temporal {
                start_millis: 1_325_376_000_000,
                months: 1,
                days: 0,
                nanos: 0,
            }
        );
    }

    #[test]
    fn positioned_child_axes_ignore_outer_facet_axis_suppression() {
        let facet_view = HiddenFacetView;
        let facet_path = vec![ScalarValue::Utf8(Some("North".to_string()))];
        let child_path = TestChildFrameView::new(0, 2, CoordinationAxis::Positioned);
        let context = GuideSharingContext::new(&facet_view, &facet_path, &child_path);

        assert!(!facet_axis_ownership_applies(context));
        assert_eq!(
            facet_axis_visibility_for_cartesian_axis(
                context,
                AxisPosition::Bottom,
                SharingLevel::GLOBAL,
                AxisOwnershipMode::DomainSlots,
                true,
            ),
            AxisVisibility::visible()
        );
    }

    #[test]
    fn parent_axes_still_use_outer_facet_axis_suppression() {
        let facet_view = HiddenFacetView;
        let facet_path = vec![ScalarValue::Utf8(Some("North".to_string()))];
        let child_path = TestChildFrameView::root();
        let context = GuideSharingContext::new(&facet_view, &facet_path, &child_path);

        assert!(facet_axis_ownership_applies(context));
        assert_eq!(
            facet_axis_visibility_for_cartesian_axis(
                context,
                AxisPosition::Bottom,
                SharingLevel::GLOBAL,
                AxisOwnershipMode::DomainSlots,
                true,
            ),
            AxisVisibility::hidden()
        );
    }

    #[test]
    fn concat_child_axes_still_use_outer_facet_axis_suppression() {
        let facet_view = HiddenFacetView;
        let facet_path = vec![ScalarValue::Utf8(Some("North".to_string()))];
        let child_path = TestChildFrameView::new(0, 2, CoordinationAxis::Horizontal);
        let context = GuideSharingContext::new(&facet_view, &facet_path, &child_path);

        assert!(facet_axis_ownership_applies(context));
        assert_eq!(
            facet_axis_visibility_for_cartesian_axis(
                context,
                AxisPosition::Left,
                SharingLevel::GLOBAL,
                AxisOwnershipMode::DomainSlots,
                true,
            ),
            AxisVisibility::hidden()
        );
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

    #[test]
    fn grid_shared_x_axis_owned_by_bottom_row_per_column() {
        let facet_view = EmptyFacetView;
        let top_left = TestChildFrameView::grid(0, 2, 0, 3);
        let top_left_context = sharing_context(&facet_view, &top_left);
        let top_left_scope = child_frame_axis_ownership_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "x",
            top_left_context,
            AxisPosition::Bottom,
            SharingLevel::GLOBAL,
        )
        .expect("grid should project x-axis ownership through row levels");
        assert!(!top_left_scope.current_position_owns());

        let bottom_left = TestChildFrameView::grid(1, 2, 0, 3);
        let bottom_left_context = sharing_context(&facet_view, &bottom_left);
        let bottom_left_scope = child_frame_axis_ownership_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "x",
            bottom_left_context,
            AxisPosition::Bottom,
            SharingLevel::GLOBAL,
        )
        .expect("grid should project x-axis ownership through row levels");
        assert!(bottom_left_scope.current_position_owns());
    }

    #[test]
    fn grid_shared_y_axis_owned_by_left_column_per_row() {
        let facet_view = EmptyFacetView;
        let top_left = TestChildFrameView::grid(0, 2, 0, 3);
        let top_left_context = sharing_context(&facet_view, &top_left);
        let top_left_scope = child_frame_axis_ownership_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "y",
            top_left_context,
            AxisPosition::Left,
            SharingLevel::GLOBAL,
        )
        .expect("grid should project y-axis ownership through column levels");
        assert!(top_left_scope.current_position_owns());

        let top_middle = TestChildFrameView::grid(0, 2, 1, 3);
        let top_middle_context = sharing_context(&facet_view, &top_middle);
        let top_middle_scope = child_frame_axis_ownership_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "y",
            top_middle_context,
            AxisPosition::Left,
            SharingLevel::GLOBAL,
        )
        .expect("grid should project y-axis ownership through column levels");
        assert!(!top_middle_scope.current_position_owns());
    }

    #[test]
    fn child_frame_outer_edges_compacts_free_x_axis() {
        let facet_view = EmptyFacetView;
        let top_left = TestChildFrameView::grid(0, 2, 0, 3);
        let top_left_context = sharing_context(&facet_view, &top_left);
        assert!(
            child_frame_axis_policy_scope(
                ChildFrameAxisOwnershipRole::Labels,
                "x",
                top_left_context,
                AxisPosition::Bottom,
                SharingLevel::FREE,
                AxisGuideVisibilityPolicy::Auto,
            )
            .is_none()
        );
        let top_left_context = sharing_context(&facet_view, &top_left);
        let outer_scope = child_frame_axis_policy_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "x",
            top_left_context,
            AxisPosition::Bottom,
            SharingLevel::FREE,
            AxisGuideVisibilityPolicy::OuterEdges,
        )
        .expect("OuterEdges should project x-axis ownership");
        assert!(!outer_scope.current_position_owns());

        let bottom_left = TestChildFrameView::grid(1, 2, 0, 3);
        let bottom_left_context = sharing_context(&facet_view, &bottom_left);
        let bottom_outer_scope = child_frame_axis_policy_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "x",
            bottom_left_context,
            AxisPosition::Bottom,
            SharingLevel::FREE,
            AxisGuideVisibilityPolicy::OuterEdges,
        )
        .expect("OuterEdges should project x-axis ownership");
        assert!(bottom_outer_scope.current_position_owns());
    }

    #[test]
    fn child_frame_all_policy_keeps_every_axis_visible() {
        let facet_view = EmptyFacetView;
        let top_left = TestChildFrameView::grid(0, 2, 0, 3);
        let top_left_context = sharing_context(&facet_view, &top_left);
        assert!(
            child_frame_axis_policy_scope(
                ChildFrameAxisOwnershipRole::Labels,
                "x",
                top_left_context,
                AxisPosition::Bottom,
                SharingLevel::GLOBAL,
                AxisGuideVisibilityPolicy::All,
            )
            .is_none()
        );
    }
}
