use avenger_guides::axis::{
    band::make_band_axis_marks,
    numeric::make_numeric_axis_marks,
    opts::{AxisConfig, AxisOrientation},
    point::make_point_axis_marks,
};
use avenger_scales::scales::{DomainKind, band::BandScale};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};

pub use avenger_chart_cartesian::{AxisPosition, CartesianAxis};

use crate::{
    chart_core::{
        CoordinationAxis, SharingLevel, evaluate_axis_position_expr, evaluate_bool_expr,
        evaluate_f32_expr, evaluate_string_expr,
    },
    error::AvengerChartError,
    facet::ownership_policy::axis_ownership_mode_from_ignore_empty_cells,
    guide::{AxisOwnershipMode, AxisVisibility, GuideSharingContext},
    layout::LayoutBounds,
    plot::compiled::{
        CoordinationKind, EdgeOwnershipRequest, EdgeOwnershipScope, SharingGroupEdge,
        edge_ownership_scope_for_request, owner_for_scope, project_container_edge_levels,
    },
    render::context::{
        AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM, INVALID_FACET_PATH_AXIS_FALLBACK_HIDDEN_PARAM,
    },
    serialization::LogicalExprNodeExt,
    theme::Theme,
};

fn axis_owner_ignore_empty_cells_from_params(
    params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
) -> bool {
    params
        .get(AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM)
        .and_then(|value| match value {
            datafusion::common::ScalarValue::Boolean(Some(v)) => Some(*v),
            _ => None,
        })
        .unwrap_or(false)
}

fn axis_ownership_mode_from_params(
    params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
) -> AxisOwnershipMode {
    axis_ownership_mode_from_ignore_empty_cells(axis_owner_ignore_empty_cells_from_params(params))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChildFrameAxisOwnershipRole {
    Labels,
    Title,
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
    role: ChildFrameAxisOwnershipRole,
    channel: &str,
    sharing_context: GuideSharingContext<'_>,
    axis_position: AxisPosition,
    sharing_level: SharingLevel,
) -> Option<EdgeOwnershipScope> {
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
    Some(edge_ownership_scope_for_request(
        EdgeOwnershipRequest::from_projection(
            CoordinationKind::GuideOwnership,
            format!("ChildFrameAxis{role:?}:{channel}:{axis_position:?}"),
            &projection,
            sharing_level,
        ),
    ))
}

fn child_frame_axis_title_scope(
    channel: &str,
    sharing_context: GuideSharingContext<'_>,
    axis_position: AxisPosition,
    sharing_level: SharingLevel,
) -> Option<EdgeOwnershipScope> {
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
pub(crate) async fn evaluate_cartesian_axis(
    axis: &CartesianAxis,
    channel: &str,
    scale: &avenger_scales::scales::ConfiguredScale,
    plot_width: f32,
    plot_height: f32,
    plot_bounds: &LayoutBounds,
    theme: &Theme,
    params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ctx: &datafusion::prelude::SessionContext,
    sharing_context: GuideSharingContext<'_>,
    facet_sharing_level: SharingLevel,
    child_frame_sharing_level: SharingLevel,
) -> Result<SceneMark, AvengerChartError> {
    // Evaluate visible expression (default to true if not set)
    let visible = if let Some(visible_node) = axis.visible.as_option().and_then(|o| o.as_ref()) {
        let visible_expr = visible_node.to_expr(ctx)?;
        evaluate_bool_expr(&visible_expr, ctx, params).await?
    } else {
        true
    };

    // Skip if invisible
    if !visible {
        return Ok(SceneMark::Group(SceneGroup {
            marks: vec![],
            ..Default::default()
        }));
    }

    // Evaluate position expression
    let position = if let Some(position_node) = axis.position.as_option().and_then(|o| o.as_ref()) {
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
    let grid = if let Some(grid_node) = axis.grid.as_option().and_then(|o| o.as_ref()) {
        let grid_expr = grid_node.to_expr(ctx)?;
        evaluate_bool_expr(&grid_expr, ctx, params).await?
    } else {
        false
    };

    // Evaluate format_number expression
    let format_number =
        if let Some(format_node) = axis.format_number.as_option().and_then(|o| o.as_ref()) {
            let format_expr = format_node.to_expr(ctx)?;
            Some(evaluate_string_expr(&format_expr, ctx, params).await?)
        } else {
            None
        };

    // Evaluate label_font_family expression
    let label_font_family = if let Some(label_font_node) =
        axis.label_font_family.as_option().and_then(|o| o.as_ref())
    {
        let label_font_expr = label_font_node.to_expr(ctx)?;
        Some(evaluate_string_expr(&label_font_expr, ctx, params).await?)
    } else {
        theme.font_family(&label_ctx)
    };

    // Evaluate title_font_family expression
    let title_font_family = if let Some(title_font_node) =
        axis.title_font_family.as_option().and_then(|o| o.as_ref())
    {
        let title_font_expr = title_font_node.to_expr(ctx)?;
        Some(evaluate_string_expr(&title_font_expr, ctx, params).await?)
    } else {
        theme.font_family(&title_ctx)
    };

    // Evaluate show_title (default true)
    let show_title_expr = if let Some(node) = axis.show_title.as_option().and_then(|o| o.as_ref()) {
        let expr = node.to_expr(ctx)?;
        evaluate_bool_expr(&expr, ctx, params).await.unwrap_or(true)
    } else {
        true
    };

    // Query facet-aware visibility based on cell path, axis position, and sharing level
    let hide_invalid_facet_path_axes = params
        .get(INVALID_FACET_PATH_AXIS_FALLBACK_HIDDEN_PARAM)
        .and_then(|value| match value {
            datafusion::common::ScalarValue::Boolean(Some(v)) => Some(*v),
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

    // Jagged grids can leave some interior subplots without edge labels.
    // Keep sharing-based ownership for titles, but force labels visible.
    let jagged_labels_override = !sharing_context.is_root_facet_path()
        && !facet_sharing_level.is_free()
        && sharing_context.facet_is_jagged_for_axis(position);

    let child_frame_labels_visible = owner_for_scope(child_frame_axis_ownership_scope(
        ChildFrameAxisOwnershipRole::Labels,
        channel,
        sharing_context,
        position,
        child_frame_sharing_level,
    ));
    let child_frame_title_visible = owner_for_scope(child_frame_axis_title_scope(
        channel,
        sharing_context,
        position,
        child_frame_sharing_level,
    ));

    // Combine user-specified show_title with facet visibility
    let show_title = show_title_expr && facet_visibility.show_title && child_frame_title_visible;
    let labels_visible = Some(
        (facet_visibility.show_labels || jagged_labels_override) && child_frame_labels_visible,
    );

    // Evaluate tick_count expression if present
    let tick_count = if let Some(tc_node) = axis.tick_count.as_option().and_then(|o| o.as_ref()) {
        let tc_expr = tc_node.to_expr(ctx)?;
        Some(evaluate_f32_expr(&tc_expr, ctx, params).await?)
    } else {
        None
    };

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
        tick_count,
    };

    // Evaluate title expression if present
    let title = if let Some(title_node) = axis.title.as_option().and_then(|o| o.as_ref()) {
        let title_expr = title_node.to_expr(ctx)?;
        evaluate_string_expr(&title_expr, ctx, params).await?
    } else {
        String::new()
    };
    let title = title.as_str();

    // Generate axis marks based on scale characteristics
    // Use domain and range kinds to determine which axis maker to use
    let domain_kind = scale.scale_impl.domain_kind();

    // For categorical domains, check the scale type
    let axis_group = match domain_kind {
        DomainKind::Categorical => {
            // Use scale_type to distinguish band/point/ordinal
            let scale_type = scale.scale_impl.scale_type();
            match scale_type {
                "band" => make_band_axis_marks(scale, title, axis_origin, &axis_config)?,
                "point" => make_point_axis_marks(scale.clone(), title, axis_origin, &axis_config)?,
                "ordinal" => {
                    // Ordinal scales with discrete ranges need band-like rendering
                    // For ordinal scales, convert to band scale for axis rendering
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

#[cfg(test)]
mod tests {
    use super::{
        AxisPosition, ChildFrameAxisOwnershipRole, axis_owner_ignore_empty_cells_from_params,
        axis_ownership_mode_from_params, child_frame_axis_ownership_scope,
        child_frame_axis_title_scope,
    };
    use crate::chart_core::SharingLevel;
    use crate::container::{ChildFrameSharingLevel, ChildFrameSharingPath};
    use crate::facet::evaluated_facet_tree::EvaluatedFacetTree;
    use crate::guide::AxisOwnershipMode;
    use crate::guide::GuideSharingContext;
    use crate::render::context::AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM;
    use datafusion::common::ScalarValue;
    use indexmap::IndexMap;

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
    fn hconcat_shared_y_axis_owned_by_left_child() {
        let facet_tree = EvaluatedFacetTree::empty();
        let path = ChildFrameSharingPath::root().appended(ChildFrameSharingLevel::hconcat_child(
            0,
            2,
            Some("left"),
        ));
        let context = GuideSharingContext::new(&facet_tree, &[], &path);
        let scope = child_frame_axis_ownership_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "y",
            context,
            AxisPosition::Left,
            SharingLevel::GLOBAL,
        )
        .expect("hconcat should project y-axis ownership");
        assert!(scope.current_position_owns());

        let right_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(1, 2, Some("right")));
        let right_context = GuideSharingContext::new(&facet_tree, &[], &right_path);
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
        let facet_tree = EvaluatedFacetTree::empty();
        let path = ChildFrameSharingPath::root().appended(ChildFrameSharingLevel::hconcat_child(
            1,
            2,
            Some("right"),
        ));
        let context = GuideSharingContext::new(&facet_tree, &[], &path);
        assert!(
            child_frame_axis_title_scope("y", context, AxisPosition::Left, SharingLevel::FREE)
                .is_none()
        );
    }

    #[test]
    fn vconcat_shared_x_axis_owned_by_bottom_child() {
        let facet_tree = EvaluatedFacetTree::empty();
        let top_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::vconcat_child(0, 2, Some("top")));
        let top_context = GuideSharingContext::new(&facet_tree, &[], &top_path);
        let top_scope = child_frame_axis_ownership_scope(
            ChildFrameAxisOwnershipRole::Labels,
            "x",
            top_context,
            AxisPosition::Bottom,
            SharingLevel::GLOBAL,
        )
        .expect("vconcat should project x-axis ownership");
        assert!(!top_scope.current_position_owns());

        let bottom_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::vconcat_child(1, 2, Some("bottom")));
        let bottom_context = GuideSharingContext::new(&facet_tree, &[], &bottom_path);
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
