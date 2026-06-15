//! Shared facet band placement helpers.
//!
//! Scale-backed bands resolve placement from the active band scale. Explicit
//! bands store placement on the measurement, then expose it through the same
//! resolved placement view.

use std::collections::HashMap;

use avenger_chart_core::LayoutBounds;
use avenger_layout::{Distribute, Layout, RegionDetail, SolveOptions, Spacing, TrackSize};
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
        BandPosition, BandPositionIterator, BoundaryDemand, Size, TrackSpacing, UniformTracks,
    },
    plot::compiled::{ChildFrameRegion, ComponentsMeasurement},
    scales::ConfiguredScaleWithSpec,
};

/// Resolved placement for a facet band, regardless of sizing mode.
#[derive(Debug, Clone)]
pub(crate) struct FacetBandPlacement {
    pub(crate) axis: FacetAxis,
    pub(crate) cells: Vec<FacetCellPlacement>,
    pub(crate) main_extent: f32,
    pub(crate) cross_extent: Option<f32>,
}

/// Resolved placement for one facet cell.
#[derive(Debug, Clone)]
pub(crate) struct FacetCellPlacement {
    pub(crate) cell_index: usize,
    pub(crate) main_start: f32,
    pub(crate) main_size: f32,
}

/// Placement model for a facet band.
///
/// Scale-backed placement resolves cell positions from the active band scale.
/// Explicit placement computes cell positions on read from the live cells
/// and the coordinated views, which is needed when the facet band's physical
/// main dimension is leaf-plot-area sized and the containing plot area grows
/// to fit the rendered subtree.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum FacetBandPlacementModel {
    #[default]
    ScaleBacked,
    Explicit,
}

impl FacetBandPlacement {
    pub(crate) fn new(
        axis: FacetAxis,
        cells: Vec<FacetCellPlacement>,
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

    pub(crate) fn cell_count(&self) -> usize {
        self.cells.len()
    }

    pub(crate) fn cell(&self, index: usize) -> Option<&FacetCellPlacement> {
        self.cells.get(index)
    }

    #[cfg(test)]
    pub(crate) fn main_starts(&self) -> impl Iterator<Item = f32> + '_ {
        self.cells.iter().map(|cell| cell.main_start)
    }

    #[cfg(test)]
    pub(crate) fn main_sizes(&self) -> impl Iterator<Item = f32> + '_ {
        self.cells.iter().map(|cell| cell.main_size)
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
                    placement.main_start,
                    placement.main_size,
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
    cross_extent: Option<f32>,
) -> Result<FacetBandPlacement, AvengerChartError> {
    let bands: Vec<_> = BandPositionIterator::from_configured_scale(configured)?.collect();
    let axis_label = match axis {
        FacetAxis::Column => "FacetCol",
        FacetAxis::Row => "FacetRow",
    };

    if cell_count == 0 && cell_values.is_empty() {
        return Ok(FacetBandPlacement::new(axis, Vec::new(), 0.0, cross_extent));
    }

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

    let main_extent = configured
        .numeric_interval_range()
        .map(|(start, end)| (end - start).abs())
        .unwrap_or_else(|_| {
            let start = bands.first().map(BandPosition::start).unwrap_or(0.0);
            let end = bands.last().map(BandPosition::end).unwrap_or(0.0);
            (end - start).abs()
        });

    // Positions come from the neutral uniform-tracks solve. Facet-written
    // band scales are uniform by construction (range + band_n +
    // padding_inner_px over default band options), so each scale-read band
    // position is shadow-asserted against the solved arithmetic progression
    // in debug builds.
    let track_size = bands.first().map(|band| band.bandwidth).unwrap_or(0.0);
    let outer_start = bands.first().map(BandPosition::start).unwrap_or(0.0);
    let min_gap = if bands.len() >= 2 {
        (bands[1].start() - bands[0].end()).max(0.0)
    } else {
        0.0
    };
    let outer_end = bands
        .last()
        .map(|band| (main_extent - band.end()).max(0.0))
        .unwrap_or(0.0);
    let tracks = UniformTracks {
        count: bands.len(),
        spacing: TrackSpacing {
            outer_start,
            outer_end,
            min_gap,
        },
    };
    let solved = tracks.solve(track_size);
    let children = solved
        .starts
        .iter()
        .enumerate()
        .map(|(id, &main_start)| {
            debug_assert!(
                (main_start - bands[id].start()).abs() <= 0.01
                    && (solved.track_size - bands[id].bandwidth).abs() <= 0.01,
                "facet band scale positions diverged from the uniform-tracks solve: \
                 solved=({main_start}, {}), scale=({}, {})",
                solved.track_size,
                bands[id].start(),
                bands[id].bandwidth,
            );
            FacetCellPlacement {
                cell_index: id,
                main_start,
                main_size: solved.track_size,
            }
        })
        .collect();

    Ok(FacetBandPlacement::new(
        axis,
        children,
        main_extent,
        cross_extent,
    ))
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

fn facet_render_cell_size(
    axis: FacetAxis,
    band_size: f32,
    cell: &FacetCellRuntime,
) -> avenger_layout::Size {
    match axis {
        FacetAxis::Column => avenger_layout::Size::new(
            band_size.max(0.0),
            cell.measurement.plot_area_height.max(0.0),
        ),
        FacetAxis::Row => avenger_layout::Size::new(
            cell.measurement.plot_area_width.max(0.0),
            band_size.max(0.0),
        ),
    }
}

fn tracks_full_rect(tracks: &avenger_layout::SolvedTracks) -> Option<avenger_layout::Rect> {
    tracks.content_rect_for_slot(avenger_layout::GridSlot {
        row: 0,
        column: 0,
        row_span: tracks.row_sizes.len(),
        column_span: tracks.column_sizes.len(),
    })
}

fn facet_render_cell_slot(axis: FacetAxis, index: usize) -> avenger_layout::GridSlot {
    match axis {
        FacetAxis::Column => avenger_layout::GridSlot {
            row: 0,
            column: index,
            row_span: 1,
            column_span: 1,
        },
        FacetAxis::Row => avenger_layout::GridSlot {
            row: index,
            column: 0,
            row_span: 1,
            column_span: 1,
        },
    }
}

fn facet_cell_cross_axis_offset(facet_band: &FacetBandCoordMeasurement) -> (f32, f32) {
    let slabs = FacetOverflowSlabs::from_coordinated(facet_band.active_overflow());
    match facet_band.axis {
        FacetAxis::Column => (0.0, slabs.legend.top),
        FacetAxis::Row => (slabs.legend.left, 0.0),
    }
}

pub(crate) fn facet_child_frame_regions_from_band_layout(
    facet_band: &FacetBandCoordMeasurement,
    placement: &FacetBandPlacement,
    plot_width: f32,
    plot_height: f32,
) -> Result<(Size, Vec<ChildFrameRegion>), AvengerChartError> {
    if placement.axis != facet_band.axis {
        return Err(AvengerChartError::InternalError(format!(
            "Facet child-frame layout axis mismatch: placement={:?}, measurement={:?}",
            placement.axis, facet_band.axis
        )));
    }
    if placement.cell_count() != facet_band.cells.len() {
        return Err(AvengerChartError::InternalError(format!(
            "Facet child-frame layout count mismatch: placement={}, cells={}",
            placement.cell_count(),
            facet_band.cells.len()
        )));
    }

    let mut children = Vec::with_capacity(facet_band.cells.len());
    let mut main_sizes = Vec::with_capacity(facet_band.cells.len());
    for (idx, cell_placement) in placement.cells.iter().enumerate() {
        if cell_placement.cell_index != idx {
            return Err(AvengerChartError::InternalError(format!(
                "Facet child-frame layout cell index mismatch: expected={idx}, actual={}",
                cell_placement.cell_index
            )));
        }
        let cell = facet_band.cells.get(idx).ok_or_else(|| {
            AvengerChartError::InternalError(format!("Missing facet cell for index {idx}"))
        })?;
        main_sizes.push(cell_placement.main_size.max(0.0));
        children.push(Layout::<usize, ()>::leaf(facet_render_cell_size(
            facet_band.axis,
            cell_placement.main_size,
            cell,
        )));
    }

    let layout = facet_band.active_layout();
    let spacing = Spacing {
        outer_start: layout.outer_start,
        outer_end: layout.outer_end,
        min_gap: layout.padding_inner_px,
    };
    let fixed_tracks = main_sizes
        .iter()
        .copied()
        .map(TrackSize::Fixed)
        .collect::<Vec<_>>();
    let render_layout = match facet_band.axis {
        FacetAxis::Column => Layout::row(children)
            .column_spacing(spacing)
            .columns(fixed_tracks)
            .distribute_x(Distribute::Start),
        FacetAxis::Row => Layout::column(children)
            .row_spacing(spacing)
            .rows(fixed_tracks)
            .distribute_y(Distribute::Start),
    };
    let solution = render_layout
        .solve(&SolveOptions {
            width: Some(plot_width.max(0.0)),
            height: Some(plot_height.max(0.0)),
        })
        .map_err(|error| {
            AvengerChartError::InternalError(format!(
                "Facet child-frame layout solve failed: {error}"
            ))
        })?;
    let root = solution.regions().next().ok_or_else(|| {
        AvengerChartError::InternalError("Facet child-frame layout produced no root region".into())
    })?;
    let RegionDetail::Grid { tracks } = &root.detail else {
        return Err(AvengerChartError::InternalError(
            "Facet child-frame layout root was not a grid".into(),
        ));
    };
    let full_rect = tracks_full_rect(tracks).unwrap_or(root.content);
    let (offset_x, offset_y) = facet_cell_cross_axis_offset(facet_band);
    let regions = (0..facet_band.cells.len())
        .map(|idx| {
            let rect = tracks
                .content_rect_for_slot(facet_render_cell_slot(facet_band.axis, idx))
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Facet child-frame layout missing track rect for cell index {idx}"
                    ))
                })?;
            Ok(ChildFrameRegion {
                child_index: idx,
                content: LayoutBounds {
                    x: rect.x + offset_x,
                    y: rect.y + offset_y,
                    width: rect.width,
                    height: rect.height,
                },
                slot: LayoutBounds {
                    x: rect.x + offset_x,
                    y: rect.y + offset_y,
                    width: rect.width,
                    height: rect.height,
                },
                content_size_override: None,
                edge_targets: None,
            })
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    Ok((Size::new(full_rect.width, full_rect.height), regions))
}

pub(crate) fn facet_band_positions_from_band_layout(
    facet_band: &FacetBandCoordMeasurement,
    placement: &FacetBandPlacement,
    plot_width: f32,
    plot_height: f32,
) -> Result<Vec<BandPosition>, AvengerChartError> {
    let (_, regions) =
        facet_child_frame_regions_from_band_layout(facet_band, placement, plot_width, plot_height)?;
    regions
        .into_iter()
        .map(|region| {
            let cell = facet_band.cells.get(region.child_index).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Facet child-frame layout missing cell for band position {}",
                    region.child_index
                ))
            })?;
            let (start, bandwidth) = match facet_band.axis {
                FacetAxis::Column => (region.content.x, region.content.width),
                FacetAxis::Row => (region.content.y, region.content.height),
            };
            Ok(BandPosition::new(cell.plan.value.clone(), start, bandwidth))
        })
        .collect()
}

pub(crate) fn compute_explicit_facet_band_placement(
    axis: FacetAxis,
    cells: &[FacetCellRuntime],
    layout: &CoordinatedLayout,
) -> FacetBandPlacement {
    if cells.is_empty() {
        return FacetBandPlacement::new(axis, Vec::new(), 0.0, Some(0.0));
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
            "plot-area-sized facet placement gap"
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

    // Only the real cells become placements; ghost tracks reserve space.
    let placed_cells = (0..cells.len())
        .map(|cell_index| FacetCellPlacement {
            cell_index,
            main_start: main_starts[cell_index],
            main_size: main_sizes[cell_index],
        })
        .collect();

    FacetBandPlacement::new(axis, placed_cells, main_extent, Some(cross_extent))
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

        let starts = placement.main_starts().collect::<Vec<_>>();
        let sizes = placement.main_sizes().collect::<Vec<_>>();
        assert_eq!(placement.axis, FacetAxis::Column);
        assert_eq!(placement.cell_count(), 2);
        assert!(starts[0] < starts[1]);
        assert!(sizes.iter().all(|size| *size > 0.0));
        assert_eq!(placement.cross_extent, Some(42.0));
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
    fn scale_backed_placement_allows_empty_filtered_cells() {
        let scale = make_band_scale((0.0, 100.0));

        let placement = resolve_scale_backed_facet_band_placement(
            FacetAxis::Column,
            &scale,
            &[],
            0,
            Some(42.0),
        )
        .unwrap();

        assert_eq!(placement.axis, FacetAxis::Column);
        assert_eq!(placement.cell_count(), 0);
        assert_eq!(placement.main_extent, 0.0);
        assert_eq!(placement.cross_extent, Some(42.0));
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
