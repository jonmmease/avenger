//! Real-tree facet lowering: the production source of the coordination
//! requirement channels (`tree_solved_round`) and current child-frame
//! geometry (`CurrentFacetGeometry`), plus an env-gated shadow census
//! (`AVENGER_SHADOW_TREE_SOLVE=1`) reporting slot-vs-live geometry deltas
//! plus the idempotence, adoption, and content-driven geometry probes (the standing
//! equilibrium check: adoption must land the tree where settled slots equal
//! live geometry).
//!
//! Lowers the LIVE facet measurement tree — real nested topology, real cell
//! plot sizes, epoch-frozen overflow envelopes — into one
//! `avenger_layout::Layout` and solves it, so a single solve produces both
//! the coordination channel values (per-node `Region.coordinated` edges,
//! solved track spacing) and every cell's slot geometry. Lowering rules:
//!
//! - Leaf cells lower at their live plot sizes with
//!   `EdgeDemand::from_guide_and_envelope` per side from the epoch
//!   envelopes (renderable-index aligned, like the band envelope fold).
//! - Nested-band cells lower behind a TWO-WRAPPER boundary (the channel
//!   contract law, pinned by
//!   `nested_boundary_two_wrapper_overrides_child_classification`): a
//!   band's channel edges are its OWN epoch cell folds, INCLUDING the
//!   parent level's guide/legend classification, which legitimately
//!   differs from the child's structural layering. A CONTAINED
//!   (`SolveFor::Content`) wrapper zeroes the child's structural lift
//!   toward the parent, and an Envelope wrapper around it declares the
//!   cell's FULL epoch envelope as layered chrome — per-layer residual
//!   chrome would double-count reclassified space.
//! - Cross-band cousins share `coordination_scope_key_for_depth` keys in
//!   ONE tree; `uniform_*` is set per band axis iff every cell is a leaf
//!   (uniform equalization is a no-op there and buys the ragged-tolerant
//!   spacing merge); ghost slots pad to the active slot count, floored by
//!   `min_slot_count`, with the trailing edge cell's demand mirrored onto
//!   the last ghost (renderable-edge law).
//! - Spacing lowers RAW (`padding_inner_px` as `min_gap`); the
//!   content-driven `main_axis_gap` floor stays a render-side concern.
//! - Physical axes are content-driven when any band with that main axis
//!   uses content-driven geometry (else by sizing policy); constrained axes
//!   pin the root `SolveOptions` at the facet root's plot area and
//!   distribute free space, content-driven axes keep natural tracks.
//!
//! Channel reads use `Region.coordinated` (the node's own post-share ask;
//! `granted` would fold in unrelated siblings) — guide from `.guide`,
//! total from `.total`, carrying the cross-cousin lift (a merged side
//! holds every member's layers at once).

use std::collections::HashMap;

use avenger_chart_core::{CoordinatedLayout, CoordinatedOverflow, FacetAxis};
use avenger_layout::{
    EdgeDemand, Edges, Layout, LayoutSolution, RegionDetail, Side, Size, SolveOptions, Spacing,
    TrackSize,
};
use tracing::{debug, info, warn};

use crate::facet::coord::{FacetBandCoordMeasurement, renderable_for_empty_policy};
use crate::facet::coordination_plans::CoordinationNodeKey;
use crate::plot::compiled::{ComponentsMeasurement, CoordinationScopeKey};
use crate::render::context::FacetRuntimeSizingMode;

/// Solved channel values for one coordination round: the group-merged layout
/// per share key and per node, plus each node's own and group-equalized
/// overflow envelopes. Produced by [`tree_solved_round`] and hand-built by
/// test fixture folds.
#[derive(Debug, Clone, Default)]
pub(crate) struct SolvedRound {
    pub(crate) merged_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
    pub(crate) own_overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
}

/// Solved geometry for the current measured facet tree.
///
/// Coordination owns immutable channel values; this artifact owns the latest
/// settled layout solve used by render, guide, and child-frame readback.
pub(crate) struct CurrentFacetGeometry {
    pub(crate) lowered: LoweredFacetTree,
    pub(crate) solution: LayoutSolution<CoordinationNodeKey>,
    pub(crate) child_frames_by_node: HashMap<CoordinationNodeKey, CurrentFacetBandFrames>,
}

#[derive(Debug, Clone)]
pub(crate) struct CurrentFacetBandFrames {
    pub(crate) content_size: Size,
    pub(crate) children: Vec<CurrentFacetChildFrame>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CurrentFacetChildFrame {
    pub(crate) child_index: usize,
    pub(crate) rect: avenger_layout::Rect,
}

impl std::fmt::Debug for CurrentFacetGeometry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurrentFacetGeometry")
            .field("bands", &self.lowered.bands.len())
            .field("frame_bands", &self.child_frames_by_node.len())
            .field("size", &self.solution.size)
            .finish()
    }
}

impl CurrentFacetGeometry {
    pub(crate) fn lowered_band(&self, node_id: &CoordinationNodeKey) -> Option<&LoweredBand> {
        self.lowered
            .bands
            .iter()
            .find(|band| &band.node_id == node_id)
    }

    pub(crate) fn band_region(
        &self,
        node_id: &CoordinationNodeKey,
    ) -> Option<&avenger_layout::Region<CoordinationNodeKey>> {
        self.solution.region(node_id)
    }

    pub(crate) fn child_region(
        &self,
        node_id: &CoordinationNodeKey,
        cell_index: usize,
    ) -> Option<&avenger_layout::Region<CoordinationNodeKey>> {
        let band = self.lowered_band(node_id)?;
        if cell_index >= band.cell_count {
            return None;
        }
        let mut child_path = self.band_region(node_id)?.path.clone();
        child_path.push(cell_index);
        self.solution.at_path(&child_path)
    }

    pub(crate) fn child_frames(
        &self,
        node_id: &CoordinationNodeKey,
    ) -> Option<&CurrentFacetBandFrames> {
        self.child_frames_by_node.get(node_id)
    }
}

/// Pre-solve chart-side folds consumed by the lowering: values decided
/// across the share group (or by render law) rather than read per band.
#[derive(Debug, Clone, Copy)]
struct BandFold {
    /// The coordinated slot count: group max of local counts over the
    /// share key for shared bands; local (floored by the structural
    /// minimum) for FREE slot sharing.
    channel_n: usize,
    /// The lowered track gap: raw `padding_inner_px` for uniform bands,
    /// the placement gap floor for content-driven bands.
    min_gap: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FacetLoweringMode {
    Coordination,
    SettledGeometry,
}

/// Compute the per-band folds from construction-time values (one walk,
/// before lowering). Reading the snapshot-stable local values keeps the
/// lowering identical across repeated coordination runs on one tree.
fn compute_band_folds(
    measurement: &ComponentsMeasurement,
    mode: FacetLoweringMode,
) -> HashMap<CoordinationNodeKey, BandFold> {
    struct FoldInput {
        key: CoordinationScopeKey,
        local_n: usize,
        min_slot_count: usize,
        free: bool,
        min_gap: f32,
    }
    let mut inputs: HashMap<CoordinationNodeKey, FoldInput> = HashMap::new();
    let mut group_n: HashMap<CoordinationScopeKey, usize> = HashMap::new();
    let mut walk_path = Vec::new();
    crate::facet::coordination_apply::visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut walk_path,
        &mut |node_id, depth, facet_band| {
            let base = facet_band;
            let key = base.coordination_scope_key_for_depth(depth);
            let layout = match mode {
                FacetLoweringMode::Coordination => &base.local_layout,
                FacetLoweringMode::SettledGeometry => base.active_layout(),
            };
            let local_n = layout.n;
            let entry = group_n.entry(key.clone()).or_default();
            *entry = (*entry).max(local_n);
            let padding = layout.padding_inner_px;
            inputs.insert(
                node_id.clone(),
                FoldInput {
                    key,
                    local_n,
                    min_slot_count: base.min_slot_count,
                    free: base.slot_sharing.is_free(),
                    min_gap: match (mode, base.content_driven_main_axis()) {
                        (_, true) => crate::facet::padding_policy::main_axis_gap(padding),
                        (FacetLoweringMode::Coordination, false) => padding,
                        (FacetLoweringMode::SettledGeometry, false) => padding,
                    },
                },
            );
        },
    );
    inputs
        .into_iter()
        .map(|(node_id, input)| {
            let channel_n = match mode {
                FacetLoweringMode::Coordination if !input.free => {
                    group_n.get(&input.key).copied().unwrap_or(input.local_n)
                }
                _ => input.local_n.max(input.min_slot_count),
            };
            (
                node_id,
                BandFold {
                    channel_n,
                    min_gap: input.min_gap,
                },
            )
        })
        .collect()
}

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
    /// The pre-folded coordinated slot count (the fold the lowering's
    /// ghost padding consumed — group max for shared bands, local for
    /// free).
    pub(crate) channel_n: usize,
    pub(crate) has_overflow_cells: bool,
}

/// The lowered tree plus per-band records for channel and geometry extraction.
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
    lower_facet_tree_with_options(
        measurement,
        sizing,
        overrides,
        FacetLoweringMode::Coordination,
    )
}

/// Lower the current settled facet tree for render/readback geometry.
///
/// Nested facet bands are already bounded by their adopted child-frame plot
/// areas. Treating nested bands as content-driven prevents the geometry solve
/// from stretching their internal tracks to a parent slot that the render
/// context does not use.
pub(crate) fn lower_settled_facet_tree_with_overrides(
    measurement: &ComponentsMeasurement,
    sizing: FacetRuntimeSizingMode,
    overrides: Option<&CellSizeOverrides>,
) -> Option<LoweredFacetTree> {
    lower_facet_tree_with_options(
        measurement,
        sizing,
        overrides,
        FacetLoweringMode::SettledGeometry,
    )
}

fn lower_facet_tree_with_options(
    measurement: &ComponentsMeasurement,
    sizing: FacetRuntimeSizingMode,
    overrides: Option<&CellSizeOverrides>,
    mode: FacetLoweringMode,
) -> Option<LoweredFacetTree> {
    let root_band = crate::facet::coord::facet_band_ref(measurement.coord_measurement.as_ref())?;
    let policy = sizing.policy();

    // Physical-axis mode: an axis is CONTENT-DRIVEN when any band whose
    // main axis uses content-driven geometry (wrap and plot-area-sized
    // bands realize leaf-derived extents — the canvas
    // constrains the wrap COUNT upstream, never the cell sizes), else by
    // the runtime sizing policy. Content-driven axes keep natural track
    // sizes (free space trails); constrained axes distribute it.
    let mut x_content = false;
    let mut y_content = false;
    {
        let mut walk_path = Vec::new();
        crate::facet::coordination_apply::visit_facet_bands_with_node_id(
            measurement,
            0,
            &mut walk_path,
            &mut |_node_id, _depth, facet_band| {
                let base = facet_band;
                if base.content_driven_main_axis() {
                    match base.axis {
                        FacetAxis::Column => x_content = true,
                        FacetAxis::Row => y_content = true,
                    }
                }
            },
        );
    }
    let x_content_driven = x_content || policy.width.is_leaf_plot_area_sized();
    let y_content_driven = y_content || policy.height.is_leaf_plot_area_sized();

    let folds = compute_band_folds(measurement, mode);
    let mut bands = Vec::new();
    let mut node_path = Vec::new();
    let layout = lower_band(
        root_band,
        &mut node_path,
        0,
        (x_content_driven, y_content_driven),
        overrides,
        &folds,
        &mut bands,
        mode,
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
        EdgeDemand::from_guide_and_envelope(guide.top, total.top),
        EdgeDemand::from_guide_and_envelope(guide.right, total.right),
        EdgeDemand::from_guide_and_envelope(guide.bottom, total.bottom),
        EdgeDemand::from_guide_and_envelope(guide.left, total.left),
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

fn measurement_main_plot_size_for_size(axis: FacetAxis, size: Size) -> f32 {
    match axis {
        FacetAxis::Column => size.width,
        FacetAxis::Row => size.height,
    }
    .max(0.0)
}

fn lower_band(
    band: &FacetBandCoordMeasurement,
    node_path: &mut Vec<usize>,
    depth: usize,
    content_driven: (bool, bool),
    overrides: Option<&CellSizeOverrides>,
    folds: &HashMap<CoordinationNodeKey, BandFold>,
    bands: &mut Vec<LoweredBand>,
    mode: FacetLoweringMode,
) -> Layout<CoordinationNodeKey, CoordinationScopeKey> {
    let node_id = CoordinationNodeKey::new(node_path.clone());
    let key = band.coordination_scope_key_for_depth(depth);
    let fold = folds
        .get(&node_id)
        .copied()
        .expect("every walked band has a fold");
    let envelopes = cell_envelopes_by_index(band);

    let mut all_leaves = true;
    let mut cell_nodes: Vec<Layout<CoordinationNodeKey, CoordinationScopeKey>> = Vec::new();
    let mut main_track_sizes = Vec::new();
    let cell_sizes = band
        .cells
        .iter()
        .enumerate()
        .map(|(idx, cell)| {
            overrides
                .and_then(|map| map.get(&(node_id.clone(), idx)).copied())
                .unwrap_or_else(|| {
                    Size::new(
                        cell.measurement.plot_area_width.max(0.0),
                        cell.measurement.plot_area_height.max(0.0),
                    )
                })
        })
        .collect::<Vec<_>>();
    for (idx, cell) in band.cells.iter().enumerate() {
        let cell_size = cell_sizes[idx];
        let main_plot_size = measurement_main_plot_size_for_size(band.axis, cell_size);
        main_track_sizes.push(main_plot_size);
        let cell_envelope = envelopes.get(idx).cloned().flatten();
        let nested =
            crate::facet::coord::facet_band_ref(cell.measurement.coord_measurement.as_ref());
        let node = if let Some(nested_band) = nested {
            all_leaves = false;
            node_path.push(idx);
            let child = lower_band(
                nested_band,
                node_path,
                depth + 1,
                content_driven,
                overrides,
                folds,
                bands,
                mode,
            );
            node_path.pop();
            // Two-wrapper boundary. Channel contract: a band's coordinated
            // edges are its OWN epoch cell folds — including each level's
            // guide/legend classification, which can legitimately differ
            // from the child's structural layering (a chunk-level legend
            // may fold as guide at the outer level). So the inner wrapper
            // is CONTAINED (SolveFor::Content zeroes the child's boundary
            // lift toward the parent), and the outer wrapper declares the
            // cell's FULL epoch envelope as layered chrome — the parent
            // sees exactly the cell's epoch envelope, while the real
            // nested structure still solves inside for geometry.
            let contained = Layout::row(vec![child]).sizing(avenger_layout::SolveFor::Content);
            let mut wrapper = Layout::row(vec![contained]);
            if let Some((cell_guide, cell_total)) = &cell_envelope {
                for (side, cell_g, cell_t) in [
                    (Side::Top, cell_guide.top, cell_total.top),
                    (Side::Right, cell_guide.right, cell_total.right),
                    (Side::Bottom, cell_guide.bottom, cell_total.bottom),
                    (Side::Left, cell_guide.left, cell_total.left),
                ] {
                    let guide = cell_g.max(0.0);
                    let legend = (cell_t - cell_g).max(0.0);
                    if guide > 0.0 {
                        wrapper = wrapper.guide(side, guide);
                    }
                    if legend > 0.0 {
                        wrapper = wrapper.legend(side, legend);
                    }
                }
            }
            wrapper
        } else {
            let size = cell_size;
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

    // Ghost slots pad to the pre-folded coordinated slot count (group
    // max for shared bands, local for FREE slot sharing) floored by the
    // structural minimum — min_slot_count holds trailing wrap holes so a
    // short chunk's cells stay uniform instead of stretching across the
    // hole. The last ghost mirrors the trailing edge cell's demands so
    // the band's structural trailing edge keeps the renderable-edge law.
    let cell_count = cell_nodes.len();
    let slot_count = match mode {
        FacetLoweringMode::Coordination => fold
            .channel_n
            .max(band.min_slot_count)
            .max(cell_count)
            .max(1),
        FacetLoweringMode::SettledGeometry => cell_count.max(1),
    };
    if shadow_enabled() {
        debug!(
            target: "avenger_chart::facet::tree_solve",
            node = ?node_id.path,
            cell_count,
            slot_count,
            channel_n = fold.channel_n,
            local_n = band.local_layout.n,
            min_slot_count = band.min_slot_count,
            content_driven_main_axis = band.content_driven_main_axis(),
            content_driven_x = content_driven.0,
            content_driven_y = content_driven.1,
            "lowered band slots"
        );
    }
    if slot_count > cell_count && cell_count > 0 {
        let fallback = Size::new(
            cell_sizes
                .iter()
                .map(|size| size.width)
                .fold(0.0f32, f32::max)
                .max(0.0),
            cell_sizes
                .iter()
                .map(|size| size.height)
                .fold(0.0f32, f32::max)
                .max(0.0),
        );
        let trailing_envelope = envelopes.iter().rev().find_map(|e| e.clone());
        for ghost_index in cell_count..slot_count {
            let mut ghost = Layout::leaf(fallback);
            main_track_sizes.push(measurement_main_plot_size_for_size(band.axis, fallback));
            if ghost_index + 1 == slot_count {
                if let Some((guide, total)) = &trailing_envelope {
                    let trailing_side = match band.axis {
                        FacetAxis::Column => Side::Right,
                        FacetAxis::Row => Side::Bottom,
                    };
                    ghost = ghost.demand(
                        trailing_side,
                        EdgeDemand::from_guide_and_envelope(
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
        // envelope).
        cell_nodes.push(Layout::leaf(Size::default()));
        main_track_sizes.push(0.0);
    }

    // Spacing lowers the local declarations with the pre-folded track
    // gap (raw padding for uniform bands, the placement gap floor for
    // content-driven bands); the share merge raises them across cousins.
    let layout = match mode {
        FacetLoweringMode::Coordination => &band.local_layout,
        FacetLoweringMode::SettledGeometry => band.active_layout(),
    };
    let spacing = Spacing {
        outer_start: layout.outer_start,
        outer_end: layout.outer_end,
        min_gap: fold.min_gap,
    };

    let mut grid = match band.axis {
        FacetAxis::Column => {
            let mut grid = Layout::row(cell_nodes).column_spacing(spacing);
            if all_leaves && matches!(mode, FacetLoweringMode::Coordination) {
                grid = grid.uniform_columns();
            }
            if matches!(mode, FacetLoweringMode::SettledGeometry) {
                grid = grid.columns(
                    main_track_sizes
                        .iter()
                        .copied()
                        .map(TrackSize::Fixed)
                        .collect::<Vec<_>>(),
                );
            }
            grid
        }
        FacetAxis::Row => {
            let mut grid = Layout::column(cell_nodes).row_spacing(spacing);
            if all_leaves && matches!(mode, FacetLoweringMode::Coordination) {
                grid = grid.uniform_rows();
            }
            if matches!(mode, FacetLoweringMode::SettledGeometry) {
                grid = grid.rows(
                    main_track_sizes
                        .iter()
                        .copied()
                        .map(TrackSize::Fixed)
                        .collect::<Vec<_>>(),
                );
            }
            grid
        }
    };
    // Free space follows the physical axis mode: a constrained axis
    // distributes it across tracks (the equal-share law); a
    // content-driven axis keeps natural track sizes and lets free space
    // trail (content-realized extents are measured facts — stretching
    // them would falsify the measurement).
    let distribute_for = |is_content_driven: bool| {
        if is_content_driven {
            avenger_layout::Distribute::Start
        } else {
            avenger_layout::Distribute::StretchTracks
        }
    };
    let band_content_driven = if matches!(mode, FacetLoweringMode::SettledGeometry) && depth > 0 {
        (true, true)
    } else {
        content_driven
    };
    grid = grid
        .distribute_x(distribute_for(band_content_driven.0))
        .distribute_y(distribute_for(band_content_driven.1));
    if matches!(mode, FacetLoweringMode::Coordination) {
        grid = grid.share(key.clone());
    }
    grid = grid.id(node_id.clone());

    bands.push(LoweredBand {
        node_id,
        key,
        axis: band.axis,
        cell_count,
        guide_slot_gap_px: band.local_layout.guide_slot_gap_px,
        channel_n: fold.channel_n,
        has_overflow_cells: band.overflow_cells.is_some(),
    });

    grid
}

/// Per-node channel values extracted from one real-tree solve.
pub(crate) struct TreeChannels {
    pub(crate) layout_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
    pub(crate) overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
}

/// Extract the layout + overflow channels from a solved real tree.
///
/// - Spacing comes from the band grid's solved tracks (post share-merge);
///   the `n` and `guide_slot_gap_px` scalars fold chart-side over the
///   share group. These are pre-construction values:
///   `build_round_solution` applies the remaining per-node adjustments
///   (lane-gap fold, global-edge outer reversion) on top of them.
/// - Overflow comes from the band's `Region.coordinated` edges: guide
///   from `.guide`, total from `.total` (the cross-cousin lift).
pub(crate) fn extract_channels(
    lowered: &LoweredFacetTree,
    solution: &LayoutSolution<CoordinationNodeKey>,
) -> TreeChannels {
    // Guide-gap scalar fold over share-key membership (slot counts were
    // pre-folded into the lowering; the bands carry them).
    let mut group_gap: HashMap<&CoordinationScopeKey, f32> = HashMap::new();
    for band in &lowered.bands {
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
                n: band.channel_n,
            },
        );
        if band.has_overflow_cells {
            let coordinated = &region.coordinated;
            overflow_by_node.insert(
                band.node_id.clone(),
                CoordinatedOverflow {
                    guide: avenger_chart_core::OverflowSpaceRequirement {
                        top: coordinated.top.guide,
                        right: coordinated.right.guide,
                        bottom: coordinated.bottom.guide,
                        left: coordinated.left.guide,
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

/// Produce one coordination round's channel values from a real-tree
/// solve. `build_requirement_pass_with_round` consumes the result;
/// chart-side folds and solution construction follow from there.
///
/// - `merged_by_node` spacing comes from solved tracks (share-merged);
///   `n`/`guide_slot_gap_px` fold over share groups
///   (`build_round_solution` applies the lane-gap and global-edge
///   adjustments).
/// - `overflow_by_node` reads `Region.coordinated` (guide stratum,
///   total = total): each node's own post-share ask.
/// - `own_overflow_by_node` is the own-envelope law: guide from pass-1
///   `requested.guide`, total from the raw `geometric` view.
///
/// Returns an empty round for band-less measurements; a solve failure is
/// a hard error (the tree solve is the only channel producer).
pub(crate) fn tree_solved_round(
    measurement: &ComponentsMeasurement,
    sizing: FacetRuntimeSizingMode,
) -> Result<SolvedRound, avenger_chart_core::AvengerChartError> {
    let Some(lowered) = lower_facet_tree(measurement, sizing, None) else {
        return Ok(SolvedRound::default());
    };
    let solved = lowered.solve().map_err(|error| {
        avenger_chart_core::AvengerChartError::InternalError(format!(
            "facet tree solve failed: {error}"
        ))
    })?;
    let channels = extract_channels(&lowered, &solved);

    let mut own_overflow_by_node = HashMap::new();
    for band in &lowered.bands {
        if !band.has_overflow_cells {
            continue;
        }
        let Some(region) = solved.region(&band.node_id) else {
            continue;
        };
        own_overflow_by_node.insert(
            band.node_id.clone(),
            CoordinatedOverflow {
                guide: avenger_chart_core::OverflowSpaceRequirement {
                    top: region.requested.top.guide,
                    right: region.requested.right.guide,
                    bottom: region.requested.bottom.guide,
                    left: region.requested.left.guide,
                },
                total: avenger_chart_core::OverflowSpaceRequirement {
                    top: region.geometric_total.top,
                    right: region.geometric_total.right,
                    bottom: region.geometric_total.bottom,
                    left: region.geometric_total.left,
                },
            },
        );
    }

    Ok(SolvedRound {
        merged_by_node: channels.layout_by_node,
        own_overflow_by_node,
        overflow_by_node: channels.overflow_by_node,
    })
}

const SHADOW_EPS: f32 = 0.01;

/// Env-gated shadow diagnostics for one coordination run: re-solve the
/// tree from the settled measurement state and report leaf-slot-vs-live
/// geometry deltas plus the idempotence, adoption, and content-driven geometry probes
/// (the equilibrium check on adoption's fixed point).
pub(crate) fn run_shadow_census(
    measurement: &ComponentsMeasurement,
    sizing: FacetRuntimeSizingMode,
) {
    let Some(lowered) = lower_facet_tree(measurement, sizing, None) else {
        return;
    };
    let solved = match lowered.solve() {
        Ok(solved) => solved,
        Err(error) => {
            warn!(target: "avenger_chart::facet::tree_solve", error, "shadow solve failed");
            return;
        }
    };

    // Geometry comparison: each band's cell slots vs the live
    // (post-retarget/final-prop) cell plot areas.
    let mut geometry_max = 0.0f32;
    let mut geometry_cells_over = 0usize;
    let mut geometry_cells = 0usize;
    // Content-driven geometry probe (F3): geometry computed on read (the
    // local strip solve) vs the settled re-solve's band tracks.
    let mut content_geometry_bands = 0usize;
    let mut content_geometry_max = 0.0f32;
    let mut content_geometry_values_over = 0usize;
    // Stratum-heterogeneity probe (named-strata study): counts share
    // groups whose members carry unequal epoch envelopes — the
    // coordination-active cohort, where cross-cousin merging changes
    // values at all. NOT a bound on the named-strata semantic round:
    // equal envelopes with different internal level splits would still
    // diverge under independent per-stratum coordination; the exact
    // budget needs per-stratum epoch measurement (the campaign's
    // instrumentation phase).
    let mut strata_groups: HashMap<
        crate::plot::compiled::CoordinationScopeKey,
        Vec<CoordinatedOverflow>,
    > = HashMap::new();
    let mut walk_path = Vec::new();
    let mut band_index_by_id: HashMap<&CoordinationNodeKey, &LoweredBand> = HashMap::new();
    for band in &lowered.bands {
        band_index_by_id.insert(&band.node_id, band);
    }
    crate::facet::coordination_apply::visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut walk_path,
        &mut |node_id, depth, facet_band| {
            if let Some(envelope) = facet_band.measured_overflow.as_ref() {
                strata_groups
                    .entry(facet_band.coordination_scope_key_for_depth(depth))
                    .or_default()
                    .push(envelope.clone());
            }
            let Some(band) = band_index_by_id.get(node_id) else {
                return;
            };
            let Some(band_region) = solved.region(&band.node_id) else {
                return;
            };
            let base = facet_band;
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
                if crate::facet::coord::facet_band_ref(cell.measurement.coord_measurement.as_ref())
                    .is_some()
                {
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

            // Geometry probe: content-driven bands only. Compare per-cell
            // main starts/sizes and band extents between the on-read strip
            // solve and the settled tree solve's band tracks.
            if base.content_driven_main_axis() {
                let geometry = base.content_driven_geometry();
                let RegionDetail::Grid { tracks } = &band_region.detail else {
                    return;
                };
                content_geometry_bands += 1;
                let vertical = matches!(base.axis, crate::coords::FacetAxis::Row);
                let (starts, sizes, cross_size, main_extent) = if vertical {
                    (
                        &tracks.row_starts,
                        &tracks.row_sizes,
                        tracks.column_sizes.first().copied().unwrap_or(0.0),
                        band_region.content.height,
                    )
                } else {
                    (
                        &tracks.column_starts,
                        &tracks.column_sizes,
                        tracks.row_sizes.first().copied().unwrap_or(0.0),
                        band_region.content.width,
                    )
                };
                let mut band_max = 0.0f32;
                let mut compare = |label: &'static str, idx: usize, ours: f32, solved: f32| {
                    let delta = (ours - solved).abs();
                    band_max = band_max.max(delta);
                    if delta > SHADOW_EPS {
                        content_geometry_values_over += 1;
                        debug!(
                            target: "avenger_chart::facet::tree_solve",
                            node = ?node_id.path,
                            label,
                            idx,
                            computed = ours,
                            solved,
                            "content-driven geometry probe divergence"
                        );
                    }
                };
                for (idx, cell) in geometry.cells.iter().enumerate() {
                    let start = starts.get(idx).copied().unwrap_or(f32::NAN);
                    let size = sizes.get(idx).copied().unwrap_or(f32::NAN);
                    compare("main_start", idx, cell.main_start, start);
                    compare("main_size", idx, cell.main_size, size);
                }
                compare("main_extent", 0, geometry.main_extent, main_extent);
                compare(
                    "cross_extent",
                    0,
                    geometry.cross_extent.unwrap_or(0.0),
                    cross_size,
                );
                content_geometry_max = content_geometry_max.max(band_max);
            }
        },
    );

    // Stratum-heterogeneity tally: a group is heterogeneous on a side
    // when members' epoch envelopes disagree there (guide or total).
    let mut strata_groups_shared = 0usize;
    let mut strata_groups_hetero = 0usize;
    let mut strata_spread_max = 0.0f32;
    for members in strata_groups.values() {
        if members.len() < 2 {
            continue;
        }
        strata_groups_shared += 1;
        let mut spread = 0.0f32;
        let sides: [fn(&crate::coords::OverflowSpaceRequirement) -> f32; 4] =
            [|s| s.top, |s| s.right, |s| s.bottom, |s| s.left];
        for side in sides {
            for guide_view in [true, false] {
                let values = members
                    .iter()
                    .map(|env| side(if guide_view { &env.guide } else { &env.total }));
                let max = values.clone().fold(f32::MIN, f32::max);
                let min = values.fold(f32::MAX, f32::min);
                spread = spread.max(max - min);
            }
        }
        if spread > SHADOW_EPS {
            strata_groups_hetero += 1;
        }
        strata_spread_max = strata_spread_max.max(spread);
    }

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

    // Current-geometry probe: the installed current geometry should match the
    // settled-state re-solve.
    let current_delta = crate::facet::coord::facet_band_ref(measurement.coord_measurement.as_ref())
        .and_then(|band| band.current_geometry_handle().map(|(geometry, _)| geometry))
        .map(|geometry| geometry.solution.content_delta(&solved));

    info!(
        target: "avenger_chart::facet::tree_solve",
        bands = lowered.bands.len(),
        geometry_cells,
        geometry_max,
        geometry_cells_over,
        idempotence_delta,
        current_delta = current_delta.unwrap_or(f32::NAN),
        content_geometry_bands,
        content_geometry_max,
        content_geometry_values_over,
        strata_groups_shared,
        strata_groups_hetero,
        strata_spread_max,
        "shadow tree-solve census"
    );
}

#[cfg(test)]
mod tests {
    use avenger_layout::{EdgeDemand, EdgeGrant, Layout, Side, Size, SolveFor, SolveOptions};

    /// The nested-boundary channel contract: a parent band's channel
    /// edges are its OWN epoch cell folds, INCLUDING the parent level's
    /// guide/legend classification — not the child band's structural
    /// layering (a chunk-level legend legitimately folds as guide at the
    /// wrap-row level). The lowering encodes this as the two-wrapper
    /// boundary; this test pins the mechanism it relies on: a CONTAINED
    /// (`SolveFor::Content`) wrapper zeroes the child's structural lift
    /// toward the parent, so the outer wrapper's full-epoch chrome alone
    /// defines the parent-visible edge. Naive per-layer residual chrome
    /// would double-count reclassified space (42 guide + 42 legend = 84
    /// for the SAME 42px legend).
    #[test]
    fn nested_boundary_two_wrapper_overrides_child_classification() {
        // Child band: one cell whose 42px right edge is structurally
        // LEGEND-classified (inner 0 / outer 42).
        let child: Layout =
            Layout::row(vec![Layout::leaf(Size::new(100.0, 50.0)).demand(
                Side::Right,
                EdgeDemand::from_guide_and_envelope(0.0, 42.0),
            )])
            .id(1);
        // Two-wrapper boundary: the contained wrapper zeroes the child's
        // structural lift; the outer wrapper declares the parent's epoch
        // view of the SAME 42px, which classifies it as GUIDE (inner).
        let contained = Layout::row(vec![child]).sizing(SolveFor::Content);
        let wrapper = Layout::row(vec![contained]).guide(Side::Right, 42.0).id(0);
        let parent: Layout = Layout::row(vec![wrapper]);
        let solved = parent.solve(&SolveOptions::default()).expect("solve");

        // Parent-visible edge = the epoch classification (guide 42,
        // total 42) — not the child's structural (0, 42, 42), and not a
        // double-counted (42, 42, 84).
        let root = solved.at_path(&[]).expect("root region");
        assert_eq!(root.coordinated.right, EdgeGrant::new(42.0, 0.0, 42.0));

        // The child band's own channel edges keep its structural fold —
        // containment isolates, it does not rewrite.
        let child_region = solved.region(&1).expect("child band region");
        assert_eq!(
            child_region.coordinated.right,
            EdgeGrant::new(0.0, 42.0, 42.0)
        );
    }
}
