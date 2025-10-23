use crate::error::AvengerChartError;
use crate::facet::coord::FacetRow;
use crate::marks::{ChannelDescriptor, ChannelValue, CompiledMark, CompiledMarkState, Mark, MarkState};
use crate::plot::CompiledPlot;
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
pub struct Facet {
    pub(crate) state: MarkState,
    // Serialized CompiledPlot to avoid Send/Sync issues in Mark
    pub(crate) compiled_subplot_json: Option<String>,
}

impl Facet {
    pub fn new() -> Self {
        Self {
            state: MarkState {
                data: crate::marks::DataContext::default(),
                facet_strategy: crate::marks::FacetStrategy::Filter,
                details: None,
                zindex: None,
                axis_configs: HashMap::new(),
            },
            compiled_subplot_json: None,
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

    /// Provide a compiled subplot (serialized as JSON)
    pub fn subplot_compiled(mut self, compiled: &CompiledPlot) -> Self {
        let json = serde_json::to_string(compiled).expect("serialize compiled subplot");
        self.compiled_subplot_json = Some(json);
        self
    }
}

/// Compiled facet mark specialized for FacetRow outer coords
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetRow {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
}

#[async_trait::async_trait]
impl Mark<FacetRow> for Facet {
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
        let compiled_subplot: CompiledPlot = self
            .compiled_subplot_json
            .as_ref()
            .ok_or_else(|| AvengerChartError::InternalError("Facet compiled subplot not set".into()))
            .and_then(|s| serde_json::from_str(s).map_err(|e| AvengerChartError::InternalError(format!("Failed to deserialize subplot: {}", e))))?;
        Ok(Arc::new(CompiledFacetRow {
            state: compiled_state,
            compiled_subplot: Arc::new(compiled_subplot),
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
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get row scale
        let row_scale = context
            .scales
            .get("row")
            .ok_or_else(|| AvengerChartError::InternalError("Missing 'row' scale for FacetRow".into()))?;

        // Build (facet_value -> (y_pos, band_height)) map
        let band_positions = iter_band_positions(row_scale)?;

        // Get the inner plot-level DataFrame
        let ctx = &context.session_context;
        let df = self
            .state
            .data
            .dataframe_with_context(ctx)
            .ok_or_else(|| AvengerChartError::InternalError("Facet mark requires plot or mark data".into()))?;

        // Extract the raw expression for the row channel to filter by facet value
        let row_expr = self
            .state
            .data
            .channels()
            .get("row")
            .and_then(|cv| cv.expr(ctx))
            .ok_or_else(|| AvengerChartError::InternalError("Facet 'row' channel not found".into()))?;

        let mut all_marks: Vec<SceneMark> = Vec::new();

        for (facet_value, (y_pos, band_height)) in band_positions {
            // Filter df by facet_value
            let filter_df: DataFrame = df
                .clone()
                .filter(row_expr.clone().eq(lit(facet_value.clone())))?;

            // Build scales and evaluate inner components for this partition
            let scales = self.compiled_subplot.build_scales_for_dataframe(
                &filter_df,
                context.plot_width,
                band_height,
                ctx,
                &context.params,
            ).await?;

            let sub = self.compiled_subplot.evaluate_components_with_scales(
                &filter_df,
                &scales,
                context.plot_width,
                band_height,
                ctx,
                &context.params,
            ).await?;

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

        Ok(all_marks)
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
            options.insert("outer_padding".to_string(), lit(0.0f32));
            options.insert("inner_padding".to_string(), lit(0.1f32));
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
        crate::scales::extensions::DomainValues::Discrete(vals) => configured.scale_scalars_to_numeric(&vals)?,
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
