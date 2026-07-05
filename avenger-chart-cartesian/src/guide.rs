//! Cartesian coordinate system guide implementation
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use avenger_chart_core::{
    AvengerChartError, AxisPosition, CompiledGuide, CompiledMarkCore, CoordMeasurement,
    CoordinateGuide, DefaultLogicalExprNodeExt, EmptyCoordMeasurement, GuideOverflowPhase,
    GuideRenderContext, GuideSharingContext, GuideUpdate, IntoExpr, LayoutBounds, Maybe,
    MaybeOptionalExpr, OverflowSpaceRequirement, SharingLevel, Theme, ThemeContext,
    evaluate_string_expr, extract_channel_title_from_marks, strip_trailing_numbers,
};
use avenger_color::{ColorOrGradient, parse_color_string_strict};
use avenger_common::value::ScalarOrArray;
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{
    group::Clip, mark::SceneMark, pattern::default_no_fill_pattern, rect::SceneRectMark,
};
use datafusion::{
    common::ScalarValue, dataframe::DataFrame, logical_expr::Expr, prelude::SessionContext,
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::axis::{CartesianAxis, evaluate_cartesian_axis};

/// Options for Cartesian coordinate system (beyond axes).
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CartesianOptions {
    /// Background color for the plot area.
    #[serde_as(as = "MaybeOptionalExpr")]
    pub plot_background_color: Maybe<Option<LogicalExprNode>>,
}

/// Guide for Cartesian coordinate system
///
/// Combines:
/// - Axes configured at the channel level (x, y)
/// - Coordinate-level options (background color)
#[derive(Clone, Serialize, Deserialize)]
pub struct CartesianGuide {
    /// Axes configured at the channel level
    pub axes: HashMap<String, CartesianAxis>,
    /// Coordinate-system-level options
    pub options: CartesianOptions,
    /// Channel titles extracted from mark renderers
    pub channel_titles: HashMap<String, String>,
    /// Child-frame scale sharing levels extracted from mark channels.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub(crate) channel_sharing_levels: HashMap<String, u8>,
    /// Nested-band per-level axis overrides extracted from position channels.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub(crate) nested_axis_levels: HashMap<String, BTreeMap<usize, Box<CartesianAxis>>>,
}

impl std::fmt::Debug for CartesianGuide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CartesianGuide")
            .field("axes", &self.axes)
            .field("options", &self.options)
            .field("channel_titles", &self.channel_titles)
            .field("channel_sharing_levels", &self.channel_sharing_levels)
            .field("nested_axis_levels", &self.nested_axis_levels)
            .finish()
    }
}

impl CartesianGuide {
    pub fn new() -> Self {
        Self {
            axes: HashMap::new(),
            options: CartesianOptions::default(),
            channel_titles: HashMap::new(),
            channel_sharing_levels: HashMap::new(),
            nested_axis_levels: HashMap::new(),
        }
    }

    /// Configure coordinate-level options
    pub fn with_options(mut self, options: CartesianOptions) -> Self {
        self.options = options;
        self
    }

    /// Set the plot background color
    pub fn plot_background_color(mut self, color: impl IntoExpr) -> Self {
        let expr = color.into_expr();
        self.options.plot_background_color = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr)
                .expect("Failed to serialize plot_background_color expr"),
        ));
        self
    }
}

impl Default for CartesianGuide {
    fn default() -> Self {
        Self::new()
    }
}

impl CartesianGuide {
    /// Get plot background color from options or theme
    async fn get_background_color(
        &self,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Option<[f32; 4]> {
        // Try to evaluate expression if set
        if let Some(color_node) = self
            .options
            .plot_background_color
            .as_option()
            .and_then(|o| o.as_ref())
            && let Ok(color_expr) = color_node.to_default_expr(ctx)
        {
            // Evaluate the expression to get color string
            if let Ok(color_str) = evaluate_string_expr(&color_expr, ctx, params).await {
                // Parse the color string
                if let Ok(color) = parse_color_string_strict(&color_str) {
                    return Some(color);
                }
            }
        }

        // Fallback to theme
        let guide_ctx = ThemeContext::new("guide", params.clone()).with_subtype("cartesian");
        theme
            .query(&guide_ctx, "background-color")
            .and_then(|v| v.as_color_array())
    }

    /// Update this guide with values from another guide
    pub fn update(mut self, other: Self) -> Self {
        // Merge axes - other's axes take precedence
        for (channel, axis) in other.axes {
            match self.axes.get(&channel) {
                Some(existing) => {
                    // Update existing axis with new configuration
                    let updated = existing.clone().update(axis);
                    self.axes.insert(channel, updated);
                }
                None => {
                    // Add new axis
                    self.axes.insert(channel, axis);
                }
            }
        }

        // Update options - other's options take precedence when set
        if other.options.plot_background_color.is_set() {
            self.options.plot_background_color = other.options.plot_background_color;
        }

        for (channel, level_axes) in other.nested_axis_levels {
            self.nested_axis_levels
                .entry(channel)
                .or_default()
                .extend(level_axes);
        }

        self
    }
}

impl GuideUpdate for CartesianGuide {
    fn update(self, other: Self) -> Self {
        CartesianGuide::update(self, other)
    }
}

impl CoordinateGuide for CartesianGuide {
    type Axis = CartesianAxis;

    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>) {
        self.axes = axes;
    }

    fn set_compiled_marks<M>(&mut self, compiled_marks: &[Arc<M>], session_context: &SessionContext)
    where
        M: CompiledMarkCore + ?Sized,
    {
        // Extract titles from mark renderers immediately
        for channel in ["x", "y"] {
            if let Some(title) =
                extract_channel_title_from_marks(compiled_marks, channel, session_context)
            {
                self.channel_titles.insert(channel.to_string(), title);
            }
        }

        self.channel_sharing_levels.clear();
        self.nested_axis_levels.clear();
        for mark in compiled_marks {
            for (channel, channel_value) in mark.data_context().channels() {
                if let Some(nested_config) = channel_value.get_nested_band_config() {
                    let channel = strip_trailing_numbers(channel).to_string();
                    let level_axes = self.nested_axis_levels.entry(channel).or_default();
                    for (level, level_config) in &nested_config.levels {
                        let Some(axis_config) = &level_config.axis_config else {
                            continue;
                        };
                        let Some(axis) = axis_config.as_any().downcast_ref::<CartesianAxis>()
                        else {
                            continue;
                        };
                        level_axes
                            .entry(*level)
                            .and_modify(|existing| {
                                let updated = existing.as_ref().clone().update(axis.clone());
                                *existing = Box::new(updated);
                            })
                            .or_insert_with(|| Box::new(axis.clone()));
                    }
                }

                let Some(sharing) = channel_value.get_domain_scope() else {
                    continue;
                };
                let channel = strip_trailing_numbers(channel).to_string();
                let sharing_level = SharingLevel::from(sharing).raw();
                self.channel_sharing_levels
                    .entry(channel)
                    .and_modify(|existing| *existing = (*existing).max(sharing_level))
                    .or_insert(sharing_level);
            }
        }
    }

    fn update(&mut self, other: Self) {
        *self = CartesianGuide::update(self.clone(), other);
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[typetag::serde]
impl CompiledGuide for CartesianGuide {
    /// Measure how much space this guide needs outside the plot area
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        ctx: &SessionContext,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        // Use provided coord_measurement or default empty one (CartesianGuide doesn't use it)
        let empty_coord = EmptyCoordMeasurement;
        let coord_measurement = coord_measurement.unwrap_or(&empty_coord);

        // For overflow measurement, we can place the plot at origin
        let initial_bounds = LayoutBounds {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        };
        let guide_overflow = OverflowSpaceRequirement::default();

        // Evaluate axes to measure their bounding box with actual params
        // Use facet_tree and facet_path for visibility-aware overflow measurement
        let axis_marks = self
            .evaluate(
                scales,
                plot_width,
                plot_height,
                &initial_bounds,
                &guide_overflow,
                theme,
                params,
                ctx,
                data_override,
                sharing_context,
                coord_measurement,
                GuideRenderContext::without_resource_sink(plot_width, plot_height),
            )
            .await?;

        // Calculate bounding box of all axis marks
        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;

        for mark in &axis_marks {
            let bbox = mark.bounding_box();
            let lower = bbox.lower();
            let upper = bbox.upper();
            min_x = min_x.min(lower[0]);
            max_x = max_x.max(upper[0]);
            min_y = min_y.min(lower[1]);
            max_y = max_y.max(upper[1]);
        }

        // Get the actual scale ranges to measure overflow against
        let x_scale = scales.get("x");
        let y_scale = scales.get("y");

        // Calculate scale boundaries (plot is at origin for measurement)
        let (scale_left, scale_right) = if let Some(x_scale) = x_scale {
            let x_range = x_scale.numeric_interval_range()?;
            (x_range.0.min(x_range.1), x_range.0.max(x_range.1))
        } else {
            (0.0, plot_width)
        };

        let (scale_top, scale_bottom) = if let Some(y_scale) = y_scale {
            let y_range = y_scale.numeric_interval_range()?;
            (y_range.0.min(y_range.1), y_range.0.max(y_range.1))
        } else {
            (0.0, plot_height)
        };

        // Calculate overflow relative to scale boundaries
        const THRESHOLD: f32 = 1.0; // Ignore overflows less than 1px
        let left = (scale_left - min_x).max(0.0);
        let right = (max_x - scale_right).max(0.0);
        let top = (scale_top - min_y).max(0.0);
        let bottom = (max_y - scale_bottom).max(0.0);

        // Round very small overflows to zero
        let left = if left < THRESHOLD { 0.0 } else { left };
        let right = if right < THRESHOLD { 0.0 } else { right };
        let top = if top < THRESHOLD { 0.0 } else { top };
        let bottom = if bottom < THRESHOLD { 0.0 } else { bottom };

        let mut overflow = OverflowSpaceRequirement {
            top,
            bottom,
            left,
            right,
        };

        if let Some(child_frame_overflow) =
            coord_measurement.positioned_subplot_overflow(plot_width, plot_height)?
        {
            overflow.top = overflow.top.max(child_frame_overflow.top);
            overflow.right = overflow.right.max(child_frame_overflow.right);
            overflow.bottom = overflow.bottom.max(child_frame_overflow.bottom);
            overflow.left = overflow.left.max(child_frame_overflow.left);
        }

        Ok(overflow)
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        sharing_context: GuideSharingContext<'_>,
        _coord_measurement: &dyn CoordMeasurement,
        render_context: GuideRenderContext<'_>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut marks = Vec::new();
        let default_number_locale = render_context
            .eval()
            .map(|eval| eval.formatting_context().resolved_number_locale())
            .unwrap_or("en-US");
        let default_number_locale_registry = match render_context.eval() {
            Some(eval) => eval
                .formatting_context()
                .number_locale_registry()
                .map_err(AvengerChartError::InvalidArgument)?,
            None => None,
        };
        let empty_number_locale_specs = avenger_text::NumberLocaleSpecs::default();
        let default_number_locale_specs = render_context
            .eval()
            .map(|eval| eval.formatting_context().number_locale_specs())
            .unwrap_or(&empty_number_locale_specs);
        let default_datetime_locale = render_context
            .eval()
            .map(|eval| eval.formatting_context().resolved_datetime_locale())
            .unwrap_or("en-US");
        let default_datetime_timezone = render_context
            .eval()
            .map(|eval| eval.formatting_context().resolved_datetime_timezone())
            .unwrap_or("UTC");
        let default_datetime_locale_registry = match render_context.eval() {
            Some(eval) => eval
                .formatting_context()
                .datetime_locale_registry()
                .map_err(AvengerChartError::InvalidArgument)?,
            None => None,
        };
        let empty_datetime_locale_specs = avenger_text::DateTimeLocaleSpecs::default();
        let default_datetime_locale_specs = render_context
            .eval()
            .map(|eval| eval.formatting_context().datetime_locale_specs())
            .unwrap_or(&empty_datetime_locale_specs);

        // Render background if specified (behind everything else)
        if let Some(bg_color) = self.get_background_color(theme, params, ctx).await {
            let bg_rect = SceneRectMark {
                name: "plot-background".to_string(),
                clip: false,
                len: 1,
                gradients: Vec::new(),
                x: ScalarOrArray::new_scalar(plot_bounds.x),
                y: ScalarOrArray::new_scalar(plot_bounds.y),
                width: Some(ScalarOrArray::new_scalar(plot_width)),
                height: Some(ScalarOrArray::new_scalar(plot_height)),
                x2: None,
                y2: None,
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color(bg_color)),
                fill_pattern: default_no_fill_pattern(),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
                stroke_width: ScalarOrArray::new_scalar(0.0),
                corner_radius: ScalarOrArray::new_scalar(0.0),
                indices: None,
                interactive: true,
                zindex: Some(-2), // Behind grid lines (which are at -1)
            };
            marks.push(SceneMark::Rect(bg_rect));
        }

        // Create default axes for all channels with scales at render time
        let mut default_axes = HashMap::new();
        for channel_name in scales.keys() {
            if channel_name == "x" || channel_name == "y" {
                // Set default position based on channel
                let position = if channel_name == "x" {
                    AxisPosition::Bottom
                } else {
                    AxisPosition::Left
                };

                // Determine if grid should be enabled based on scale type
                let grid = scales
                    .get(channel_name)
                    .is_some_and(|s| s.ticks(None).is_ok());

                let mut axis = CartesianAxis::new()
                    .position(position)
                    .visible(true)
                    .grid(grid);

                if let Some(title) = self.channel_titles.get(channel_name) {
                    axis = axis.title(title.clone());
                }

                default_axes.insert(channel_name.clone(), axis);
            }
        }

        // Merge with user-configured axes
        let mut all_axes = default_axes;

        // Apply user configurations on top of defaults.
        for (channel, user_axis) in &self.axes {
            let axis_to_apply = user_axis.clone();

            if let Some(default_axis) = all_axes.get_mut(channel) {
                *default_axis = std::mem::take(default_axis).update(axis_to_apply);
            } else {
                all_axes.insert(channel.clone(), axis_to_apply);
            }
        }

        // Render each axis in a deterministic (channel-sorted) order. `all_axes`
        // is a HashMap whose iteration order is seeded per-eval, so pushing axis
        // marks in that order made the scene-graph mark order nondeterministic
        // across evaluations (e.g. x-axis vs y-axis gridline groups swapping).
        let mut axis_entries: Vec<(&String, &CartesianAxis)> = all_axes.iter().collect();
        axis_entries.sort_by(|a, b| a.0.cmp(b.0));
        for (channel, axis) in axis_entries {
            if let Some(scale) = scales.get(channel) {
                // Facets store channel sharing on the facet tree. Concat-like
                // child-frame containers have no facet tree, so they use the
                // guide's mark-derived channel sharing map instead.
                let facet_sharing_level = sharing_context.channel_domain_sharing_level(channel);
                let child_frame_sharing_level = self
                    .channel_sharing_levels
                    .get(channel)
                    .copied()
                    .map(SharingLevel::from_raw)
                    .unwrap_or(SharingLevel::FREE);
                let axis_mark = evaluate_cartesian_axis(
                    axis,
                    channel,
                    scale,
                    plot_width,
                    plot_height,
                    plot_bounds,
                    theme,
                    params,
                    ctx,
                    sharing_context,
                    facet_sharing_level,
                    child_frame_sharing_level,
                    self.nested_axis_levels.get(channel),
                    default_number_locale,
                    default_number_locale_registry.clone(),
                    default_number_locale_specs,
                    default_datetime_locale,
                    default_datetime_timezone,
                    default_datetime_locale_registry.clone(),
                    default_datetime_locale_specs,
                )
                .await?;
                marks.push(axis_mark);
            }
        }

        Ok(marks)
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        // Cartesian coordinates use a rectangular clip
        Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }

    fn axis_position(&self, channel: &str) -> Option<AxisPosition> {
        // Check if we have an axis configured for this channel
        if let Some(axis) = self.axes.get(channel) {
            // If axis has explicit position expression, try to extract it if it's a simple literal
            if let Some(position_node) = axis.position.as_option().and_then(|o| o.as_ref()) {
                // Try to convert to datafusion Expr and check if it's a literal
                if let Ok(expr) = position_node.to_default_expr(&SessionContext::new())
                    && let Expr::Literal(ScalarValue::Utf8(Some(pos_str)), _) = expr
                {
                    // Got a literal string, parse it as an axis position
                    return match pos_str.to_lowercase().as_str() {
                        "top" => Some(AxisPosition::Top),
                        "bottom" => Some(AxisPosition::Bottom),
                        "left" => Some(AxisPosition::Left),
                        "right" => Some(AxisPosition::Right),
                        _ => None,
                    };
                }
                // Has position expression but can't extract it - return None for fallback
                None
            } else {
                // No explicit position, use defaults based on channel name
                match channel {
                    "x" => Some(AxisPosition::Bottom),
                    "y" => Some(AxisPosition::Left),
                    _ => Some(AxisPosition::Bottom),
                }
            }
        } else {
            // No axis configured for this channel, use defaults
            match channel {
                "x" => Some(AxisPosition::Bottom),
                "y" => Some(AxisPosition::Left),
                _ => None,
            }
        }
    }

    fn overflow_cache_discriminator(
        &self,
        sharing_context: GuideSharingContext<'_>,
        phase: GuideOverflowPhase,
    ) -> Option<String> {
        if phase != GuideOverflowPhase::Final
            || !sharing_context.child_frame_position_indices().is_empty()
        {
            return None;
        }

        let mut parts = Vec::new();
        for channel in ["x", "y"] {
            let position = self.axis_position(channel)?;
            let sharing_level = sharing_context.channel_domain_sharing_level(channel);
            let visibility = sharing_context
                .channel_axis_visibility_for_path_checked(position, sharing_level.raw())
                .unwrap_or_else(avenger_chart_core::AxisVisibility::visible);
            let jagged = sharing_context.facet_is_jagged_for_axis(position);
            let axis_policy = sharing_context.axis_guide_visibility_config(position);
            let axis_policy_key = axis_policy
                .map(|config| format!("{:?}/{:?}", config.labels, config.title))
                .unwrap_or_else(|| "none".to_string());
            parts.push(format!(
                "{channel}:{position:?}:{}:{}:{}:{}:{}",
                sharing_level.raw(),
                visibility.show_labels,
                visibility.show_title,
                jagged,
                axis_policy_key
            ));
        }
        Some(format!("cartesian:v2:{}", parts.join("|")))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::CartesianGuide;
    use avenger_chart_core::{
        AxisGuideVisibilityConfig, AxisGuideVisibilityPolicy, AxisOwnershipMode, AxisPosition,
        AxisVisibility, ChildFrameGuideSharingView, CompiledGuide, CoordinationAxis,
        FacetGuideSharingView, GuideOverflowPhase, GuideSharingContext, SharingLevel,
    };
    use datafusion::common::ScalarValue;

    #[derive(Debug)]
    struct TestFacetView {
        sharing: SharingLevel,
        visibility: AxisVisibility,
        axis_policy: Option<AxisGuideVisibilityConfig>,
        jagged: bool,
    }

    impl TestFacetView {
        fn new(axis_policy: AxisGuideVisibilityConfig) -> Self {
            Self {
                sharing: SharingLevel::GLOBAL,
                visibility: AxisVisibility::visible(),
                axis_policy: Some(axis_policy),
                jagged: false,
            }
        }
    }

    impl FacetGuideSharingView for TestFacetView {
        fn channel_axis_visibility_for_path_checked(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
        ) -> Option<AxisVisibility> {
            Some(self.visibility)
        }

        fn channel_axis_visibility_for_path_checked_with_mode(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
            _ownership_mode: AxisOwnershipMode,
        ) -> Option<AxisVisibility> {
            Some(self.visibility)
        }

        fn is_jagged_for_axis(&self, _axis_position: AxisPosition) -> bool {
            self.jagged
        }

        fn channel_domain_sharing_level(&self, _channel: &str) -> SharingLevel {
            self.sharing
        }

        fn axis_guide_visibility_config_for_path(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
        ) -> Option<AxisGuideVisibilityConfig> {
            self.axis_policy
        }

        fn effective_edge_indices_for_values_at_path(
            &self,
            _facet_path: &[ScalarValue],
            _values: &[ScalarValue],
        ) -> Option<(usize, usize)> {
            None
        }
    }

    #[derive(Debug, Default)]
    struct TestChildFrameView {
        position_indices: Vec<usize>,
        level_counts: Vec<usize>,
        level_axes: Vec<CoordinationAxis>,
    }

    impl TestChildFrameView {
        fn root() -> Self {
            Self::default()
        }

        fn hconcat_child(index: usize) -> Self {
            Self {
                position_indices: vec![index],
                level_counts: vec![2],
                level_axes: vec![CoordinationAxis::Horizontal],
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

    #[test]
    fn overflow_discriminator_includes_axis_visibility_policy() {
        let all_policy = TestFacetView::new(AxisGuideVisibilityConfig::same(
            AxisGuideVisibilityPolicy::All,
        ));
        let outer_policy = TestFacetView::new(AxisGuideVisibilityConfig::same(
            AxisGuideVisibilityPolicy::OuterEdges,
        ));
        let child_frame = TestChildFrameView::root();
        let guide = CartesianGuide::new();

        let all_key = guide
            .overflow_cache_discriminator(
                GuideSharingContext::new(&all_policy, &[], &child_frame),
                GuideOverflowPhase::Final,
            )
            .expect("root final cartesian guide should provide a discriminator");
        let outer_key = guide
            .overflow_cache_discriminator(
                GuideSharingContext::new(&outer_policy, &[], &child_frame),
                GuideOverflowPhase::Final,
            )
            .expect("root final cartesian guide should provide a discriminator");

        assert_ne!(all_key, outer_key);
        assert!(all_key.contains("All/All"));
        assert!(outer_key.contains("OuterEdges/OuterEdges"));
    }

    #[test]
    fn overflow_discriminator_opts_out_for_child_frame_paths() {
        let facet_view = TestFacetView::new(AxisGuideVisibilityConfig::same(
            AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups,
        ));
        let child_frame = TestChildFrameView::hconcat_child(1);
        let guide = CartesianGuide::new();

        assert_eq!(
            guide.overflow_cache_discriminator(
                GuideSharingContext::new(&facet_view, &[], &child_frame),
                GuideOverflowPhase::Final,
            ),
            None
        );
    }
}
