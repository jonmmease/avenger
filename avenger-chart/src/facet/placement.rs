//! Shared facet band placement helpers.
//!
//! Scale-backed bands resolve placement from the active band scale. Explicit
//! bands store placement on the measurement, then expose it through the same
//! resolved placement view.

use std::collections::HashMap;

use avenger_scales::scales::ConfiguredScale;
use datafusion_common::ScalarValue;
use tracing::trace;

use crate::{
    coords::{CoordMeasurement, CoordinatedLayout, FacetAxis},
    error::AvengerChartError,
    facet::{
        coord::{FacetBandCoordMeasurement, FacetCellRuntime},
        overflow_projection::{FacetOverflowSlabs, rendered_boundary_demand_for_measurement},
        padding_policy,
    },
    layout::{
        BandChildFrameInput, BandChildFramePlacement, BandDirection, BandPlacedChild, BandPosition,
        BandPositionIterator, BandSpacing, BoundaryDemand1D, ChildFramePlacementResult, Size2D,
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

/// Explicit placement model used when a band dimension is leaf-plot-area-sized.
#[derive(Debug, Clone, Default)]
pub(crate) struct FacetBandExplicitPlacement {
    pub(crate) main_axis_positions: Vec<f32>,
    pub(crate) main_axis_size: f32,
    pub(crate) cross_axis_size: f32,
}

/// Placement model for a facet band.
///
/// Scale-backed placement resolves cell positions from the active band scale.
/// Explicit placement stores the final cell positions on the measurement, which
/// is needed when the facet band's physical main dimension is leaf-plot-area
/// sized and the containing plot area grows to fit the rendered subtree.
#[derive(Debug, Clone, Default)]
pub(crate) enum FacetBandPlacementModel {
    #[default]
    ScaleBacked,
    Explicit(FacetBandExplicitPlacement),
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

    fn from_child_frame_band(axis: FacetAxis, band: BandChildFramePlacement) -> Self {
        let cells = band
            .children
            .iter()
            .map(FacetCellPlacement::from_placed_child)
            .collect();
        Self::new(axis, cells, band.main_axis_extent, band.cross_axis_extent)
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

        let children = explicit
            .main_axis_positions
            .iter()
            .copied()
            .zip(cells.iter())
            .enumerate()
            .map(|(child_index, (main_axis_start, cell))| {
                BandPlacedChild::new(
                    child_index,
                    main_axis_start,
                    cell_main_plot_size(axis, &cell.measurement),
                )
            })
            .collect();
        let band = BandChildFramePlacement::from_positioned_children(
            band_direction(axis),
            children,
            explicit.main_axis_size,
            Some(explicit.cross_axis_size),
        );

        Ok(Self::from_child_frame_band(axis, band))
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

impl FacetCellPlacement {
    fn from_placed_child(child: &BandPlacedChild) -> Self {
        Self {
            cell_index: child.child_index,
            main_axis_start: child.main_axis_start,
            main_axis_size: child.main_axis_size,
        }
    }

    fn to_placed_child(&self) -> BandPlacedChild {
        BandPlacedChild::new(self.cell_index, self.main_axis_start, self.main_axis_size)
    }
}

fn band_direction(axis: FacetAxis) -> BandDirection {
    match axis {
        FacetAxis::Column => BandDirection::Horizontal,
        FacetAxis::Row => BandDirection::Vertical,
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

    let children = bands
        .into_iter()
        .enumerate()
        .map(|(child_index, band)| BandPlacedChild::new(child_index, band.start(), band.bandwidth))
        .collect();
    let band = BandChildFramePlacement::from_positioned_children(
        band_direction(axis),
        children,
        main_axis_extent,
        cross_axis_extent,
    );

    Ok(FacetBandPlacement::from_child_frame_band(axis, band))
}

pub(crate) fn resolve_facet_band_placement_from_configured_scales(
    coord_measurement: &dyn CoordMeasurement,
    scales: &HashMap<String, ConfiguredScale>,
) -> Result<Option<FacetBandPlacement>, AvengerChartError> {
    let Some(facet_band) = coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
    else {
        return Ok(None);
    };

    Ok(Some(
        facet_band.resolved_placement_from_configured_scales(scales)?,
    ))
}

pub(crate) fn resolve_facet_band_placement_from_scale_specs(
    coord_measurement: &dyn CoordMeasurement,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
) -> Result<Option<FacetBandPlacement>, AvengerChartError> {
    let Some(facet_band) = coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
    else {
        return Ok(None);
    };

    Ok(Some(
        facet_band.resolved_placement_from_scale_specs(scales)?,
    ))
}

pub(crate) fn resolve_facet_band_placement(
    measurement: &ComponentsMeasurement,
) -> Result<Option<FacetBandPlacement>, AvengerChartError> {
    resolve_facet_band_placement_from_scale_specs(
        measurement.coord_measurement.as_ref(),
        &measurement.scales,
    )
}

fn facet_cell_main_axis_start_offset(facet_band: &FacetBandCoordMeasurement) -> (f32, f32) {
    let slabs = FacetOverflowSlabs::from_coordinated(&facet_band.coordinated_overflow);
    match facet_band.axis {
        FacetAxis::Column => (0.0, slabs.legend.top),
        FacetAxis::Row => (slabs.legend.left, 0.0),
    }
}

pub(crate) fn facet_child_frame_placement_from_band(
    facet_band: &FacetBandCoordMeasurement,
    placement: &FacetBandPlacement,
    fallback_content_size: Size2D,
) -> Result<ChildFramePlacementResult, AvengerChartError> {
    if placement.axis != facet_band.axis {
        return Err(AvengerChartError::InternalError(format!(
            "Facet child-frame placement axis mismatch: placement={:?}, measurement={:?}",
            placement.axis, facet_band.axis
        )));
    }
    if placement.cell_count() != facet_band.cells.len() {
        return Err(AvengerChartError::InternalError(format!(
            "Facet child-frame placement count mismatch: placement={}, cells={}",
            placement.cell_count(),
            facet_band.cells.len()
        )));
    }

    let children = placement
        .cells
        .iter()
        .enumerate()
        .map(|(idx, cell_placement)| {
            if cell_placement.cell_index != idx {
                return Err(AvengerChartError::InternalError(format!(
                    "Facet child-frame placement cell index mismatch: expected={idx}, actual={}",
                    cell_placement.cell_index
                )));
            }
            Ok(cell_placement.to_placed_child())
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    let band = BandChildFramePlacement::from_positioned_children(
        band_direction(placement.axis),
        children,
        placement.main_axis_extent,
        placement.cross_axis_extent,
    );

    let (origin_offset_x, origin_offset_y) = facet_cell_main_axis_start_offset(facet_band);
    Ok(band
        .to_child_frame_placement_result([origin_offset_x, origin_offset_y], fallback_content_size))
}

pub(crate) fn resolve_facet_child_frame_placement_from_scale_specs(
    facet_band: &FacetBandCoordMeasurement,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    fallback_content_size: Size2D,
) -> Result<ChildFramePlacementResult, AvengerChartError> {
    let placement = facet_band.resolved_placement_from_scale_specs(scales)?;
    facet_child_frame_placement_from_band(facet_band, &placement, fallback_content_size)
}

pub(crate) fn resolve_facet_child_frame_placement(
    measurement: &ComponentsMeasurement,
    facet_band: &FacetBandCoordMeasurement,
) -> Result<ChildFramePlacementResult, AvengerChartError> {
    resolve_facet_child_frame_placement_from_scale_specs(
        facet_band,
        &measurement.scales,
        Size2D::new(measurement.plot_area_width, measurement.plot_area_height),
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

    let gap = layout
        .padding_inner_px
        .max(padding_policy::MIN_SUBPLOT_MAIN_GAP);

    let inputs = cells
        .iter()
        .enumerate()
        .map(|(child_index, cell)| {
            let boundary = rendered_boundary_demand_for_measurement(axis, &cell.measurement);
            BandChildFrameInput {
                child_index,
                main_axis_size: cell_main_plot_size(axis, &cell.measurement),
                cross_axis_size: cell_cross_plot_size(axis, &cell.measurement),
                boundary: BoundaryDemand1D {
                    before: boundary.before,
                    after: boundary.after,
                },
            }
        })
        .collect::<Vec<_>>();

    for window in inputs.windows(2) {
        let current = &window[0];
        let next = &window[1];
        let after_current = current.boundary.after.max(0.0);
        let before_next = next.boundary.before.max(0.0);
        let gap_size = gap.max(after_current + before_next);
        trace!(
            axis = ?axis,
            cell_index = current.child_index,
            base_gap = gap,
            after_current,
            before_next,
            gap_size,
            "plot-area-sized facet placement gap"
        );
    }

    let slot_count = layout.n.max(inputs.len());
    let band_inputs = if slot_count > inputs.len() {
        let fallback_main_axis_size = inputs
            .iter()
            .map(|input| input.main_axis_size)
            .fold(0.0f32, f32::max);
        let fallback_cross_axis_size = inputs
            .iter()
            .map(|input| input.cross_axis_size)
            .fold(0.0f32, f32::max);
        let mut band_inputs = inputs.clone();
        for child_index in inputs.len()..slot_count {
            band_inputs.push(BandChildFrameInput {
                child_index,
                main_axis_size: fallback_main_axis_size,
                cross_axis_size: fallback_cross_axis_size,
                boundary: BoundaryDemand1D::default(),
            });
        }
        band_inputs
    } else {
        inputs.clone()
    };

    let band = BandChildFramePlacement::from_sized_children(
        band_direction(axis),
        &band_inputs,
        BandSpacing {
            outer_start: layout.outer_start,
            outer_end: layout.outer_end,
            min_inner_gap: gap,
            ..Default::default()
        },
    );
    let main_axis_positions = band
        .children
        .iter()
        .take(cells.len())
        .map(|child| child.main_axis_start)
        .collect();
    let main_axis_size = band.main_axis_extent;
    let cross_axis_size = band.cross_axis_extent.unwrap_or(0.0);

    FacetBandExplicitPlacement {
        main_axis_positions,
        main_axis_size,
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
        .downcast_ref::<FacetBandCoordMeasurement>()
        .filter(|facet_band| facet_band.uses_explicit_placement())
    {
        if facet_band.cells.is_empty() {
            return match axis {
                FacetAxis::Column => measurement.plot_area_width,
                FacetAxis::Row => measurement.plot_area_height,
            }
            .max(0.0);
        }

        let (plot_width, plot_height) = facet_band.plot_area_extent();
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
        .downcast_ref::<FacetBandCoordMeasurement>()
        .filter(|facet_band| facet_band.uses_explicit_placement())
    {
        if facet_band.cells.is_empty() {
            return match axis {
                FacetAxis::Column => measurement.plot_area_height,
                FacetAxis::Row => measurement.plot_area_width,
            }
            .max(0.0);
        }

        let (plot_width, plot_height) = facet_band.plot_area_extent();
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
