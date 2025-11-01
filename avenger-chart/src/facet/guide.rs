use crate::channel::config_traits::ScaleSharing;
use crate::facet::dimension_config::{
    ColDimensionConfig, FacetDimensionConfig, RowDimensionConfig,
};
use crate::guide::{CompiledGuide, CoordinateGuide, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::marks::CompiledMark;
use crate::scales::ConfiguredScaleLegendExt;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_text::measurement::TextMeasurer;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetRowGuide {
    // Collected facet sources from compiled marks (populated via set_compiled_marks)
    facet_sources: Vec<FacetSource>,
    /// Optional facet title rendered above the label column
    pub facet_title: Option<String>,
    unified_y_title: Option<String>,
    /// The channel that can be unified (from subplot guide declaration)
    unifiable_channel: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct FacetSource {
    subplot: std::sync::Arc<crate::plot::CompiledPlot>,
    data: crate::marks::CompiledDataContext,
    user_title: Option<String>,
    /// Cached Pass 1 edge overflow measurements from facet evaluation
    #[serde(skip)]
    cached_edge_overflow: std::sync::Arc<
        std::sync::Mutex<
            Option<(
                crate::guide::OverflowSpaceRequirement,
                crate::guide::OverflowSpaceRequirement,
            )>,
        >,
    >,
}

impl FacetRowGuide {}

impl CoordinateGuide for FacetRowGuide {
    type Axis = crate::cartesian::axis::CartesianAxis;

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<std::sync::Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
        self.facet_sources.clear();
        for m in compiled_marks {
            if let Some(facet) = m
                .as_any()
                .downcast_ref::<crate::facet::marks::facet::CompiledFacetRow>()
            {
                self.facet_sources.push(FacetSource {
                    subplot: facet.compiled_subplot.clone(),
                    data: facet.state.data.clone(),
                    user_title: facet.facet_title.clone(),
                    cached_edge_overflow: facet.cached_edge_overflow.clone(),
                });
            }
        }
        // Derive default facet title if not explicitly set on any facet mark
        if self.facet_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                // Try to extract a column name from channel
                if let Some(cv) = src.data.channels().get(RowDimensionConfig::channel_name()) {
                    if let Some(name) = cv.as_column_name(_session_context) {
                        self.facet_title = Some(name);
                    }
                }
                // Prefer user-specified title if available
                if let Some(title) = &src.user_title {
                    self.facet_title = Some(title.clone());
                }
            }
        }
        // Derive unified title from subplot's guide declaration
        // The subplot guide declares what channel can be unified for row faceting
        if self.unified_y_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(info) = src.subplot.compiled_guide.as_ref().and_then(|g| {
                    g.facet_unifiable_channel(
                        RowDimensionConfig::facet_direction(),
                        src.subplot.marks(),
                        _session_context,
                    )
                }) {
                    self.unifiable_channel = Some(info.channel);
                    self.unified_y_title = info.title;
                }
            }
        }
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for FacetRowGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, crate::error::AvengerChartError> {
        use datafusion::logical_expr::lit;

        // Need row scale
        let row_scale = scales
            .get(RowDimensionConfig::channel_name())
            .ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(
                    format!(
                        "Missing '{}' scale for FacetRowGuide",
                        RowDimensionConfig::channel_name()
                    )
                    .into(),
                )
            })?;

        // Extract discrete domain order
        let domain_vals = match row_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };

        let mut max_left: f32 = 0.0;
        let mut max_right: f32 = 0.0;
        let mut top: f32 = 0.0;
        let mut bottom: f32 = 0.0;

        // For each facet source (there could be more than one Facet mark)
        for source in &self.facet_sources {
            // Try to use cached Pass 1 edge overflow measurements
            let cached_overflow = source
                .cached_edge_overflow
                .lock()
                .ok()
                .and_then(|cache| cache.clone());

            if let Some((first_overflow, last_overflow)) = cached_overflow {
                // Use cached Pass 1 measurements (from initial subplot dimensions)
                top = top.max(first_overflow.top);
                bottom = bottom.max(last_overflow.bottom);
                max_left = max_left.max(first_overflow.left.max(last_overflow.left));
                max_right = max_right.max(first_overflow.right.max(last_overflow.right));
            } else {
                // Fallback: Remeasure if cache not available (shouldn't happen in normal flow)
                // Row expression from channels
                let row_expr = source
                    .data
                    .channels()
                    .get(RowDimensionConfig::channel_name())
                    .and_then(|cv| cv.expr(ctx))
                    .ok_or_else(|| {
                        crate::error::AvengerChartError::InternalError(
                            format!(
                                "Facet '{}' channel not found in guide",
                                RowDimensionConfig::channel_name()
                            )
                            .into(),
                        )
                    })?;

                // DataFrame for this facet source
                let df = source.data.dataframe_with_context(ctx).ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        "Facet guide could not access data".into(),
                    )
                })?;

                // Compute per-channel scale sharing preferences BEFORE iteration
                let coord_channels: Vec<&str> =
                    source.subplot.coord_transform.required_channels().to_vec();
                let mut scale_sharing_by_channel = std::collections::HashMap::new();
                for &ch in &coord_channels {
                    let mut mode = ScaleSharing::Free;
                    for m in &source.subplot.marks {
                        if let Some(cv) = m.data_context().channels().get(ch) {
                            if let Some(share_mode) = cv.get_share_mode() {
                                mode = match (mode, share_mode) {
                                    (ScaleSharing::Free, new_mode) => new_mode,
                                    (ScaleSharing::Shared, _) => ScaleSharing::Shared,
                                    (_, ScaleSharing::Shared) => ScaleSharing::Shared,
                                    (existing, _) => existing,
                                };
                            }
                        }
                    }
                    scale_sharing_by_channel.insert(ch.to_string(), mode);
                }
                let any_shared = scale_sharing_by_channel
                    .values()
                    .any(|v| *v == ScaleSharing::Shared);

                // Use SubplotIterator to ensure consistent FacetContext across all subplots
                use crate::facet::subplot_iterator::SubplotIterator;
                let subplot_iter = SubplotIterator::<RowDimensionConfig>::new(
                    domain_vals.clone(),
                    params.clone(),
                    scale_sharing_by_channel.clone(),
                );
                let band_h = plot_height / subplot_iter.len() as f32;

                for iteration in subplot_iter {
                    let filter_df = df
                        .clone()
                        .filter(row_expr.clone().eq(lit(iteration.facet_value.clone())))?;

                    let inner_scales = if any_shared {
                        source
                            .subplot
                            .build_scales_for_dataframe(&df, plot_width, band_h, ctx, params)
                            .await?
                    } else {
                        source
                            .subplot
                            .build_scales_for_dataframe(&filter_df, plot_width, band_h, ctx, params)
                            .await?
                    };

                    // iteration.params already has correct FacetContext - guaranteed by SubplotIterator
                    let overflow = source
                        .subplot
                        .measure_guide_overflow_with_scales(
                            &inner_scales,
                            plot_width,
                            band_h,
                            ctx,
                            &iteration.params,
                        )
                        .await?;

                    if iteration.index == 0 {
                        top = top.max(overflow.top);
                    }
                    if iteration.index == domain_vals.len() - 1 {
                        bottom = bottom.max(overflow.bottom);
                    }
                    max_left = max_left.max(overflow.left);
                    max_right = max_right.max(overflow.right);
                }
            }
        }
        // Add space for facet labels by measuring text bounds
        // For 90° rotation, horizontal footprint ≈ text height
        let labels = row_scale.domain_labels().unwrap_or_default();

        // Resolve facet-label theme (fallbacks kept for now)
        let guide_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("label");
        let label_font_px = theme.font_size(&guide_ctx).unwrap_or(12.0_f32);
        let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
        let label_font_family = theme
            .font_family(&guide_ctx)
            .or_else(|| theme.font_family(&root_ctx))
            .unwrap_or_else(|| "sans-serif".to_string());

        // Resolve facet title theme
        let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("title");
        let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
        let title_font_family = theme
            .font_family(&title_ctx)
            .unwrap_or_else(|| "sans-serif".to_string());

        // Use guide_utils to measure facet label slab
        use crate::facet::guide_utils::{FacetLabelMeasurementConfig, measure_facet_label_slab};

        let measurement_config = FacetLabelMeasurementConfig {
            labels: labels.clone(),
            is_rotated: true, // Row labels are vertical
            font_family: label_font_family.clone(),
            font_size_px: label_font_px,
            title: self.facet_title.clone(),
            title_font_family: title_font_family.clone(),
            title_font_size_px: title_font_px,
        };
        let estimated_right = measure_facet_label_slab(&measurement_config);

        // Prefer placing labels/title on left when child right overflow exceeds left
        let place_on_left = max_right > max_left;
        // Unified y title should be on the axis side (child-dominant side)
        let axis_on_right = max_right > max_left;
        // Measure unified y title height (rotated width)
        let unified_y_height = if let Some(y_title) = &self.unified_y_title {
            let y_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let y_font_px = theme.font_size(&y_ctx).unwrap_or(12.0_f32);
            let y_family_owned = theme
                .font_family(&y_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());
            let cfg_y = avenger_text::measurement::TextMeasurementConfig {
                text: y_title,
                font: y_family_owned.as_str(),
                font_size: y_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let b_y =
                avenger_text::measurement::default_text_measurer().measure_text_bounds(&cfg_y);
            b_y.height + 1.0
        } else {
            0.0
        };
        let gap_axis = if self.unified_y_title.is_some() {
            10.0
        } else {
            0.0
        };
        // Axis side overflow should include child extent + gap + unified y title height
        let left_final = if axis_on_right {
            // Axis on right: left side has facet labels (if placed left) OUTSIDE subplot overflow
            if place_on_left {
                max_left + estimated_right
            } else {
                max_left
            }
        } else {
            // Axis on left: child left + gap + title height
            max_left
                + if unified_y_height > 0.0 {
                    gap_axis + unified_y_height
                } else {
                    0.0
                }
        };
        let right_final = if axis_on_right {
            // Axis on right: child right + gap + title height
            max_right
                + if unified_y_height > 0.0 {
                    gap_axis + unified_y_height
                } else {
                    0.0
                }
        } else {
            // Axis on left: right side has facet labels (if placed right) OUTSIDE subplot overflow
            if place_on_left {
                max_right
            } else {
                max_right + estimated_right
            }
        };
        Ok(OverflowSpaceRequirement {
            top,
            bottom,
            left: left_final,
            right: right_final,
        })
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        _ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, crate::error::AvengerChartError> {
        use crate::scales::ConfiguredScaleLegendExt;
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{TextAlign, TextBaseline};
        use std::sync::Arc as StdArc;

        let mut marks: Vec<SceneMark> = Vec::new();

        // Row scale
        let row_scale = match scales.get(RowDimensionConfig::channel_name()) {
            Some(s) => s,
            None => return Ok(marks),
        };

        // Domain labels
        let labels = row_scale.domain_labels()?;

        // Get band positions from the scale
        // The scale now has the correct padding_inner_px from the facet mark (via scale updates).
        // We'll use .center() on each BandPosition for label/tick positioning.
        use crate::facet::band_positions::BandPositionIterator;
        let band_positions: Vec<_> =
            BandPositionIterator::from_configured_scale(row_scale)?.collect();

        // Theme-based font for rendering (match measurement)
        let guide_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("label");
        let font_px = theme.font_size(&guide_ctx).unwrap_or(12.0_f32);
        let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
        let font_family_owned = theme
            .font_family(&guide_ctx)
            .or_else(|| theme.font_family(&root_ctx))
            .unwrap_or_else(|| "sans-serif".to_string());
        let font_family = font_family_owned.as_str();

        // Decide side based on child overflow (prefer left if right child overflow > left)
        let domain_labels_eval = row_scale.domain_labels().unwrap_or_default();

        // Convert domain labels to ScalarValues for SubplotIterator
        let domain_vals_eval: Vec<datafusion::common::ScalarValue> = domain_labels_eval
            .iter()
            .map(|s| datafusion::common::ScalarValue::Utf8(Some(s.clone())))
            .collect();

        let band_h_eval = plot_height / domain_vals_eval.len().max(1) as f32;
        let mut max_left_child = 0.0_f32;
        let mut max_right_child = 0.0_f32;

        for source in &self.facet_sources {
            let row_expr = source
                .data
                .channels()
                .get(RowDimensionConfig::channel_name())
                .and_then(|cv| cv.expr(_ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        format!(
                            "Facet '{}' channel not found in guide",
                            RowDimensionConfig::channel_name()
                        )
                        .into(),
                    )
                })?;
            let df_src = source.data.dataframe_with_context(_ctx).ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(
                    "Facet guide could not access data".into(),
                )
            })?;
            // Compute per-channel scale sharing preferences
            let coord_channels: Vec<&str> =
                source.subplot.coord_transform.required_channels().to_vec();
            let mut scale_sharing_by_channel = std::collections::HashMap::new();
            for &ch in &coord_channels {
                let mut mode = ScaleSharing::Free;
                for m in &source.subplot.marks {
                    if let Some(cv) = m.data_context().channels().get(ch) {
                        if let Some(share_mode) = cv.get_share_mode() {
                            mode = match (mode, share_mode) {
                                (ScaleSharing::Free, new_mode) => new_mode,
                                (ScaleSharing::Shared, _) => ScaleSharing::Shared,
                                (_, ScaleSharing::Shared) => ScaleSharing::Shared,
                                (existing, _) => existing,
                            };
                        }
                    }
                }
                scale_sharing_by_channel.insert(ch.to_string(), mode);
            }
            let any_shared = scale_sharing_by_channel
                .values()
                .any(|v| *v == ScaleSharing::Shared);

            // Use SubplotIterator to ensure consistent FacetContext across all subplots
            use crate::facet::subplot_iterator::SubplotIterator;
            let subplot_iter = SubplotIterator::<RowDimensionConfig>::new(
                domain_vals_eval.clone(),
                params.clone(),
                scale_sharing_by_channel.clone(),
            );

            for iteration in subplot_iter {
                let filter_df = df_src.clone().filter(
                    row_expr
                        .clone()
                        .eq(datafusion::logical_expr::lit(iteration.facet_value.clone())),
                )?;
                let inner_scales = if any_shared {
                    source
                        .subplot
                        .build_scales_for_dataframe(&df_src, plot_width, band_h_eval, _ctx, params)
                        .await?
                } else {
                    source
                        .subplot
                        .build_scales_for_dataframe(
                            &filter_df,
                            plot_width,
                            band_h_eval,
                            _ctx,
                            params,
                        )
                        .await?
                };

                // iteration.params already has correct FacetContext - guaranteed by SubplotIterator
                let overflow = source
                    .subplot
                    .measure_guide_overflow_with_scales(
                        &inner_scales,
                        plot_width,
                        band_h_eval,
                        _ctx,
                        &iteration.params,
                    )
                    .await?;
                max_left_child = max_left_child.max(overflow.left);
                max_right_child = max_right_child.max(overflow.right);
            }
        }
        let place_on_left = max_right_child > max_left_child;

        // Resolve title font properties for rendering
        let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("title");
        let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
        let title_font_family = theme
            .font_family(&title_ctx)
            .unwrap_or_else(|| "sans-serif".to_string());

        // Use guide_utils to render facet label slab (labels + rule + title)
        use crate::facet::guide_utils::{FacetLabelRenderConfig, render_facet_label_slab};

        // Create plot bounds with correct width/height from parameters
        // Offset by subplot overflow so facet labels sit outside subplot guides
        let render_plot_bounds = if place_on_left {
            // Labels on left: extend leftward by max_left_child
            LayoutBounds {
                x: plot_bounds.x - max_left_child,
                y: plot_bounds.y,
                width: plot_width + max_left_child,
                height: plot_height,
            }
        } else {
            // Labels on right: extend rightward by max_right_child
            LayoutBounds {
                x: plot_bounds.x,
                y: plot_bounds.y,
                width: plot_width + max_right_child,
                height: plot_height,
            }
        };

        let render_config = FacetLabelRenderConfig {
            labels: labels.clone(),
            band_positions: band_positions.clone(),
            plot_bounds: render_plot_bounds,
            is_rotated: true,             // Row labels are vertical
            place_at_end: !place_on_left, // place_at_end=true means right side
            font_family: font_family.to_string(),
            font_size_px: font_px,
            title: self.facet_title.clone(),
            title_font_family: title_font_family.clone(),
            title_font_size_px: title_font_px,
        };

        marks.extend(render_facet_label_slab(&render_config, theme, params));

        // Render unified y-axis title if available (always on, placed on the axis side)
        if let Some(y_title) = &self.unified_y_title {
            let y_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let y_font_px = theme.font_size(&y_ctx).unwrap_or(12.0_f32);
            let y_color = theme.text_color(&y_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let y_family_owned = theme
                .font_family(&y_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());
            // Axis side corresponds to child-dominant side: if labels are on left, axis is on right
            let axis_on_right = place_on_left;
            let gap = 6.0_f32;
            // Position title adjacent to subplot axis labels (max_left/right_child is just label space)
            // Center the title in the gap between labels and plot edge
            let x_bottom = if axis_on_right {
                plot_bounds.x + plot_width + max_right_child + gap
            } else {
                plot_bounds.x - max_left_child - gap
            };
            let y_center = plot_bounds.y + 0.5 * plot_height;
            let y_mark = SceneTextMark {
                text: y_title.clone().into(),
                x: x_bottom.into(),
                y: y_center.into(),
                align: TextAlign::Center.into(),
                baseline: TextBaseline::Bottom.into(),
                font: y_family_owned.clone().into(),
                font_size: y_font_px.into(),
                angle: if axis_on_right {
                    90.0_f32.into()
                } else {
                    (-90.0_f32).into()
                },
                color: avenger_common::types::ColorOrGradient::Color(y_color).into(),
                zindex: Some(6),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(y_mark)));
        }

        Ok(marks)
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> avenger_scenegraph::marks::group::Clip {
        avenger_scenegraph::marks::group::Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }
}

// ============================================================================
// FacetColGuide - Column Faceting Guide
// ============================================================================

/// Guide for FacetCol coordinate system
///
/// Renders facet labels horizontally below (or above) the plot area with one label per column.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetColGuide {
    facet_sources: Vec<FacetSource>,
    pub facet_title: Option<String>,
    unified_x_title: Option<String>,
    unifiable_channel: Option<String>,
}

impl FacetColGuide {}

impl CoordinateGuide for FacetColGuide {
    type Axis = crate::cartesian::axis::CartesianAxis;

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<std::sync::Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
        self.facet_sources.clear();
        for m in compiled_marks {
            if let Some(facet) = m
                .as_any()
                .downcast_ref::<crate::facet::marks::facet::CompiledFacetCol>()
            {
                self.facet_sources.push(FacetSource {
                    subplot: facet.compiled_subplot.clone(),
                    data: facet.state.data.clone(),
                    user_title: facet.facet_title.clone(),
                    cached_edge_overflow: facet.cached_edge_overflow.clone(),
                });
            }
        }
        // Derive default facet title if not explicitly set
        if self.facet_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(cv) = src.data.channels().get(ColDimensionConfig::channel_name()) {
                    if let Some(name) = cv.as_column_name(_session_context) {
                        self.facet_title = Some(name);
                    }
                }
                if let Some(title) = &src.user_title {
                    self.facet_title = Some(title.clone());
                }
            }
        }
        // Derive unified x-axis title from subplot guide
        if self.unified_x_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(info) = src.subplot.compiled_guide.as_ref().and_then(|g| {
                    g.facet_unifiable_channel(
                        ColDimensionConfig::facet_direction(),
                        src.subplot.marks(),
                        _session_context,
                    )
                }) {
                    self.unifiable_channel = Some(info.channel);
                    self.unified_x_title = info.title;
                }
            }
        }
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for FacetColGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        _theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::scalar::ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, crate::error::AvengerChartError> {
        use crate::scales::ConfiguredScaleLegendExt;
        use datafusion::logical_expr::lit;

        // Get col scale
        let col_scale = scales
            .get(ColDimensionConfig::channel_name())
            .ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(
                    format!(
                        "Missing '{}' scale for FacetColGuide",
                        ColDimensionConfig::channel_name()
                    )
                    .into(),
                )
            })?;

        // Extract discrete domain values
        let domain_vals = match col_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };

        // Simplified, robust path: measure only edge subplots at final sizes
        if !domain_vals.is_empty() {
            use crate::facet::band_positions::BandPositionIterator;
            // Final band width and positions
            let bp_iter = BandPositionIterator::from_configured_scale(col_scale)?;
            let band_w = bp_iter.bandwidth();
            let positions: Vec<(datafusion::common::ScalarValue, f32)> =
                BandPositionIterator::from_configured_scale(col_scale)?
                    .map(|bp| (bp.value.clone(), bp.start()))
                    .collect();

            // Identify leftmost and rightmost facet values by band start
            let mut left_val = domain_vals[0].clone();
            let mut right_val = domain_vals[0].clone();
            let mut min_pos = f32::INFINITY;
            let mut max_pos = f32::NEG_INFINITY;
            for (val, p) in &positions {
                if *p < min_pos {
                    min_pos = *p;
                    left_val = val.clone();
                }
                if *p > max_pos {
                    max_pos = *p;
                    right_val = val.clone();
                }
            }

            let mut left_max = 0.0_f32;
            let mut right_max = 0.0_f32;
            let mut top_max = 0.0_f32;
            let mut bottom_max = 0.0_f32;

            // Measure edge subplots via the same path as rendering
            for source in &self.facet_sources {
                // Try to use cached Pass 1 edge overflow measurements
                let cached_overflow = source
                    .cached_edge_overflow
                    .lock()
                    .ok()
                    .and_then(|cache| cache.clone());

                if let Some((first_overflow, last_overflow)) = cached_overflow {
                    // Use cached Pass 1 measurements (from initial subplot dimensions)
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetColGuide: Using cached overflow - left={} right={}",
                            first_overflow.left, last_overflow.right
                        );
                    }
                    left_max = left_max.max(first_overflow.left);
                    right_max = right_max.max(last_overflow.right);
                    top_max = top_max.max(first_overflow.top.max(last_overflow.top));
                    bottom_max = bottom_max.max(first_overflow.bottom.max(last_overflow.bottom));
                    continue; // Skip remeasurement for this source
                }

                // Fallback: Remeasure if cache not available
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!("FacetColGuide: Cache miss, remeasuring overflow");
                }
                use datafusion::logical_expr::lit;

                // Channel expr and dataframes
                let col_expr = source
                    .data
                    .channels()
                    .get(ColDimensionConfig::channel_name())
                    .and_then(|cv| cv.expr(ctx))
                    .ok_or_else(|| {
                        crate::error::AvengerChartError::InternalError(
                            format!(
                                "Facet '{}' channel not found in guide",
                                ColDimensionConfig::channel_name()
                            )
                            .into(),
                        )
                    })?;

                let df_full = source.data.dataframe_with_context(ctx).ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        "Facet guide could not access data".into(),
                    )
                })?;
                let df_left = df_full
                    .clone()
                    .filter(col_expr.clone().eq(lit(left_val.clone())))?;
                let df_right = df_full
                    .clone()
                    .filter(col_expr.eq(lit(right_val.clone())))?;

                // Compute scale sharing from marks
                let coord_channels: Vec<&str> =
                    source.subplot.coord_transform.required_channels().to_vec();
                let mut scale_sharing_by_channel = std::collections::HashMap::new();
                for &ch in &coord_channels {
                    let mut mode = ScaleSharing::Free;
                    for m in &source.subplot.marks {
                        if let Some(cv) = m.data_context().channels().get(ch) {
                            if let Some(share_mode) = cv.get_share_mode() {
                                mode = match (mode, share_mode) {
                                    (ScaleSharing::Free, new_mode) => new_mode,
                                    (ScaleSharing::Shared, _) | (_, ScaleSharing::Shared) => {
                                        ScaleSharing::Shared
                                    }
                                    (existing, _) => existing,
                                };
                            }
                        }
                    }
                    scale_sharing_by_channel.insert(ch.to_string(), mode);
                }

                // Build facet params for left/right via SubplotIterator
                use crate::facet::subplot_iterator::SubplotIterator;
                let iter = SubplotIterator::<ColDimensionConfig>::new(
                    domain_vals.clone(),
                    params.clone(),
                    scale_sharing_by_channel.clone(),
                );
                let mut left_params = params.clone();
                let mut right_params = params.clone();
                for it in iter {
                    if it.facet_value == left_val {
                        left_params = it.params.clone();
                    }
                    if it.facet_value == right_val {
                        right_params = it.params.clone();
                    }
                }

                // Measure using build_plot_components (ensures mark evaluation + scale merging)
                // This matches the exact path used by individual subplot rendering
                use crate::plot::compiled::scale_provider::DefaultScaleProvider;
                use crate::plot::compiled::scales::build_scale_builder_from_marks;

                // Left edge
                let left_builder = build_scale_builder_from_marks(
                    &source.subplot.marks,
                    &source.subplot.scale_specs,
                    &source.subplot.coord_transform,
                    &source.subplot.data,
                    Some(df_left.clone()),
                    ctx,
                    &left_params,
                    _theme,
                )
                .await?;
                let left_provider = DefaultScaleProvider {
                    builder: &left_builder,
                    plot: &source.subplot,
                };
                let left_components = source
                    .subplot
                    .build_plot_components(
                        band_w,
                        plot_height,
                        ctx,
                        &left_params,
                        &left_provider,
                        crate::plot::compiled::EvaluationMode::Measure,
                        Some(&df_left),
                        true,
                    )
                    .await?;
                let left_over = left_components.overflow.unwrap_or_default();

                // Right edge
                let right_builder = build_scale_builder_from_marks(
                    &source.subplot.marks,
                    &source.subplot.scale_specs,
                    &source.subplot.coord_transform,
                    &source.subplot.data,
                    Some(df_right.clone()),
                    ctx,
                    &right_params,
                    _theme,
                )
                .await?;
                let right_provider = DefaultScaleProvider {
                    builder: &right_builder,
                    plot: &source.subplot,
                };
                let right_components = source
                    .subplot
                    .build_plot_components(
                        band_w,
                        plot_height,
                        ctx,
                        &right_params,
                        &right_provider,
                        crate::plot::compiled::EvaluationMode::Measure,
                        Some(&df_right),
                        true,
                    )
                    .await?;
                let right_over = right_components.overflow.unwrap_or_default();

                left_max = left_max.max(left_over.left);
                right_max = right_max.max(right_over.right);
                top_max = top_max.max(left_over.top.max(right_over.top));
                bottom_max = bottom_max.max(left_over.bottom.max(right_over.bottom));
            }

            // Add facet guide space (labels/titles) on top of child overflows
            // Measure facet label slab (same theme contexts used elsewhere)
            let labels = col_scale.domain_labels().unwrap_or_default();
            let measurer = avenger_text::measurement::default_text_measurer();
            let label_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("label");
            let label_font_px = _theme.font_size(&label_ctx).unwrap_or(12.0_f32);
            let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
            let label_font_family_owned = _theme
                .font_family(&label_ctx)
                .or_else(|| _theme.font_family(&root_ctx))
                .unwrap_or_else(|| "sans-serif".to_string());
            let label_font_family = label_font_family_owned.as_str();

            let mut max_label_height = 0.0_f32;
            for label in &labels {
                let config = avenger_text::measurement::TextMeasurementConfig {
                    text: label,
                    font: label_font_family,
                    font_size: label_font_px,
                    font_weight: &avenger_text::types::FontWeight::Name(
                        avenger_text::types::FontWeightNameSpec::Normal,
                    ),
                    font_style: &avenger_text::types::FontStyle::Normal,
                };
                let bounds = measurer.measure_text_bounds(&config);
                max_label_height = max_label_height.max(bounds.height);
            }

            // Facet title (per column) space
            let title_height = if let Some(title_text) = &self.facet_title {
                let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                    .child("facet")
                    .child("title");
                let title_font_px = _theme.font_size(&title_ctx).unwrap_or(12.0_f32);
                let title_family_owned = _theme
                    .font_family(&title_ctx)
                    .unwrap_or_else(|| "sans-serif".to_string());
                let cfg = avenger_text::measurement::TextMeasurementConfig {
                    text: title_text,
                    font: title_family_owned.as_str(),
                    font_size: title_font_px,
                    font_weight: &avenger_text::types::FontWeight::Name(
                        avenger_text::types::FontWeightNameSpec::Normal,
                    ),
                    font_style: &avenger_text::types::FontStyle::Normal,
                };
                let b = measurer.measure_text_bounds(&cfg);
                b.height
            } else {
                0.0
            };

            // Unified x-axis title space (if present)
            let unified_title_height = if let Some(unified_text) = &self.unified_x_title {
                let unified_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                    .child("facet")
                    .child("title");
                let unified_font_px = _theme.font_size(&unified_ctx).unwrap_or(12.0_f32);
                let unified_family_owned = _theme
                    .font_family(&unified_ctx)
                    .unwrap_or_else(|| "sans-serif".to_string());
                let cfg = avenger_text::measurement::TextMeasurementConfig {
                    text: unified_text,
                    font: unified_family_owned.as_str(),
                    font_size: unified_font_px,
                    font_weight: &avenger_text::types::FontWeight::Name(
                        avenger_text::types::FontWeightNameSpec::Normal,
                    ),
                    font_style: &avenger_text::types::FontStyle::Normal,
                };
                let b = measurer.measure_text_bounds(&cfg);
                b.height
            } else {
                0.0
            };

            let gap_title = if self.facet_title.is_some() {
                10.0
            } else {
                0.0
            };
            let facet_label_space = if self.facet_title.is_some() {
                max_label_height + gap_title + title_height + 1.0
            } else {
                max_label_height + 1.0
            };
            let gap_axis = if self.unified_x_title.is_some() {
                6.0
            } else {
                0.0
            };
            let x_axis_title_space = if self.unified_x_title.is_some() {
                gap_axis + unified_title_height + 1.0
            } else {
                0.0
            };

            // Decide placement: if x-axis is top, labels go below; else labels above
            // Use overflow to infer axis position: larger overflow indicates where axis is located
            let place_below = top_max > bottom_max;

            let mut top_final = top_max;
            let mut bottom_final = bottom_max;
            if place_below {
                top_final += x_axis_title_space;
                bottom_final += facet_label_space;
            } else {
                top_final += facet_label_space;
                bottom_final += x_axis_title_space;
            }

            let result = OverflowSpaceRequirement {
                left: left_max,
                right: right_max,
                top: top_final,
                bottom: bottom_final,
            };
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FACET_COL (edge-measured): place_below={} top_max={:.3} bottom_max={:.3} facet_label_space={:.3} x_axis_title_space={:.3}",
                    place_below, top_max, bottom_max, facet_label_space, x_axis_title_space
                );
                eprintln!(
                    "FACET_COL (edge-measured) result: left={:.3} right={:.3} top={:.3} bottom={:.3}",
                    result.left, result.right, result.top, result.bottom
                );
            }
            return Ok(result);
        }

        let mut max_top: f32 = 0.0;
        let mut max_bottom: f32 = 0.0;
        let mut left: f32 = 0.0;
        let mut right: f32 = 0.0;

        for source in &self.facet_sources {
            let col_expr = source
                .data
                .channels()
                .get(ColDimensionConfig::channel_name())
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        format!(
                            "Facet '{}' channel not found in guide",
                            ColDimensionConfig::channel_name()
                        )
                        .into(),
                    )
                })?;

            let df = source.data.dataframe_with_context(ctx).ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(
                    "Facet guide could not access data".into(),
                )
            })?;

            // Compute scale sharing
            let coord_channels: Vec<&str> =
                source.subplot.coord_transform.required_channels().to_vec();
            let mut scale_sharing_by_channel = std::collections::HashMap::new();
            for &ch in &coord_channels {
                let mut mode = ScaleSharing::Free;
                for m in &source.subplot.marks {
                    if let Some(cv) = m.data_context().channels().get(ch) {
                        if let Some(share_mode) = cv.get_share_mode() {
                            mode = match (mode, share_mode) {
                                (ScaleSharing::Free, new_mode) => new_mode,
                                (ScaleSharing::Shared, _) => ScaleSharing::Shared,
                                (_, ScaleSharing::Shared) => ScaleSharing::Shared,
                                (existing, _) => existing,
                            };
                        }
                    }
                }
                scale_sharing_by_channel.insert(ch.to_string(), mode);
            }
            let any_shared = scale_sharing_by_channel
                .values()
                .any(|v| *v == ScaleSharing::Shared);

            use crate::facet::subplot_iterator::SubplotIterator;
            let subplot_iter = SubplotIterator::<ColDimensionConfig>::new(
                domain_vals.clone(),
                params.clone(),
                scale_sharing_by_channel.clone(),
            );

            // Determine band positions to reliably identify leftmost/rightmost columns
            use crate::facet::band_positions::BandPositionIterator;
            let band_positions: Vec<(datafusion::common::ScalarValue, f32)> =
                BandPositionIterator::from_configured_scale(col_scale)?
                    .map(|bp| (bp.value.clone(), bp.start()))
                    .collect();
            let mut min_pos = f32::INFINITY;
            let mut max_pos = f32::NEG_INFINITY;

            // Use the actual band width from the facet dimension scale so
            // overflow reflects the same geometry as subplot rendering.
            let subplot_count = subplot_iter.len();
            let band_w = {
                use crate::facet::band_positions::BandPositionIterator;
                BandPositionIterator::from_configured_scale(col_scale)
                    .map(|it| it.bandwidth())
                    .unwrap_or_else(|_| plot_width / subplot_count.max(1) as f32)
            };

            // Build shared scales once if any channel is shared
            let df_src = if any_shared { df.clone() } else { df.clone() };

            for (idx, iteration) in subplot_iter.enumerate() {
                let filter_df = df
                    .clone()
                    .filter(col_expr.clone().eq(lit(iteration.facet_value.clone())))?;

                // Build scales mirroring subplot rendering behavior:
                // start with shared scales from full DF, then override non-shared
                // channels with scales built from the filtered DF for this column.
                let mut inner_scales = if any_shared {
                    source
                        .subplot
                        .build_scales_for_dataframe(&df_src, band_w, plot_height, ctx, params)
                        .await?
                } else {
                    source
                        .subplot
                        .build_scales_for_dataframe(&filter_df, band_w, plot_height, ctx, params)
                        .await?
                };

                if any_shared {
                    let facet_scales = source
                        .subplot
                        .build_scales_for_dataframe(
                            &filter_df,
                            band_w,
                            plot_height,
                            ctx,
                            &iteration.params,
                        )
                        .await?;
                    for (ch, sharing_mode) in &scale_sharing_by_channel {
                        if *sharing_mode != ScaleSharing::Shared {
                            if let Some(s) = facet_scales.get(ch) {
                                inner_scales.insert(ch.clone(), s.clone());
                            }
                        }
                    }
                }

                let overflow = source
                    .subplot
                    .measure_guide_overflow_with_scales(
                        &inner_scales,
                        band_w,
                        plot_height,
                        ctx,
                        &iteration.params,
                    )
                    .await?;

                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FACET_COL child idx={} of {}: band_w={:.3} -> child overflow L={:.3} R={:.3} T={:.3} B={:.3}",
                        idx,
                        subplot_count,
                        band_w,
                        overflow.left,
                        overflow.right,
                        overflow.top,
                        overflow.bottom
                    );
                }

                // Aggregate top/bottom across all columns
                max_top = max_top.max(overflow.top);
                max_bottom = max_bottom.max(overflow.bottom);

                // Track left/right by actual band position rather than index
                // Find the numeric band start for this facet value
                let mut pos = 0.0_f32;
                for (val, p) in &band_positions {
                    if val == &iteration.facet_value {
                        pos = *p;
                        break;
                    }
                }
                if pos <= min_pos + f32::EPSILON {
                    if pos < min_pos - f32::EPSILON {
                        left = 0.0;
                    }
                    min_pos = pos.min(min_pos);
                    left = left.max(overflow.left);
                }
                if pos >= max_pos - f32::EPSILON {
                    if pos > max_pos + f32::EPSILON {
                        right = 0.0;
                    }
                    max_pos = pos.max(max_pos);
                    right = right.max(overflow.right);
                }
            }
        }

        // Determine label placement based on x-axis position
        // If axis_position returns None (explicit position expression), use overflow to infer
        let place_below = if let Some(source) = self.facet_sources.first() {
            if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                match guide.axis_position("x") {
                    Some(crate::cartesian::axis::AxisPosition::Top) => true, // x at top → labels below
                    Some(crate::cartesian::axis::AxisPosition::Bottom) => false, // x at bottom → labels above
                    None => {
                        // axis_position returns None when position is explicit expression
                        // Infer from overflow: if top > bottom, x-axis is likely at top
                        max_top > max_bottom
                    }
                    _ => false, // fallback: labels above
                }
            } else {
                false // no guide → assume bottom x-axis, labels above
            }
        } else {
            false // no subplots → labels above
        };

        // Measure text bounds for facet labels (use facet label theme context matching RowFacet)
        let labels = col_scale.domain_labels().unwrap_or_default();
        let measurer = avenger_text::measurement::default_text_measurer();
        let label_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("label");
        let label_font_px = _theme.font_size(&label_ctx).unwrap_or(12.0_f32);
        let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
        let label_font_family_owned = _theme
            .font_family(&label_ctx)
            .or_else(|| _theme.font_family(&root_ctx))
            .unwrap_or_else(|| "sans-serif".to_string());
        let label_font_family = label_font_family_owned.as_str();

        let mut max_label_height = 0.0_f32;
        for label in &labels {
            let config = avenger_text::measurement::TextMeasurementConfig {
                text: label,
                font: label_font_family,
                font_size: label_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let bounds = measurer.measure_text_bounds(&config);
            max_label_height = max_label_height.max(bounds.height);
        }

        // Measure facet title if present (use facet title theme context matching RowFacet)
        let title_height = if let Some(title_text) = &self.facet_title {
            let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let title_font_px = _theme.font_size(&title_ctx).unwrap_or(12.0_f32);
            let title_family_owned = _theme
                .font_family(&title_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());
            let cfg = avenger_text::measurement::TextMeasurementConfig {
                text: title_text,
                font: title_family_owned.as_str(),
                font_size: title_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let b = measurer.measure_text_bounds(&cfg);
            b.height
        } else {
            0.0
        };

        // Measure unified x-axis title if present (use facet title theme context matching RowFacet)
        let unified_title_height = if let Some(unified_text) = &self.unified_x_title {
            let unified_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let unified_font_px = _theme.font_size(&unified_ctx).unwrap_or(12.0_f32);
            let unified_family_owned = _theme
                .font_family(&unified_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());
            let cfg = avenger_text::measurement::TextMeasurementConfig {
                text: unified_text,
                font: unified_family_owned.as_str(),
                font_size: unified_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let b = measurer.measure_text_bounds(&cfg);
            b.height
        } else {
            0.0
        };

        // Configurable gaps (matching RowFacet pattern)
        let gap = if self.facet_title.is_some() {
            10.0
        } else {
            0.0
        };

        // Facet labels + gap + facet title (when title present)
        let facet_label_space = if self.facet_title.is_some() {
            max_label_height + gap + title_height + 1.0
        } else {
            max_label_height + 1.0
        };

        // Unified x-axis title space (with gap if present, matching FacetRowGuide)
        let gap_axis = if self.unified_x_title.is_some() {
            6.0
        } else {
            0.0
        };
        let x_axis_title_space = if self.unified_x_title.is_some() {
            gap_axis + unified_title_height + 1.0
        } else {
            0.0
        };

        if place_below {
            // X-axis at top, facet labels below
            // Unified x-axis title goes at top with x-axis
            max_top += x_axis_title_space;
            // Facet labels and title go at bottom BELOW the subplot overflow
            max_bottom += facet_label_space;
        } else {
            // X-axis at bottom (default), facet labels above
            // Facet labels and title go at top ABOVE the subplot overflow
            max_top += facet_label_space;
            // Unified x-axis title goes at bottom with x-axis
            max_bottom += x_axis_title_space;
        }

        let result = OverflowSpaceRequirement {
            top: max_top,
            bottom: max_bottom,
            left,
            right,
        };

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FACET_COL measure_overflow result: left={:.3} right={:.3} top={:.3} bottom={:.3}",
                result.left, result.right, result.top, result.bottom
            );
        }

        Ok(result)
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::scalar::ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, crate::error::AvengerChartError> {
        use crate::scales::ConfiguredScaleLegendExt;
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{TextAlign, TextBaseline};
        use std::sync::Arc as StdArc;

        let mut marks: Vec<SceneMark> = Vec::new();

        let col_scale = match scales.get(ColDimensionConfig::channel_name()) {
            Some(s) => s,
            None => return Ok(marks),
        };

        let labels = col_scale.domain_labels()?;

        // We need to measure subplot overflow to position unified title correctly
        // This mirrors the logic in measure_overflow()
        let domain_vals = match col_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };

        let mut subplot_max_bottom: f32 = 0.0;
        let mut subplot_max_top: f32 = 0.0;

        // Measure subplot overflow to know where x-axis labels end
        if let Some(source) = self.facet_sources.first() {
            let col_expr = source
                .data
                .channels()
                .get(ColDimensionConfig::channel_name())
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        format!(
                            "Facet '{}' channel not found",
                            ColDimensionConfig::channel_name()
                        )
                        .into(),
                    )
                })?;

            let df = source.data.dataframe_with_context(ctx).ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(
                    "Facet guide could not access data".into(),
                )
            })?;

            let coord_channels: Vec<&str> =
                source.subplot.coord_transform.required_channels().to_vec();
            let mut scale_sharing_by_channel = std::collections::HashMap::new();
            for &ch in &coord_channels {
                let mut mode = ScaleSharing::Free;
                for m in &source.subplot.marks {
                    if let Some(cv) = m.data_context().channels().get(ch) {
                        if let Some(share_mode) = cv.get_share_mode() {
                            mode = match (mode, share_mode) {
                                (ScaleSharing::Free, new_mode) => new_mode,
                                (ScaleSharing::Shared, _) => ScaleSharing::Shared,
                                (_, ScaleSharing::Shared) => ScaleSharing::Shared,
                                (existing, _) => existing,
                            };
                        }
                    }
                }
                scale_sharing_by_channel.insert(ch.to_string(), mode);
            }
            let any_shared = scale_sharing_by_channel
                .values()
                .any(|v| *v == ScaleSharing::Shared);

            use crate::facet::subplot_iterator::SubplotIterator;
            let subplot_iter = SubplotIterator::<ColDimensionConfig>::new(
                domain_vals.clone(),
                params.clone(),
                scale_sharing_by_channel.clone(),
            );

            let subplot_count = subplot_iter.len();
            let band_w = plot_width / subplot_count as f32;
            let df_src = if any_shared { df.clone() } else { df.clone() };

            for iteration in subplot_iter {
                let filter_df = df.clone().filter(
                    col_expr
                        .clone()
                        .eq(datafusion::logical_expr::lit(iteration.facet_value.clone())),
                )?;

                let inner_scales = if any_shared {
                    source
                        .subplot
                        .build_scales_for_dataframe(&df_src, band_w, plot_height, ctx, params)
                        .await?
                } else {
                    source
                        .subplot
                        .build_scales_for_dataframe(&filter_df, band_w, plot_height, ctx, params)
                        .await?
                };

                let overflow = source
                    .subplot
                    .measure_guide_overflow_with_scales(
                        &inner_scales,
                        band_w,
                        plot_height,
                        ctx,
                        &iteration.params,
                    )
                    .await?;

                subplot_max_bottom = subplot_max_bottom.max(overflow.bottom);
                subplot_max_top = subplot_max_top.max(overflow.top);
            }
        }

        use crate::facet::band_positions::BandPositionIterator;
        let band_positions: Vec<_> =
            BandPositionIterator::from_configured_scale(col_scale)?.collect();

        // Theme-based font for labels (use facet label theme context matching RowFacet)
        let label_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("label");
        let label_font_px = theme.font_size(&label_ctx).unwrap_or(12.0_f32);
        let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
        let label_font_family_owned = theme
            .font_family(&label_ctx)
            .or_else(|| theme.font_family(&root_ctx))
            .unwrap_or_else(|| "sans-serif".to_string());
        let label_font_family = label_font_family_owned.as_str();

        // Determine label placement based on x-axis position
        // If x-axis is at bottom (default), place facet labels above to avoid collision
        // If x-axis is at top, place facet labels below
        // If axis_position returns None (explicit position expression), use overflow to infer
        let place_below = if let Some(source) = self.facet_sources.first() {
            if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                // Check x-axis position
                match guide.axis_position("x") {
                    Some(crate::cartesian::axis::AxisPosition::Top) => true, // x at top → labels below
                    Some(crate::cartesian::axis::AxisPosition::Bottom) => false, // x at bottom → labels above
                    None => {
                        // axis_position returns None when position is explicit expression
                        // Infer from overflow: if top > bottom, x-axis is likely at top
                        subplot_max_top > subplot_max_bottom
                    }
                    _ => false, // fallback: labels above
                }
            } else {
                false // no guide → assume bottom x-axis, labels above
            }
        } else {
            false // no subplots → labels above
        };

        // Resolve title font properties for rendering
        let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("title");
        let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
        let title_font_family = theme
            .font_family(&title_ctx)
            .unwrap_or_else(|| "sans-serif".to_string());

        // Use guide_utils to render facet label slab (labels + rule + title)
        use crate::facet::guide_utils::{FacetLabelRenderConfig, render_facet_label_slab};

        // Create plot bounds with correct width/height from parameters
        // Offset by subplot overflow so facet labels sit outside subplot guides
        let render_plot_bounds = if place_below {
            // X-axis at top: labels below, extend downward by subplot_max_bottom
            LayoutBounds {
                x: plot_bounds.x,
                y: plot_bounds.y,
                width: plot_width,
                height: plot_height + subplot_max_bottom,
            }
        } else {
            // X-axis at bottom: labels above, extend upward by subplot_max_top
            LayoutBounds {
                x: plot_bounds.x,
                y: plot_bounds.y - subplot_max_top,
                width: plot_width,
                height: plot_height + subplot_max_top,
            }
        };

        let render_config = FacetLabelRenderConfig {
            labels: labels.clone(),
            band_positions: band_positions.clone(),
            plot_bounds: render_plot_bounds,
            is_rotated: false,         // Col labels are horizontal
            place_at_end: place_below, // place_at_end=true means bottom
            font_family: label_font_family.to_string(),
            font_size_px: label_font_px,
            title: self.facet_title.clone(),
            title_font_family: title_font_family.clone(),
            title_font_size_px: title_font_px,
        };

        marks.extend(render_facet_label_slab(&render_config, theme, params));

        // Render unified x-axis title (use facet title theme context matching RowFacet)
        // The unified x-axis title should always be positioned near the x-axes,
        // not move with facet labels. It goes below plot when x-axis is at bottom,
        // above plot when x-axis is at top.
        if let Some(unified_title) = &self.unified_x_title {
            let unified_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let unified_font_px = theme.font_size(&unified_ctx).unwrap_or(12.0_f32);
            let unified_font_family_owned = theme
                .font_family(&unified_ctx)
                .or_else(|| theme.font_family(&root_ctx))
                .unwrap_or_else(|| "sans-serif".to_string());
            let unified_font_family = unified_font_family_owned.as_str();

            // Check x-axis position to determine where unified title should go
            // Use same logic as place_below to handle explicit position expressions
            let x_axis_at_top = if let Some(source) = self.facet_sources.first() {
                if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                    match guide.axis_position("x") {
                        Some(crate::cartesian::axis::AxisPosition::Top) => true,
                        Some(crate::cartesian::axis::AxisPosition::Bottom) => false,
                        None => {
                            // Infer from overflow: if top > bottom, x-axis is likely at top
                            subplot_max_top > subplot_max_bottom
                        }
                        _ => false,
                    }
                } else {
                    false
                }
            } else {
                false
            };

            // Configurable gap (matching measure_overflow and FacetRowGuide)
            let gap_axis = 6.0_f32;

            let y_unified = if x_axis_at_top {
                // X-axis at top: unified title goes above plot, positioned above subplot guides
                // subplot_max_top tells us where the x-axis labels end above the plot
                plot_bounds.y - subplot_max_top - gap_axis
            } else {
                // X-axis at bottom (default): unified title goes just below x-axis labels
                // subplot_max_bottom tells us where the x-axis labels end
                plot_bounds.y + plot_height + subplot_max_bottom + gap_axis
            };

            let unified_mark = SceneTextMark {
                text: unified_title.clone().into(),
                x: (plot_bounds.x + plot_width / 2.0).into(),
                y: y_unified.into(),
                align: TextAlign::Center.into(),
                baseline: if x_axis_at_top {
                    TextBaseline::Bottom
                } else {
                    TextBaseline::Top
                }
                .into(),
                angle: 0.0_f32.into(),
                font: unified_font_family.to_string().into(),
                font_size: unified_font_px.into(),
                color: avenger_common::types::ColorOrGradient::Color(
                    theme
                        .text_color(&unified_ctx)
                        .unwrap_or([0.0, 0.0, 0.0, 1.0]),
                )
                .into(),
                zindex: Some(6),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(unified_mark)));
        }

        Ok(marks)
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> avenger_scenegraph::marks::group::Clip {
        avenger_scenegraph::marks::group::Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }
}

// ============================================================================
// GridFacetGuide - 2D Grid Faceting Guide
// ============================================================================

/// Guide for GridFacet coordinate system
///
/// Renders facet labels for both row (vertical) and column (horizontal) dimensions,
/// creating a 2D grid of subplots. Row labels appear on the left, column labels
/// appear above or below based on x-axis position.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct GridFacetGuide {
    facet_sources: Vec<FacetSource>,

    // Row dimension
    pub row_title: Option<String>,
    unified_y_title: Option<String>,
    unifiable_row_channel: Option<String>,

    // Column dimension
    pub col_title: Option<String>,
    unified_x_title: Option<String>,
    unifiable_col_channel: Option<String>,
}

impl GridFacetGuide {}

impl CoordinateGuide for GridFacetGuide {
    type Axis = crate::cartesian::axis::CartesianAxis;

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<std::sync::Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
        self.facet_sources.clear();
        for m in compiled_marks {
            if let Some(facet) = m
                .as_any()
                .downcast_ref::<crate::facet::marks::facet::CompiledFacetGrid>()
            {
                self.facet_sources.push(FacetSource {
                    subplot: facet.compiled_subplot.clone(),
                    data: facet.state.data.clone(),
                    user_title: None, // Grid stores separate row/col titles
                    cached_edge_overflow: std::sync::Arc::new(std::sync::Mutex::new(None)), // Grid facets don't cache (yet)
                });

                // Use user-specified titles from facet if available
                if self.row_title.is_none() && facet.row_title.is_some() {
                    self.row_title = facet.row_title.clone();
                }
                if self.col_title.is_none() && facet.col_title.is_some() {
                    self.col_title = facet.col_title.clone();
                }
            }
        }

        // Derive row title from row channel (fallback if not explicitly set)
        if self.row_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(cv) = src.data.channels().get("row") {
                    if let Some(name) = cv.as_column_name(_session_context) {
                        self.row_title = Some(name);
                    }
                }
            }
        }

        // Derive col title from col channel (fallback if not explicitly set)
        if self.col_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(cv) = src.data.channels().get("col") {
                    if let Some(name) = cv.as_column_name(_session_context) {
                        self.col_title = Some(name);
                    }
                }
            }
        }

        // Derive unified y-title from subplot guide (for row dimension)
        if self.unified_y_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                use crate::facet::dimension_config::RowDimensionConfig;
                if let Some(info) = src.subplot.compiled_guide.as_ref().and_then(|g| {
                    g.facet_unifiable_channel(
                        RowDimensionConfig::facet_direction(),
                        src.subplot.marks(),
                        _session_context,
                    )
                }) {
                    self.unifiable_row_channel = Some(info.channel);
                    self.unified_y_title = info.title;
                }
            }
        }

        // Derive unified x-title from subplot guide (for col dimension)
        if self.unified_x_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                use crate::facet::dimension_config::ColDimensionConfig;
                if let Some(info) = src.subplot.compiled_guide.as_ref().and_then(|g| {
                    g.facet_unifiable_channel(
                        ColDimensionConfig::facet_direction(),
                        src.subplot.marks(),
                        _session_context,
                    )
                }) {
                    self.unifiable_col_channel = Some(info.channel);
                    self.unified_x_title = info.title;
                }
            }
        }
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for GridFacetGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, crate::error::AvengerChartError> {
        use crate::scales::ConfiguredScaleLegendExt;
        use datafusion::logical_expr::lit;

        // Get row and col scales (may not exist for degenerate single-value cases)
        let row_scale_opt = scales.get("row");
        let col_scale_opt = scales.get("col");

        // If either scale is missing, this is a degenerate case - return empty marks
        if row_scale_opt.is_none() || col_scale_opt.is_none() {
            return Ok(OverflowSpaceRequirement::default());
        }

        let row_scale = row_scale_opt.unwrap();
        let col_scale = col_scale_opt.unwrap();

        // Extract domain values
        let row_domain_vals = match row_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };
        let col_domain_vals = match col_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };

        // Skip if single row or single col (degenerate grid)
        let skip_row_labels = row_domain_vals.len() <= 1;
        let skip_col_labels = col_domain_vals.len() <= 1;

        let mut max_top: f32 = 0.0;
        let mut max_bottom: f32 = 0.0;
        let mut max_left: f32 = 0.0;
        let mut max_right: f32 = 0.0;

        // Measure subplot overflows using GridSubplotIterator
        for source in &self.facet_sources {
            let row_expr = source
                .data
                .channels()
                .get("row")
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        "Facet 'row' channel not found in guide".into(),
                    )
                })?;
            let col_expr = source
                .data
                .channels()
                .get("col")
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        "Facet 'col' channel not found in guide".into(),
                    )
                })?;

            let df = source.data.dataframe_with_context(ctx).ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(
                    "Facet guide could not access data".into(),
                )
            })?;

            // Compute scale sharing
            let coord_channels: Vec<&str> =
                source.subplot.coord_transform.required_channels().to_vec();
            let mut scale_sharing_by_channel = std::collections::HashMap::new();
            for &ch in &coord_channels {
                let mut mode = ScaleSharing::Free;
                for m in &source.subplot.marks {
                    if let Some(cv) = m.data_context().channels().get(ch) {
                        if let Some(share_mode) = cv.get_share_mode() {
                            mode = match (mode, share_mode) {
                                (ScaleSharing::Free, new_mode) => new_mode,
                                (ScaleSharing::Shared, _) => ScaleSharing::Shared,
                                (_, ScaleSharing::Shared) => ScaleSharing::Shared,
                                (existing, _) => existing,
                            };
                        }
                    }
                }
                scale_sharing_by_channel.insert(ch.to_string(), mode);
            }

            use crate::facet::dimension_config::{ColDimensionConfig, RowDimensionConfig};
            use crate::facet::subplot_iterator::SubplotIterator;

            let num_rows = row_domain_vals.len();
            let num_cols = col_domain_vals.len();
            let band_w = plot_width / num_cols.max(1) as f32;
            let band_h = plot_height / num_rows.max(1) as f32;

            // Build base scales from full DataFrame (used as fallback for empty cells)
            let base_scales = source
                .subplot
                .build_scales_for_dataframe(&df, band_w, band_h, ctx, params)
                .await?;

            // Use nested SubplotIterators to iterate over grid cells
            let row_iter = SubplotIterator::<RowDimensionConfig>::new(
                row_domain_vals.clone(),
                params.clone(),
                scale_sharing_by_channel.clone(),
            );

            for row_iteration in row_iter {
                let col_iter = SubplotIterator::<ColDimensionConfig>::new(
                    col_domain_vals.clone(),
                    params.clone(),
                    scale_sharing_by_channel.clone(),
                );

                for col_iteration in col_iter {
                    // Merge row and col contexts into unified GridFacet context
                    let merged_params = crate::facet::marks::facet::merge_grid_facet_contexts(
                        &row_iteration,
                        &col_iteration,
                        num_rows,
                        num_cols,
                    );

                    // Filter to rows matching both row AND col values
                    let filter_df = df
                        .clone()
                        .filter(row_expr.clone().eq(lit(row_iteration.facet_value.clone())))?
                        .filter(col_expr.clone().eq(lit(col_iteration.facet_value.clone())))?;

                    // Check if this cell has any data
                    let cell_count = filter_df.clone().count().await?;
                    let cell_is_empty = cell_count == 0;

                    // Build scales for this subplot
                    let mut inner_scales = base_scales.clone();

                    // For independent (free) channels, rebuild from cell data if cell is non-empty
                    if !cell_is_empty {
                        for (ch, sharing_mode) in &scale_sharing_by_channel {
                            if *sharing_mode != ScaleSharing::Shared {
                                // This channel is independent - rebuild from filtered data
                                let facet_scales = source
                                    .subplot
                                    .build_scales_for_dataframe(
                                        &filter_df,
                                        band_w,
                                        band_h,
                                        ctx,
                                        &merged_params,
                                    )
                                    .await?;

                                if let Some(s) = facet_scales.get(ch) {
                                    inner_scales.insert(ch.clone(), s.clone());
                                }
                            }
                        }
                    }

                    let overflow = source
                        .subplot
                        .measure_guide_overflow_with_scales(
                            &inner_scales,
                            band_w,
                            band_h,
                            ctx,
                            &merged_params,
                        )
                        .await?;

                    // Track top/bottom overflow (for first/last row)
                    if row_iteration.index == 0 {
                        max_top = max_top.max(overflow.top);
                    }
                    if row_iteration.index == num_rows - 1 {
                        max_bottom = max_bottom.max(overflow.bottom);
                    }

                    // Track left/right overflow (for first/last col)
                    if col_iteration.index == 0 {
                        max_left = max_left.max(overflow.left);
                    }
                    if col_iteration.index == num_cols - 1 {
                        max_right = max_right.max(overflow.right);
                    }
                }
            }
        }

        // Measure row labels (if not degenerate)
        let row_label_space = if !skip_row_labels {
            let row_labels = row_scale.domain_labels()?;
            let guide_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("label");
            let label_font_px = theme.font_size(&guide_ctx).unwrap_or(12.0_f32);
            let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
            let label_font_family = theme
                .font_family(&guide_ctx)
                .or_else(|| theme.font_family(&root_ctx))
                .unwrap_or_else(|| "sans-serif".to_string());

            let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
            let title_font_family = theme
                .font_family(&title_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());

            use crate::facet::guide_utils::{
                FacetLabelMeasurementConfig, measure_facet_label_slab,
            };
            let measurement_config = FacetLabelMeasurementConfig {
                labels: row_labels,
                is_rotated: true, // Row labels are vertical
                font_family: label_font_family,
                font_size_px: label_font_px,
                title: self.row_title.clone(),
                title_font_family: title_font_family,
                title_font_size_px: title_font_px,
            };
            measure_facet_label_slab(&measurement_config)
        } else {
            0.0
        };

        // Measure col labels (if not degenerate)
        let col_label_space = if !skip_col_labels {
            let col_labels = col_scale.domain_labels()?;
            let guide_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("label");
            let label_font_px = theme.font_size(&guide_ctx).unwrap_or(12.0_f32);
            let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
            let label_font_family = theme
                .font_family(&guide_ctx)
                .or_else(|| theme.font_family(&root_ctx))
                .unwrap_or_else(|| "sans-serif".to_string());

            let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
            let title_font_family = theme
                .font_family(&title_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());

            use crate::facet::guide_utils::{
                FacetLabelMeasurementConfig, measure_facet_label_slab,
            };
            let measurement_config = FacetLabelMeasurementConfig {
                labels: col_labels,
                is_rotated: false, // Col labels are horizontal
                font_family: label_font_family,
                font_size_px: label_font_px,
                title: self.col_title.clone(),
                title_font_family: title_font_family,
                title_font_size_px: title_font_px,
            };
            measure_facet_label_slab(&measurement_config)
        } else {
            0.0
        };

        // Measure unified y-title (rotated, on right side)
        let unified_y_height = if let Some(y_title) = &self.unified_y_title {
            let y_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let y_font_px = theme.font_size(&y_ctx).unwrap_or(12.0_f32);
            let y_family_owned = theme
                .font_family(&y_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());
            let cfg_y = avenger_text::measurement::TextMeasurementConfig {
                text: y_title,
                font: y_family_owned.as_str(),
                font_size: y_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let b_y =
                avenger_text::measurement::default_text_measurer().measure_text_bounds(&cfg_y);
            b_y.height + 1.0
        } else {
            0.0
        };

        // Measure unified x-title (horizontal, position based on x-axis)
        let unified_x_height = if let Some(x_title) = &self.unified_x_title {
            let x_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let x_font_px = theme.font_size(&x_ctx).unwrap_or(12.0_f32);
            let x_family_owned = theme
                .font_family(&x_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());
            let cfg_x = avenger_text::measurement::TextMeasurementConfig {
                text: x_title,
                font: x_family_owned.as_str(),
                font_size: x_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let b_x =
                avenger_text::measurement::default_text_measurer().measure_text_bounds(&cfg_x);
            b_x.height + 1.0
        } else {
            0.0
        };

        // Determine if col labels go below (when x-axis at top)
        let place_col_below = if let Some(source) = self.facet_sources.first() {
            if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                match guide.axis_position("x") {
                    Some(crate::cartesian::axis::AxisPosition::Top) => true,
                    Some(crate::cartesian::axis::AxisPosition::Bottom) => false,
                    None => max_top > max_bottom,
                    _ => false,
                }
            } else {
                false
            }
        } else {
            false
        };

        // Determine row label placement (opposite side of y-axis, like FacetRow)
        // If right overflow > left overflow, y-axis is on right, so row labels go left
        let place_row_on_left = max_right > max_left;

        // Allocate overflow space
        let gap_axis = 6.0_f32;

        // Left/right overflow depends on row label placement
        let (left_final, right_final) = if place_row_on_left {
            // Row labels on left, y-axis on right
            (
                row_label_space + max_left,
                max_right
                    + if unified_y_height > 0.0 {
                        gap_axis + unified_y_height
                    } else {
                        0.0
                    },
            )
        } else {
            // Row labels on right, y-axis on left
            (
                max_left
                    + if unified_y_height > 0.0 {
                        gap_axis + unified_y_height
                    } else {
                        0.0
                    },
                row_label_space + max_right,
            )
        };

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FACET_COL final overflow: left_final={:.3} right_final={:.3} (max_left_child={:.3} max_right_child={:.3} place_row_on_left={} unified_y_title_present={})",
                left_final,
                right_final,
                max_left,
                max_right,
                place_row_on_left,
                self.unified_y_title.is_some()
            );
        }

        // Top/bottom overflow depends on col label placement
        let (top_final, bottom_final) = if place_col_below {
            (
                // Top: subplot top overflow + optional unified x-title
                max_top
                    + if unified_x_height > 0.0 {
                        gap_axis + unified_x_height
                    } else {
                        0.0
                    },
                // Bottom: col labels + subplot bottom overflow
                col_label_space + max_bottom,
            )
        } else {
            (
                // Top: col labels + subplot top overflow
                col_label_space + max_top,
                // Bottom: subplot bottom overflow + optional unified x-title
                max_bottom
                    + if unified_x_height > 0.0 {
                        gap_axis + unified_x_height
                    } else {
                        0.0
                    },
            )
        };

        Ok(OverflowSpaceRequirement {
            top: top_final,
            bottom: bottom_final,
            left: left_final,
            right: right_final,
        })
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, crate::error::AvengerChartError> {
        use crate::scales::ConfiguredScaleLegendExt;
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{TextAlign, TextBaseline};
        use std::sync::Arc as StdArc;

        let mut marks: Vec<SceneMark> = Vec::new();

        // Get row and col scales (may not exist for degenerate single-value cases)
        let row_scale = match scales.get("row") {
            Some(s) => s,
            None => return Ok(marks), // Degenerate case: no row scale
        };
        let col_scale = match scales.get("col") {
            Some(s) => s,
            None => return Ok(marks), // Degenerate case: no col scale
        };

        // Extract domain values and labels
        let row_domain_vals = match row_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };
        let col_domain_vals = match col_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };

        let row_labels = row_scale.domain_labels()?;
        let col_labels = col_scale.domain_labels()?;

        // Skip if single row or single col (degenerate grid)
        let skip_row_labels = row_domain_vals.len() <= 1;
        let skip_col_labels = col_domain_vals.len() <= 1;

        // Get band positions for both dimensions
        // These scales have the correct spacing from the facet mark (via scale updates).
        // We'll use .center() on each BandPosition for label/tick positioning.
        use crate::facet::band_positions::BandPositionIterator;
        let row_band_positions: Vec<_> =
            BandPositionIterator::from_configured_scale(row_scale)?.collect();
        let col_band_positions: Vec<_> =
            BandPositionIterator::from_configured_scale(col_scale)?.collect();

        // Debug: log scale options and computed band centers used for labels
        if cfg!(debug_assertions) || std::env::var("RUST_LOG").is_ok() {
            use avenger_scales::scales::band;
            let row_bw = band::bandwidth(&row_scale.config)?;
            let col_bw = band::bandwidth(&col_scale.config)?;
            let row_pad_inner_px = row_scale.config.option_f32("padding_inner_px", 0.0);
            let col_pad_inner_px = col_scale.config.option_f32("padding_inner_px", 0.0);
            tracing::debug!(
                row_bw = row_bw,
                col_bw = col_bw,
                row_pad_inner_px = row_pad_inner_px,
                col_pad_inner_px = col_pad_inner_px,
                "GridFacetGuide: bandwidth and padding_inner_px"
            );
            for (i, bp) in col_band_positions.iter().enumerate() {
                let label = col_labels.get(i).cloned().unwrap_or_default();
                tracing::debug!(
                    i = i,
                    ?label,
                    center = bp.center(),
                    bw = col_bw,
                    "GridFacetGuide: col label center position"
                );
            }
            for (i, bp) in row_band_positions.iter().enumerate() {
                let label = row_labels.get(i).cloned().unwrap_or_default();
                tracing::debug!(
                    i = i,
                    ?label,
                    center = bp.center(),
                    bw = row_bw,
                    "GridFacetGuide: row label center position"
                );
            }
        }

        // Measure subplot overflows to determine row and col label placement
        let mut subplot_max_top: f32 = 0.0;
        let mut subplot_max_bottom: f32 = 0.0;
        let mut subplot_max_left: f32 = 0.0;
        let mut subplot_max_right: f32 = 0.0;

        if let Some(source) = self.facet_sources.first() {
            let row_expr = source
                .data
                .channels()
                .get("row")
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        "Facet 'row' channel not found".into(),
                    )
                })?;
            let col_expr = source
                .data
                .channels()
                .get("col")
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        "Facet 'col' channel not found".into(),
                    )
                })?;

            let df = source.data.dataframe_with_context(ctx).ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(
                    "Facet guide could not access data".into(),
                )
            })?;

            let coord_channels: Vec<&str> =
                source.subplot.coord_transform.required_channels().to_vec();
            let mut scale_sharing_by_channel = std::collections::HashMap::new();
            for &ch in &coord_channels {
                let mut mode = ScaleSharing::Free;
                for m in &source.subplot.marks {
                    if let Some(cv) = m.data_context().channels().get(ch) {
                        if let Some(share_mode) = cv.get_share_mode() {
                            mode = match (mode, share_mode) {
                                (ScaleSharing::Free, new_mode) => new_mode,
                                (ScaleSharing::Shared, _) => ScaleSharing::Shared,
                                (_, ScaleSharing::Shared) => ScaleSharing::Shared,
                                (existing, _) => existing,
                            };
                        }
                    }
                }
                scale_sharing_by_channel.insert(ch.to_string(), mode);
            }

            use crate::facet::dimension_config::{ColDimensionConfig, RowDimensionConfig};
            use crate::facet::subplot_iterator::SubplotIterator;

            let num_rows = row_domain_vals.len();
            let num_cols = col_domain_vals.len();
            let band_w = plot_width / num_cols.max(1) as f32;
            let band_h = plot_height / num_rows.max(1) as f32;

            // Build base scales from full DataFrame (used as fallback for empty cells)
            let base_scales = source
                .subplot
                .build_scales_for_dataframe(&df, band_w, band_h, ctx, params)
                .await?;

            // Use nested SubplotIterators to iterate over grid cells
            let row_iter = SubplotIterator::<RowDimensionConfig>::new(
                row_domain_vals.clone(),
                params.clone(),
                scale_sharing_by_channel.clone(),
            );

            for row_iteration in row_iter {
                let col_iter = SubplotIterator::<ColDimensionConfig>::new(
                    col_domain_vals.clone(),
                    params.clone(),
                    scale_sharing_by_channel.clone(),
                );

                for col_iteration in col_iter {
                    // Merge row and col contexts into unified GridFacet context
                    let merged_params = crate::facet::marks::facet::merge_grid_facet_contexts(
                        &row_iteration,
                        &col_iteration,
                        num_rows,
                        num_cols,
                    );

                    let filter_df = df
                        .clone()
                        .filter(row_expr.clone().eq(datafusion::logical_expr::lit(
                            row_iteration.facet_value.clone(),
                        )))?
                        .filter(col_expr.clone().eq(datafusion::logical_expr::lit(
                            col_iteration.facet_value.clone(),
                        )))?;

                    // Check if this cell has any data
                    let cell_count = filter_df.clone().count().await?;
                    let cell_is_empty = cell_count == 0;

                    // Build scales for this subplot
                    let mut inner_scales = base_scales.clone();

                    // For independent (free) channels, rebuild from cell data if cell is non-empty
                    if !cell_is_empty {
                        for (ch, sharing_mode) in &scale_sharing_by_channel {
                            if *sharing_mode != ScaleSharing::Shared {
                                // This channel is independent - rebuild from filtered data
                                let facet_scales = source
                                    .subplot
                                    .build_scales_for_dataframe(
                                        &filter_df,
                                        band_w,
                                        band_h,
                                        ctx,
                                        &merged_params,
                                    )
                                    .await?;

                                if let Some(s) = facet_scales.get(ch) {
                                    inner_scales.insert(ch.clone(), s.clone());
                                }
                            }
                        }
                    }

                    let overflow = source
                        .subplot
                        .measure_guide_overflow_with_scales(
                            &inner_scales,
                            band_w,
                            band_h,
                            ctx,
                            &merged_params,
                        )
                        .await?;

                    subplot_max_top = subplot_max_top.max(overflow.top);
                    subplot_max_bottom = subplot_max_bottom.max(overflow.bottom);
                    subplot_max_left = subplot_max_left.max(overflow.left);
                    subplot_max_right = subplot_max_right.max(overflow.right);
                }
            }
        }

        // Determine row label placement (opposite side of y-axis, like FacetRow)
        // If right overflow > left overflow, y-axis is on right, so row labels go left
        let place_row_on_left = subplot_max_right > subplot_max_left;

        // Determine col label placement
        let place_col_below = if let Some(source) = self.facet_sources.first() {
            if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                match guide.axis_position("x") {
                    Some(crate::cartesian::axis::AxisPosition::Top) => true,
                    Some(crate::cartesian::axis::AxisPosition::Bottom) => false,
                    None => subplot_max_top > subplot_max_bottom,
                    _ => false,
                }
            } else {
                false
            }
        } else {
            false
        };

        // Theme contexts
        let guide_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("label");
        let label_font_px = theme.font_size(&guide_ctx).unwrap_or(12.0_f32);
        let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
        let label_font_family = theme
            .font_family(&guide_ctx)
            .or_else(|| theme.font_family(&root_ctx))
            .unwrap_or_else(|| "sans-serif".to_string());

        let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("title");
        let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
        let title_font_family = theme
            .font_family(&title_ctx)
            .unwrap_or_else(|| "sans-serif".to_string());

        // Render row labels (on opposite side of y-axis)
        if !skip_row_labels {
            use crate::facet::guide_utils::{FacetLabelRenderConfig, render_facet_label_slab};

            // Adjust plot bounds to account for subplot overflow when positioning facet labels
            // When labels are on right, extend width by right overflow so labels appear after it
            // When labels are on left, shift x by left overflow so labels appear before it
            let render_plot_bounds = if place_row_on_left {
                LayoutBounds {
                    x: plot_bounds.x - subplot_max_left,
                    y: plot_bounds.y,
                    width: plot_width + subplot_max_left,
                    height: plot_height,
                }
            } else {
                LayoutBounds {
                    x: plot_bounds.x,
                    y: plot_bounds.y,
                    width: plot_width + subplot_max_right,
                    height: plot_height,
                }
            };

            let row_config = FacetLabelRenderConfig {
                labels: row_labels.clone(),
                band_positions: row_band_positions.clone(),
                plot_bounds: render_plot_bounds,
                is_rotated: true,                 // Row labels are vertical
                place_at_end: !place_row_on_left, // Opposite side of y-axis
                font_family: label_font_family.clone(),
                font_size_px: label_font_px,
                title: self.row_title.clone(),
                title_font_family: title_font_family.clone(),
                title_font_size_px: title_font_px,
            };
            marks.extend(render_facet_label_slab(&row_config, theme, params));
        }

        // Render col labels (top or bottom, unless degenerate)
        if !skip_col_labels {
            use crate::facet::guide_utils::{FacetLabelRenderConfig, render_facet_label_slab};

            // Adjust plot bounds to account for subplot overflow when positioning facet labels
            // When labels are below, extend height by bottom overflow so labels appear after it
            // When labels are above, shift y by top overflow so labels appear before it
            let render_plot_bounds = if place_col_below {
                LayoutBounds {
                    x: plot_bounds.x,
                    y: plot_bounds.y,
                    width: plot_width,
                    height: plot_height + subplot_max_bottom,
                }
            } else {
                LayoutBounds {
                    x: plot_bounds.x,
                    y: plot_bounds.y - subplot_max_top,
                    width: plot_width,
                    height: plot_height + subplot_max_top,
                }
            };

            let col_config = FacetLabelRenderConfig {
                labels: col_labels.clone(),
                band_positions: col_band_positions.clone(),
                plot_bounds: render_plot_bounds,
                is_rotated: false, // Col labels are horizontal
                place_at_end: place_col_below,
                font_family: label_font_family.clone(),
                font_size_px: label_font_px,
                title: self.col_title.clone(),
                title_font_family: title_font_family.clone(),
                title_font_size_px: title_font_px,
            };
            marks.extend(render_facet_label_slab(&col_config, theme, params));
        }

        // Render unified y-axis title (rotated, on same side as y-axis, opposite of row labels)
        if let Some(y_title) = &self.unified_y_title {
            let y_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let y_font_px = theme.font_size(&y_ctx).unwrap_or(12.0_f32);
            let y_color = theme.text_color(&y_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let y_family_owned = theme
                .font_family(&y_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());

            // Y-axis is on the opposite side of row labels
            let axis_on_right = place_row_on_left;
            let gap = 6.0_f32;

            let x_bottom = if axis_on_right {
                plot_bounds.x + plot_width + subplot_max_right + gap
            } else {
                plot_bounds.x - subplot_max_left - gap
            };
            let y_center = plot_bounds.y + 0.5 * plot_height;

            let y_mark = SceneTextMark {
                text: y_title.clone().into(),
                x: x_bottom.into(),
                y: y_center.into(),
                align: TextAlign::Center.into(),
                baseline: TextBaseline::Bottom.into(),
                font: y_family_owned.clone().into(),
                font_size: y_font_px.into(),
                angle: if axis_on_right {
                    90.0_f32.into()
                } else {
                    (-90.0_f32).into()
                },
                color: avenger_common::types::ColorOrGradient::Color(y_color).into(),
                zindex: Some(6),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(y_mark)));
        }

        // Render unified x-axis title (horizontal, position based on x-axis)
        if let Some(x_title) = &self.unified_x_title {
            let x_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let x_font_px = theme.font_size(&x_ctx).unwrap_or(12.0_f32);
            let x_color = theme.text_color(&x_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let x_family_owned = theme
                .font_family(&x_ctx)
                .or_else(|| theme.font_family(&root_ctx))
                .unwrap_or_else(|| "sans-serif".to_string());

            let gap_axis = 6.0_f32;
            let x_axis_at_top = place_col_below;

            let y_unified = if x_axis_at_top {
                plot_bounds.y - subplot_max_top - gap_axis
            } else {
                plot_bounds.y + plot_height + subplot_max_bottom + gap_axis
            };

            let unified_mark = SceneTextMark {
                text: x_title.clone().into(),
                x: (plot_bounds.x + plot_width / 2.0).into(),
                y: y_unified.into(),
                align: TextAlign::Center.into(),
                baseline: if x_axis_at_top {
                    TextBaseline::Bottom
                } else {
                    TextBaseline::Top
                }
                .into(),
                angle: 0.0_f32.into(),
                font: x_family_owned.to_string().into(),
                font_size: x_font_px.into(),
                color: avenger_common::types::ColorOrGradient::Color(x_color).into(),
                zindex: Some(6),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(unified_mark)));
        }

        Ok(marks)
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> avenger_scenegraph::marks::group::Clip {
        avenger_scenegraph::marks::group::Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }
}
