//! Real-tree facet lowering and the shadow census (solver unification P5).
//!
//! Lowers the LIVE facet measurement tree — real nested topology, real cell
//! plot sizes, epoch-frozen overflow envelopes — into one
//! `avenger_layout::Layout` and solves it, so a single solve produces both
//! the coordination channel values (per-node `Region.coordinated` edges,
//! solved track spacing) and every cell's slot geometry.
//!
//! In P5 this runs as an env-gated SHADOW (`AVENGER_SHADOW_TREE_SOLVE=1`):
//! per coordination run it solves the shadow tree and REPORTS deltas
//! against the legacy pipeline's post-everything values, without changing
//! behavior. The census across the visual suite prices Phases 6–7 (which
//! adopt the solve's channels and geometry) and validates the lowering
//! rules:
//!
//! - Leaf cells lower at their live plot sizes with
//!   `EdgeDemand::from_inner_and_envelope` per side from the epoch
//!   envelopes (renderable-index aligned, like the band envelope fold).
//! - Nested-band cells lower as nested grids inside a 1×1 chrome wrapper
//!   carrying the layered RESIDUAL between the parent's epoch cell
//!   envelope and the child band's own epoch envelope (inner = guide
//!   residual, outer = legend residual). The wrapper keeps the nested
//!   band's own `Region.coordinated` chrome-free for channel reads.
//! - Cross-band cousins share `coordination_scope_key_for_depth` keys in
//!   ONE tree; `uniform_*` is set per band axis iff every cell is a leaf
//!   (uniform equalization is a no-op there and buys the ragged-tolerant
//!   spacing merge); ghost slots pad to the active slot count with the
//!   trailing edge cell's demand mirrored onto the last ghost
//!   (renderable-edge law).
//! - Spacing lowers RAW (`padding_inner_px` as `min_gap`, like the legacy
//!   round lowering) so the layout channel stays comparable; the
//!   placement-time `main_axis_gap` floor is a known, classified
//!   geometry-side divergence the census reports.
//! - Canvas-constrained axes constrain the root `SolveOptions` at the
//!   facet root's plot area; leaf-plot-sized axes solve content-driven.
//!
//! Channel reads use `Region.coordinated` (the node's own post-share ask;
//! `granted` would fold in unrelated siblings) — guide from `.inner`,
//! total from `.total`, which reproduces the legacy cross-cousin lift.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use avenger_chart_core::{CoordinatedLayout, CoordinatedOverflow, FacetAxis};
use avenger_layout::{
    EdgeDemand, Edges, Layout, LayoutSolution, RegionDetail, Side, Size, SolveOptions, Spacing,
};
use tracing::{debug, info, warn};

use crate::facet::coord::{FacetBandCoordMeasurement, renderable_for_empty_policy};
use crate::facet::coordination_plans::CoordinationNodeKey;
use crate::facet::coordination_policy::FacetCoordinationPolicy;
use crate::plot::compiled::{ComponentsMeasurement, CoordinationScopeKey};
use crate::render::context::FacetRuntimeSizingMode;

/// Whether the shadow census is enabled for this process.
pub(crate) fn shadow_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var("AVENGER_SHADOW_TREE_SOLVE").is_ok_and(|value| value == "1"))
}

/// One lowered facet band's bookkeeping (walk order).
pub(crate) struct LoweredBand {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) key: CoordinationScopeKey,
    pub(crate) axis: FacetAxis,
    /// Real cell count (before ghost padding).
    pub(crate) cell_count: usize,
    pub(crate) guide_slot_gap_px: f32,
    pub(crate) local_n: usize,
    pub(crate) has_overflow_cells: bool,
}

/// The retained lowering: the tree (re-solvable at new envelopes — the
/// remap law) plus per-band records for channel extraction.
pub(crate) struct LoweredFacetTree {
    pub(crate) layout: Layout<CoordinationNodeKey, CoordinationScopeKey>,
    pub(crate) root_options: SolveOptions,
    pub(crate) bands: Vec<LoweredBand>,
}

impl LoweredFacetTree {
    pub(crate) fn solve(&self) -> Result<LayoutSolution<CoordinationNodeKey>, String> {
        self.layout
            .solve(&self.root_options)
            .map_err(|error| error.to_string())
    }
}

/// Per-cell size overrides for the idempotence probe, keyed by
/// (band node, cell index).
pub(crate) type CellSizeOverrides = HashMap<(CoordinationNodeKey, usize), Size>;

/// Lower the facet tree rooted in `measurement`. Returns `None` when the
/// measurement holds no facet band.
pub(crate) fn lower_facet_tree(
    measurement: &ComponentsMeasurement,
    sizing: FacetRuntimeSizingMode,
    overrides: Option<&CellSizeOverrides>,
) -> Option<LoweredFacetTree> {
    let root_band = FacetCoordinationPolicy::facet_band_ref(measurement)?;
    let root_band = root_band.base();
    let policy = sizing.policy();

    // Physical-axis mode: an axis is CONTENT-DRIVEN when any band whose
    // main axis is that physical axis uses explicit placement (wrap and
    // plot-area-sized bands realize leaf-derived extents — the canvas
    // constrains the wrap COUNT upstream, never the cell sizes), else by
    // the runtime sizing policy. Content-driven axes keep natural track
    // sizes (free space trails); constrained axes distribute it.
    let mut x_explicit = false;
    let mut y_explicit = false;
    {
        let mut walk_path = Vec::new();
        crate::facet::coordination_apply::visit_facet_bands_with_node_id(
            measurement,
            0,
            &mut walk_path,
            &mut |_node_id, _depth, facet_band| {
                let base = facet_band.base();
                if base.uses_explicit_placement() {
                    match base.axis {
                        FacetAxis::Column => x_explicit = true,
                        FacetAxis::Row => y_explicit = true,
                    }
                }
            },
        );
    }
    let x_content_driven = x_explicit || policy.width.is_leaf_plot_area_sized();
    let y_content_driven = y_explicit || policy.height.is_leaf_plot_area_sized();

    let mut bands = Vec::new();
    let mut node_path = Vec::new();
    let mut layout_path = Vec::new();
    let layout = lower_band(
        root_band,
        &mut node_path,
        &mut layout_path,
        0,
        (x_content_driven, y_content_driven),
        overrides,
        &mut bands,
    );

    // Root constraints: a constrained physical axis pins the root
    // envelope at the facet root's available plot area; a content-driven
    // axis solves naturally.
    let width = (!x_content_driven).then_some(measurement.plot_area_width.max(0.0));
    let height = (!y_content_driven).then_some(measurement.plot_area_height.max(0.0));

    Some(LoweredFacetTree {
        layout,
        root_options: SolveOptions { width, height },
        bands,
    })
}

/// Map a band's per-cell epoch envelopes (renderable cells only, plus the
/// none-renderable fallback) onto ALL cell indices; non-renderable cells
/// get `None` (zero demands).
fn cell_envelopes_by_index(
    band: &FacetBandCoordMeasurement,
) -> Vec<
    Option<(
        avenger_chart_core::OverflowSpaceRequirement,
        avenger_chart_core::OverflowSpaceRequirement,
    )>,
> {
    let mut by_index = vec![None; band.cells.len()];
    let Some(envelopes) = band.overflow_cell_envelopes() else {
        return by_index;
    };
    let renderable_indices: Vec<usize> = band
        .cells
        .iter()
        .enumerate()
        .filter_map(|(idx, cell)| {
            renderable_for_empty_policy(band.empty_cell_policy, !cell.plan.has_data_rows)
                .then_some(idx)
        })
        .collect();
    let targets: Vec<usize> = if renderable_indices.is_empty() {
        if band.cells.is_empty() {
            Vec::new()
        } else {
            // The none-renderable fallback law: the first cell carries the
            // band's envelope.
            vec![0]
        }
    } else {
        renderable_indices
    };
    for (envelope, index) in envelopes.iter().zip(targets) {
        by_index[index] = Some(envelope.clone());
    }
    by_index
}

fn demand_edges_from_envelope(
    guide: &avenger_chart_core::OverflowSpaceRequirement,
    total: &avenger_chart_core::OverflowSpaceRequirement,
) -> Edges<EdgeDemand> {
    Edges::new(
        EdgeDemand::from_inner_and_envelope(guide.top, total.top),
        EdgeDemand::from_inner_and_envelope(guide.right, total.right),
        EdgeDemand::from_inner_and_envelope(guide.bottom, total.bottom),
        EdgeDemand::from_inner_and_envelope(guide.left, total.left),
    )
}

fn apply_demands(
    mut node: Layout<CoordinationNodeKey, CoordinationScopeKey>,
    demands: &Edges<EdgeDemand>,
) -> Layout<CoordinationNodeKey, CoordinationScopeKey> {
    for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
        node = node.demand(side, *demands.side(side));
    }
    node
}

fn lower_band(
    band: &FacetBandCoordMeasurement,
    node_path: &mut Vec<usize>,
    layout_path: &mut Vec<usize>,
    depth: usize,
    content_driven: (bool, bool),
    overrides: Option<&CellSizeOverrides>,
    bands: &mut Vec<LoweredBand>,
) -> Layout<CoordinationNodeKey, CoordinationScopeKey> {
    let node_id = CoordinationNodeKey::new(node_path.clone());
    let key = band.coordination_scope_key_for_depth(depth);
    let envelopes = cell_envelopes_by_index(band);

    // Band-level chrome: the legend slabs realized as band-owned (hoisted
    // legends and parent-allocation-owned slabs) — space the band's cells
    // do not carry. measured_overflow covers rendered CHILDREN only, so
    // the owned slabs are the band-level slice; they ride a wrapper (see
    // below) to keep the band grid's coordinated edges cells-only.
    let owned_slabs =
        band.owned_legend_slabs_for_overflow(band.active_boundary_overflow(), band.active_layout());
    let chrome_inner = [0.0f32; 4]; // top, right, bottom, left
    let chrome_outer = [
        owned_slabs.top.max(0.0),
        owned_slabs.right.max(0.0),
        owned_slabs.bottom.max(0.0),
        owned_slabs.left.max(0.0),
    ];

    let mut all_leaves = true;
    let mut cell_nodes: Vec<Layout<CoordinationNodeKey, CoordinationScopeKey>> = Vec::new();
    for (idx, cell) in band.cells.iter().enumerate() {
        let cell_envelope = envelopes.get(idx).cloned().flatten();
        let nested = FacetCoordinationPolicy::facet_band_ref(&cell.measurement);
        let node = if let Some(nested) = nested {
            all_leaves = false;
            let nested_band = nested.base();
            let child_epoch = nested_band
                .measured_overflow_value()
                .unwrap_or_else(CoordinatedOverflow::default);
            node_path.push(idx);
            layout_path.push(idx);
            // The wrapper (1×1 grid) carries the layered residual between
            // the parent's epoch cell envelope and the child band's own
            // epoch envelope, keeping the child's coordinated edges
            // chrome-free for channel reads.
            layout_path.push(0);
            let child = lower_band(
                nested_band,
                node_path,
                layout_path,
                depth + 1,
                content_driven,
                overrides,
                bands,
            );
            layout_path.pop();
            node_path.pop();
            layout_path.pop();
            let mut wrapper = Layout::row(vec![child]);
            if let Some((cell_guide, cell_total)) = &cell_envelope {
                for (side, cell_g, cell_t, band_g, band_t) in [
                    (
                        Side::Top,
                        cell_guide.top,
                        cell_total.top,
                        child_epoch.guide.top,
                        child_epoch.total.top,
                    ),
                    (
                        Side::Right,
                        cell_guide.right,
                        cell_total.right,
                        child_epoch.guide.right,
                        child_epoch.total.right,
                    ),
                    (
                        Side::Bottom,
                        cell_guide.bottom,
                        cell_total.bottom,
                        child_epoch.guide.bottom,
                        child_epoch.total.bottom,
                    ),
                    (
                        Side::Left,
                        cell_guide.left,
                        cell_total.left,
                        child_epoch.guide.left,
                        child_epoch.total.left,
                    ),
                ] {
                    let guide_residual = (cell_g - band_g).max(0.0);
                    let legend_residual = ((cell_t - cell_g) - (band_t - band_g)).max(0.0);
                    wrapper = wrapper
                        .inner(side, guide_residual)
                        .outer(side, legend_residual);
                }
            }
            wrapper
        } else {
            let size = overrides
                .and_then(|map| map.get(&(node_id.clone(), idx)).copied())
                .unwrap_or_else(|| {
                    Size::new(
                        cell.measurement.plot_area_width.max(0.0),
                        cell.measurement.plot_area_height.max(0.0),
                    )
                });
            let leaf = Layout::leaf(size);
            match &cell_envelope {
                Some((guide, total)) => {
                    apply_demands(leaf, &demand_edges_from_envelope(guide, total))
                }
                None => leaf,
            }
        };
        cell_nodes.push(node);
    }

    // Ghost slots pad to the active slot count (free bands keep local n
    // through the active view) floored by the structural minimum —
    // min_slot_count holds trailing wrap holes so a short chunk's cells
    // stay uniform instead of stretching across the hole. The last ghost
    // mirrors the trailing edge cell's demands so the band's structural
    // trailing edge keeps the renderable-edge law.
    let cell_count = cell_nodes.len();
    let slot_count = band
        .active_layout()
        .n
        .max(band.min_slot_count)
        .max(cell_count)
        .max(1);
    if shadow_enabled() {
        debug!(
            target: "avenger_chart::facet::tree_solve",
            node = ?node_id.path,
            cell_count,
            slot_count,
            active_n = band.active_layout().n,
            local_n = band.local_layout.n,
            min_slot_count = band.min_slot_count,
            explicit = band.uses_explicit_placement(),
            content_driven_x = content_driven.0,
            content_driven_y = content_driven.1,
            "lowered band slots"
        );
    }
    if slot_count > cell_count && cell_count > 0 {
        let fallback = Size::new(
            band.cells
                .iter()
                .map(|cell| cell.measurement.plot_area_width)
                .fold(0.0f32, f32::max)
                .max(0.0),
            band.cells
                .iter()
                .map(|cell| cell.measurement.plot_area_height)
                .fold(0.0f32, f32::max)
                .max(0.0),
        );
        let trailing_envelope = envelopes.iter().rev().find_map(|e| e.clone());
        for ghost_index in cell_count..slot_count {
            let mut ghost = Layout::leaf(fallback);
            if ghost_index + 1 == slot_count {
                if let Some((guide, total)) = &trailing_envelope {
                    let trailing_side = match band.axis {
                        FacetAxis::Column => Side::Right,
                        FacetAxis::Row => Side::Bottom,
                    };
                    ghost = ghost.demand(
                        trailing_side,
                        EdgeDemand::from_inner_and_envelope(
                            *match band.axis {
                                FacetAxis::Column => &guide.right,
                                FacetAxis::Row => &guide.bottom,
                            },
                            *match band.axis {
                                FacetAxis::Column => &total.right,
                                FacetAxis::Row => &total.bottom,
                            },
                        ),
                    );
                }
            }
            cell_nodes.push(ghost);
        }
    }
    if cell_nodes.is_empty() {
        // Cell-less bands keep one zero placeholder leaf (the zero
        // envelope), as the legacy round lowering did.
        cell_nodes.push(Layout::leaf(Size::default()));
    }

    // Spacing lowers RAW local declarations; the share merge raises them
    // across cousins exactly like the legacy round solve.
    let spacing = Spacing {
        outer_start: band.local_layout.outer_start,
        outer_end: band.local_layout.outer_end,
        min_gap: band.local_layout.padding_inner_px,
    };

    let mut grid = match band.axis {
        FacetAxis::Column => {
            let mut grid = Layout::row(cell_nodes).column_spacing(spacing);
            if all_leaves {
                grid = grid.uniform_columns();
            }
            grid
        }
        FacetAxis::Row => {
            let mut grid = Layout::column(cell_nodes).row_spacing(spacing);
            if all_leaves {
                grid = grid.uniform_rows();
            }
            grid
        }
    };
    // Free space follows the physical axis mode: a constrained axis
    // distributes it across tracks (the equal-share law); a
    // content-driven axis keeps natural track sizes and lets free space
    // trail (the legacy never stretches content-realized extents).
    let distribute_for = |is_content_driven: bool| {
        if is_content_driven {
            avenger_layout::Distribute::Start
        } else {
            avenger_layout::Distribute::StretchTracks
        }
    };
    grid = grid
        .distribute_x(distribute_for(content_driven.0))
        .distribute_y(distribute_for(content_driven.1));
    grid = grid.share(key.clone()).id(node_id.clone());

    // Band chrome rides a wrapper so the band grid's own coordinated
    // edges stay cells-only (channel parity with the legacy round, which
    // lowered cell envelopes alone); the wrapper still lifts the chrome
    // toward the parent and consumes root width on constrained axes.
    let has_chrome = chrome_inner.iter().any(|v| *v > 0.0) || chrome_outer.iter().any(|v| *v > 0.0);
    if has_chrome {
        let mut wrapper = Layout::row(vec![grid]);
        for (slot, side) in [
            (0usize, Side::Top),
            (1, Side::Right),
            (2, Side::Bottom),
            (3, Side::Left),
        ] {
            if chrome_inner[slot] > 0.0 {
                wrapper = wrapper.inner(side, chrome_inner[slot]);
            }
            if chrome_outer[slot] > 0.0 {
                wrapper = wrapper.outer(side, chrome_outer[slot]);
            }
        }
        grid = wrapper;
    }

    bands.push(LoweredBand {
        node_id,
        key,
        axis: band.axis,
        cell_count,
        guide_slot_gap_px: band.local_layout.guide_slot_gap_px,
        local_n: band.local_layout.n,
        has_overflow_cells: band.overflow_cells.is_some(),
    });

    grid
}

/// Channel values extracted from one real-tree solve, shaped like the
/// legacy `SolvedRound` maps for comparison (and, in P6, replacement).
pub(crate) struct TreeChannels {
    pub(crate) layout_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
    pub(crate) overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
}

/// Extract the layout + overflow channels from a solved real tree.
///
/// - Spacing comes from the band grid's solved tracks (post share-merge);
///   the `n` and `guide_slot_gap_px` scalars fold chart-side over the
///   share group exactly like the legacy round's PRE-adjustment merge
///   (`SolvedRound.merged_by_node`). The write-back adjustments (free-n
///   reversion, lane-gap fold, global-edge outer reversion) stay in
///   `build_round_solution`, which in P6 runs on top of these values.
/// - Overflow comes from the band's `Region.coordinated` edges: guide
///   from `.inner`, total from `.total` (the cross-cousin lift).
pub(crate) fn extract_channels(
    lowered: &LoweredFacetTree,
    solution: &LayoutSolution<CoordinationNodeKey>,
) -> TreeChannels {
    // Group scalar folds over share-key membership.
    let mut group_n: HashMap<&CoordinationScopeKey, usize> = HashMap::new();
    let mut group_gap: HashMap<&CoordinationScopeKey, f32> = HashMap::new();
    for band in &lowered.bands {
        let n = group_n.entry(&band.key).or_default();
        *n = (*n).max(band.local_n);
        let gap = group_gap.entry(&band.key).or_default();
        *gap = gap.max(band.guide_slot_gap_px);
    }

    let mut layout_by_node = HashMap::new();
    let mut overflow_by_node = HashMap::new();
    for band in &lowered.bands {
        let Some(region) = solution.region(&band.node_id) else {
            continue;
        };
        let RegionDetail::Grid { tracks } = &region.detail else {
            continue;
        };
        let spacing = match band.axis {
            FacetAxis::Column => tracks.column_spacing,
            FacetAxis::Row => tracks.row_spacing,
        };
        let n = group_n.get(&band.key).copied().unwrap_or(band.local_n);
        layout_by_node.insert(
            band.node_id.clone(),
            CoordinatedLayout {
                padding_inner_px: spacing.min_gap,
                guide_slot_gap_px: group_gap
                    .get(&band.key)
                    .copied()
                    .unwrap_or(band.guide_slot_gap_px),
                outer_start: spacing.outer_start,
                outer_end: spacing.outer_end,
                n,
            },
        );
        if band.has_overflow_cells {
            let (main_lead, main_trail) = match band.axis {
                FacetAxis::Column => (Side::Left, Side::Right),
                FacetAxis::Row => (Side::Top, Side::Bottom),
            };
            let _ = (main_lead, main_trail);
            let coordinated = &region.coordinated;
            overflow_by_node.insert(
                band.node_id.clone(),
                CoordinatedOverflow {
                    guide: avenger_chart_core::OverflowSpaceRequirement {
                        top: coordinated.top.inner,
                        right: coordinated.right.inner,
                        bottom: coordinated.bottom.inner,
                        left: coordinated.left.inner,
                    },
                    total: avenger_chart_core::OverflowSpaceRequirement {
                        top: coordinated.top.total,
                        right: coordinated.right.total,
                        bottom: coordinated.bottom.total,
                        left: coordinated.left.total,
                    },
                },
            );
        }
    }
    TreeChannels {
        layout_by_node,
        overflow_by_node,
    }
}

const SHADOW_EPS: f32 = 0.01;

fn overflow_delta(a: &CoordinatedOverflow, b: &CoordinatedOverflow) -> f32 {
    let side = |x: f32, y: f32| (x - y).abs();
    side(a.guide.top, b.guide.top)
        .max(side(a.guide.right, b.guide.right))
        .max(side(a.guide.bottom, b.guide.bottom))
        .max(side(a.guide.left, b.guide.left))
        .max(side(a.total.top, b.total.top))
        .max(side(a.total.right, b.total.right))
        .max(side(a.total.bottom, b.total.bottom))
        .max(side(a.total.left, b.total.left))
}

fn layout_delta(a: &CoordinatedLayout, b: &CoordinatedLayout) -> f32 {
    (a.padding_inner_px - b.padding_inner_px)
        .abs()
        .max((a.outer_start - b.outer_start).abs())
        .max((a.outer_end - b.outer_end).abs())
        .max((a.guide_slot_gap_px - b.guide_slot_gap_px).abs())
        .max(a.n.abs_diff(b.n) as f32)
}

/// Run the shadow census for one coordination run: solve the real tree
/// from the post-everything measurement state and report channel +
/// geometry deltas against the legacy pipeline's settled values.
pub(crate) fn run_shadow_census(
    measurement: &ComponentsMeasurement,
    sizing: FacetRuntimeSizingMode,
    snapshot_hashes: &[u64],
) {
    let Some(lowered) = lower_facet_tree(measurement, sizing, None) else {
        return;
    };
    // The legacy comparison target is the diagonal round solve's
    // PRE-adjustment maps — the exact seam P6 replaces. A fresh snapshot
    // here equals the rounds' snapshots (fact 2; the identity probe
    // reports on it).
    let snapshot = crate::facet::coordination::collect_requirement_snapshot(measurement);
    let legacy = crate::facet::round_tree::solve_round(&snapshot.nodes);
    let solved = match lowered.solve() {
        Ok(solved) => solved,
        Err(error) => {
            warn!(target: "avenger_chart::facet::tree_solve", error, "shadow solve failed");
            return;
        }
    };
    let channels = extract_channels(&lowered, &solved);

    // Channel comparison vs the installed (last-pass) legacy solution.
    let mut layout_max = 0.0f32;
    let mut layout_nodes_over = 0usize;
    let mut overflow_max = 0.0f32;
    let mut overflow_nodes_over = 0usize;
    for band in &lowered.bands {
        if let (Some(mine), Some(legacy)) = (
            channels.layout_by_node.get(&band.node_id),
            legacy.merged_by_node.get(&band.node_id),
        ) {
            let delta = layout_delta(mine, legacy);
            if delta > SHADOW_EPS {
                layout_nodes_over += 1;
                debug!(
                    target: "avenger_chart::facet::tree_solve",
                    node = ?band.node_id.path,
                    delta,
                    mine = ?mine,
                    legacy = ?legacy,
                    "shadow layout channel divergence"
                );
            }
            layout_max = layout_max.max(delta);
        }
        if let (Some(mine), Some(legacy)) = (
            channels.overflow_by_node.get(&band.node_id),
            legacy.overflow_by_node.get(&band.node_id),
        ) {
            let delta = overflow_delta(mine, legacy);
            if delta > SHADOW_EPS {
                overflow_nodes_over += 1;
                debug!(
                    target: "avenger_chart::facet::tree_solve",
                    node = ?band.node_id.path,
                    delta,
                    mine = ?mine,
                    legacy = ?legacy,
                    "shadow overflow channel divergence"
                );
                if let Some(region) = solved.region(&band.node_id) {
                    debug!(
                        target: "avenger_chart::facet::tree_solve",
                        node = ?band.node_id.path,
                        requested_top = ?region.requested.top,
                        coordinated_top = ?region.coordinated.top,
                        granted_top = ?region.granted.top,
                        "shadow overflow divergence detail"
                    );
                }
            }
            overflow_max = overflow_max.max(delta);
        }
    }

    // Geometry comparison: each band's cell slots vs the live
    // (post-retarget/final-prop) cell plot areas.
    let mut geometry_max = 0.0f32;
    let mut geometry_cells_over = 0usize;
    let mut geometry_cells = 0usize;
    let mut walk_path = Vec::new();
    let mut band_index_by_id: HashMap<&CoordinationNodeKey, &LoweredBand> = HashMap::new();
    for band in &lowered.bands {
        band_index_by_id.insert(&band.node_id, band);
    }
    crate::facet::coordination_apply::visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut walk_path,
        &mut |node_id, _depth, facet_band| {
            let Some(band) = band_index_by_id.get(node_id) else {
                return;
            };
            let Some(band_region) = solved.region(&band.node_id) else {
                return;
            };
            let base = facet_band.base();
            for (idx, cell) in base.cells.iter().enumerate() {
                let mut child_path = band_region.path.clone();
                child_path.push(idx);
                let Some(cell_region) = solved.at_path(&child_path) else {
                    continue;
                };
                // Nested cells: the wrapper's slot vs the subtree extent
                // would need the child's own band extent; compare leaf
                // cells only (nested geometry is the adoption substrate's
                // re-solve domain, classified separately).
                if FacetCoordinationPolicy::facet_band_ref(&cell.measurement).is_some() {
                    continue;
                }
                geometry_cells += 1;
                let live_w = cell.measurement.plot_area_width.max(0.0);
                let live_h = cell.measurement.plot_area_height.max(0.0);
                let delta = (cell_region.slot.width - live_w)
                    .abs()
                    .max((cell_region.slot.height - live_h).abs());
                if delta > SHADOW_EPS {
                    geometry_cells_over += 1;
                    debug!(
                        target: "avenger_chart::facet::tree_solve",
                        node = ?node_id.path,
                        cell = idx,
                        slot_w = cell_region.slot.width,
                        slot_h = cell_region.slot.height,
                        live_w,
                        live_h,
                        "shadow cell slot divergence"
                    );
                    if idx == 0 {
                        let owned = base.owned_legend_slabs_for_overflow(
                            base.active_boundary_overflow(),
                            base.active_layout(),
                        );
                        let boundary = base.active_boundary_overflow();
                        let (paw, pah) = base.plot_area_extent();
                        debug!(
                            target: "avenger_chart::facet::tree_solve",
                            node = ?node_id.path,
                            band_slot_w = band_region.slot.width,
                            band_content_w = band_region.content.width,
                            live_band_w = paw,
                            live_band_h = pah,
                            owned = ?owned,
                            boundary_guide_right = boundary.guide.right,
                            boundary_total_right = boundary.total.right,
                            boundary_guide_left = boundary.guide.left,
                            boundary_total_left = boundary.total.left,
                            outer_start = base.active_layout().outer_start,
                            outer_end = base.active_layout().outer_end,
                            "shadow band geometry context"
                        );
                    }
                }
                geometry_max = geometry_max.max(delta);
            }
        },
    );

    // Idempotence probe: re-lower with the shadow's own leaf slots as
    // cell sizes and re-solve; a converged lowering changes nothing.
    let mut overrides: CellSizeOverrides = HashMap::new();
    for band in &lowered.bands {
        let Some(band_region) = solved.region(&band.node_id) else {
            continue;
        };
        for idx in 0..band.cell_count {
            let mut child_path = band_region.path.clone();
            child_path.push(idx);
            if let Some(cell_region) = solved.at_path(&child_path) {
                if matches!(cell_region.detail, RegionDetail::Leaf) {
                    overrides.insert(
                        (band.node_id.clone(), idx),
                        Size::new(cell_region.slot.width, cell_region.slot.height),
                    );
                }
            }
        }
    }
    let idempotence_delta = lower_facet_tree(measurement, sizing, Some(&overrides))
        .and_then(|relowered| relowered.solve().ok())
        .map(|resolved| solved.content_delta(&resolved))
        .unwrap_or(f32::INFINITY);

    let snapshots_identical = snapshot_hashes.windows(2).all(|pair| pair[0] == pair[1]);

    info!(
        target: "avenger_chart::facet::tree_solve",
        bands = lowered.bands.len(),
        layout_max,
        layout_nodes_over,
        overflow_max,
        overflow_nodes_over,
        geometry_cells,
        geometry_max,
        geometry_cells_over,
        idempotence_delta,
        snapshots_identical,
        snapshot_rounds = snapshot_hashes.len(),
        "shadow tree-solve census"
    );
}

/// Hash one requirement snapshot for the round-identity probe (fact 2:
/// round-2 snapshots should be input-identical to round-1).
pub(crate) fn snapshot_hash(
    snapshot: &crate::facet::coordination_plans::RequirementSnapshot,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{snapshot:?}").hash(&mut hasher);
    hasher.finish()
}
