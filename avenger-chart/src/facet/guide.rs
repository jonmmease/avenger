use crate::facet::dimension_config::{ColDimensionConfig, FacetDimensionConfig, RowDimensionConfig};
use crate::guide::{CompiledGuide, CoordinateGuide, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::marks::CompiledMark;
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
}

impl FacetRowGuide {
    /// Get the unified channel (if any) for this facet guide
    fn get_unified_channel(&self) -> Option<String> {
        self.unifiable_channel.clone()
    }
}

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
        use crate::scales::ConfiguredScaleLegendExt;
        use datafusion::logical_expr::lit;

        // Need row scale
        let row_scale = scales.get(RowDimensionConfig::channel_name()).ok_or_else(|| {
            crate::error::AvengerChartError::InternalError(
                format!("Missing '{}' scale for FacetRowGuide", RowDimensionConfig::channel_name()).into(),
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

        // Get unified channel once from stored configuration
        let unified_channel = self.get_unified_channel();

        // For each facet source (there could be more than one Facet mark)
        for source in &self.facet_sources {
            // Row expression from channels
            let row_expr = source
                .data
                .channels()
                .get(RowDimensionConfig::channel_name())
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        format!("Facet '{}' channel not found in guide", RowDimensionConfig::channel_name()).into(),
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
                let mut shared = false;
                for m in &source.subplot.marks {
                    if let Some(cv) = m.data_context().channels().get(ch) {
                        if let Some(true) = cv.get_share_across_facets() {
                            shared = true;
                            break;
                        }
                    }
                }
                scale_sharing_by_channel.insert(ch.to_string(), shared);
            }
            let any_shared = scale_sharing_by_channel.values().any(|v| *v);

            // Use SubplotIterator to ensure consistent FacetContext across all subplots
            use crate::facet::subplot_iterator::SubplotIterator;
            let subplot_iter = SubplotIterator::<RowDimensionConfig>::new(
                domain_vals.clone(),
                unified_channel.clone(),
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
        // Add space for facet labels by measuring text bounds
        // For 90° rotation, horizontal footprint ≈ text height
        let labels = row_scale.domain_labels().unwrap_or_default();
        let measurer = avenger_text::measurement::default_text_measurer();
        // Resolve facet-label theme (fallbacks kept for now)
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
        let _text_rgba = theme.text_color(&guide_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let mut max_label_height = 0.0_f32;
        for label in &labels {
            let config = avenger_text::measurement::TextMeasurementConfig {
                text: label,
                font: font_family,
                font_size: font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let bounds = measurer.measure_text_bounds(&config);
            max_label_height = max_label_height.max(bounds.height);
        }
        // If there's a facet title, measure it (used for facet-by side placement and overflow)
        let title_height = if let Some(title_text) = &self.facet_title {
            let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
            let title_family_owned = theme
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
            let b = avenger_text::measurement::default_text_measurer().measure_text_bounds(&cfg);
            b.height
        } else {
            0.0
        };

        // Compute facet-by side overflow: include label column + spacing + facet title (rotated width)
        let gap = if self.facet_title.is_some() { 10.0 } else { 0.0 };
        let estimated_right = if self.facet_title.is_some() {
            max_label_height + gap + title_height + 1.0
        } else {
            // No title: just the label column (use full label_height for edge safety)
            max_label_height + 1.0
        };

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
            // Axis on right: left side uses facet-by (if placed left) otherwise just child left
            if place_on_left {
                max_left.max(estimated_right)
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
            // Axis on left: right side uses facet-by (if placed right) otherwise just child right
            if place_on_left {
                max_right
            } else {
                max_right.max(estimated_right)
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

        // Get centered positions using band=0.5
        // The scale now has the correct padding_inner_px from the facet mark (via scale updates),
        // Use BandPositionIterator with band=0.5 to get positions at the center of each band
        use crate::facet::band_positions::BandPositionIterator;
        let band_positions: Vec<_> = BandPositionIterator::from_configured_scale_with_band(row_scale, 0.5)?
            .collect();

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

        // Get unified channel once from stored configuration
        let unified_channel = self.get_unified_channel();

        for source in &self.facet_sources {
            let row_expr = source
                .data
                .channels()
                .get(RowDimensionConfig::channel_name())
                .and_then(|cv| cv.expr(_ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        format!("Facet '{}' channel not found in guide", RowDimensionConfig::channel_name()).into(),
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
                let mut shared = false;
                for m in &source.subplot.marks {
                    if let Some(cv) = m.data_context().channels().get(ch) {
                        if let Some(true) = cv.get_share_across_facets() {
                            shared = true;
                            break;
                        }
                    }
                }
                scale_sharing_by_channel.insert(ch.to_string(), shared);
            }
            let any_shared = scale_sharing_by_channel.values().any(|v| *v);

            // Use SubplotIterator to ensure consistent FacetContext across all subplots
            use crate::facet::subplot_iterator::SubplotIterator;
            let subplot_iter = SubplotIterator::<RowDimensionConfig>::new(
                domain_vals_eval.clone(),
                unified_channel.clone(),
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

        // Place facet labels at band centers, rotated 90 (CW on right, CCW on left)
        // Anchor at the text center so after rotation it's vertically centered.
        // Positions are already centered (band=0.5), so no offset needed
        for (label, band_pos) in labels.iter().zip(&band_positions) {
            let y_center = plot_bounds.y + band_pos.position;
            // Measure this label to position its center so left edge is at plot edge
            let config = avenger_text::measurement::TextMeasurementConfig {
                text: label,
                font: font_family,
                font_size: font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let bounds =
                avenger_text::measurement::default_text_measurer().measure_text_bounds(&config);
            // With rotation, horizontal span ≈ bounds.height; side decides x and angle
            let x_center = if place_on_left {
                plot_bounds.x - 0.5 * bounds.height
            } else {
                plot_bounds.x + plot_width + 0.5 * bounds.height
            };
            let text = SceneTextMark {
                text: label.clone().into(),
                x: x_center.into(),
                y: y_center.into(),
                align: TextAlign::Center.into(),
                baseline: TextBaseline::Middle.into(),
                angle: if place_on_left {
                    (-90.0_f32).into()
                } else {
                    90.0_f32.into()
                },
                font: font_family.to_string().into(),
                font_size: font_px.into(),
                color: avenger_common::types::ColorOrGradient::Color(
                    theme.text_color(&guide_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]),
                )
                .into(),
                zindex: Some(5),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(text)));
        }

        // Render vertical rule between labels and title (if title present)
        if let Some(_title_text) = &self.facet_title {
            if !labels.is_empty() && labels.len() > 1 {
                // Get y positions of first and last labels (already centered)
                let y_top = plot_bounds.y + band_positions.first().map(|bp| bp.position).unwrap_or(0.0);
                let y_bottom = plot_bounds.y + band_positions.last().map(|bp| bp.position).unwrap_or(0.0);

                // Measure label column width
                let labels_for_rule = row_scale.domain_labels().unwrap_or_default();
                let measurer = avenger_text::measurement::default_text_measurer();
                let label_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                    .child("facet")
                    .child("label");
                let label_font_px = theme.font_size(&label_ctx).unwrap_or(12.0_f32);
                let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
                let label_family = theme
                    .font_family(&label_ctx)
                    .or_else(|| theme.font_family(&root_ctx))
                    .unwrap_or_else(|| "sans-serif".to_string());
                let mut max_label_height = 0.0_f32;
                for l in &labels_for_rule {
                    let cfg = avenger_text::measurement::TextMeasurementConfig {
                        text: l,
                        font: label_family.as_str(),
                        font_size: label_font_px,
                        font_weight: &avenger_text::types::FontWeight::Name(
                            avenger_text::types::FontWeightNameSpec::Normal,
                        ),
                        font_style: &avenger_text::types::FontStyle::Normal,
                    };
                    let b = measurer.measure_text_bounds(&cfg);
                    max_label_height = max_label_height.max(b.height);
                }

                // Position rule between label column and title (gap/2)
                let gap = 10.0_f32;
                let x_rule = if place_on_left {
                    plot_bounds.x - (max_label_height + gap / 2.0)
                } else {
                    plot_bounds.x + plot_width + max_label_height + gap / 2.0
                };

                // Query rule styling from theme
                let rule_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                    .child("facet")
                    .child("rule");
                let mut rule_stroke = theme.text_color(&rule_ctx).unwrap_or([0.5, 0.5, 0.5, 1.0]);
                // Ensure fully opaque to avoid overlapping darkening
                rule_stroke[3] = 1.0;
                let rule_stroke_width = theme
                    .query(&rule_ctx, "stroke-width")
                    .and_then(|v| v.as_number())
                    .map(|n| n as f32)
                    .unwrap_or(1.0);

                // Query tick size from theme
                let tick_size = theme
                    .query(&rule_ctx, "tick-size")
                    .and_then(|v| v.as_number())
                    .map(|n| n as f32)
                    .unwrap_or(4.0);

                // Extend rule by half stroke width on each end for cleaner edges
                let half_stroke = rule_stroke_width / 2.0;

                // Create vertical rule mark
                let rule_mark = avenger_scenegraph::marks::rule::SceneRuleMark {
                    x: x_rule.into(),
                    y: (y_top - half_stroke).into(),
                    x2: x_rule.into(),
                    y2: (y_bottom + half_stroke).into(),
                    stroke: avenger_common::types::ColorOrGradient::Color(rule_stroke).into(),
                    stroke_width: rule_stroke_width.into(),
                    zindex: Some(5),
                    ..Default::default()
                };
                marks.push(SceneMark::Rule(rule_mark));

                // Add tick marks at each label position (already centered)
                for band_pos in &band_positions {
                    let y_center = plot_bounds.y + band_pos.position;

                    let (x_tick_start, x_tick_end) = if place_on_left {
                        (x_rule, x_rule + tick_size)
                    } else {
                        (x_rule - tick_size, x_rule)
                    };

                    let tick_mark = avenger_scenegraph::marks::rule::SceneRuleMark {
                        x: x_tick_start.into(),
                        y: y_center.into(),
                        x2: x_tick_end.into(),
                        y2: y_center.into(),
                        stroke: avenger_common::types::ColorOrGradient::Color(rule_stroke).into(),
                        stroke_width: rule_stroke_width.into(),
                        zindex: Some(5),
                        ..Default::default()
                    };
                    marks.push(SceneMark::Rule(tick_mark));
                }
            }
        }

        // Render facet title (if present), rotated 90 CW and centered vertically on RHS
        if let Some(title_text) = &self.facet_title {
            // Measure label column height to position title to the right of it
            let labels = row_scale.domain_labels().unwrap_or_default();
            let measurer = avenger_text::measurement::default_text_measurer();
            let label_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("label");
            let label_font_px = theme.font_size(&label_ctx).unwrap_or(12.0_f32);
            let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
            let label_family = theme
                .font_family(&label_ctx)
                .or_else(|| theme.font_family(&root_ctx))
                .unwrap_or_else(|| "sans-serif".to_string());
            let mut max_label_height = 0.0_f32;
            for l in &labels {
                let cfg = avenger_text::measurement::TextMeasurementConfig {
                    text: l,
                    font: label_family.as_str(),
                    font_size: label_font_px,
                    font_weight: &avenger_text::types::FontWeight::Name(
                        avenger_text::types::FontWeightNameSpec::Normal,
                    ),
                    font_style: &avenger_text::types::FontStyle::Normal,
                };
                let b = measurer.measure_text_bounds(&cfg);
                max_label_height = max_label_height.max(b.height);
            }

            // Title font
            let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
            let title_color = theme.text_color(&title_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            // Center vertically, and place next to label column (gap = 6px)
            let gap = 10.0_f32;
            let title_family_owned2 = theme
                .font_family(&title_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());
            let cfg_title = avenger_text::measurement::TextMeasurementConfig {
                text: title_text,
                font: title_family_owned2.as_str(),
                font_size: title_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let b_title = measurer.measure_text_bounds(&cfg_title);
            let x_center = if place_on_left {
                // Place to the left of the label column: subtract label col width + gap + half title width
                plot_bounds.x - (max_label_height + gap + 0.5 * b_title.height)
            } else {
                // Place to the right of the label column
                plot_bounds.x + plot_width + max_label_height + gap + 0.5 * b_title.height
            };
            let y_center = plot_bounds.y + 0.5 * plot_height;
            let title_mark = avenger_scenegraph::marks::text::SceneTextMark {
                text: title_text.clone().into(),
                x: x_center.into(),
                y: y_center.into(),
                align: avenger_text::types::TextAlign::Center.into(),
                baseline: avenger_text::types::TextBaseline::Middle.into(),
                font: title_family_owned2.clone().into(),
                font_size: title_font_px.into(),
                angle: if place_on_left {
                    (-90.0_f32).into()
                } else {
                    90.0_f32.into()
                },
                color: avenger_common::types::ColorOrGradient::Color(title_color).into(),
                zindex: Some(6),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(title_mark)));
        }

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

impl FacetColGuide {
    fn get_unified_channel(&self) -> Option<String> {
        self.unifiable_channel.clone()
    }
}

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
        let col_scale = scales.get(ColDimensionConfig::channel_name()).ok_or_else(|| {
            crate::error::AvengerChartError::InternalError(
                format!("Missing '{}' scale for FacetColGuide", ColDimensionConfig::channel_name()).into(),
            )
        })?;

        // Extract discrete domain values
        let domain_vals = match col_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };

        let mut max_top: f32 = 0.0;
        let mut max_bottom: f32 = 0.0;
        let mut left: f32 = 0.0;
        let mut right: f32 = 0.0;

        let unified_channel = self.get_unified_channel();

        for source in &self.facet_sources {
            let col_expr = source
                .data
                .channels()
                .get(ColDimensionConfig::channel_name())
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        format!("Facet '{}' channel not found in guide", ColDimensionConfig::channel_name()).into(),
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
                let mut shared = false;
                for m in &source.subplot.marks {
                    if let Some(cv) = m.data_context().channels().get(ch) {
                        if let Some(true) = cv.get_share_across_facets() {
                            shared = true;
                            break;
                        }
                    }
                }
                scale_sharing_by_channel.insert(ch.to_string(), shared);
            }
            let any_shared = scale_sharing_by_channel.values().any(|v| *v);

            use crate::facet::subplot_iterator::SubplotIterator;
            let subplot_iter = SubplotIterator::<ColDimensionConfig>::new(
                domain_vals.clone(),
                unified_channel.clone(),
                params.clone(),
                scale_sharing_by_channel.clone(),
            );

            let subplot_count = subplot_iter.len();
            let band_w = plot_width / subplot_count as f32;

            // Build shared scales once if any channel is shared
            let df_src = if any_shared { df.clone() } else { df.clone() };

            for (idx, iteration) in subplot_iter.enumerate() {
                let filter_df = df
                    .clone()
                    .filter(col_expr.clone().eq(lit(iteration.facet_value.clone())))?;

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

                // Aggregate top/bottom across all columns
                max_top = max_top.max(overflow.top);
                max_bottom = max_bottom.max(overflow.bottom);

                // Track first and last column overflow for left/right
                if idx == 0 {
                    left = overflow.left;
                }
                if idx == subplot_count - 1 {
                    right = overflow.right;
                }
            }
        }

        // Determine label placement based on x-axis position
        // If axis_position returns None (explicit position expression), use overflow to infer
        let place_below = if let Some(source) = self.facet_sources.first() {
            if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                match guide.axis_position("x") {
                    Some(crate::cartesian::axis::AxisPosition::Top) => true,  // x at top → labels below
                    Some(crate::cartesian::axis::AxisPosition::Bottom) => false,  // x at bottom → labels above
                    None => {
                        // axis_position returns None when position is explicit expression
                        // Infer from overflow: if top > bottom, x-axis is likely at top
                        max_top > max_bottom
                    }
                    _ => false,  // fallback: labels above
                }
            } else {
                false  // no guide → assume bottom x-axis, labels above
            }
        } else {
            false  // no subplots → labels above
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
        let gap = if self.facet_title.is_some() { 10.0 } else { 0.0 };

        // Facet labels + gap + facet title (when title present)
        let facet_label_space = if self.facet_title.is_some() {
            max_label_height + gap + title_height + 1.0
        } else {
            max_label_height + 1.0
        };

        // Unified x-axis title space (with gap if present, matching FacetRowGuide)
        let gap_axis = if self.unified_x_title.is_some() { 6.0 } else { 0.0 };
        let x_axis_title_space = if self.unified_x_title.is_some() {
            gap_axis + unified_title_height + 1.0
        } else {
            0.0
        };

        if place_below {
            // X-axis at top, facet labels below
            // Unified x-axis title goes at top with x-axis
            max_top += x_axis_title_space;
            // Facet labels and title go at bottom
            max_bottom = facet_label_space;
        } else {
            // X-axis at bottom (default), facet labels above
            // Facet labels and title go at top
            max_top = facet_label_space;
            // Unified x-axis title goes at bottom with x-axis
            max_bottom += x_axis_title_space;
        }

        Ok(OverflowSpaceRequirement {
            top: max_top,
            bottom: max_bottom,
            left,
            right,
        })
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
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{TextAlign, TextBaseline};
        use crate::scales::ConfiguredScaleLegendExt;
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
        let unified_channel = self.get_unified_channel();

        // Measure subplot overflow to know where x-axis labels end
        if let Some(source) = self.facet_sources.first() {
            let col_expr = source
                .data
                .channels()
                .get(ColDimensionConfig::channel_name())
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        format!("Facet '{}' channel not found", ColDimensionConfig::channel_name()).into(),
                    )
                })?;

            let df = source.data.dataframe_with_context(ctx).ok_or_else(|| {
                crate::error::AvengerChartError::InternalError("Facet guide could not access data".into())
            })?;

            let coord_channels: Vec<&str> = source.subplot.coord_transform.required_channels().to_vec();
            let mut scale_sharing_by_channel = std::collections::HashMap::new();
            for &ch in &coord_channels {
                let mut shared = false;
                for m in &source.subplot.marks {
                    if let Some(cv) = m.data_context().channels().get(ch) {
                        if let Some(true) = cv.get_share_across_facets() {
                            shared = true;
                            break;
                        }
                    }
                }
                scale_sharing_by_channel.insert(ch.to_string(), shared);
            }
            let any_shared = scale_sharing_by_channel.values().any(|v| *v);

            use crate::facet::subplot_iterator::SubplotIterator;
            let subplot_iter = SubplotIterator::<ColDimensionConfig>::new(
                domain_vals.clone(),
                unified_channel.clone(),
                params.clone(),
                scale_sharing_by_channel.clone(),
            );

            let subplot_count = subplot_iter.len();
            let band_w = plot_width / subplot_count as f32;
            let df_src = if any_shared { df.clone() } else { df.clone() };

            for iteration in subplot_iter {
                let filter_df = df.clone().filter(
                    col_expr.clone().eq(datafusion::logical_expr::lit(iteration.facet_value.clone()))
                )?;

                let inner_scales = if any_shared {
                    source.subplot.build_scales_for_dataframe(&df_src, band_w, plot_height, ctx, params).await?
                } else {
                    source.subplot.build_scales_for_dataframe(&filter_df, band_w, plot_height, ctx, params).await?
                };

                let overflow = source.subplot.measure_guide_overflow_with_scales(
                    &inner_scales,
                    band_w,
                    plot_height,
                    ctx,
                    &iteration.params,
                ).await?;

                subplot_max_bottom = subplot_max_bottom.max(overflow.bottom);
                subplot_max_top = subplot_max_top.max(overflow.top);
            }
        }

        use crate::facet::band_positions::BandPositionIterator;
        let band_positions: Vec<_> = BandPositionIterator::from_configured_scale_with_band(col_scale, 0.5)?
            .collect();

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
                    Some(crate::cartesian::axis::AxisPosition::Top) => true,  // x at top → labels below
                    Some(crate::cartesian::axis::AxisPosition::Bottom) => false,  // x at bottom → labels above
                    None => {
                        // axis_position returns None when position is explicit expression
                        // Infer from overflow: if top > bottom, x-axis is likely at top
                        subplot_max_top > subplot_max_bottom
                    }
                    _ => false,  // fallback: labels above
                }
            } else {
                false  // no guide → assume bottom x-axis, labels above
            }
        } else {
            false  // no subplots → labels above
        };

        // Measure label heights to position from overflow boundary
        let measurer = avenger_text::measurement::default_text_measurer();
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

        // Position labels from the plot edge working into overflow region
        // Use baseline positioning: Top baseline when below, Bottom baseline when above
        let y_label = if place_below {
            plot_bounds.y + plot_height  // Start at bottom edge, baseline Top grows down
        } else {
            plot_bounds.y  // Start at top edge, baseline Bottom grows up
        };

        // Render labels horizontally centered at band centers
        for (label, band_pos) in labels.iter().zip(&band_positions) {
            let x_center = plot_bounds.x + band_pos.position;

            let label_mark = SceneTextMark {
                text: label.clone().into(),
                x: x_center.into(),
                y: y_label.into(),
                align: TextAlign::Center.into(),
                baseline: if place_below { TextBaseline::Top } else { TextBaseline::Bottom }.into(),
                angle: 0.0_f32.into(),
                font: label_font_family.to_string().into(),
                font_size: label_font_px.into(),
                color: avenger_common::types::ColorOrGradient::Color(
                    theme.text_color(&label_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]),
                )
                .into(),
                zindex: Some(5),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(label_mark)));
        }

        // Render horizontal rule between labels and title (if title present)
        if let Some(_title_text) = &self.facet_title {
            if !labels.is_empty() && labels.len() > 1 {
                // Get x positions of first and last labels (already centered at band positions)
                let x_left = plot_bounds.x + band_positions.first().map(|bp| bp.position).unwrap_or(0.0);
                let x_right = plot_bounds.x + band_positions.last().map(|bp| bp.position).unwrap_or(0.0);

                // Position rule between label row and title (gap/2)
                let gap = 10.0_f32;
                let y_rule = if place_below {
                    y_label + max_label_height + gap / 2.0
                } else {
                    y_label - max_label_height - gap / 2.0
                };

                // Query rule styling from theme
                let rule_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                    .child("facet")
                    .child("rule");
                let mut rule_stroke = theme.text_color(&rule_ctx).unwrap_or([0.5, 0.5, 0.5, 1.0]);
                // Ensure fully opaque to avoid overlapping darkening
                rule_stroke[3] = 1.0;
                let rule_stroke_width = theme
                    .query(&rule_ctx, "stroke-width")
                    .and_then(|v| v.as_number())
                    .map(|n| n as f32)
                    .unwrap_or(1.0);

                // Query tick size from theme
                let tick_size = theme
                    .query(&rule_ctx, "tick-size")
                    .and_then(|v| v.as_number())
                    .map(|n| n as f32)
                    .unwrap_or(4.0);

                // Extend rule by half stroke width on each end for cleaner edges
                let half_stroke = rule_stroke_width / 2.0;

                // Create horizontal rule mark
                let rule_mark = avenger_scenegraph::marks::rule::SceneRuleMark {
                    x: (x_left - half_stroke).into(),
                    y: y_rule.into(),
                    x2: (x_right + half_stroke).into(),
                    y2: y_rule.into(),
                    stroke: avenger_common::types::ColorOrGradient::Color(rule_stroke).into(),
                    stroke_width: rule_stroke_width.into(),
                    zindex: Some(5),
                    ..Default::default()
                };
                marks.push(SceneMark::Rule(rule_mark));

                // Add tick marks at each label position (band centers)
                // Ticks point toward labels (away from title)
                for band_pos in &band_positions {
                    let x_center = plot_bounds.x + band_pos.position;

                    let (y_tick_start, y_tick_end) = if place_below {
                        (y_rule - tick_size, y_rule)  // Ticks point down toward labels below
                    } else {
                        (y_rule, y_rule + tick_size)  // Ticks point up toward labels above
                    };

                    let tick_mark = avenger_scenegraph::marks::rule::SceneRuleMark {
                        x: x_center.into(),
                        y: y_tick_start.into(),
                        x2: x_center.into(),
                        y2: y_tick_end.into(),
                        stroke: avenger_common::types::ColorOrGradient::Color(rule_stroke).into(),
                        stroke_width: rule_stroke_width.into(),
                        zindex: Some(5),
                        ..Default::default()
                    };
                    marks.push(SceneMark::Rule(tick_mark));
                }
            }
        }

        // Render facet title (use facet title theme context matching RowFacet)
        if let Some(title) = &self.facet_title {
            let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
            let title_font_family_owned = theme
                .font_family(&title_ctx)
                .or_else(|| theme.font_family(&root_ctx))
                .unwrap_or_else(|| "sans-serif".to_string());
            let title_font_family = title_font_family_owned.as_str();

            // Configurable gap between labels and title (matching measure_overflow)
            let gap = 10.0_f32;

            // Position title after labels with gap
            let y_title = if place_below {
                y_label + max_label_height + gap
            } else {
                y_label - max_label_height - gap
            };

            let title_mark = SceneTextMark {
                text: title.clone().into(),
                x: (plot_bounds.x + plot_width / 2.0).into(),
                y: y_title.into(),
                align: TextAlign::Center.into(),
                baseline: if place_below { TextBaseline::Top } else { TextBaseline::Bottom }.into(),
                angle: 0.0_f32.into(),
                font: title_font_family.to_string().into(),
                font_size: title_font_px.into(),
                color: avenger_common::types::ColorOrGradient::Color(
                    theme.text_color(&title_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]),
                )
                .into(),
                zindex: Some(6),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(title_mark)));
        }

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
                baseline: if x_axis_at_top { TextBaseline::Bottom } else { TextBaseline::Top }.into(),
                angle: 0.0_f32.into(),
                font: unified_font_family.to_string().into(),
                font_size: unified_font_px.into(),
                color: avenger_common::types::ColorOrGradient::Color(
                    theme.text_color(&unified_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]),
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
