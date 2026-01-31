use crate::coords::{
    CoordMeasurement, CoordinateSystem, CoordinateSystemTransform, CoordinatedOverflow,
    OverflowSpaceRequirement,
};
use crate::error::AvengerChartError;
use crate::facet::guide::{FacetColGuideConfig, FacetRowGuideConfig};
use crate::facet::marks::facet::CompiledFacetCol;
use crate::layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode};
use crate::marks::CompiledMark;
use crate::plot::compiled::{CompiledPlot, ComponentsMeasurement};
use crate::render::EvaluationContext;
use crate::scales::ConfiguredScaleWithSpec;
use avenger_common::value::ScalarOrArray;
use datafusion::common::ScalarValue;
use datafusion::dataframe::DataFrame;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// Measurement data for FacetColumn coordinate system.
///
/// This captures the computed padding from overflow measurement and pre-computed
/// subplot measurements. Marks use the stored measurements directly without re-measuring.
pub struct FacetColCoordMeasurement {
    /// Facet column values (one per cell)
    pub cell_values: Vec<ScalarValue>,
    /// Computed padding between cells in pixels (MAX of adjacent overflow combinations)
    pub padding_inner_px: f32,
    /// Outer left total_overflow (first cell's left edge) - used to adjust scale range
    pub outer_left: f32,
    /// Outer right total_overflow (last cell's right edge) - used to adjust scale range
    pub outer_right: f32,
    /// Filtered DataFrames for each cell
    pub data_overrides: Vec<DataFrame>,
    /// Pre-built shared scales for subplots (domains from full data).
    /// Ranges may need updating for final subplot dimensions.
    pub shared_scales: HashMap<String, ConfiguredScaleWithSpec>,
    /// Parent facet path (for nested facets). This is the path to reach this facet level.
    /// When constructing cell paths for nested subplots, prepend this to the cell value.
    pub parent_path: Vec<ScalarValue>,
    /// Pre-computed subplot measurements (computed with final subplot width after padding_inner_px).
    /// These are used directly by render_from_data to avoid re-measuring.
    pub subplot_measurements: Vec<ComponentsMeasurement>,
    /// Coordinated overflow values aggregated across ALL facets at this nesting level.
    /// Populated by `coordinate_overflow_for_guides()` after measurement.
    pub coordinated_overflow: CoordinatedOverflow,
    /// Reference to compiled subplot for re-measurement after coordination.
    /// Used by `apply_coordinated_overflow` to re-measure with adjusted height.
    pub compiled_subplot: Arc<CompiledPlot>,
    /// Subplot width (bandwidth) for re-measurement.
    pub subplot_width: f32,
}

#[async_trait::async_trait]
impl CoordMeasurement for FacetColCoordMeasurement {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn child_measurements(&self) -> &[ComponentsMeasurement] {
        &self.subplot_measurements
    }

    fn child_measurements_mut(&mut self) -> &mut [ComponentsMeasurement] {
        &mut self.subplot_measurements
    }

    fn local_overflow(&self) -> Option<CoordinatedOverflow> {
        // Compute local overflow from children's overflow (guide-only and total)
        let guide = OverflowSpaceRequirement {
            top: self
                .subplot_measurements
                .iter()
                .map(|m| m.layout.overflow.top)
                .fold(0.0f32, f32::max),
            bottom: self
                .subplot_measurements
                .iter()
                .map(|m| m.layout.overflow.bottom)
                .fold(0.0f32, f32::max),
            left: 0.0,
            right: 0.0,
        };

        let total = OverflowSpaceRequirement {
            top: self
                .subplot_measurements
                .iter()
                .map(|m| m.layout.total_overflow.top)
                .fold(0.0f32, f32::max),
            bottom: self
                .subplot_measurements
                .iter()
                .map(|m| m.layout.total_overflow.bottom)
                .fold(0.0f32, f32::max),
            left: 0.0,
            right: 0.0,
        };

        Some(CoordinatedOverflow { guide, total })
    }

    fn coordinated_overflow(&self) -> Option<&CoordinatedOverflow> {
        Some(&self.coordinated_overflow)
    }

    fn set_coordinated_overflow(&mut self, overflow: CoordinatedOverflow) {
        self.coordinated_overflow = overflow;
    }

    fn apply_scale_adjustments(&self, scales: &mut HashMap<String, ConfiguredScaleWithSpec>) {
        // Apply padding_inner_px and outer edge adjustments to the column scale.
        // The outer_left/outer_right values represent legend space at the outer edges
        // of the first/last cells. We reduce the scale range width to account for this space.
        //
        // The total available width is reduced by BOTH outer_left and outer_right.
        // The scale start remains at 0 (or the original start), but the end is reduced
        // so that cells fit within the remaining space after legends are accounted for.
        if let Some(column_scale) = scales.get_mut("column") {
            let mut updated_config = column_scale.configured().clone();

            // Apply padding_inner_px for cell spacing
            if self.padding_inner_px > 0.0 {
                updated_config = updated_config.with_option("padding_inner_px", self.padding_inner_px);
            }

            // Reduce scale range to account for outer legend space
            if self.outer_left > 0.0 || self.outer_right > 0.0 {
                if let Ok((range_start, range_end)) = updated_config.config.numeric_interval_range() {
                    // Keep start unchanged, reduce end by both outer edges
                    // This shrinks the available width for cells while keeping them
                    // starting at position 0 within the plot area
                    let new_end = range_end - self.outer_left - self.outer_right;
                    updated_config = updated_config.with_range_interval((range_start, new_end));
                }
            }

            *column_scale = ConfiguredScaleWithSpec::new(column_scale.spec().clone(), updated_config);
        }
    }

    fn update_child_dimensions(&mut self, new_height: f32) {
        // Update subplot measurements to use the correct height from second-pass layout.
        // This is needed when legends (or other elements) change the available plot area
        // after the initial measurement pass.
        for measurement in &mut self.subplot_measurements {
            if (measurement.plot_area_height - new_height).abs() > 0.1 {
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetCol update_child_dimensions: height {:.1} -> {:.1}",
                        measurement.plot_area_height, new_height
                    );
                }
                measurement.plot_area_height = new_height;
            }
        }
    }

    async fn apply_coordinated_overflow(
        &mut self,
        eval_ctx: &EvaluationContext,
    ) -> Result<(), crate::error::AvengerChartError> {
        // Compute legend-adjusted height from coordinated overflow
        let coordinated = &self.coordinated_overflow;
        let legend_top = (coordinated.total.top - coordinated.guide.top).max(0.0);
        let legend_bottom = (coordinated.total.bottom - coordinated.guide.bottom).max(0.0);

        // Only re-measure if there's legend overflow affecting height
        if legend_top <= 0.0 && legend_bottom <= 0.0 {
            return Ok(());
        }

        // Get original height from first measurement (all should be same)
        let original_height = self
            .subplot_measurements
            .first()
            .map(|m| m.plot_area_height)
            .unwrap_or(0.0);

        let adjusted_height = (original_height - legend_top - legend_bottom).max(1.0);

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetCol apply_coordinated_overflow: height {:.1} -> {:.1} (legend_top={:.1}, legend_bottom={:.1})",
                original_height, adjusted_height, legend_top, legend_bottom
            );
        }

        // Re-measure each subplot with the corrected height
        use crate::plot::compiled::scale_provider::PrebuiltScaleProvider;
        let scale_provider = PrebuiltScaleProvider {
            scales: self.shared_scales.clone(),
        };

        // Create subplot EvaluationContext with merged params
        let subplot_eval_ctx = {
            let mut params = self.compiled_subplot.get_default_params().clone();
            params.extend(eval_ctx.params.clone());
            eval_ctx.with_params(params)
        };

        let mut new_measurements = Vec::with_capacity(self.subplot_measurements.len());

        for (idx, (value, data_override)) in self
            .cell_values
            .iter()
            .zip(self.data_overrides.iter())
            .enumerate()
        {
            // Build full cell path from parent_path + current cell value
            let mut cell_path: Vec<ScalarValue> = self.parent_path.clone();
            cell_path.push(value.clone());

            // Build layout spec with fixed plot area (subplot dimensions)
            let subplot_layout_spec = EvaluatedLayoutSpec {
                canvas: EvaluatedSizeMode::Auto,
                plot_area: EvaluatedSizeMode::Fixed {
                    width: self.subplot_width,
                    height: adjusted_height,
                },
                margins: EvaluatedMargins {
                    top: 0.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                },
            };

            let measurement = self
                .compiled_subplot
                .measure_plot_components(
                    &subplot_eval_ctx,
                    &subplot_layout_spec,
                    &scale_provider,
                    Some(data_override),
                    &cell_path,
                )
                .await?;

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FacetCol apply_coordinated_overflow: re-measured cell[{}]={:?} at height={:.1}",
                    idx, value, adjusted_height
                );
            }

            new_measurements.push(measurement);
        }

        // Replace measurements with re-measured ones
        self.subplot_measurements = new_measurements;

        Ok(())
    }
}

/// Compute padding_inner_px from the MAX of all adjacent overflow combinations.
///
/// For a FacetCol layout, the gap between cell[i] and cell[i+1] must accommodate:
/// - cell[i].right overflow (typically tick-only for interior, full for last)
/// - cell[i+1].left overflow (typically full for first, tick-only for interior)
///
/// We compute the MAX across all pairs to ensure uniform spacing.
fn compute_padding_from_overflows(overflows: &[OverflowSpaceRequirement]) -> f32 {
    if overflows.len() < 2 {
        return 0.0;
    }

    let mut max_padding = 0.0f32;
    for i in 0..overflows.len() - 1 {
        let combined = overflows[i].right + overflows[i + 1].left;
        max_padding = max_padding.max(combined);
    }
    max_padding
}

/// Row faceting coordinate system
///
/// Facets data along the row dimension, creating a vertical stack of subplots.
/// Each subplot represents one unique value from the `row` channel.
///
/// # Example
/// ```ignore
/// let plot = Plot::<FacetRow>::new()
///     .data(df)
///     .mark(
///         Facet::new()
///             .row(col("species"))
///             .subplot(
///                 Plot::<Cartesian>::new().mark(Symbol::new()...),
///             ),
///     );
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FacetRow {
    pub(crate) padding_px: Option<f32>,
    pub(crate) overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
}

impl CoordinateSystem for FacetRow {
    type Guide = FacetRowGuideConfig;

    fn required_channels(&self) -> &'static [&'static str] {
        &["row"]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

fn compute_band_layout(positions: &[f32], extent: f32, padding_px: Option<f32>) -> (Vec<f32>, f32) {
    if positions.is_empty() {
        return (Vec::new(), 0.0);
    }

    let mut sorted = positions.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));

    let mut base_bandwidth = if sorted.len() > 1 {
        sorted
            .windows(2)
            .filter_map(|pair| {
                let gap = (pair[1] - pair[0]).abs();
                if gap.is_finite() && gap > 0.0 {
                    Some(gap)
                } else {
                    None
                }
            })
            .fold(f32::INFINITY, f32::min)
    } else {
        extent
    };

    if !base_bandwidth.is_finite() || base_bandwidth <= 0.0 {
        base_bandwidth = extent;
    }

    let effective_bandwidth = (base_bandwidth - padding_px.unwrap_or(0.0)).max(0.0);

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "compute_band_layout: positions={:?} base_bandwidth={:.3} padding_px={:?} effective_bandwidth={:.3}",
            positions, base_bandwidth, padding_px, effective_bandwidth
        );
    }

    // Input positions are already starts (not centers), so use them directly
    let starts: Vec<f32> = positions.to_vec();

    (starts, effective_bandwidth)
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for FacetRow {
    fn required_channels(&self) -> &'static [&'static str] {
        &["row"]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    fn with_measured_padding(
        &self,
        spec: &crate::coords::PaddingSpec,
    ) -> Box<dyn CoordinateSystemTransform> {
        match spec {
            crate::coords::PaddingSpec::Single {
                padding_px,
                overflow,
            } => {
                let mut updated = self.clone();
                updated.padding_px = Some(*padding_px);
                updated.overflow_by_facet = Some(overflow.clone());
                Box::new(updated)
            }
        }
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        let row_positions = position_channels.get("row").ok_or_else(|| {
            AvengerChartError::InternalError("Missing 'row' channel for FacetRow transform".into())
        })?;

        let count = row_positions.len();
        if count == 0 {
            return Ok(Box::new(crate::coords::SubplotGeometry::default()));
        }

        let centers = row_positions.as_vec(count, None);
        let (starts, bandwidth) = compute_band_layout(&centers, plot_height, self.padding_px);

        if starts.is_empty() {
            return Ok(Box::new(crate::coords::SubplotGeometry::default()));
        }

        // Extract actual row values from position_values (if provided)
        let row_values = position_values
            .and_then(|pv| pv.get("row"))
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let rects = starts
            .into_iter()
            .enumerate()
            .map(|(i, start)| {
                // Use actual facet value if available, otherwise Null
                let value = row_values.get(i).cloned().unwrap_or(ScalarValue::Null);
                crate::coords::SubplotRect::new(value, 0.0, start, plot_width, bandwidth)
            })
            .collect();

        Ok(Box::new(crate::coords::SubplotGeometry::new(rects)))
    }

    fn default_range(
        &self,
        channel: &str,
        _plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        match channel {
            "row" => Some((0.0, plot_area_height)),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, datafusion::scalar::ScalarValue> {
        use datafusion::scalar::ScalarValue;
        let mut options = HashMap::new();
        if channel == "row" && scale_impl.scale_type() == "band" {
            // Set outer padding to 0 to avoid extra space at top/bottom
            // Set inner padding to 0.1 for default spacing between facets
            options.insert("padding_inner".to_string(), ScalarValue::Float64(Some(0.1)));
            options.insert("padding_outer".to_string(), ScalarValue::Float64(Some(0.0)));
            options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
        }
        options
    }
}

/// Column faceting coordinate system
///
/// Facets data along the column dimension, creating a horizontal row of subplots.
/// Each subplot represents one unique value from the `column` channel.
///
/// # Example
/// ```ignore
/// let plot = Plot::<FacetColumn>::new()
///     .data(df)
///     .mark(
///         Facet::new()
///             .column(col("year"))
///             .subplot(
///                 Plot::<Cartesian>::new().mark(Symbol::new()...),
///             ),
///     );
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FacetColumn {
    pub(crate) padding_px: Option<f32>,
    pub(crate) overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
}

impl CoordinateSystem for FacetColumn {
    type Guide = FacetColGuideConfig;

    fn required_channels(&self) -> &'static [&'static str] {
        &["column"]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for FacetColumn {
    fn required_channels(&self) -> &'static [&'static str] {
        &["column"]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    fn with_measured_padding(
        &self,
        spec: &crate::coords::PaddingSpec,
    ) -> Box<dyn CoordinateSystemTransform> {
        match spec {
            crate::coords::PaddingSpec::Single {
                padding_px,
                overflow,
            } => {
                let mut updated = self.clone();
                updated.padding_px = Some(*padding_px);
                updated.overflow_by_facet = Some(overflow.clone());
                Box::new(updated)
            }
        }
    }

    async fn measure(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        _plot_width: f32,
        plot_height: f32,
        eval_ctx: &EvaluationContext,
        data: Option<&DataFrame>,
        compiled_marks: &[Arc<dyn CompiledMark>],
        facet_path: &[ScalarValue],
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        use crate::plot::compiled::scale_provider::PrebuiltScaleProvider;
        use avenger_scales::scales::band::bandwidth;

        // Find the CompiledFacetCol mark to access its subplot
        let facet_mark = compiled_marks
            .iter()
            .find_map(|m| m.as_any().downcast_ref::<CompiledFacetCol>())
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "FacetColumn coord requires a CompiledFacetCol mark".into(),
                )
            })?;

        let compiled_subplot = &facet_mark.compiled_subplot;

        // Get the column scale for layout calculations
        let column_scale = scales
            .get("column")
            .ok_or_else(|| AvengerChartError::InternalError("No column scale found".into()))?;

        // Get bandwidth (subplot width) from the band scale
        let subplot_width = bandwidth(&column_scale.configured().config).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to get bandwidth: {}", e))
        })?;

        // Get cell values from the facet tree, navigating to the correct node for nested facets
        let facet_tree = &eval_ctx.facet_tree;
        let current_node = if facet_path.is_empty() {
            facet_tree.root()
        } else {
            facet_tree.node_at_path(facet_path)
        };
        let current_node = current_node.ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "No facet tree node found at path {:?}",
                facet_path
            ))
        })?;

        let cell_values: Vec<ScalarValue> = current_node.values().cloned().collect();

        if cell_values.is_empty() {
            return Ok(Box::new(FacetColCoordMeasurement {
                cell_values: Vec::new(),
                padding_inner_px: 0.0,
                outer_left: 0.0,
                outer_right: 0.0,
                data_overrides: Vec::new(),
                shared_scales: HashMap::new(),
                parent_path: facet_path.to_vec(),
                subplot_measurements: Vec::new(),
                coordinated_overflow: CoordinatedOverflow::default(),
                compiled_subplot: compiled_subplot.clone(),
                subplot_width: 0.0,
            }));
        }

        // Get the data to filter
        let data_df = data.ok_or_else(|| {
            AvengerChartError::InternalError("FacetColumn measure requires data".into())
        })?;

        // Build shared scales from the FULL data (for shared domain computation)
        let shared_scales = compiled_subplot
            .build_scales_for_dataframe(
                data_df,
                subplot_width,
                plot_height,
                &eval_ctx.session_context,
                &eval_ctx.params,
            )
            .await?;

        // Use PrebuiltScaleProvider to pass shared scales (with pre-computed domains)
        // to subplots. This preserves the exact ConfiguredScale and only updates ranges.
        let scale_provider = PrebuiltScaleProvider {
            scales: shared_scales.clone(),
        };

        // Create subplot EvaluationContext with merged params
        let subplot_eval_ctx = {
            let mut params = compiled_subplot.get_default_params().clone();
            params.extend(eval_ctx.params.clone());
            eval_ctx.with_params(params)
        };

        // === PASS 1: Measure each cell to compute overflow (for padding calculation) ===
        let mut data_overrides = Vec::with_capacity(cell_values.len());
        let mut cell_overflows = Vec::with_capacity(cell_values.len());

        for (idx, value) in cell_values.iter().enumerate() {
            // Build the full path for this cell (parent path + current value)
            let mut cell_path: Vec<ScalarValue> = facet_path.to_vec();
            cell_path.push(value.clone());

            // Get filter predicate for this cell using the full path
            let predicate = facet_tree.cell_predicate(&cell_path, 0);

            // Filter the data
            let filtered_df = if let Some(pred) = predicate {
                data_df.clone().filter(pred).map_err(|e| {
                    AvengerChartError::InternalError(format!(
                        "Failed to filter data for column {:?}: {}",
                        value, e
                    ))
                })?
            } else {
                data_df.clone()
            };

            // Measure the subplot with filtered data to get overflow
            // Pass cell_path for visibility-aware overflow measurement (value-based path, not indices)
            let subplot_layout_spec = EvaluatedLayoutSpec {
                canvas: EvaluatedSizeMode::Auto,
                plot_area: EvaluatedSizeMode::Fixed {
                    width: subplot_width,
                    height: plot_height,
                },
                margins: EvaluatedMargins {
                    top: 0.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                },
            };

            let measurement = compiled_subplot
                .measure_plot_components(
                    &subplot_eval_ctx,
                    &subplot_layout_spec,
                    &scale_provider,
                    Some(&filtered_df),
                    &cell_path, // Pass extended path for nested facets AND visibility
                )
                .await?;

            // Get both guide-only overflow and total overflow (guide + legend)
            let guide_overflow = measurement.layout.overflow.clone();
            let total_overflow = measurement.layout.total_overflow.clone();

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FacetCol coord measure pass 1: cell[{}]={:?} guide_overflow={{top={:.1}, bottom={:.1}, left={:.1}, right={:.1}}} total_overflow={{top={:.1}, bottom={:.1}, left={:.1}, right={:.1}}}",
                    idx,
                    value,
                    guide_overflow.top,
                    guide_overflow.bottom,
                    guide_overflow.left,
                    guide_overflow.right,
                    total_overflow.top,
                    total_overflow.bottom,
                    total_overflow.left,
                    total_overflow.right
                );
            }

            data_overrides.push(filtered_df);
            // Store (guide_overflow, total_overflow) pairs for computing both padding and legend adjustment
            cell_overflows.push((guide_overflow, total_overflow));
        }

        // Compute padding_inner_px from adjacent total overflow combinations
        // This ensures gaps between cells accommodate both axes and legends
        let padding_inner_px = compute_padding_from_overflows(
            &cell_overflows
                .iter()
                .map(|(_, total)| total.clone())
                .collect::<Vec<_>>(),
        );

        // Compute outer edge legend-only overflows for scale range adjustment.
        // We only adjust for LEGEND overflow, not guide overflow, because:
        // - Guide overflow (axes, tick labels) is already handled by each subplot's internal layout
        // - Legend overflow extends beyond the subplot, requiring the facet to allocate extra space
        let outer_left = cell_overflows
            .first()
            .map(|(guide, total)| (total.left - guide.left).max(0.0))
            .unwrap_or(0.0);
        let outer_right = cell_overflows
            .last()
            .map(|(guide, total)| (total.right - guide.right).max(0.0))
            .unwrap_or(0.0);

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetCol coord measure: padding_inner_px={:.1}, outer_left={:.1}, outer_right={:.1} from {} cells",
                padding_inner_px,
                outer_left,
                outer_right,
                cell_values.len()
            );
        }

        // === PASS 2: Rebuild column scale with padding_inner_px AND range adjustment ===
        // This ensures measurements are computed with the final subplot width that accounts for:
        // 1. Inner padding between cells (padding_inner_px)
        // 2. Outer edge legend space (outer_left + outer_right reduce the range)
        //
        // We must reduce the range here to match what apply_scale_adjustments() does during render.
        // Without this, cells would be measured at a larger width than they're rendered at.
        let mut updated_column_scale = column_scale
            .configured()
            .clone()
            .with_option("padding_inner_px", padding_inner_px);

        // Also reduce range for outer legend space (same logic as apply_scale_adjustments)
        // Both outer_left and outer_right reduce the total available width
        if outer_left > 0.0 || outer_right > 0.0 {
            if let Ok((range_start, range_end)) = updated_column_scale.config.numeric_interval_range()
            {
                // Keep start unchanged, reduce end by both outer edges
                let new_end = range_end - outer_left - outer_right;
                updated_column_scale = updated_column_scale.with_range_interval((range_start, new_end));
            }
        }

        let final_subplot_width = bandwidth(&updated_column_scale.config).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to get final bandwidth: {}", e))
        })?;

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetCol coord measure pass 2: initial_width={:.1} -> final_width={:.1}",
                subplot_width, final_subplot_width
            );
        }

        let mut subplot_measurements = Vec::with_capacity(cell_values.len());

        for (idx, (value, filtered_df)) in cell_values.iter().zip(data_overrides.iter()).enumerate()
        {
            let mut cell_path: Vec<ScalarValue> = facet_path.to_vec();
            cell_path.push(value.clone());

            let subplot_layout_spec = EvaluatedLayoutSpec {
                canvas: EvaluatedSizeMode::Auto,
                plot_area: EvaluatedSizeMode::Fixed {
                    width: final_subplot_width,
                    height: plot_height,
                },
                margins: EvaluatedMargins {
                    top: 0.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                },
            };

            let measurement = compiled_subplot
                .measure_plot_components(
                    &subplot_eval_ctx,
                    &subplot_layout_spec,
                    &scale_provider,
                    Some(filtered_df),
                    &cell_path,
                )
                .await?;

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FacetCol coord measure pass 2: cell[{}]={:?} measured at width={:.1}",
                    idx, value, final_subplot_width
                );
            }

            subplot_measurements.push(measurement);
        }

        Ok(Box::new(FacetColCoordMeasurement {
            cell_values,
            padding_inner_px,
            outer_left,
            outer_right,
            data_overrides,
            shared_scales,
            parent_path: facet_path.to_vec(),
            subplot_measurements,
            coordinated_overflow: CoordinatedOverflow::default(),
            compiled_subplot: compiled_subplot.clone(),
            subplot_width: final_subplot_width,
        }))
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        let column_positions = position_channels.get("column").ok_or_else(|| {
            AvengerChartError::InternalError(
                "Missing 'column' channel for FacetColumn transform".into(),
            )
        })?;

        let count = column_positions.len();
        if count == 0 {
            return Ok(Box::new(crate::coords::SubplotGeometry::default()));
        }

        let centers = column_positions.as_vec(count, None);
        let (starts, bandwidth) = compute_band_layout(&centers, plot_width, self.padding_px);

        if starts.is_empty() {
            return Ok(Box::new(crate::coords::SubplotGeometry::default()));
        }

        // Extract actual column values from position_values (if provided)
        let column_values = position_values
            .and_then(|pv| pv.get("column"))
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let rects = starts
            .into_iter()
            .enumerate()
            .map(|(i, start)| {
                // Use actual facet value if available, otherwise Null
                let value = column_values.get(i).cloned().unwrap_or(ScalarValue::Null);
                crate::coords::SubplotRect::new(value, start, 0.0, bandwidth, plot_height)
            })
            .collect();

        Ok(Box::new(crate::coords::SubplotGeometry::new(rects)))
    }

    fn default_range(
        &self,
        channel: &str,
        plot_area_width: f64,
        _plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        match channel {
            "column" => Some((0.0, plot_area_width)),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, datafusion::scalar::ScalarValue> {
        use datafusion::scalar::ScalarValue;
        let mut options = HashMap::new();
        if channel == "column" && scale_impl.scale_type() == "band" {
            // Set outer padding to 0 to avoid extra space at left/right
            // Set inner padding to 0.1 for default spacing between facets
            options.insert("padding_inner".to_string(), ScalarValue::Float64(Some(0.1)));
            options.insert("padding_outer".to_string(), ScalarValue::Float64(Some(0.0)));
            options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
        }
        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::OverflowSpaceRequirement;

    #[test]
    fn facet_row_constructs_struct() {
        let coord = FacetRow {
            padding_px: Some(5.0),
            overflow_by_facet: Some(vec![OverflowSpaceRequirement {
                top: 1.0,
                bottom: 2.0,
                left: 3.0,
                right: 4.0,
            }]),
        };
        assert_eq!(coord.padding_px, Some(5.0));
        assert!(coord.overflow_by_facet.is_some());
    }
}
