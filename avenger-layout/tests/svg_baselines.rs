//! Public-API examples rendered to SVG and compared against committed
//! baselines.
//!
//! Each test builds a layout through the public API, captures it as a
//! [`DebugScene`], and compares the SVG against
//! `tests/baselines/<name>.svg`. The baselines double as a visual gallery
//! of what every solver does — open them in a browser, or see
//! `tests/baselines/README.md` for the index.
//!
//! To regenerate baselines after an intentional change:
//!
//! ```sh
//! AVENGER_LAYOUT_BLESS=1 cargo test -p avenger-layout --features svg --test svg_baselines
//! ```
//!
//! On mismatch the actual SVG is written to `tests/failures/<name>.svg`.

use std::fs;
use std::path::PathBuf;

use avenger_layout::{
    AlignmentNode, BandItem, BandSolution, BoundaryDemand, CrossAlign, DebugScene, EdgeDemand,
    Edges, Frame, FrameAxis, FrameAxisSizing, FrameSide, GridItem, GridRequirements, GridShape,
    GridSlot, LayoutItem, LayoutNode, LayoutSlotContent, Orientation, SingletonPolicy, Size,
    TrackSpacing, TreeEnvelopeKind, UniformTracks, align, align_by,
};

fn baseline_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/baselines")
}

/// Render the SVG to a PNG next to it (2x scale, white background).
///
/// PNGs are a viewing convenience only: rasterization goes through system
/// fonts, so they are not byte-stable across machines and are never
/// compared — the SVG string is the snapshot.
fn write_png(svg: &str, png_path: &std::path::Path) {
    use resvg::{tiny_skia, usvg};

    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(svg, &options).expect("baseline SVG parses");
    let size = tree
        .size()
        .to_int_size()
        .scale_by(2.0)
        .expect("scaled pixmap size");
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height()).expect("allocate pixmap");
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(2.0, 2.0),
        &mut pixmap.as_mut(),
    );
    pixmap.save_png(png_path).expect("write png");
}

fn assert_svg_baseline(name: &str, svg: &str) {
    let path = baseline_dir().join(format!("{name}.svg"));
    if std::env::var_os("AVENGER_LAYOUT_BLESS").is_some() {
        fs::create_dir_all(path.parent().unwrap()).expect("create baselines dir");
        fs::write(&path, svg).expect("write blessed baseline");
        write_png(svg, &path.with_extension("png"));
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing baseline {path:?}; regenerate with \
             AVENGER_LAYOUT_BLESS=1 cargo test -p avenger-layout --features svg"
        )
    });
    if expected != svg {
        let failures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/failures");
        fs::create_dir_all(&failures).expect("create failures dir");
        let actual_path = failures.join(format!("{name}.svg"));
        fs::write(&actual_path, svg).expect("write failure svg");
        write_png(svg, &actual_path.with_extension("png"));
        panic!(
            "SVG for '{name}' differs from baseline {path:?}; \
             actual (svg + png) written to {actual_path:?}. If the change is \
             intentional, re-bless with AVENGER_LAYOUT_BLESS=1."
        );
    }
}

fn slot(row: usize, column: usize) -> GridSlot {
    GridSlot {
        row,
        column,
        row_span: 1,
        column_span: 1,
    }
}

// --- band -------------------------------------------------------------

/// Sibling boundary chrome becomes inter-child gaps, floored by `min_gap`:
/// the first pair's chrome (8 + 14 = 22) wins over the floor, the second
/// pair's (2 + 1 = 3) is floored to 12. `outer_start`/`outer_end` reserve
/// space before the first and after the last child.
#[test]
fn band_horizontal_gaps_and_min_gap_floor() {
    let items = [
        BandItem {
            id: 0,
            main_size: 60.0,
            cross_size: 80.0,
            boundary: BoundaryDemand {
                before: 5.0,
                after: 8.0,
            },
        },
        BandItem {
            id: 1,
            main_size: 90.0,
            cross_size: 80.0,
            boundary: BoundaryDemand {
                before: 14.0,
                after: 2.0,
            },
        },
        BandItem {
            id: 2,
            main_size: 45.0,
            cross_size: 80.0,
            boundary: BoundaryDemand {
                before: 1.0,
                after: 0.0,
            },
        },
    ];
    let band = BandSolution::solve(
        Orientation::Horizontal,
        &items,
        TrackSpacing {
            outer_start: 6.0,
            outer_end: 10.0,
            min_gap: 12.0,
        },
        CrossAlign::default(),
    );

    assert_eq!(band.items[1].main_start - band.items[0].main_start, 82.0); // 60 + (8 + 14)
    assert_eq!(band.items[2].main_start - band.items[1].main_start, 102.0); // 90 + floored 12

    assert_svg_baseline(
        "band_horizontal_gaps_and_min_gap_floor",
        &DebugScene::from_band(&band).to_svg(),
    );
}

/// A vertical band with ragged cross sizes, centered on the cross axis.
#[test]
fn band_vertical_cross_align_center() {
    let items = [
        BandItem {
            id: 0,
            main_size: 40.0,
            cross_size: 120.0,
            boundary: BoundaryDemand::default(),
        },
        BandItem {
            id: 1,
            main_size: 40.0,
            cross_size: 70.0,
            boundary: BoundaryDemand::default(),
        },
        BandItem {
            id: 2,
            main_size: 40.0,
            cross_size: 95.0,
            boundary: BoundaryDemand::default(),
        },
    ];
    let band = BandSolution::solve(
        Orientation::Vertical,
        &items,
        TrackSpacing {
            min_gap: 8.0,
            ..Default::default()
        },
        CrossAlign::Center,
    );

    assert_svg_baseline(
        "band_vertical_cross_align_center",
        &DebugScene::from_band(&band).to_svg(),
    );
}

/// The placement handoff: a solved band converted into per-child origins
/// in the parent's content space, drawn as labeled origin markers.
#[test]
fn band_placement_handoff_markers() {
    let items = [
        BandItem {
            id: 0,
            main_size: 70.0,
            cross_size: 50.0,
            boundary: BoundaryDemand::default(),
        },
        BandItem {
            id: 1,
            main_size: 50.0,
            cross_size: 60.0,
            boundary: BoundaryDemand::default(),
        },
    ];
    let band = BandSolution::solve(
        Orientation::Horizontal,
        &items,
        TrackSpacing {
            min_gap: 16.0,
            ..Default::default()
        },
        CrossAlign::End,
    );
    let placement = band.to_placement_solution([20.0, 10.0], Size::new(300.0, 80.0));

    let child = placement.child(1).expect("child 1 is placed");
    assert_eq!(child.origin, [106.0, 10.0]); // 20 + 70 + 16, end-aligned at 10 + (60 - 60)

    let mut scene = DebugScene::from_placements(&placement);
    // Overlay the band's content rectangles so the markers have context.
    scene.regions.extend(
        DebugScene::from_band(&band)
            .regions
            .into_iter()
            .map(|mut region| {
                region.content.x += 20.0;
                region.content.y += 10.0;
                region
            }),
    );
    assert_svg_baseline("band_placement_handoff_markers", &scene.to_svg());
}

// --- grid -------------------------------------------------------------

/// A 3x3 grid with a hole (no item in r1c1), a two-column span whose
/// content exceeds its tracks' natural widths, and a `base_cell_size`
/// floor for every track.
#[test]
fn grid_spans_holes_and_base_cell_size() {
    let shape = GridShape {
        rows: 3,
        columns: 3,
    };
    let items = vec![
        GridItem {
            id: "a",
            slot: slot(0, 0),
            content_size: Size::new(70.0, 40.0),
            inner_edges: Edges::default(),
            outer_edges: Edges::default(),
            total_edges: Edges::default(),
        },
        GridItem {
            id: "b",
            slot: slot(0, 2),
            content_size: Size::new(50.0, 55.0),
            inner_edges: Edges::default(),
            outer_edges: Edges::default(),
            total_edges: Edges::default(),
        },
        GridItem {
            id: "wide",
            slot: GridSlot {
                row: 1,
                column: 0,
                row_span: 1,
                column_span: 2,
            },
            content_size: Size::new(160.0, 45.0),
            inner_edges: Edges::default(),
            outer_edges: Edges::default(),
            total_edges: Edges::default(),
        },
        GridItem {
            id: "c",
            slot: slot(2, 1),
            content_size: Size::new(45.0, 35.0),
            inner_edges: Edges::default(),
            outer_edges: Edges::default(),
            total_edges: Edges::default(),
        },
    ];
    let requirements = GridRequirements::from_items(shape, Size::new(40.0, 30.0), &items)
        .expect("slots fit the shape");
    let solution = requirements.solve(&items);

    assert_svg_baseline(
        "grid_spans_holes_and_base_cell_size",
        &DebugScene::from_grid(&solution, &items).to_svg(),
    );
}

/// Layered edge demand around grid items: dotted blue shows the inner
/// (guide-like) envelope, dashed purple the total envelope, and the gap
/// between columns is `max(min_gap, trailing total + leading total)`.
#[test]
fn grid_edge_demand_layers_and_gap_law() {
    let shape = GridShape {
        rows: 1,
        columns: 2,
    };
    let items = vec![
        GridItem {
            id: 0,
            slot: slot(0, 0),
            content_size: Size::new(110.0, 70.0),
            inner_edges: Edges::new(8.0, 10.0, 8.0, 24.0),
            outer_edges: Edges::new(0.0, 18.0, 0.0, 0.0),
            total_edges: Edges::new(8.0, 28.0, 8.0, 24.0),
        },
        GridItem {
            id: 1,
            slot: slot(0, 1),
            content_size: Size::new(110.0, 70.0),
            inner_edges: Edges::new(8.0, 6.0, 8.0, 9.0),
            outer_edges: Edges::default(),
            total_edges: Edges::new(8.0, 6.0, 8.0, 9.0),
        },
    ];
    let requirements =
        GridRequirements::from_items(shape, Size::default(), &items).expect("slots fit");
    let solution = requirements.solve(&items);

    // Gap law: item 0 trailing total (28) + item 1 leading total (9) = 37.
    let targets = solution.edge_targets_for_slot(items[0].slot);
    assert_eq!(targets.total.right, 28.0);
    assert_eq!(
        solution.content_origin_for_slot(items[1].slot)[0],
        110.0 + 37.0
    );

    assert_svg_baseline(
        "grid_edge_demand_layers_and_gap_law",
        &DebugScene::from_grid(&solution, &items).to_svg(),
    );
}

/// Uniform tracks: a count-and-spacing policy solved at a given track
/// size, after merging two policies by component-wise max.
#[test]
fn uniform_tracks_merged_policy() {
    let a = UniformTracks {
        count: 3,
        spacing: TrackSpacing {
            outer_start: 4.0,
            outer_end: 4.0,
            min_gap: 6.0,
        },
    };
    let b = UniformTracks {
        count: 4,
        spacing: TrackSpacing {
            outer_start: 10.0,
            outer_end: 2.0,
            min_gap: 14.0,
        },
    };
    let merged = a.merge_max(b);
    assert_eq!(merged.count, 4);
    assert_eq!(merged.spacing.min_gap, 14.0);
    assert_eq!(merged.spacing.outer_end, 4.0); // component-wise max

    let solution = merged.solve(56.0);
    assert_eq!(solution.extent, 10.0 + 4.0 * 56.0 + 3.0 * 14.0 + 4.0);

    assert_svg_baseline(
        "uniform_tracks_merged_policy",
        &DebugScene::from_uniform_tracks(&solution, 40.0).to_svg(),
    );
}

// --- frame ------------------------------------------------------------

fn chart_like_frame(horizontal: FrameAxisSizing, vertical: FrameAxisSizing) -> Frame {
    Frame {
        horizontal: FrameAxis {
            sizing: horizontal,
            leading: FrameSide {
                margin: 10.0,
                bands: vec![],
                outer: 0.0,
                inner: 34.0,
            },
            trailing: FrameSide {
                margin: 10.0,
                bands: vec![],
                outer: 52.0,
                inner: 6.0,
            },
            content_min: 50.0,
        },
        vertical: FrameAxis {
            sizing: vertical,
            leading: FrameSide {
                margin: 10.0,
                bands: vec![22.0, 16.0],
                outer: 0.0,
                inner: 4.0,
            },
            trailing: FrameSide {
                margin: 10.0,
                bands: vec![],
                outer: 0.0,
                inner: 18.0,
            },
            content_min: 50.0,
        },
    }
}

/// Envelope-fixed: a 400x300 envelope, chart-like chrome (margins, two
/// title bands, a right legend strip, guide strips), and the content takes
/// the remainder.
#[test]
fn frame_envelope_fixed_chart_chrome() {
    let frame = chart_like_frame(
        FrameAxisSizing::EnvelopeFixed { extent: 400.0 },
        FrameAxisSizing::EnvelopeFixed { extent: 300.0 },
    );
    let solution = frame.solve();

    assert_eq!(solution.extent(), Size::new(400.0, 300.0));
    assert_eq!(
        solution.content_rect().width,
        400.0 - 10.0 - 34.0 - 6.0 - 52.0 - 10.0
    );

    // The solved frame emits the demand vocabulary containers consume.
    let demands = solution.edge_demands();
    assert_eq!(demands.top.total, 10.0 + 22.0 + 16.0 + 4.0);
    assert_eq!(demands.right.inner, 6.0);
    assert_eq!(demands.right.outer, 52.0);

    assert_svg_baseline(
        "frame_envelope_fixed_chart_chrome",
        &DebugScene::from_frame(&solution).to_svg(),
    );
}

/// Content-fixed: the same chrome, but the content extent is given and the
/// envelope is the sum of all layers.
#[test]
fn frame_content_fixed_envelope_derived() {
    let frame = chart_like_frame(
        FrameAxisSizing::ContentFixed { content: 240.0 },
        FrameAxisSizing::ContentFixed { content: 160.0 },
    );
    let solution = frame.solve();

    assert_eq!(
        solution.extent(),
        Size::new(240.0 + 112.0, 160.0 + 80.0) // chrome sums per axis
    );

    assert_svg_baseline(
        "frame_content_fixed_envelope_derived",
        &DebugScene::from_frame(&solution).to_svg(),
    );
}

/// Envelope and content both fixed: the margins absorb the slack, half
/// each (declared margins are replaced — see `FrameAxisSizing` docs).
#[test]
fn frame_envelope_and_content_fixed_margin_slack() {
    let frame = chart_like_frame(
        FrameAxisSizing::EnvelopeAndContentFixed {
            extent: 460.0,
            content: 240.0,
        },
        FrameAxisSizing::EnvelopeAndContentFixed {
            extent: 320.0,
            content: 160.0,
        },
    );
    let solution = frame.solve();

    let slack_h = 460.0 - (34.0 + 6.0 + 52.0 + 240.0);
    assert_eq!(solution.horizontal.leading.margin.size, slack_h / 2.0);

    assert_svg_baseline(
        "frame_envelope_and_content_fixed_margin_slack",
        &DebugScene::from_frame(&solution).to_svg(),
    );
}

/// The content floor: chrome alone exceeds the requested envelope, so the
/// content holds at `content_min` and the solved extent grows past the
/// request.
#[test]
fn frame_content_min_floor_overflows_envelope() {
    let frame = chart_like_frame(
        FrameAxisSizing::EnvelopeFixed { extent: 130.0 },
        FrameAxisSizing::EnvelopeFixed { extent: 100.0 },
    );
    let solution = frame.solve();

    assert_eq!(solution.content_rect().width, 50.0);
    assert_eq!(solution.horizontal.extent, 112.0 + 50.0);

    assert_svg_baseline(
        "frame_content_min_floor_overflows_envelope",
        &DebugScene::from_frame(&solution).to_svg(),
    );
}

// --- tree -------------------------------------------------------------

fn tree_leaf(id: usize, slot: GridSlot, size: Size, total: Edges<f32>) -> LayoutItem<usize> {
    LayoutItem {
        id,
        slot,
        content: LayoutSlotContent::Leaf {
            content_size: size,
            inner_edges: total,
            outer_edges: Edges::default(),
            total_edges: total,
        },
    }
}

fn nested_tree() -> LayoutNode<usize> {
    let inner = LayoutNode {
        shape: GridShape {
            rows: 1,
            columns: 2,
        },
        column_spacing: TrackSpacing {
            min_gap: 8.0,
            ..Default::default()
        },
        row_spacing: TrackSpacing::default(),
        base_cell_size: Size::default(),
        // Band-level chrome stacks beyond the aggregated child envelope:
        // inner (e.g. facet labels) on top, outer (e.g. a band legend) on
        // the right.
        stacked_inner_edges: Edges::new(14.0, 0.0, 0.0, 0.0),
        stacked_outer_edges: Edges::new(0.0, 20.0, 0.0, 0.0),
        items: vec![
            tree_leaf(
                10,
                slot(0, 0),
                Size::new(60.0, 50.0),
                Edges::new(4.0, 6.0, 4.0, 12.0),
            ),
            // The trailing child's right edge has inner + outer (6 + 18)
            // exceeding its raw rendered total (12): the layered envelope
            // lifts it, the geometric envelope reports it as measured.
            LayoutItem {
                id: 11,
                slot: slot(0, 1),
                content: LayoutSlotContent::Leaf {
                    content_size: Size::new(60.0, 50.0),
                    inner_edges: Edges::new(4.0, 6.0, 4.0, 6.0),
                    outer_edges: Edges::new(0.0, 18.0, 0.0, 0.0),
                    total_edges: Edges::new(4.0, 12.0, 4.0, 6.0),
                },
            },
        ],
    };
    LayoutNode {
        shape: GridShape {
            rows: 2,
            columns: 1,
        },
        column_spacing: TrackSpacing::default(),
        row_spacing: TrackSpacing {
            min_gap: 16.0,
            ..Default::default()
        },
        base_cell_size: Size::default(),
        stacked_inner_edges: Edges::default(),
        stacked_outer_edges: Edges::default(),
        items: vec![
            tree_leaf(
                0,
                slot(0, 0),
                Size::new(140.0, 40.0),
                Edges::new(0.0, 0.0, 6.0, 0.0),
            ),
            LayoutItem {
                id: 1,
                slot: slot(1, 0),
                content: LayoutSlotContent::Node(inner),
            },
        ],
    }
}

/// Wrap a tree in a margins-only frame and compose one scene: the tree's
/// layered envelope becomes the frame's inner/outer reservations, and the
/// frame's content (the tree's natural extent plus `extra`, if any)
/// allocates the tree. Framed scenes are easier to reason about than bare
/// trees, whose boundary chrome floats outside their own bounds.
fn frame_around_tree(tree: &LayoutNode<usize>, extra: Option<Size>) -> DebugScene {
    let envelope = tree
        .envelope(TreeEnvelopeKind::Layered)
        .expect("envelope solves");
    let extra = extra.unwrap_or_default();
    let chrome_side = |outer: f32, inner: f32| FrameSide {
        margin: 12.0,
        bands: vec![],
        outer,
        inner,
    };
    let frame = Frame {
        horizontal: FrameAxis {
            sizing: FrameAxisSizing::ContentFixed {
                content: envelope.content_size.width + extra.width,
            },
            leading: chrome_side(envelope.outer_edges.left, envelope.inner_edges.left),
            trailing: chrome_side(envelope.outer_edges.right, envelope.inner_edges.right),
            content_min: 50.0,
        },
        vertical: FrameAxis {
            sizing: FrameAxisSizing::ContentFixed {
                content: envelope.content_size.height + extra.height,
            },
            leading: chrome_side(envelope.outer_edges.top, envelope.inner_edges.top),
            trailing: chrome_side(envelope.outer_edges.bottom, envelope.inner_edges.bottom),
            content_min: 50.0,
        },
    };
    let frame_solution = frame.solve();
    let content = frame_solution.content_rect();
    let solved = tree
        .solve(Some(Size::new(content.width, content.height)))
        .expect("tree solves in the frame's content");

    let mut scene = DebugScene::from_frame(&frame_solution);
    scene.regions.extend(
        DebugScene::from_tree(&solved)
            .regions
            .into_iter()
            .map(|mut region| {
                region.content.x += content.x;
                region.content.y += content.y;
                if let Some(anchor) = &mut region.label_anchor {
                    anchor[0] += content.x;
                    // One extra line down so depth-0 labels clear the
                    // frame's own "content" label.
                    anchor[1] += content.y + 12.0;
                }
                region
            }),
    );
    scene
}

/// A nested tree solved at its natural extent: a column of one leaf over a
/// two-child band whose stacked chrome (labels above, a band legend right)
/// extends the band's envelope.
#[test]
fn tree_nested_with_stacked_chrome() {
    let root = nested_tree();

    // The two envelope laws: the layered envelope lifts each total to at
    // least inner + outer (post-coordination occupancy); the geometric
    // envelope reports raw measured maxima. Both include the band's
    // stacked outer chrome on the right.
    let layered = root
        .envelope(TreeEnvelopeKind::Layered)
        .expect("envelope solves");
    let geometric = root
        .envelope(TreeEnvelopeKind::Geometric)
        .expect("envelope solves");
    // Layered: lift(max(12, 6 + 18)) + stacked outer 20 = 44.
    // Geometric: raw measured total 12 + stacked outer 20 = 32.
    assert_eq!(layered.total_edges.right, 44.0);
    assert_eq!(geometric.total_edges.right, 32.0);

    assert_svg_baseline(
        "tree_nested_with_stacked_chrome",
        &frame_around_tree(&root, None).to_svg(),
    );
}

/// The same tree given a larger allocation: every track stretches evenly
/// and the stretch propagates into the nested band.
#[test]
fn tree_allocation_stretches_tracks_evenly() {
    let root = nested_tree();
    let natural = root.solve(None).expect("tree solves").content_size;
    let solved = root
        .solve(Some(Size::new(natural.width + 60.0, natural.height + 40.0)))
        .expect("tree solves with allocation");
    assert_eq!(solved.content_size.width, natural.width + 60.0);

    assert_svg_baseline(
        "tree_allocation_stretches_tracks_evenly",
        &frame_around_tree(&root, Some(Size::new(60.0, 40.0))).to_svg(),
    );
}

/// A faceted chart in miniature: a frame (margins, title band, a frame-
/// level legend) whose content is the nested facet tree. This is the
/// handoff between the two solvers, both ways: the tree's layered
/// envelope becomes the frame's inner/outer reservations, and the frame's
/// solved content rectangle becomes the tree's allocation. Composed
/// test-side by offsetting the tree scene into the frame's content rect.
#[test]
fn frame_wrapping_facet_tree() {
    let tree = nested_tree();
    let envelope = tree
        .envelope(TreeEnvelopeKind::Layered)
        .expect("envelope solves");

    let chrome_side = |margin: f32, bands: Vec<f32>, outer: f32, inner: f32| FrameSide {
        margin,
        bands,
        outer,
        inner,
    };
    let frame = Frame {
        horizontal: FrameAxis {
            sizing: FrameAxisSizing::EnvelopeFixed { extent: 460.0 },
            leading: chrome_side(
                12.0,
                vec![],
                envelope.outer_edges.left,
                envelope.inner_edges.left,
            ),
            // The tree's band-level legend and a frame-level legend share
            // the outer ring on the right.
            trailing: chrome_side(
                12.0,
                vec![],
                envelope.outer_edges.right + 48.0,
                envelope.inner_edges.right,
            ),
            content_min: 50.0,
        },
        vertical: FrameAxis {
            sizing: FrameAxisSizing::EnvelopeFixed { extent: 380.0 },
            leading: chrome_side(
                12.0,
                vec![20.0],
                envelope.outer_edges.top,
                envelope.inner_edges.top,
            ),
            trailing: chrome_side(
                12.0,
                vec![],
                envelope.outer_edges.bottom,
                envelope.inner_edges.bottom,
            ),
            content_min: 50.0,
        },
    };
    let frame_solution = frame.solve();
    let content = frame_solution.content_rect();

    // The frame's content allocates the tree; both axes exceed the tree's
    // natural extent here, so every track stretches.
    let solved_tree = tree
        .solve(Some(Size::new(content.width, content.height)))
        .expect("tree solves in the frame's content");
    assert_eq!(
        solved_tree.content_size,
        Size::new(content.width, content.height)
    );

    let mut scene = DebugScene::from_frame(&frame_solution);
    scene
        .regions
        .extend(
            DebugScene::from_tree(&solved_tree)
                .regions
                .into_iter()
                .map(|mut region| {
                    region.content.x += content.x;
                    region.content.y += content.y;
                    if let Some(anchor) = &mut region.label_anchor {
                        anchor[0] += content.x;
                        // One extra line down so depth-0 labels clear the
                        // frame's own "content" label.
                        anchor[1] += content.y + 12.0;
                    }
                    region
                }),
        );
    assert_svg_baseline("frame_wrapping_facet_tree", &scene.to_svg());
}

// --- alignment --------------------------------------------------------

/// Stack captioned scenes vertically into one gallery image (test-side
/// composition; the crate's scenes stay single-arrangement).
fn stack_scenes(scenes: Vec<(&str, DebugScene)>) -> DebugScene {
    const CAPTION: f32 = 16.0;
    const GAP: f32 = 20.0;
    let mut combined = DebugScene {
        content_size: Size::new(0.0, 0.0),
        // The panels are independent arrangements: no shared bounds frame;
        // each panel gets its own Bounds region instead.
        draw_bounds: false,
        regions: Vec::new(),
        markers: Vec::new(),
        dividers: Vec::new(),
    };
    let mut y_offset = 0.0;
    let mut divider_ys = Vec::new();
    for (caption, scene) in scenes {
        if y_offset > 0.0 {
            // Divider midway through the gap above this panel's caption.
            divider_ys.push(y_offset - GAP / 2.0);
        }
        // A zero-size region carries the caption above the panel.
        combined.regions.push(avenger_layout::DebugRegion {
            label: caption.to_string(),
            kind: avenger_layout::DebugRegionKind::Content,
            content: avenger_layout::Rect::new(0.0, y_offset, 0.0, 0.0),
            requested: None,
            target: None,
            label_anchor: Some([0.0, y_offset + 10.0]),
            label_rotated: false,
            depth: 0,
        });
        y_offset += CAPTION;
        for mut region in scene.regions {
            // Demands are content-relative, so only the rect moves.
            region.content.y += y_offset;
            combined.regions.push(region);
        }
        // Drawn after the panel's regions so the boundary stays visible
        // where content edges coincide with it.
        combined.regions.push(avenger_layout::DebugRegion {
            label: String::new(),
            kind: avenger_layout::DebugRegionKind::Bounds,
            content: avenger_layout::Rect::new(
                0.0,
                y_offset,
                scene.content_size.width,
                scene.content_size.height,
            ),
            requested: None,
            target: None,
            label_anchor: None,
            label_rotated: false,
            depth: 0,
        });
        combined.content_size.width = combined.content_size.width.max(scene.content_size.width);
        y_offset += scene.content_size.height + GAP;
        combined.content_size.height = y_offset - GAP;
    }
    // Thin separators between the independent panels, drawn edge to edge
    // by the renderer.
    combined.dividers = divider_ys;
    combined
}

/// Two instances of the same two-column arrangement measured with
/// different content and chrome. `align` merges their requirements by
/// max; re-solving each from the merged requirements makes their tracks
/// (and so their content rectangles) line up exactly. The gallery shows
/// instance A, instance B, then both re-solved on the merged grid.
#[test]
fn alignment_merges_grids_across_instances() {
    let shape = GridShape {
        rows: 1,
        columns: 2,
    };
    let items_a = vec![
        GridItem {
            id: 0,
            slot: slot(0, 0),
            content_size: Size::new(80.0, 60.0),
            inner_edges: Edges::new(0.0, 0.0, 0.0, 26.0),
            outer_edges: Edges::default(),
            total_edges: Edges::new(0.0, 0.0, 0.0, 26.0),
        },
        GridItem {
            id: 1,
            slot: slot(0, 1),
            content_size: Size::new(120.0, 60.0),
            inner_edges: Edges::default(),
            outer_edges: Edges::default(),
            total_edges: Edges::default(),
        },
    ];
    let items_b = vec![
        GridItem {
            id: 0,
            slot: slot(0, 0),
            content_size: Size::new(110.0, 45.0),
            inner_edges: Edges::new(0.0, 0.0, 0.0, 9.0),
            outer_edges: Edges::default(),
            total_edges: Edges::new(0.0, 0.0, 0.0, 9.0),
        },
        GridItem {
            id: 1,
            slot: slot(0, 1),
            content_size: Size::new(70.0, 45.0),
            inner_edges: Edges::default(),
            outer_edges: Edges::default(),
            total_edges: Edges::default(),
        },
    ];
    let requirements_a =
        GridRequirements::from_items(shape, Size::default(), &items_a).expect("a fits");
    let requirements_b =
        GridRequirements::from_items(shape, Size::default(), &items_b).expect("b fits");

    let plan = align(&[
        AlignmentNode {
            id: "a",
            group_key: "row",
            requirements: requirements_a.clone(),
        },
        AlignmentNode {
            id: "b",
            group_key: "row",
            requirements: requirements_b.clone(),
        },
    ]);
    assert_eq!(plan.group_count(), 1);
    let merged = &plan.groups[0].merged;
    assert_eq!(merged.column_widths, vec![110.0, 120.0]);
    assert_eq!(merged.column_left[0].total, 26.0);

    let scenes = vec![
        (
            "a — own requirements",
            DebugScene::from_grid(&requirements_a.solve(&items_a), &items_a),
        ),
        (
            "b — own requirements",
            DebugScene::from_grid(&requirements_b.solve(&items_b), &items_b),
        ),
        (
            "a — merged (already the max)",
            DebugScene::from_grid(&merged.solve(&items_a), &items_a),
        ),
        (
            "b — merged (granted a's chrome)",
            DebugScene::from_grid(&merged.solve(&items_b), &items_b),
        ),
    ];
    assert_svg_baseline(
        "alignment_merges_grids_across_instances",
        &stack_scenes(scenes).to_svg(),
    );
}

/// `align_by` with a caller-owned payload and merge/delta laws, plus the
/// singleton policy: under `Merge`, a group of one still yields a patch.
/// Numeric only — alignment of custom payloads has no geometry to draw.
#[test]
fn align_by_custom_payload_and_singleton_policy() {
    #[derive(Clone, Debug, PartialEq)]
    struct Lane {
        before: f32,
        after: f32,
    }

    let plan = align_by(
        &[
            AlignmentNode {
                id: 0,
                group_key: "lane-0",
                requirements: Lane {
                    before: 4.0,
                    after: 10.0,
                },
            },
            AlignmentNode {
                id: 1,
                group_key: "lane-0",
                requirements: Lane {
                    before: 9.0,
                    after: 2.0,
                },
            },
            AlignmentNode {
                id: 2,
                group_key: "lane-1",
                requirements: Lane {
                    before: 1.0,
                    after: 1.0,
                },
            },
        ],
        SingletonPolicy::Merge,
        |lanes: &[&Lane]| {
            Some(lanes.iter().fold(
                Lane {
                    before: 0.0,
                    after: 0.0,
                },
                |merged, lane| Lane {
                    before: merged.before.max(lane.before),
                    after: merged.after.max(lane.after),
                },
            ))
        },
        |local, merged| {
            (
                0.0,
                (merged.before - local.before).abs() + (merged.after - local.after).abs(),
            )
        },
    );

    assert_eq!(plan.group_count(), 2);
    assert_eq!(plan.skipped.len(), 0);
    let lane0 = &plan.groups[0];
    assert_eq!(
        lane0.merged,
        Lane {
            before: 9.0,
            after: 10.0,
        }
    );
    assert_eq!(plan.changed_node_count(), 2);

    // EdgeDemand's lift law, the heart of layered merging: totals lift to
    // inner + outer so merged totals equal max(inner) + max(outer).
    let lifted = EdgeDemand::new(5.0, 8.0, 4.0);
    assert_eq!(lifted.total, 13.0);
}
