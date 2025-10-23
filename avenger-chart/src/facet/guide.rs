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
    #[serde(skip)]
    facet_sources: Vec<FacetSource>,
    /// Optional facet title rendered above the label column
    pub facet_title: Option<String>,
}

#[derive(Clone)]
struct FacetSource {
    subplot: std::sync::Arc<crate::plot::CompiledPlot>,
    data: crate::marks::CompiledDataContext,
    user_title: Option<String>,
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
            if let Some(facet) = m.as_any().downcast_ref::<crate::facet::marks::facet::CompiledFacetRow>() {
                self.facet_sources.push(FacetSource {
                    subplot: facet.compiled_subplot.clone(),
                    data: facet.state.data.clone(),
                    user_title: facet.facet_title.clone(),
                });
            }
        }
        // Derive default title if not explicitly set on any facet mark
        if self.facet_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                // Try to extract a column name from 'row' channel
                if let Some(cv) = src.data.channels().get("row") {
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

        // Need 'row' scale
        let row_scale = scales
            .get("row")
            .ok_or_else(|| crate::error::AvengerChartError::InternalError("Missing 'row' scale for FacetRowGuide".into()))?;

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
            // Row expression from channels
            let row_expr = source
                .data
                .channels()
                .get("row")
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| crate::error::AvengerChartError::InternalError("Facet 'row' channel not found in guide".into()))?;

            // DataFrame for this facet source
            let df = source
                .data
                .dataframe_with_context(ctx)
                .ok_or_else(|| crate::error::AvengerChartError::InternalError("Facet guide could not access data".into()))?;

            for (i, facet_val) in domain_vals.iter().enumerate() {
                let filter_df = df.clone().filter(row_expr.clone().eq(lit(facet_val.clone())))?;
                // Build inner scales and measure inner guide overflow for this band height
                // Determine per-channel sharing for this subplot
                let coord_channels: Vec<&str> = source.subplot.coord_transform.required_channels().to_vec();
                let mut any_shared = false;
                for &ch in &coord_channels {
                    for m in &source.subplot.marks {
                        if let Some(cv) = m.data_context().channels().get(ch) {
                            if let Some(true) = cv.get_share_across_facets() {
                                any_shared = true;
                                break;
                            }
                        }
                    }
                    if any_shared { break; }
                }
                let band_h = plot_height / domain_vals.len() as f32;
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
                let overflow = source
                    .subplot
                    .measure_guide_overflow_with_scales(&inner_scales, plot_width, band_h, ctx, params)
                    .await?;

                if i == 0 {
                    top = top.max(overflow.top);
                }
                if i == domain_vals.len() - 1 {
                    bottom = bottom.max(overflow.bottom);
                }
                max_left = max_left.max(overflow.left);
                max_right = max_right.max(overflow.right);
            }
        }
        // Add space for facet labels on the right by measuring text bounds
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
        // If there's a facet title, measure it (used for RHS placement and overflow)
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

        // Compute right overflow: include label column + spacing + title (rotated width)
        // For center-anchored labels, horizontal extent beyond plot edge is 0.5 * label_height.
        // But the furthest right edge equals label_height. Since we place title to the right of
        // labels with a small gap, total RHS = label_height + gap + title_height.
        let gap = if self.facet_title.is_some() { 6.0 } else { 0.0 };
        let estimated_right = if self.facet_title.is_some() {
            max_label_height + gap + title_height + 1.0
        } else {
            // No title: just the label column (use full label_height for edge safety)
            max_label_height + 1.0
        };

        Ok(OverflowSpaceRequirement {
            top,
            bottom,
            left: max_left,
            right: max_right.max(estimated_right),
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
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{TextAlign, TextBaseline};
        use std::sync::Arc as StdArc;
        use crate::scales::ConfiguredScaleLegendExt;
        use avenger_scales::scales::band;

        let mut marks: Vec<SceneMark> = Vec::new();

        // Row scale
        let row_scale = match scales.get("row") {
            Some(s) => s,
            None => return Ok(marks),
        };

        // Domain labels and numeric positions
        let labels = row_scale.domain_labels()?;
        let positions = match row_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => row_scale.scale_scalars_to_numeric(&vals)?,
            _ => Vec::new(),
        };
        let bandwidth = band::bandwidth(&row_scale.config)?;

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

        // Place facet labels on the right at band centers, rotated 90 CW
        // Anchor at the text center so after rotation it's vertically centered.
        for (i, label) in labels.iter().enumerate() {
            let y_center = plot_bounds.y + positions.get(i).cloned().unwrap_or(0.0) + bandwidth / 2.0;
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
            let bounds = avenger_text::measurement::default_text_measurer().measure_text_bounds(&config);
            // With 90° CW rotation, horizontal span ≈ bounds.height; set center so left edge touches plot edge
            let x_center = plot_bounds.x + plot_width + 0.5 * bounds.height;
            let text = SceneTextMark {
                text: label.clone().into(),
                x: x_center.into(),
                y: y_center.into(),
                align: TextAlign::Center.into(),
                baseline: TextBaseline::Middle.into(),
                angle: 90.0_f32.into(),
                font_size: font_px.into(),
                color: avenger_common::types::ColorOrGradient::Color(theme.text_color(&guide_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0])).into(),
                zindex: Some(5),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(text)));
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
            let title_color = theme
                .text_color(&title_ctx)
                .unwrap_or([0.0, 0.0, 0.0, 1.0]);
            // Center vertically, and place to the right of label column (gap = 6px)
            let gap = 6.0_f32;
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
            let x_center = plot_bounds.x + plot_width + max_label_height + gap + 0.5 * b_title.height;
            let y_center = plot_bounds.y + 0.5 * plot_height;
            let title_mark = avenger_scenegraph::marks::text::SceneTextMark {
                text: title_text.clone().into(),
                x: x_center.into(),
                y: y_center.into(),
                align: avenger_text::types::TextAlign::Center.into(),
                baseline: avenger_text::types::TextBaseline::Middle.into(),
                font_size: title_font_px.into(),
                angle: 90.0_f32.into(),
                color: avenger_common::types::ColorOrGradient::Color(title_color).into(),
                zindex: Some(6),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(title_mark)));
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
