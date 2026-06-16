//! Shared facet band geometry helpers.
//!
//! Facet geometry is chart-owned layout state. It is derived either from the
//! active coordinated layout over a known parent extent, or from measured child
//! content when the facet's main axis is content-driven.

use tracing::trace;

use crate::{
    coords::{CoordinatedLayout, FacetAxis},
    facet::{
        coord::{FacetBandCoordMeasurement, FacetCellRuntime},
        overflow_projection::rendered_boundary_demand_for_measurement,
        padding_policy,
    },
    layout::{BoundaryDemand, Size, TrackSpacing},
    plot::compiled::ComponentsMeasurement,
};

/// Resolved geometry for a facet band, regardless of sizing mode.
#[derive(Debug, Clone)]
pub(crate) struct FacetBandGeometry {
    pub(crate) axis: FacetAxis,
    pub(crate) cells: Vec<FacetCellGeometry>,
    pub(crate) main_extent: f32,
    pub(crate) cross_extent: Option<f32>,
}

/// Resolved geometry for one facet cell.
#[derive(Debug, Clone)]
pub(crate) struct FacetCellGeometry {
    pub(crate) cell_index: usize,
    pub(crate) main_start: f32,
    pub(crate) main_size: f32,
}

impl FacetBandGeometry {
    pub(crate) fn new(
        axis: FacetAxis,
        cells: Vec<FacetCellGeometry>,
        main_extent: f32,
        cross_extent: Option<f32>,
    ) -> Self {
        Self {
            axis,
            cells,
            main_extent,
            cross_extent,
        }
    }

    #[cfg(test)]
    pub(crate) fn cell_count(&self) -> usize {
        self.cells.len()
    }

    #[cfg(test)]
    pub(crate) fn cell(&self, index: usize) -> Option<&FacetCellGeometry> {
        self.cells.get(index)
    }

    #[cfg(test)]
    pub(crate) fn main_sizes(&self) -> impl Iterator<Item = f32> + '_ {
        self.cells.iter().map(|cell| cell.main_size)
    }
}

pub(crate) fn compute_uniform_facet_band_geometry(
    axis: FacetAxis,
    real_cell_count: usize,
    layout: &CoordinatedLayout,
    parent_main_extent: f32,
    cross_extent: Option<f32>,
) -> FacetBandGeometry {
    if real_cell_count == 0 {
        return FacetBandGeometry::new(axis, Vec::new(), 0.0, cross_extent);
    }

    let slot_count = layout.n.max(real_cell_count).max(1);
    let range_start = layout.outer_start.max(0.0);
    let range_size =
        (parent_main_extent.max(0.0) - range_start - layout.outer_end.max(0.0)).max(0.0);
    let gap = layout.padding_inner_px.max(0.0);

    // Uniform chart-owned layout geometry: reserve coordinated outer edges,
    // divide the remaining parent extent across real and ghost slots, and
    // keep the requested inner gap between neighboring slots.
    //
    // Column child plot areas are rendered on pixel-snapped bounds. If
    // snapping the uniform track size still fits in the parent range, use
    // that stable pixel size for readback too; otherwise keep the fractional
    // size so the final track does not overrun the parent. Row facets keep
    // the fractional band math; vertical snapping compounds visibly across
    // rows and diverges from the historical row layout.
    let available = (range_size - gap * slot_count.saturating_sub(1) as f32).max(0.0);
    let exact_track_size = (available / slot_count as f32).max(0.0);
    let snapped_track_size = exact_track_size.round().max(0.0);
    let snapped_extent =
        snapped_track_size * slot_count as f32 + gap * slot_count.saturating_sub(1) as f32;
    let track_size = if matches!(axis, FacetAxis::Column) && snapped_extent <= range_size + 0.01 {
        snapped_track_size
    } else {
        exact_track_size
    };
    let step = track_size + gap;

    let cells = (0..real_cell_count)
        .map(|cell_index| FacetCellGeometry {
            cell_index,
            main_start: range_start + cell_index as f32 * step,
            main_size: track_size,
        })
        .collect::<Vec<_>>();
    let main_extent = parent_main_extent.max(0.0);

    FacetBandGeometry::new(axis, cells, main_extent, cross_extent)
}

pub(crate) fn compute_content_driven_facet_band_geometry(
    axis: FacetAxis,
    cells: &[FacetCellRuntime],
    layout: &CoordinatedLayout,
) -> FacetBandGeometry {
    if cells.is_empty() {
        return FacetBandGeometry::new(axis, Vec::new(), 0.0, Some(0.0));
    }

    let gap = padding_policy::main_axis_gap(layout.padding_inner_px);

    let inputs = cells
        .iter()
        .map(|cell| {
            let boundary = rendered_boundary_demand_for_measurement(axis, &cell.measurement);
            (
                cell_main_plot_size(axis, &cell.measurement),
                cell_cross_plot_size(axis, &cell.measurement),
                BoundaryDemand {
                    before: boundary.before,
                    after: boundary.after,
                },
            )
        })
        .collect::<Vec<_>>();

    // Trace-only: mirrors the solver's gap rule
    // `max(min_gap, after + before)` over the raw boundaries below.
    // Nothing computed here feeds the solve.
    for (cell_index, window) in inputs.windows(2).enumerate() {
        let (_, _, current) = &window[0];
        let (_, _, next) = &window[1];
        let after_current = current.after.max(0.0);
        let before_next = next.before.max(0.0);
        let gap_size = gap.max(after_current + before_next);
        trace!(
            axis = ?axis,
            cell_index,
            base_gap = gap,
            after_current,
            before_next,
            gap_size,
            "plot-area-sized facet geometry gap"
        );
    }

    // Ghost slots pad the solve to the coordinated slot count at the
    // largest real cell size, with no boundary demands.
    let slot_count = layout.n.max(inputs.len());
    let mut band_inputs = inputs;
    if slot_count > band_inputs.len() {
        let fallback_main_size = band_inputs
            .iter()
            .map(|(main, _, _)| *main)
            .fold(0.0f32, f32::max);
        let fallback_cross_size = band_inputs
            .iter()
            .map(|(_, cross, _)| *cross)
            .fold(0.0f32, f32::max);
        for _ in band_inputs.len()..slot_count {
            band_inputs.push((
                fallback_main_size,
                fallback_cross_size,
                BoundaryDemand::default(),
            ));
        }
    }

    // One track per slot: leaves at cell sizes, sibling boundaries as
    // inner-stratum edge demands, the coordinated spacing as the track
    // policy.
    let vertical = matches!(axis, FacetAxis::Row);
    let (before_side, after_side) = if vertical {
        (avenger_layout::Side::Top, avenger_layout::Side::Bottom)
    } else {
        (avenger_layout::Side::Left, avenger_layout::Side::Right)
    };
    let leaves = band_inputs.iter().map(|(main, cross, boundary)| {
        let size = if vertical {
            Size::new(*cross, *main)
        } else {
            Size::new(*main, *cross)
        };
        avenger_layout::Layout::<usize>::leaf(size)
            .demand(
                before_side,
                avenger_layout::EdgeDemand {
                    guide: boundary.before,
                    legend: 0.0,
                },
            )
            .demand(
                after_side,
                avenger_layout::EdgeDemand {
                    guide: boundary.after,
                    legend: 0.0,
                },
            )
    });
    let spacing = TrackSpacing {
        outer_start: layout.outer_start,
        outer_end: layout.outer_end,
        min_gap: gap,
    };
    let stack = if vertical {
        avenger_layout::Layout::column(leaves).row_spacing(spacing)
    } else {
        avenger_layout::Layout::row(leaves).column_spacing(spacing)
    };
    let solved = stack
        .solve(&avenger_layout::SolveOptions::default())
        .expect("a band of leaves always solves");
    let root = solved.at_path(&[]).expect("root region exists");
    let avenger_layout::RegionDetail::Grid { tracks } = &root.detail else {
        unreachable!("a band root is a grid");
    };
    let (main_starts, main_sizes, cross_extent, main_extent) = if vertical {
        (
            &tracks.row_starts,
            &tracks.row_sizes,
            tracks.column_sizes[0],
            root.content.height,
        )
    } else {
        (
            &tracks.column_starts,
            &tracks.column_sizes,
            tracks.row_sizes[0],
            root.content.width,
        )
    };

    // Only the real cells become geometry entries; ghost tracks reserve space.
    let placed_cells = (0..cells.len())
        .map(|cell_index| FacetCellGeometry {
            cell_index,
            main_start: main_starts[cell_index],
            main_size: main_sizes[cell_index],
        })
        .collect();

    FacetBandGeometry::new(axis, placed_cells, main_extent, Some(cross_extent))
}

/// Returns the main-axis extent of a cell for plot-area-sized positioning.
///
/// For leaf cells, this is simply `plot_area_width` (or height for rows).
/// For intermediate cells (containing a plot-area-sized facet band), the
/// content-driven geometry carries the actual subtree extent, including
/// inter-cell gaps and trailing outer padding.
fn cell_main_plot_size(axis: FacetAxis, measurement: &ComponentsMeasurement) -> f32 {
    if let Some(facet_band) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
        .filter(|facet_band| facet_band.content_driven_main_axis())
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
        .filter(|facet_band| facet_band.content_driven_main_axis())
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

    fn layout(
        padding_inner_px: f32,
        outer_start: f32,
        outer_end: f32,
        n: usize,
    ) -> CoordinatedLayout {
        CoordinatedLayout {
            padding_inner_px,
            guide_slot_gap_px: 0.0,
            outer_start,
            outer_end,
            n,
        }
    }

    #[test]
    fn uniform_geometry_places_column_cells_from_layout() {
        let geometry = compute_uniform_facet_band_geometry(
            FacetAxis::Column,
            2,
            &layout(10.0, 5.0, 15.0, 2),
            120.0,
            Some(42.0),
        );

        assert_eq!(geometry.axis, FacetAxis::Column);
        assert_eq!(geometry.cell_count(), 2);
        assert_eq!(geometry.cross_extent, Some(42.0));
        assert!((geometry.main_extent - 120.0).abs() <= 0.01);
        assert!((geometry.cell(0).unwrap().main_start - 5.0).abs() <= 0.01);
        assert!((geometry.cell(0).unwrap().main_size - 45.0).abs() <= 0.01);
        assert!((geometry.cell(1).unwrap().main_start - 60.0).abs() <= 0.01);
        assert_eq!(geometry.cell(1).unwrap().cell_index, 1);
    }

    #[test]
    fn uniform_geometry_reserves_ghost_slots_without_returning_them() {
        let geometry = compute_uniform_facet_band_geometry(
            FacetAxis::Row,
            2,
            &layout(4.0, 0.0, 0.0, 4),
            100.0,
            None,
        );

        assert_eq!(geometry.axis, FacetAxis::Row);
        assert_eq!(geometry.cell_count(), 2);
        assert!((geometry.main_extent - 100.0).abs() <= 0.01);
        assert!((geometry.cell(0).unwrap().main_size - 22.0).abs() <= 0.01);
        assert!((geometry.cell(1).unwrap().main_start - 26.0).abs() <= 0.01);
    }

    #[test]
    fn uniform_geometry_allows_empty_filtered_cells() {
        let geometry = compute_uniform_facet_band_geometry(
            FacetAxis::Column,
            0,
            &layout(10.0, 5.0, 15.0, 4),
            120.0,
            Some(42.0),
        );

        assert_eq!(geometry.axis, FacetAxis::Column);
        assert_eq!(geometry.cell_count(), 0);
        assert_eq!(geometry.main_extent, 0.0);
        assert_eq!(geometry.cross_extent, Some(42.0));
    }

    #[test]
    fn uniform_geometry_zero_gap_and_outer_edges() {
        let geometry = compute_uniform_facet_band_geometry(
            FacetAxis::Column,
            3,
            &layout(0.0, 10.0, 20.0, 3),
            100.0,
            None,
        );

        assert_eq!(geometry.cell_count(), 3);
        assert!((geometry.cell(0).unwrap().main_start - 10.0).abs() <= 0.01);
        assert!((geometry.cell(0).unwrap().main_size - 23.0).abs() <= 0.01);
        assert!((geometry.cell(2).unwrap().main_start - 56.0).abs() <= 0.01);
    }

    #[test]
    fn uniform_geometry_clamps_overlarge_gap_to_nonnegative_tracks() {
        let geometry = compute_uniform_facet_band_geometry(
            FacetAxis::Row,
            3,
            &layout(80.0, 0.0, 0.0, 3),
            100.0,
            None,
        );

        assert_eq!(geometry.cell_count(), 3);
        assert!(geometry.main_sizes().all(|size| size >= 0.0));
        assert_eq!(geometry.cell(0).unwrap().main_size, 0.0);
        assert_eq!(geometry.cell(1).unwrap().main_start, 80.0);
    }
}
