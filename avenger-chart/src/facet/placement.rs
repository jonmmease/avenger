//! Shared facet band placement helpers.
//!
//! Canvas-fit facets resolve placement from the active band scale. Plot-area-sized
//! facets store explicit placement, then expose it through the same
//! resolved placement view.

use std::collections::HashMap;

use avenger_scales::scales::ConfiguredScale;
use datafusion_common::ScalarValue;
use tracing::trace;

use crate::{
    coords::{CoordMeasurement, CoordinatedLayout, FacetAxis},
    error::AvengerChartError,
    facet::{
        band_positions::{BandPosition, BandPositionIterator},
        coord::{
            FacetBandCoordMeasurementCanvasFit, FacetBandCoordMeasurementPlotAreaSized,
            FacetCellRuntime,
        },
        overflow_projection::rendered_boundary_demand_for_measurement,
        padding_policy,
    },
    plot::compiled::ComponentsMeasurement,
    scales::ConfiguredScaleWithSpec,
};

/// Resolved placement for a facet band, regardless of sizing mode.
#[derive(Debug, Clone)]
pub(crate) struct FacetBandPlacement {
    pub(crate) axis: FacetAxis,
    pub(crate) cells: Vec<FacetCellPlacement>,
    pub(crate) main_axis_extent: f32,
    pub(crate) cross_axis_extent: Option<f32>,
}

/// Resolved placement for one facet cell.
#[derive(Debug, Clone)]
pub(crate) struct FacetCellPlacement {
    pub(crate) cell_index: usize,
    pub(crate) main_axis_start: f32,
    pub(crate) main_axis_size: f32,
}

/// Explicit placement model used by plot-area-sized facets.
#[derive(Debug, Clone, Default)]
pub(crate) struct FacetBandExplicitPlacement {
    pub(crate) main_axis_positions: Vec<f32>,
    pub(crate) main_axis_size: f32,
    pub(crate) cross_axis_size: f32,
}

impl FacetBandPlacement {
    pub(crate) fn new(
        axis: FacetAxis,
        cells: Vec<FacetCellPlacement>,
        main_axis_extent: f32,
        cross_axis_extent: Option<f32>,
    ) -> Self {
        Self {
            axis,
            cells,
            main_axis_extent,
            cross_axis_extent,
        }
    }

    pub(crate) fn from_explicit(
        axis: FacetAxis,
        explicit: &FacetBandExplicitPlacement,
        cells: &[FacetCellRuntime],
    ) -> Result<Self, AvengerChartError> {
        if explicit.main_axis_positions.len() != cells.len() {
            return Err(AvengerChartError::InternalError(format!(
                "Facet explicit placement count mismatch: positions={}, cells={}",
                explicit.main_axis_positions.len(),
                cells.len()
            )));
        }

        let cells = explicit
            .main_axis_positions
            .iter()
            .copied()
            .zip(cells.iter())
            .enumerate()
            .map(|(cell_index, (main_axis_start, cell))| FacetCellPlacement {
                cell_index,
                main_axis_start,
                main_axis_size: cell_main_plot_size(axis, &cell.measurement),
            })
            .collect();

        Ok(Self::new(
            axis,
            cells,
            explicit.main_axis_size,
            Some(explicit.cross_axis_size),
        ))
    }

    pub(crate) fn cell_count(&self) -> usize {
        self.cells.len()
    }

    pub(crate) fn cell(&self, index: usize) -> Option<&FacetCellPlacement> {
        self.cells.get(index)
    }

    #[cfg(test)]
    pub(crate) fn main_axis_starts(&self) -> impl Iterator<Item = f32> + '_ {
        self.cells.iter().map(|cell| cell.main_axis_start)
    }

    #[cfg(test)]
    pub(crate) fn main_axis_sizes(&self) -> impl Iterator<Item = f32> + '_ {
        self.cells.iter().map(|cell| cell.main_axis_size)
    }

    pub(crate) fn band_positions(
        &self,
        cells: &[FacetCellRuntime],
    ) -> Result<Vec<BandPosition>, AvengerChartError> {
        if self.cells.len() != cells.len() {
            return Err(AvengerChartError::InternalError(format!(
                "Facet placement count mismatch: placement={}, cells={}",
                self.cells.len(),
                cells.len()
            )));
        }

        Ok(self
            .cells
            .iter()
            .zip(cells.iter())
            .map(|(placement, cell)| {
                BandPosition::new(
                    cell.plan.value.clone(),
                    placement.main_axis_start,
                    placement.main_axis_size,
                )
            })
            .collect())
    }
}

pub(crate) fn resolve_scale_backed_facet_band_placement(
    axis: FacetAxis,
    configured: &ConfiguredScale,
    cell_values: &[ScalarValue],
    cell_count: usize,
    cross_axis_extent: Option<f32>,
) -> Result<FacetBandPlacement, AvengerChartError> {
    let bands: Vec<_> = BandPositionIterator::from_configured_scale(configured)?.collect();
    let axis_label = match axis {
        FacetAxis::Column => "FacetCol",
        FacetAxis::Row => "FacetRow",
    };

    if bands.len() != cell_count {
        return Err(AvengerChartError::InternalError(format!(
            "{axis_label} placement: band positions length {} did not match facet cell count {}",
            bands.len(),
            cell_count
        )));
    }

    if bands.len() != cell_values.len() {
        return Err(AvengerChartError::InternalError(format!(
            "{axis_label} placement: band positions length {} did not match facet cell value count {}",
            bands.len(),
            cell_values.len()
        )));
    }

    if !bands
        .iter()
        .zip(cell_values.iter())
        .all(|(band, value)| band.value == *value)
    {
        return Err(AvengerChartError::InternalError(format!(
            "{axis_label} placement: band position order did not align with facet cell order"
        )));
    }

    let main_axis_extent = configured
        .numeric_interval_range()
        .map(|(start, end)| (end - start).abs())
        .unwrap_or_else(|_| {
            let start = bands.first().map(BandPosition::start).unwrap_or(0.0);
            let end = bands.last().map(BandPosition::end).unwrap_or(0.0);
            (end - start).abs()
        });

    let cells = bands
        .into_iter()
        .enumerate()
        .map(|(cell_index, band)| FacetCellPlacement {
            cell_index,
            main_axis_start: band.start(),
            main_axis_size: band.bandwidth,
        })
        .collect();

    Ok(FacetBandPlacement::new(
        axis,
        cells,
        main_axis_extent,
        cross_axis_extent,
    ))
}

pub(crate) fn resolve_facet_band_placement_from_configured_scales(
    coord_measurement: &dyn CoordMeasurement,
    scales: &HashMap<String, ConfiguredScale>,
) -> Result<Option<FacetBandPlacement>, AvengerChartError> {
    if let Some(facet_band) = coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementPlotAreaSized>()
    {
        return Ok(Some(facet_band.resolved_placement()?));
    }

    let Some(facet_band) = coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementCanvasFit>()
    else {
        return Ok(None);
    };

    let configured = scales.get(facet_band.axis.scale_name()).ok_or_else(|| {
        AvengerChartError::InternalError(format!(
            "Missing {} scale while resolving facet placement",
            facet_band.axis.scale_name()
        ))
    })?;
    let cell_values = facet_band.cell_values().cloned().collect::<Vec<_>>();
    Ok(Some(resolve_scale_backed_facet_band_placement(
        facet_band.axis,
        configured,
        &cell_values,
        facet_band.cells.len(),
        facet_band.coordinated_subplot_cross_size(),
    )?))
}

pub(crate) fn resolve_facet_band_placement_from_scale_specs(
    coord_measurement: &dyn CoordMeasurement,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
) -> Result<Option<FacetBandPlacement>, AvengerChartError> {
    if let Some(facet_band) = coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementPlotAreaSized>()
    {
        return Ok(Some(facet_band.resolved_placement()?));
    }

    let Some(facet_band) = coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementCanvasFit>()
    else {
        return Ok(None);
    };

    let configured = scales.get(facet_band.axis.scale_name()).ok_or_else(|| {
        AvengerChartError::InternalError(format!(
            "Missing {} scale while resolving facet placement",
            facet_band.axis.scale_name()
        ))
    })?;
    let cell_values = facet_band.cell_values().cloned().collect::<Vec<_>>();
    Ok(Some(resolve_scale_backed_facet_band_placement(
        facet_band.axis,
        configured.configured(),
        &cell_values,
        facet_band.cells.len(),
        facet_band.coordinated_subplot_cross_size(),
    )?))
}

pub(crate) fn resolve_facet_band_placement(
    measurement: &ComponentsMeasurement,
) -> Result<Option<FacetBandPlacement>, AvengerChartError> {
    resolve_facet_band_placement_from_scale_specs(
        measurement.coord_measurement.as_ref(),
        &measurement.scales,
    )
}

pub(crate) fn compute_explicit_facet_band_placement(
    axis: FacetAxis,
    cells: &[FacetCellRuntime],
    layout: &CoordinatedLayout,
) -> FacetBandExplicitPlacement {
    if cells.is_empty() {
        return FacetBandExplicitPlacement::default();
    }

    let mut positions = Vec::with_capacity(cells.len());
    let mut cursor = layout.outer_start.max(0.0);
    let mut cross_axis_size = 0.0f32;
    let gap = layout
        .padding_inner_px
        .max(padding_policy::MIN_SUBPLOT_MAIN_GAP);

    for (idx, cell) in cells.iter().enumerate() {
        positions.push(cursor);
        cross_axis_size = cross_axis_size.max(cell_cross_plot_size(axis, &cell.measurement));
        let cell_size = cell_main_plot_size(axis, &cell.measurement);
        cursor += cell_size;
        if idx + 1 < cells.len() {
            // Fixed plot-area mode keeps leaf plot areas locked, so inter-cell
            // gaps absorb any rendered content that extends past plot bounds.
            let current_boundary =
                rendered_boundary_demand_for_measurement(axis, &cell.measurement);
            let next_boundary =
                rendered_boundary_demand_for_measurement(axis, &cells[idx + 1].measurement);
            let after_current = current_boundary.after;
            let before_next = next_boundary.before;
            let gap_size = gap.max(after_current + before_next);
            trace!(
                axis = ?axis,
                cell_index = idx,
                base_gap = gap,
                after_current,
                before_next,
                gap_size,
                "plot-area-sized facet placement gap"
            );
            cursor += gap_size;
        }
    }

    FacetBandExplicitPlacement {
        main_axis_positions: positions,
        main_axis_size: (cursor + layout.outer_end.max(0.0)).max(0.0),
        cross_axis_size,
    }
}

#[cfg(test)]
pub(crate) fn compute_explicit_main_axis_positions(
    axis: FacetAxis,
    cells: &[FacetCellRuntime],
    layout: &CoordinatedLayout,
) -> Vec<f32> {
    compute_explicit_facet_band_placement(axis, cells, layout).main_axis_positions
}

/// Returns the main-axis extent of a cell for plot-area-sized positioning.
///
/// For leaf cells, this is simply `plot_area_width` (or height for rows).
/// For intermediate cells (containing a plot-area-sized facet band), the
/// stored explicit placement carries the actual subtree extent, including
/// inter-cell gaps and trailing outer padding.
fn cell_main_plot_size(axis: FacetAxis, measurement: &ComponentsMeasurement) -> f32 {
    if let Some(facet_band) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementPlotAreaSized>()
    {
        if facet_band.cells.is_empty() {
            return match axis {
                FacetAxis::Column => measurement.plot_area_width,
                FacetAxis::Row => measurement.plot_area_height,
            }
            .max(0.0);
        }

        let (plot_width, plot_height) = facet_band.plot_area_sized_extent();
        return match axis {
            FacetAxis::Column => plot_width,
            FacetAxis::Row => plot_height,
        }
        .max(0.0);
    }

    match axis {
        FacetAxis::Column => measurement.plot_area_width,
        FacetAxis::Row => measurement.plot_area_height,
    }
    .max(0.0)
}

fn cell_cross_plot_size(axis: FacetAxis, measurement: &ComponentsMeasurement) -> f32 {
    if let Some(facet_band) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementPlotAreaSized>()
    {
        if facet_band.cells.is_empty() {
            return match axis {
                FacetAxis::Column => measurement.plot_area_height,
                FacetAxis::Row => measurement.plot_area_width,
            }
            .max(0.0);
        }

        let (plot_width, plot_height) = facet_band.plot_area_sized_extent();
        return match axis {
            FacetAxis::Column => plot_height,
            FacetAxis::Row => plot_width,
        }
        .max(0.0);
    }

    match axis {
        FacetAxis::Column => measurement.plot_area_height,
        FacetAxis::Row => measurement.plot_area_width,
    }
    .max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_scales::scales::band::BandScale;

    fn make_band_scale(range: (f32, f32)) -> ConfiguredScale {
        let domain = ScalarValue::iter_to_array(vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
        ])
        .unwrap();
        BandScale::configured(domain, range)
    }

    #[test]
    fn scale_backed_placement_matches_active_band_scale() {
        let scale = make_band_scale((0.0, 100.0));
        let cell_values = vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
        ];

        let placement = resolve_scale_backed_facet_band_placement(
            FacetAxis::Column,
            &scale,
            &cell_values,
            cell_values.len(),
            Some(42.0),
        )
        .unwrap();

        let starts = placement.main_axis_starts().collect::<Vec<_>>();
        let sizes = placement.main_axis_sizes().collect::<Vec<_>>();
        assert_eq!(placement.axis, FacetAxis::Column);
        assert_eq!(placement.cell_count(), 2);
        assert!(starts[0] < starts[1]);
        assert!(sizes.iter().all(|size| *size > 0.0));
        assert_eq!(placement.cross_axis_extent, Some(42.0));
        assert_eq!(placement.cell(1).unwrap().cell_index, 1);
    }

    #[test]
    fn scale_backed_placement_errors_on_count_mismatch() {
        let scale = make_band_scale((0.0, 100.0));
        let cell_values = vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
        ];

        let err = resolve_scale_backed_facet_band_placement(
            FacetAxis::Column,
            &scale,
            &cell_values,
            cell_values.len() + 1,
            None,
        )
        .unwrap_err();
        let message = format!("{}", err);
        assert!(message.contains("band positions length"));
        assert!(message.contains("facet cell count"));
    }

    #[test]
    fn scale_backed_placement_errors_on_order_mismatch() {
        let scale = make_band_scale((0.0, 100.0));
        let cell_values = vec![
            ScalarValue::Utf8(Some("b".to_string())),
            ScalarValue::Utf8(Some("a".to_string())),
        ];

        let err = resolve_scale_backed_facet_band_placement(
            FacetAxis::Column,
            &scale,
            &cell_values,
            cell_values.len(),
            None,
        )
        .unwrap_err();
        let message = format!("{}", err);
        assert!(message.contains("band position order"));
        assert!(message.contains("facet cell order"));
    }
}
