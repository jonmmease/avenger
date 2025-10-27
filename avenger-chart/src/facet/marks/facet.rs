use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::facet::coord::FacetRow;
use crate::facet::marks::facet_config::FacetRowChannelConfig;
use crate::marks::{
    ChannelDescriptor, ChannelValue, CompiledMark, CompiledMarkState, Mark, MarkState,
};
use crate::plot::{CompiledPlot, Plot};
use crate::render::RenderContext;
use crate::scales::ConfiguredScaleWithSpec;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::lit;
use datafusion::prelude::SessionContext;
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Facet mark for FacetRow outer coordinate system.
/// Renders a provided inner plot for each band value in the `row` channel.
#[derive(Clone)]
pub struct Facet<InnerC: CoordinateSystem> {
    pub(crate) state: MarkState,
    pub(crate) subplot: Option<Plot<InnerC>>,
    pub(crate) facet_row_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
}

impl<InnerC: CoordinateSystem> Facet<InnerC> {
    pub fn new() -> Self {
        Self {
            state: MarkState {
                data: crate::marks::DataContext::default(),
                facet_strategy: crate::marks::FacetStrategy::Filter,
                details: None,
                zindex: None,
                axis_configs: HashMap::new(),
            },
            subplot: None,
            facet_row_title: None,
            facet_spacing: None,
        }
    }

    /// Set explicit data for this mark
    pub fn data(mut self, dataframe: datafusion::dataframe::DataFrame) -> Self {
        self.state.data = crate::marks::DataContext::new(dataframe);
        self
    }

    /// Set zindex
    pub fn zindex(mut self, zindex: i32) -> Self {
        self.state.zindex = Some(zindex);
        self
    }

    /// Set the faceting channel for rows
    pub fn row<V: Into<ChannelValue>>(self, value: V) -> Self {
        let mut s = self;
        s.state.data = s.state.data.with_channel_value("row", value.into());
        s
    }

    /// Configure row with facet options (e.g., title, spacing)
    pub fn row_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetRowChannelConfig) -> FacetRowChannelConfig,
    {
        let mut s = self.row(value);
        let cfg = f(FacetRowChannelConfig::default());
        s.facet_row_title = cfg.title;
        s.facet_spacing = cfg.spacing;
        s
    }

    /// Provide a subplot Plot<InnerC>
    pub fn subplot(mut self, plot: Plot<InnerC>) -> Self {
        self.subplot = Some(plot);
        self
    }
}

/// Compiled facet mark specialized for FacetRow outer coords
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetRow {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
}

#[async_trait::async_trait]
impl<InnerC: CoordinateSystem + Clone> Mark<FacetRow> for Facet<InnerC> {
    fn state(&self) -> &MarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    fn data_context(&self) -> &crate::marks::DataContext {
        &self.state.data
    }

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        let compiled_subplot: Arc<CompiledPlot> = {
            let plot_ref = self
                .subplot
                .as_ref()
                .ok_or_else(|| AvengerChartError::InternalError("Facet subplot not set".into()))?;
            let plot_clone = plot_ref.clone();
            // Plot<InnerC> implements Clone; explicitly call Clone::clone
            let plot_owned: Plot<InnerC> = Clone::clone(&plot_clone);
            Arc::new(plot_owned.compile(session_context).await?)
        };
        Ok(Arc::new(CompiledFacetRow {
            state: compiled_state,
            compiled_subplot,
            facet_title: self.facet_row_title.clone(),
            facet_spacing: self.facet_spacing,
        }))
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledFacetRow {
    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &crate::marks::CompiledDataContext {
        &self.state.data
    }

    fn mark_type(&self) -> &str {
        "facet_row"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![ChannelDescriptor {
            name: "row",
            required: true,
            default_value: None,
            allow_column_ref: true,
        }]
    }

    async fn evaluate_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<(Vec<SceneMark>, Box<dyn crate::layout::LayoutInfo>), AvengerChartError> {
        // Get row scale
        let row_scale = context.scales.get("row").ok_or_else(|| {
            AvengerChartError::InternalError("Missing 'row' scale for FacetRow".into())
        })?;

        // Build initial (facet_value -> (y_pos, band_height)) map with current scale
        let initial_band_positions = iter_band_positions(row_scale)?;

        // Get the inner plot-level DataFrame
        let ctx = &context.session_context;
        let df = self.state.data.dataframe_with_context(ctx).ok_or_else(|| {
            AvengerChartError::InternalError("Facet mark requires plot or mark data".into())
        })?;

        // Extract the raw expression for the row channel to filter by facet value
        let row_expr = self
            .state
            .data
            .channels()
            .get("row")
            .and_then(|cv| cv.expr(ctx))
            .ok_or_else(|| {
                AvengerChartError::InternalError("Facet 'row' channel not found".into())
            })?;

        let mut all_marks: Vec<SceneMark> = Vec::new();

        // Compute per-channel sharing preferences by scanning inner marks
        let required_channels: Vec<&str> = self
            .compiled_subplot
            .coord_transform
            .required_channels()
            .to_vec();
        let mut channel_shared: HashMap<String, bool> = HashMap::new();
        for &ch in &required_channels {
            let mut shared = false;
            for m in &self.compiled_subplot.marks {
                if let Some(cv) = m.data_context().channels().get(ch) {
                    if let Some(s) = cv.get_share_across_facets() {
                        if s {
                            shared = true;
                            break;
                        }
                    }
                }
            }
            channel_shared.insert(ch.to_string(), shared);
        }

        // If any channel is shared, compute scales once across full data using approximate band height
        let any_shared = channel_shared.values().any(|v| *v);
        let approx_h = initial_band_positions
            .first()
            .map(|(_, (_, h))| *h)
            .unwrap_or(context.plot_height);
        let shared_scales = if any_shared {
            Some(
                self.compiled_subplot
                    .build_scales_for_dataframe(
                        &df,
                        context.plot_width,
                        approx_h,
                        ctx,
                        &context.params,
                    )
                    .await?,
            )
        } else {
            None
        };

        // PASS 1: Measure overflow for each facet partition to determine required spacing
        // Determine which channel is unified (needed for accurate measurement)
        let unified_channel = if let Some(guide) = self.compiled_subplot.compiled_guide.as_ref() {
            guide
                .facet_unifiable_channel(
                    crate::guide::FacetDirection::Row,
                    self.compiled_subplot.marks(),
                    ctx,
                )
                .map(|info| info.channel)
        } else {
            None
        };

        let total_rows = initial_band_positions.len();
        let mut overflow_measurements = Vec::new();

        for (row_idx, (facet_value, (_y_pos, band_height))) in initial_band_positions.iter().enumerate() {
            // Filter df by facet_value
            let filter_df: DataFrame = df
                .clone()
                .filter(row_expr.clone().eq(lit(facet_value.clone())))?;

            // Build scales for this partition (use shared scales if available)
            let scales = if let Some(ref shared) = shared_scales {
                shared.clone()
            } else {
                self.compiled_subplot
                    .build_scales_for_dataframe(
                        &filter_df,
                        context.plot_width,
                        *band_height,
                        ctx,
                        &context.params,
                    )
                    .await?
            };

            // Create FacetContext for accurate measurement (so hidden titles don't contribute)
            use crate::facet::context::FacetContext;
            let measure_facet_ctx = FacetContext {
                position: (row_idx, 0),
                grid_dimensions: (total_rows, 1),
                unified_channel: unified_channel.clone(),
                scale_sharing: channel_shared.clone(),
            };

            // Merge FacetContext into params for measurement
            let mut measure_params = context.params.clone();
            measure_params.extend(measure_facet_ctx.to_params());

            // Measure guide overflow for this partition with FacetContext applied
            let overflow = self
                .compiled_subplot
                .measure_guide_overflow_with_scales(
                    &scales,
                    context.plot_width,
                    *band_height,
                    ctx,
                    &measure_params,
                )
                .await?;

            overflow_measurements.push(overflow);
        }

        // Calculate required padding based on adjacent overflow measurements
        let mut max_required_gap = 0.0f32;
        for i in 0..overflow_measurements.len().saturating_sub(1) {
            let gap = overflow_measurements[i].bottom + overflow_measurements[i + 1].top;
            max_required_gap = max_required_gap.max(gap);
        }

        // Add spacing from facet configuration or theme
        // Priority: 1) facet_spacing field, 2) theme 'facet { spacing }', 3) default 3.0
        let spacing = if let Some(explicit_spacing) = self.facet_spacing {
            explicit_spacing
        } else {
            let facet_ctx = context.theme.facet_context_with_params(context.params.clone());
            context
                .theme
                .query(&facet_ctx, "spacing")
                .and_then(|v| v.as_number())
                .map(|n| n as f32)
                .unwrap_or(3.0)
        };
        max_required_gap += spacing;

        // Rebuild the row scale with measured padding_inner_px
        let mut updated_scales = HashMap::new();
        if max_required_gap > 0.0 {
            use avenger_scales::scalar::Scalar;

            // Clone the existing row scale config and modify padding_inner_px option
            let mut new_config = row_scale.configured().config.clone();
            new_config.options.insert(
                "padding_inner_px".to_string(),
                Scalar::from_f32(max_required_gap),
            );

            // Create a new ConfiguredScale with the modified config
            let new_configured = avenger_scales::scales::ConfiguredScale {
                scale_impl: row_scale.configured().scale_impl.clone(),
                config: new_config,
            };

            // Wrap in ConfiguredScaleWithSpec
            updated_scales.insert(
                "row".to_string(),
                ConfiguredScaleWithSpec::new(row_scale.spec().clone(), new_configured),
            );
        }

        // Create a merged scales map for pass 2 rendering
        let mut merged_scales_for_rendering = context.scales.clone();
        merged_scales_for_rendering.extend(updated_scales.clone());

        // PASS 2: Render with updated scales
        // Get updated band positions from rebuilt scale
        let final_row_scale = merged_scales_for_rendering.get("row").ok_or_else(|| {
            AvengerChartError::InternalError("Missing rebuilt 'row' scale".into())
        })?;
        let band_positions = iter_band_positions(final_row_scale)?;

        // CRITICAL: Rebuild shared scales with the NEW band height after padding adjustment
        // The initial shared_scales were built with the approximate height BEFORE padding,
        // which causes incorrect data scaling (x-axis doesn't align with y=0)
        let shared_scales = if any_shared {
            let new_band_height = band_positions
                .first()
                .map(|(_, (_, h))| *h)
                .unwrap_or(context.plot_height);

            Some(
                self.compiled_subplot
                    .build_scales_for_dataframe(
                        &df,
                        context.plot_width,
                        new_band_height,
                        ctx,
                        &context.params,
                    )
                    .await?,
            )
        } else {
            shared_scales
        };

        // Get total number of rows for grid dimensions
        let total_rows = band_positions.len();

        for (row_idx, (facet_value, (y_pos, band_height))) in band_positions.into_iter().enumerate()
        {
            // Filter df by facet_value
            let filter_df: DataFrame = df
                .clone()
                .filter(row_expr.clone().eq(lit(facet_value.clone())))?;

            // Build scales and evaluate inner components for this partition
            let mut scales = if let Some(ref shared) = shared_scales {
                shared.clone()
            } else {
                self.compiled_subplot
                    .build_scales_for_dataframe(
                        &filter_df,
                        context.plot_width,
                        band_height,
                        ctx,
                        &context.params,
                    )
                    .await?
            };

            // If some channels are free, rebuild facet-specific scales and override those channels
            if any_shared {
                let facet_scales = self
                    .compiled_subplot
                    .build_scales_for_dataframe(
                        &filter_df,
                        context.plot_width,
                        band_height,
                        ctx,
                        &context.params,
                    )
                    .await?;
                for (ch, shared_flag) in &channel_shared {
                    if !*shared_flag {
                        if let Some(s) = facet_scales.get(ch) {
                            scales.insert(ch.clone(), s.clone());
                        }
                    }
                }
            }

            // Create FacetContext with position and sharing information
            use crate::facet::context::FacetContext;

            // Build FacetContext (unified_channel already computed in Pass 1)
            let facet_ctx = FacetContext {
                position: (row_idx, 0),           // col always 0 for row faceting
                grid_dimensions: (total_rows, 1), // num_cols always 1 for row faceting
                unified_channel: unified_channel.clone(),
                scale_sharing: channel_shared.clone(),
            };

            // Merge context into params
            let mut facet_params = context.params.clone();
            facet_params.extend(facet_ctx.to_params());

            let sub = self
                .compiled_subplot
                .evaluate_components_with_scales(
                    &filter_df,
                    &scales,
                    context.plot_width,
                    band_height,
                    ctx,
                    &facet_params,
                )
                .await?;

            // Wrap data marks in a clipped group translated to band position
            let data_group = SceneGroup {
                origin: [0.0, y_pos],
                marks: sub.data_marks,
                clip: sub.clip,
                zindex: Some(0),
                ..Default::default()
            };
            all_marks.push(SceneMark::Group(data_group));

            // Wrap guide marks (axes) in a non-clipped translated group
            if !sub.guide_marks.is_empty() {
                let guide_group = SceneGroup {
                    origin: [0.0, y_pos],
                    marks: sub.guide_marks,
                    clip: avenger_scenegraph::marks::group::Clip::None,
                    zindex: Some(1),
                    ..Default::default()
                };
                all_marks.push(SceneMark::Group(guide_group));
            }
        }

        Ok((
            all_marks,
            Box::new(crate::layout::ScaleUpdates::new(updated_scales)),
        ))
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Box<dyn crate::scales::ScaleSpec>> {
        if channel == "row" {
            // Use band scale for row faceting regardless of domain type (categorical input expected)
            Some(Box::new(crate::scales::spec::Band::default()))
        } else {
            crate::marks::default_scale_for_data_type(data_type)
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
        _data_type: &datafusion::arrow::datatypes::DataType,
    ) -> std::collections::HashMap<String, datafusion::logical_expr::Expr> {
        use datafusion::logical_expr::lit;
        use std::collections::HashMap;
        let mut options = HashMap::new();

        // Configure band scale padding for facet row channel
        if channel == "row" && scale_impl.scale_type() == "band" {
            options.insert("padding_outer".to_string(), lit(0.0f32));
            // Initial padding_inner_px of 0 - will be dynamically measured and rebuilt
            // during evaluate_from_data based on actual subplot overflow
            options.insert("padding_inner_px".to_string(), lit(0.0f32));
        }

        options
    }
}

/// Helper: iterate band positions for a configured band scale
fn iter_band_positions(
    scale: &ConfiguredScaleWithSpec,
) -> Result<Vec<(ScalarValue, (f32, f32))>, AvengerChartError> {
    use crate::scales::ConfiguredScaleLegendExt;
    use avenger_scales::scales::band;

    let configured = scale.configured();
    let domain_vals = configured.domain_values()?;
    let positions = match domain_vals {
        crate::scales::extensions::DomainValues::Discrete(vals) => {
            configured.scale_scalars_to_numeric(&vals)?
        }
        _ => Vec::new(),
    };
    let bandwidth = band::bandwidth(&configured.config)?;

    let mut out = Vec::new();
    if let crate::scales::extensions::DomainValues::Discrete(vals) = configured.domain_values()? {
        for (i, v) in vals.into_iter().enumerate() {
            let pos = positions.get(i).cloned().unwrap_or(0.0);
            out.push((v, (pos, bandwidth)));
        }
    }
    Ok(out)
}
