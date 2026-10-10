//! Public-API examples rendered to SVG and compared against committed
//! baselines.
//!
//! Each test builds a [`Layout`], solves it in one step, and compares the
//! solution's SVG against `tests/baselines/<name>.svg`. The baselines double
//! as a visual gallery of everything the unified API does — open them in a
//! browser, or see `tests/baselines/README.md` for the index.
//!
//! To regenerate baselines after an intentional change:
//!
//! ```sh
//! AVENGER_LAYOUT_BLESS=1 cargo test --release -p avenger-layout --test svg_baselines
//! ```
//!
//! On mismatch the actual SVG is written to `tests/failures/<name>.svg`.

use std::fs;
use std::path::PathBuf;

use avenger_layout::{
    CellAlign, Distribute, EdgeDemand, Layout, RegionDetail, Side, Size, SolveFor, SolveOptions,
    Spacing, TrackSize, svg_panels,
};

/// Most tests share string ids and string share keys.
type L = Layout<&'static str, &'static str>;

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
             AVENGER_LAYOUT_BLESS=1 cargo test --release -p avenger-layout --test svg_baselines"
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

fn natural() -> SolveOptions {
    SolveOptions::default()
}

fn allocated(width: f32, height: f32) -> SolveOptions {
    SolveOptions {
        width: Some(width),
        height: Some(height),
    }
}

// --- rows, columns, and the gap law -----------------------------------------

/// Sibling boundary chrome becomes inter-child gaps, floored by `min_gap`:
/// the first pair's chrome (8 + 14 = 22) wins over the floor, the second
/// pair's (2 + 1 = 3) is floored to 12. `outer_start`/`outer_end` reserve
/// space before the first and after the last child.
#[test]
fn row_gaps_and_min_gap_floor() {
    let root: L = Layout::row(vec![
        Layout::leaf(Size::new(60.0, 80.0))
            .demand(
                Side::Left,
                EdgeDemand {
                    guide: 5.0,
                    legend: 0.0,
                },
            )
            .demand(
                Side::Right,
                EdgeDemand {
                    guide: 8.0,
                    legend: 0.0,
                },
            )
            .id("a"),
        Layout::leaf(Size::new(90.0, 80.0))
            .demand(
                Side::Left,
                EdgeDemand {
                    guide: 14.0,
                    legend: 0.0,
                },
            )
            .demand(
                Side::Right,
                EdgeDemand {
                    guide: 2.0,
                    legend: 0.0,
                },
            )
            .id("b"),
        Layout::leaf(Size::new(45.0, 80.0))
            .demand(
                Side::Left,
                EdgeDemand {
                    guide: 1.0,
                    legend: 0.0,
                },
            )
            .id("c"),
    ])
    .column_spacing(Spacing {
        outer_start: 6.0,
        outer_end: 10.0,
        min_gap: 12.0,
    });
    let solved = root.solve(&natural()).expect("solve");

    let a = solved.region(&"a").unwrap();
    let b = solved.region(&"b").unwrap();
    let c = solved.region(&"c").unwrap();
    assert_eq!(b.slot.x - a.slot.x, 82.0); // 60 + (8 + 14)
    assert_eq!(c.slot.x - b.slot.x, 102.0); // 90 + floored 12

    assert_svg_baseline("row_gaps_and_min_gap_floor", &solved.to_svg());
}

/// A vertical arrangement with ragged widths: per-child `CellAlign` centers
/// or end-aligns the narrower children, and the dashed slot outlines show
/// the granted space their content does not fill.
#[test]
fn column_cell_align_ragged_children() {
    let root: L = Layout::column(vec![
        Layout::leaf(Size::new(120.0, 40.0)).id("wide"),
        Layout::leaf(Size::new(70.0, 40.0))
            .align_in_cell(CellAlign::Center, CellAlign::Start)
            .id("center"),
        Layout::leaf(Size::new(95.0, 40.0))
            .align_in_cell(CellAlign::End, CellAlign::Start)
            .id("end"),
    ])
    .min_gap(8.0);
    let solved = root.solve(&natural()).expect("solve");

    let centered = solved.region(&"center").unwrap();
    assert_eq!(centered.slot.width, 120.0);
    assert_eq!(centered.content.x, 25.0);
    assert_eq!(centered.content.width, 70.0);

    assert_svg_baseline("column_cell_align_ragged_children", &solved.to_svg());
}

/// The solution is queryable: regions by caller id or structural path, with
/// the slot (allotment) and honest content rectangles distinct.
#[test]
fn solution_query_by_id_and_path() {
    let root: L = Layout::row(vec![
        Layout::leaf(Size::new(70.0, 50.0)).id("first"),
        Layout::leaf(Size::new(50.0, 60.0)).id("second"),
    ])
    .min_gap(16.0);
    let solved = root.solve(&natural()).expect("solve");

    assert_eq!(solved.region(&"second").unwrap().slot.x, 86.0); // 70 + 16
    assert_eq!(solved.at_path(&[1]).unwrap().id, Some("second"));
    assert_eq!(solved.at_path(&[]).unwrap().depth, 0); // the root grid

    assert_svg_baseline("solution_query_by_id_and_path", &solved.to_svg());
}

// --- grids -------------------------------------------------------------------

/// Column span, an empty slot, and a per-track base cell size floor.
#[test]
fn grid_spans_holes_and_base_cell_size() {
    let root: L = Layout::grid(2, 3)
        .cell(0, 0, Layout::leaf(Size::new(100.0, 60.0)).id("a"))
        .cell(1, 2, Layout::leaf(Size::new(100.0, 60.0)).id("b"))
        .cell_span(
            1,
            0,
            1,
            2,
            Layout::leaf(Size::new(170.0, 60.0)).id("wide span"),
        )
        .base_cell_size(Size::new(70.0, 50.0));
    let solved = root.solve(&natural()).expect("solve");

    let RegionDetail::Grid { tracks } = &solved.at_path(&[]).unwrap().detail else {
        panic!("grid expected");
    };
    assert_eq!(tracks.column_sizes.len(), 3);
    assert_eq!(tracks.row_sizes, vec![60.0, 60.0]);

    assert_svg_baseline("grid_spans_holes_and_base_cell_size", &solved.to_svg());
}

/// Layered demand and the gap law: each side carries guide (red) and legend
/// (green) layers; interior boundary edges become gaps via
/// `max(min_gap, after + before)` while first/last edges overlap the
/// container's envelope.
#[test]
fn grid_edge_demand_layers_and_gap_law() {
    let root: L = Layout::row(vec![
        Layout::leaf(Size::new(100.0, 60.0))
            .demand(
                Side::Right,
                EdgeDemand {
                    guide: 6.0,
                    legend: 20.0,
                },
            )
            .demand(
                Side::Left,
                EdgeDemand {
                    guide: 9.0,
                    legend: 0.0,
                },
            )
            .id("a"),
        Layout::leaf(Size::new(100.0, 60.0))
            .demand(
                Side::Left,
                EdgeDemand {
                    guide: 0.0,
                    legend: 3.0,
                },
            )
            .demand(
                Side::Top,
                EdgeDemand {
                    guide: 7.0,
                    legend: 11.0,
                },
            )
            .id("b"),
    ]);
    let solved = root.solve(&natural()).expect("solve");

    let a = solved.region(&"a").unwrap();
    let b = solved.region(&"b").unwrap();
    assert_eq!(b.slot.x - a.slot.x, 129.0); // 100 + gap (26 + 3)
    assert_eq!(solved.envelope().coordinated.left.guide, 9.0);
    assert_eq!(solved.envelope().coordinated.top.total, 18.0);

    assert_svg_baseline("grid_edge_demand_layers_and_gap_law", &solved.to_svg());
}

// --- chromed leaves (the chart canvas) ---------------------------------------

fn chart_canvas() -> L {
    Layout::leaf(Size::default())
        .margin(10.0)
        .strip(Side::Top, 18.0)
        .strip(Side::Top, 12.0)
        .legend(Side::Right, 40.0)
        .legend(Side::Bottom, 26.0)
        .guide(Side::Left, 30.0)
        .guide(Side::Bottom, 16.0)
        .content_min(Size::new(50.0, 40.0))
        .id("chart")
}

/// Canvas-style sizing: the canvas (the root envelope) is given
/// (`SolveFor::Content`) and the chrome is subtracted from it — title and
/// subtitle strips, a legend column and caption row (legend), and axis
/// strips (guide).
#[test]
fn chromed_leaf_solve_for_content() {
    let solved = chart_canvas()
        .sizing(SolveFor::Content)
        .solve(&allocated(360.0, 240.0))
        .expect("solve");

    let chart = solved.region(&"chart").unwrap();
    // 360 - (10 + 30) - (10 + 40) = 270 content width.
    assert_eq!(chart.content.width, 270.0);
    assert_eq!(solved.size, Size::new(360.0, 240.0));

    assert_svg_baseline("chromed_leaf_solve_for_content", &solved.to_svg());
}

/// Content-first sizing: the plot area is given and the envelope (the
/// canvas, at the root) is the sum of all layers (`SolveFor::Envelope`,
/// the default).
#[test]
fn chromed_leaf_solve_for_envelope() {
    let chart: L = Layout::leaf(Size::new(220.0, 140.0))
        .margin(10.0)
        .strip(Side::Top, 18.0)
        .legend(Side::Right, 40.0)
        .guide(Side::Left, 30.0)
        .guide(Side::Bottom, 16.0)
        .id("chart");
    let solved = chart.solve(&natural()).expect("solve");

    assert_eq!(solved.size, Size::new(310.0, 194.0));
    let chart = solved.region(&"chart").unwrap();
    assert_eq!(chart.content.x, 40.0); // margin 10 + guide 30

    assert_svg_baseline("chromed_leaf_solve_for_envelope", &solved.to_svg());
}

/// Both canvas and content given: the margins absorb the slack
/// (`SolveFor::Margins`), centering the content.
#[test]
fn chromed_leaf_solve_for_margins() {
    let chart: L = Layout::leaf(Size::new(180.0, 110.0))
        .margin(5.0) // declared, but ignored: margins are the flexible layer
        .guide(Side::Left, 30.0)
        .guide(Side::Bottom, 16.0)
        .id("chart")
        .sizing(SolveFor::Margins);
    let solved = chart.solve(&allocated(340.0, 200.0)).expect("solve");

    let chart = solved.region(&"chart").unwrap();
    assert_eq!(chart.content.width, 180.0);
    // Horizontal slack: 340 - 30 - 180 = 130, split 65/65.
    assert_eq!(chart.content.x, 65.0 + 30.0);

    assert_svg_baseline("chromed_leaf_solve_for_margins", &solved.to_svg());
}

/// The content floor wins over a too-small envelope: chrome plus
/// `content_min` exceed the allocation, so the solved envelope grows past
/// it.
#[test]
fn chromed_leaf_content_min_overflows_envelope() {
    let solved = chart_canvas()
        .sizing(SolveFor::Content)
        .solve(&allocated(110.0, 100.0))
        .expect("solve");

    assert!(solved.size.width > 110.0);
    let chart = solved.region(&"chart").unwrap();
    assert_eq!(chart.content.width, 50.0); // the floor

    assert_svg_baseline(
        "chromed_leaf_content_min_overflows_envelope",
        &solved.to_svg(),
    );
}

/// Per-axis constraints: width figure-sized, height plot-area-sized — the
/// fixed-subplot pattern, loop-free because the height-axis measurement is
/// authoritative.
#[test]
fn per_axis_allocation_plot_sized_height() {
    let chart: L = Layout::leaf(Size::new(0.0, 150.0))
        .margin(8.0)
        .guide(Side::Left, 30.0)
        .guide(Side::Bottom, 16.0)
        .sizing_x(SolveFor::Content)
        .id("chart");
    let solved = chart
        .solve(&SolveOptions {
            width: Some(320.0),
            height: None,
        })
        .expect("solve");

    assert_eq!(solved.size.width, 320.0);
    assert_eq!(solved.size.height, 8.0 + 150.0 + 16.0 + 8.0);

    assert_svg_baseline("per_axis_allocation_plot_sized_height", &solved.to_svg());
}

/// Bands on all four sides carve with the corner-ownership rule: layers
/// carve outside-in, vertical sides before horizontal, so the top strip runs
/// wider than the left strip of the same layer.
#[test]
fn strips_all_four_sides_corner_rule() {
    let solved = Layout::<&str>::leaf(Size::default())
        .margin(10.0)
        .strip(Side::Top, 20.0)
        .strip(Side::Left, 26.0)
        .strip(Side::Bottom, 14.0)
        .strip(Side::Right, 18.0)
        .guide(Side::Left, 12.0)
        .sizing(SolveFor::Content)
        .id("boxed")
        .solve(&allocated(320.0, 200.0))
        .expect("solve");

    assert_svg_baseline("strips_all_four_sides_corner_rule", &solved.to_svg());
}

// --- nested grids with chrome -------------------------------------------------

/// A nested column inside a row, where the nested grid carries its own
/// chrome: a header strip on the guide stratum (so cousins would
/// coordinate it) and a legend strip on the legend stratum.
/// Chrome on the grid replaces the old `stacked_inner/outer_edges`.
#[test]
fn nested_grid_with_chrome() {
    let nested: L = Layout::column(vec![
        Layout::leaf(Size::new(90.0, 50.0))
            .demand(
                Side::Right,
                EdgeDemand {
                    guide: 15.0,
                    legend: 0.0,
                },
            )
            .id("c0"),
        Layout::leaf(Size::new(90.0, 56.0)).id("c1"),
    ])
    .min_gap(10.0)
    .guide(Side::Top, 16.0)
    .legend(Side::Right, 22.0)
    .id("group");
    let root: L = Layout::row(vec![
        Layout::leaf(Size::new(70.0, 120.0)).id("solo"),
        nested,
    ])
    .min_gap(12.0)
    .margin(12.0);
    let solved = root.solve(&natural()).expect("solve");

    let group = solved.region(&"group").unwrap();
    assert_eq!(group.requested.top.guide, 16.0);
    assert_eq!(group.requested.right.total, 15.0 + 22.0);

    assert_svg_baseline("nested_grid_with_chrome", &solved.to_svg());
}

/// The same nested arrangement granted a larger canvas: tracks stretch
/// evenly (the default `Distribute::StretchTracks`), slots grow, and the
/// honest leaf content shows the slack as dashed slot outlines.
#[test]
fn allocation_stretches_tracks_evenly() {
    let nested: L = Layout::row(vec![
        Layout::leaf(Size::new(60.0, 60.0)).id("i0"),
        Layout::leaf(Size::new(60.0, 60.0)).id("i1"),
    ])
    .min_gap(6.0);
    let root: L = Layout::row(vec![Layout::leaf(Size::new(50.0, 60.0)).id("solo"), nested])
        .min_gap(10.0)
        .margin(10.0)
        .sizing(SolveFor::Content);
    let solved = root.solve(&allocated(320.0, 110.0)).expect("solve");

    let i1 = solved.region(&"i1").unwrap();
    assert!(i1.slot.width > 60.0, "stretched slot");
    assert_eq!(i1.content.width, 60.0, "honest content");

    assert_svg_baseline("allocation_stretches_tracks_evenly", &solved.to_svg());
}

// --- track sizing and distribution ---------------------------------------------

/// Uneven tracks, CSS-style: a rigid `Fixed` gutter, weighted `Flex`
/// tracks splitting the leftover 2:1, and a content-sized `Auto` track.
#[test]
fn track_size_fixed_and_flex() {
    let root: L = Layout::row(vec![
        Layout::leaf(Size::new(30.0, 70.0)).id("fixed 50"),
        Layout::leaf(Size::new(40.0, 70.0)).id("flex 2"),
        Layout::leaf(Size::new(40.0, 70.0)).id("flex 1"),
        Layout::leaf(Size::new(60.0, 70.0)).id("auto"),
    ])
    .columns([
        TrackSize::Fixed(50.0),
        TrackSize::Flex(2.0),
        TrackSize::Flex(1.0),
        TrackSize::Auto,
    ])
    .min_gap(8.0)
    .id("grid");
    let solved = root
        .solve(&SolveOptions {
            width: Some(370.0),
            height: None,
        })
        .expect("solve");

    let RegionDetail::Grid { tracks } = &solved.region(&"grid").unwrap().detail else {
        panic!("grid expected");
    };
    // Natural: 50 + 40 + 40 + 60 + 3 gaps of 8 = 214; 156 free split 2:1.
    assert_eq!(tracks.column_sizes, vec![50.0, 144.0, 92.0, 60.0]);

    assert_svg_baseline("track_size_fixed_and_flex", &solved.to_svg());
}

/// `Distribute::SpaceBetween`: with no `Flex` tracks, the free space lands
/// in the inter-track gaps instead of stretching the tracks.
#[test]
fn distribute_space_between() {
    let root: L = Layout::row(vec![
        Layout::leaf(Size::new(60.0, 50.0)).id("a"),
        Layout::leaf(Size::new(60.0, 50.0)).id("b"),
        Layout::leaf(Size::new(60.0, 50.0)).id("c"),
    ])
    .distribute_x(Distribute::SpaceBetween);
    let solved = root
        .solve(&SolveOptions {
            width: Some(300.0),
            height: None,
        })
        .expect("solve");

    assert_eq!(solved.region(&"b").unwrap().slot.x, 120.0);
    assert_eq!(solved.region(&"c").unwrap().slot.x, 240.0);

    assert_svg_baseline("distribute_space_between", &solved.to_svg());
}

/// `Fixed` never grows for oversized content: the child keeps its measured
/// size and honestly overflows the rigid track.
#[test]
fn fixed_track_content_overflow() {
    let root: L = Layout::row(vec![
        Layout::leaf(Size::new(110.0, 50.0)).id("too wide"),
        Layout::leaf(Size::new(70.0, 50.0)).id("fits"),
    ])
    .columns([TrackSize::Fixed(60.0), TrackSize::Auto])
    .min_gap(10.0);
    let solved = root.solve(&natural()).expect("solve");

    let wide = solved.region(&"too wide").unwrap();
    assert_eq!(wide.slot.width, 60.0);
    assert_eq!(wide.content.width, 110.0);

    assert_svg_baseline("fixed_track_content_overflow", &solved.to_svg());
}

// --- share-key coordination -----------------------------------------------------

/// Two uniform bands with different track counts share one key: the ragged
/// member adopts the merged uniform track size (the policy merge that
/// subsumes the chart's `band_n` trick).
#[test]
fn uniform_share_tolerates_ragged_counts() {
    let build = |share: bool| -> L {
        let with_key = |grid: L| if share { grid.share("bands") } else { grid };
        Layout::column(vec![
            with_key(
                Layout::row(vec![
                    Layout::leaf(Size::new(100.0, 50.0)).id("a0"),
                    Layout::leaf(Size::new(80.0, 50.0)).id("a1"),
                    Layout::leaf(Size::new(90.0, 50.0)).id("a2"),
                ])
                .uniform_columns()
                .min_gap(10.0)
                .id("three"),
            ),
            with_key(
                Layout::row(vec![
                    Layout::leaf(Size::new(60.0, 50.0)).id("b0"),
                    Layout::leaf(Size::new(120.0, 50.0)).id("b1"),
                ])
                .uniform_columns()
                .min_gap(10.0)
                .id("two"),
            ),
        ])
        .min_gap(16.0)
    };
    let before = build(false).solve(&natural()).expect("solve");
    let after = build(true).solve(&natural()).expect("solve");

    let RegionDetail::Grid { tracks } = &after.region(&"two").unwrap().detail else {
        panic!("grid expected");
    };
    assert!(tracks.column_sizes.iter().all(|&width| width == 120.0));

    assert_svg_baseline(
        "uniform_share_tolerates_ragged_counts",
        &svg_panels(&[("measured", &before), ("coordinated", &after)]),
    );
}

/// The nested row/col facet lowering, whole: two facet columns measured
/// with different cell sizes and chrome, shared as cousins, inside a
/// margined figure. Coordination grants column b the larger chrome
/// (hatched) and grows column a's slots to the merged width (dashed
/// outlines); the inter-column gap absorbs the granted chrome.
#[test]
fn nested_facet_columns_coordinated() {
    let cell = |width: f32, height: f32, left: f32, bottom: f32| -> L {
        Layout::leaf(Size::new(width, height))
            .demand(
                Side::Left,
                EdgeDemand {
                    guide: left,
                    legend: 0.0,
                },
            )
            .demand(
                Side::Bottom,
                EdgeDemand {
                    guide: bottom,
                    legend: 0.0,
                },
            )
    };
    let build = |share: bool| -> L {
        let with_key = |grid: L| if share { grid.share("cols") } else { grid };
        Layout::row(vec![
            with_key(
                Layout::column(vec![
                    cell(110.0, 50.0, 26.0, 18.0).id("a0"),
                    cell(110.0, 70.0, 26.0, 0.0).id("a1"),
                ])
                .min_gap(14.0)
                .id("col a"),
            ),
            with_key(
                Layout::column(vec![
                    cell(150.0, 64.0, 9.0, 5.0).id("b0"),
                    cell(150.0, 40.0, 9.0, 0.0).id("b1"),
                ])
                .min_gap(14.0)
                .id("col b"),
            ),
        ])
        .min_gap(14.0)
        .margin(12.0)
    };
    let before = build(false).solve(&natural()).expect("solve");
    let after = build(true).solve(&natural()).expect("solve");

    let b0 = after.region(&"b0").unwrap();
    assert_eq!(b0.granted.left.total, 26.0);
    assert_eq!(b0.requested.left.total, 9.0);
    let a0 = after.region(&"a0").unwrap();
    assert_eq!(a0.slot.width, 150.0);
    assert_eq!(a0.slot.y, after.region(&"b0").unwrap().slot.y);

    assert_svg_baseline(
        "nested_facet_columns_coordinated",
        &svg_panels(&[("measured", &before), ("coordinated", &after)]),
    );
}

/// The full loop on whole charts under one root: two chart-like groups
/// (margined grids of cells with axis chrome) share a key; coordination
/// makes their plot grids congruent in one solve.
#[test]
fn shared_charts_coordinate_in_one_solve() {
    let chart = |id: &'static str, width: f32, left: f32, bottom: f32| -> L {
        Layout::row(vec![
            Layout::leaf(Size::new(width, 90.0))
                .demand(
                    Side::Left,
                    EdgeDemand {
                        guide: left,
                        legend: 0.0,
                    },
                )
                .demand(
                    Side::Bottom,
                    EdgeDemand {
                        guide: bottom,
                        legend: 0.0,
                    },
                ),
            Layout::leaf(Size::new(width, 90.0)).demand(
                Side::Bottom,
                EdgeDemand {
                    guide: bottom,
                    legend: 0.0,
                },
            ),
        ])
        .min_gap(12.0)
        .share("plots")
        .margin(10.0)
        .strip(Side::Top, 14.0)
        .id(id)
    };
    let root: L = Layout::row(vec![
        chart("left chart", 80.0, 24.0, 18.0),
        chart("right chart", 110.0, 9.0, 4.0),
    ])
    .min_gap(18.0);
    let solved = root.solve(&natural()).expect("solve");

    let left = solved.region(&"left chart").unwrap();
    let right = solved.region(&"right chart").unwrap();
    let (RegionDetail::Grid { tracks: l }, RegionDetail::Grid { tracks: r }) =
        (&left.detail, &right.detail)
    else {
        panic!("grids expected");
    };
    assert_eq!(l.column_sizes, r.column_sizes, "congruent cousins");

    assert_svg_baseline("shared_charts_coordinate_in_one_solve", &solved.to_svg());
}

/// Asymmetric offers and the min-slack rule: one shared grid sits in a slot
/// widened by a sibling, the other does not. The group stretches by the
/// minimum offer (zero), so the cousins stay congruent and the wide slot's
/// slack shows as a dashed outline.
#[test]
fn min_slack_asymmetric_share() {
    let plot = |id: &'static str| -> L {
        Layout::row(vec![Layout::leaf(Size::new(100.0, 50.0))])
            .share("g")
            .id(id)
    };
    let root: L = Layout::row(vec![
        Layout::column(vec![
            plot("cramped"),
            Layout::leaf(Size::new(200.0, 50.0)).id("wide"),
        ])
        .min_gap(12.0),
        Layout::column(vec![plot("roomy")]).min_gap(12.0),
    ])
    .min_gap(16.0);
    let solved = root.solve(&natural()).expect("solve");

    assert_eq!(solved.region(&"cramped").unwrap().content.width, 100.0);
    assert_eq!(solved.region(&"roomy").unwrap().content.width, 100.0);
    assert_eq!(solved.region(&"cramped").unwrap().slot.width, 200.0);

    assert_svg_baseline("min_slack_asymmetric_share", &solved.to_svg());
}

/// Ways to reserve 18px on a cell's trailing edge, demonstrated against
/// a share-key cousin whose matching edge carries layered chrome
/// (guide 14 + legend 8, total 22). The mechanisms differ in which
/// coordination contract the space signs:
///
/// - `EdgeDemand { guide: 0, legend: 18 }` meets the cousin in DIFFERENT
///   strata: the merged edge must hold the worst guide AND the worst
///   legend at common offsets, so the coexistence lift takes both charts'
///   gaps to 14 + 18 = 32 — wider than either member's own ask.
/// - `EdgeDemand { guide: 18, legend: 0 }` meets the cousin's guide layer
///   in the SAME stratum, where coordination is containment: the merged
///   guide is max(18, 14) = 18, the cousin's legend 8 still stacks, and
///   both gaps settle at 26.
/// - `.strip(Side::Right, 18.0)` signs only the extent clause (chrome
///   lifts into the total, the private envelope): the cousin's 22
///   CONTAINS the 18 and the gaps stay at 22. The space is a DECLARATION
///   the solver owns: it returns a positioned slab (amber, visible
///   through the hatched grant) for the caller to fill, where demands
///   only clear room for material the caller already placed.
///
/// The fourth panel shows where demand and strip geometry genuinely
/// diverge: under `SolveFor::Content` the strip is part of the box (the
/// contained leaf is 18 wider, slab inside), while the demand is
/// overflow and is zeroed at the containment boundary — the 18 vanishes
/// from the solution entirely.
#[test]
fn edge_reservation_layered_vs_strip() {
    let scene = |reserve: &dyn Fn(L) -> L| {
        let cell = |id: &'static str| Layout::leaf(Size::new(110.0, 56.0)).id(id);
        let reference: L = Layout::row(vec![
            cell("a0").demand(
                Side::Right,
                EdgeDemand {
                    guide: 14.0,
                    legend: 8.0,
                },
            ),
            cell("a1"),
        ])
        .min_gap(6.0)
        .share("plots")
        .id("reference");
        let variant: L = Layout::row(vec![reserve(cell("b0")), cell("b1")])
            .min_gap(6.0)
            .share("plots")
            .id("variant");
        Layout::column(vec![reference, variant])
            .min_gap(16.0)
            .margin(8.0)
            .solve(&natural())
            .expect("solve")
    };
    let gap = |solved: &avenger_layout::LayoutSolution<&'static str>, left: &str, right: &str| {
        let left = solved.region(&left).unwrap().slot;
        solved.region(&right).unwrap().slot.x - (left.x + left.width)
    };

    let layered = scene(&|leaf| {
        leaf.demand(
            Side::Right,
            EdgeDemand {
                guide: 0.0,
                legend: 18.0,
            },
        )
    });
    // Coexistence: merged edge (14, 18) lifts the total to 32; the lift
    // exceeds every member's own ask (22 and 18), and cousins stay
    // congruent.
    assert_eq!(gap(&layered, "b0", "b1"), 32.0);
    assert_eq!(gap(&layered, "a0", "a1"), 32.0);

    let inner_stratum = scene(&|leaf| {
        leaf.demand(
            Side::Right,
            EdgeDemand {
                guide: 18.0,
                legend: 0.0,
            },
        )
    });
    // Same-stratum containment: merged guide max(18, 14) = 18 holds the
    // cousin's 14; the cousin's legend 8 still stacks on top.
    assert_eq!(gap(&inner_stratum, "b0", "b1"), 26.0);
    assert_eq!(gap(&inner_stratum, "a0", "a1"), 26.0);

    let strip = scene(&|leaf| leaf.strip(Side::Right, 18.0));
    // Extent-only contract: the cousin's 22 contains the 18, plus a
    // solver-positioned slab.
    assert_eq!(gap(&strip, "b0", "b1"), 22.0);
    let slabs = &strip.region(&"b0").unwrap().slabs;
    assert_eq!(slabs.len(), 1);
    assert_eq!(slabs[0].rect.width, 18.0);

    // Containment: the same two reservations behind a `SolveFor::Content`
    // boundary, one per row (the cousin pair can't show this — the
    // reference chart's layered 22 would keep the merged gap reserved and
    // mask the vanishing). The strip folds INTO the box (18 wider, slab
    // inside); the demand is overflow, zeroed at the boundary — no
    // reservation anywhere.
    // Each box sits in its own Start-distributed row: the column's shared
    // track offers both rows the wider extent (a contained box ADOPTS its
    // allocation as the box, so direct stretching would silently widen the
    // shed one), and Start keeps each box honest — the slack shows as a
    // dashed slot outline instead.
    let contained_row =
        |child: L| -> L { Layout::row(vec![child]).distribute_x(Distribute::Start) };
    let containment = Layout::<&'static str, &'static str>::column(vec![
        contained_row(
            Layout::leaf(Size::new(110.0, 56.0))
                .demand(
                    Side::Right,
                    EdgeDemand {
                        guide: 18.0,
                        legend: 0.0,
                    },
                )
                .sizing(SolveFor::Content)
                .id("contained demand"),
        ),
        contained_row(
            Layout::leaf(Size::new(110.0, 56.0))
                .strip(Side::Right, 18.0)
                .sizing(SolveFor::Content)
                .id("contained strip"),
        ),
    ])
    .min_gap(12.0)
    .margin(8.0)
    .solve(&natural())
    .expect("solve");
    let shed = containment.region(&"contained demand").unwrap();
    let kept = containment.region(&"contained strip").unwrap();
    assert_eq!(shed.slot.width, 110.0, "the demand vanished");
    assert_eq!(shed.requested.right.total, 0.0);
    assert_eq!(kept.slot.width, 128.0, "the strip is box structure");
    assert_eq!(kept.slabs[0].rect.width, 18.0);
    assert_eq!(kept.slot.y - (shed.slot.y + shed.slot.height), 12.0);

    assert_svg_baseline(
        "edge_reservation_layered_vs_strip",
        &svg_panels(&[
            (
                "legend 18 vs cousin 14+8: strata coexist, gaps 32",
                &layered,
            ),
            (
                "guide 18 vs cousin 14+8: same stratum contains, gaps 26",
                &inner_stratum,
            ),
            (
                "strip 18: extent-only, contained by the cousin's 22, gaps 22",
                &strip,
            ),
            (
                "containment, one box per row: demand 18 vanishes, strip stays",
                &containment,
            ),
        ]),
    );
}

/// A non-uniform share group whose members have different shapes is
/// skipped, reported in diagnostics, and rendered uncoordinated.
#[test]
fn share_group_shape_mismatch_diagnostics() {
    let root: L = Layout::column(vec![
        Layout::row(vec![
            Layout::leaf(Size::new(90.0, 40.0)).id("a0"),
            Layout::leaf(Size::new(70.0, 40.0)).id("a1"),
        ])
        .share("g")
        .id("pair"),
        Layout::row(vec![Layout::leaf(Size::new(120.0, 40.0)).id("b0")])
            .share("g")
            .id("single"),
    ])
    .min_gap(14.0);
    let solved = root.solve(&natural()).expect("solve");

    assert_eq!(solved.diagnostics().skipped_groups.len(), 1);

    assert_svg_baseline("share_group_shape_mismatch_diagnostics", &solved.to_svg());
}

// --- the aspect-ratio recipe ----------------------------------------------------

/// Contain-fit slack, the caller-side aspect recipe's terminal state: a
/// square (ratio-respecting) measurement inside a Flex track that
/// re-stretches wider every round. The standing slot-vs-content difference
/// is the durable record of the deliberate slack.
#[test]
fn aspect_contain_fit_slack() {
    let root: L = Layout::row(vec![
        Layout::leaf(Size::new(120.0, 120.0))
            .align_in_cell(CellAlign::Center, CellAlign::Start)
            .id("square"),
    ])
    .columns([TrackSize::Flex(1.0)])
    .id("grid");
    let solved = root
        .solve(&SolveOptions {
            width: Some(260.0),
            height: None,
        })
        .expect("solve");

    let square = solved.region(&"square").unwrap();
    assert_eq!(square.content.width, 120.0);
    assert_eq!(square.slot.width, 260.0);

    assert_svg_baseline("aspect_contain_fit_slack", &solved.to_svg());
}
