//! Diagnostic images, one per concept of the layout model, compared with baselines.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

use avenger_layout::diagnostics::{diagnostic_layer, diagnostic_scene};
use avenger_layout::{
    Column, Direction, Gap, Grid, GridSlot, Layout, Leaf, Measure, Measurement, Overflow, Overlay,
    Placement, Position, Row, Side, Size, Slab, Solution, SolveOptions, Track, Wrap,
};
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_svg::SvgRenderer;
use avenger_typst_label::bundled_label_engine;

#[path = "../examples/chart_grid/chart.rs"]
mod chart;

type Id = &'static str;
type L = Layout<Id>;

const H: Direction = Direction::Horizontal;
const V: Direction = Direction::Vertical;

fn leaf(width: f32, height: f32) -> Leaf<Id> {
    Leaf::new().min_size(Size::new(width, height))
}

fn auto(count: usize) -> Vec<Track> {
    vec![Track::auto(); count]
}

/// A Row whose columns are Auto, so that each takes its content's size.
fn auto_row(children: impl IntoIterator<Item = impl Into<L>>) -> Row<Id> {
    Row::new(children).tracks(Track::auto())
}

/// A Column whose rows are Auto, so that each takes its content's size.
fn auto_column(children: impl IntoIterator<Item = impl Into<L>>) -> Column<Id> {
    Column::new(children).tracks(Track::auto())
}

fn explained(width: Option<f32>, height: Option<f32>) -> SolveOptions {
    SolveOptions {
        width,
        height,
        explain: true,
        ..SolveOptions::default()
    }
}

fn render(name: &str, solution: &Solution<Id>) {
    let engine = bundled_label_engine();
    let scene = diagnostic_scene(solution, &|id| id.to_string(), &engine).unwrap();
    let svg = SvgRenderer::new()
        .render_scene_graph(&scene, &engine)
        .unwrap();
    let png = svg_to_png(&svg, 2.0);
    assert_baseline(env!("CARGO_MANIFEST_DIR"), &png, name);
}

fn solve(layout: impl Into<L>, options: SolveOptions) -> Solution<Id> {
    layout.into().solve(&options).unwrap()
}

#[test]
fn leaves_and_grids() {
    let grid = Grid::new(auto(3), auto(2))
        .column_gap(Gap::new(10.0))
        .row_gap(Gap::new(10.0))
        .child(
            GridSlot::new(0, 0),
            leaf(40.0, 20.0)
                .id("a")
                .place(H, Placement::fit(Position::Start))
                .place(V, Placement::fit(Position::Center)),
        )
        .child(GridSlot::new(1, 0), leaf(60.0, 30.0).id("b"))
        .child(
            GridSlot::new(0, 1).span(2, 1),
            leaf(150.0, 10.0).id("spanning"),
        )
        .child(GridSlot::new(2, 1), leaf(30.0, 30.0).id("d"));
    render("leaves_and_grids", &solve(grid, explained(None, None)));
}

#[test]
fn slabs() {
    let plot = leaf(220.0, 140.0)
        .id("plot")
        .slab(Side::Left, Slab::new().min_width(36.0).id("y axis"))
        .slab(Side::Left, Slab::new().min_width(16.0).id("y title"))
        .slab(Side::Bottom, Slab::new().min_height(24.0).id("x axis"))
        .slab(Side::Top, Slab::new().min_height(18.0).id("header"))
        .slab(Side::Top, Slab::new().min_height(26.0).id("title").full());
    render("slabs", &solve(plot, explained(None, None)));
}

#[test]
fn overlays() {
    let plot = leaf(260.0, 160.0)
        .id("plot")
        .slab(Side::Left, Slab::new().min_width(30.0).id("y axis"))
        .overlay(Overlay::new(leaf(0.0, 0.0).id("inset")).rect(0.6, 0.05, 0.95, 0.4));
    render("overlays", &solve(plot, explained(None, None)));
}

#[test]
fn overflow_policies() {
    let stack = |policy: Overflow, prefix: [Id; 4]| {
        let plot = |id: Id, axis: Id, width: f32| {
            leaf(90.0, 50.0)
                .id(id)
                .slab(Side::Left, Slab::new().min_width(width).id(axis))
                .overflow(Side::Left, policy)
        };
        auto_column([
            plot(prefix[0], prefix[1], 30.0),
            plot(prefix[2], prefix[3], 50.0),
        ])
        .gap(Gap::new(10.0))
    };
    let row = auto_row([
        stack(
            Overflow::Propagate,
            ["propagate a", "axis 1", "propagate b", "axis 2"],
        ),
        stack(
            Overflow::Contain,
            ["contain a", "axis 3", "contain b", "axis 4"],
        ),
        stack(Overflow::Flush, ["flush a", "axis 5", "flush b", "axis 6"]),
    ])
    .share(false)
    .gap(Gap::new(30.0));
    render("overflow_policies", &solve(row, explained(None, None)));
}

#[test]
fn gaps() {
    let row = auto_row([
        leaf(80.0, 60.0).id("a"),
        leaf(80.0, 60.0)
            .id("b")
            .slab(Side::Left, Slab::new().min_width(28.0).id("axis")),
        leaf(80.0, 60.0).id("c"),
    ])
    .gap(Gap::new(12.0).clearance(6.0));
    render("gaps", &solve(row, explained(None, None)));
}

#[test]
fn tracks() {
    let grid = Grid::new(
        [Track::fr_max(1.0, 300.0), Track::auto()],
        [Track::px(80.0), Track::fr(1.0)],
    )
    .child(GridSlot::new(0, 0), leaf(0.0, 0.0).id("x histogram"))
    .child(GridSlot::new(0, 1), leaf(0.0, 0.0).id("scatter"))
    .child(GridSlot::new(1, 1), leaf(60.0, 0.0).id("y histogram"));
    render("tracks", &solve(grid, explained(Some(420.0), Some(320.0))));
}

#[test]
fn aspect_tracks() {
    let mut maps = Grid::new(vec![Track::fr(1.0); 2], vec![Track::aspect(0, 1.0); 2])
        .column_gap(Gap::new(20.0))
        .row_gap(Gap::new(20.0))
        .fit_aspect();
    for (i, id) in ["m0", "m1", "m2", "m3"].into_iter().enumerate() {
        maps = maps.child(GridSlot::new(i % 2, i / 2), leaf(0.0, 0.0).id(id));
    }
    render(
        "aspect_tracks",
        &solve(maps, explained(Some(420.0), Some(320.0))),
    );
}

#[test]
fn table_facets() {
    let facet = |ids: &[Id]| {
        Row::new(ids.iter().map(|&id| leaf(0.0, 50.0).id(id)))
            .tracks(Track::fr(1.0).tag("column"))
            .gap(Gap::new(10.0))
    };
    let outer = auto_column([facet(&["a0", "a1", "a2"]), facet(&["b0", "b1", "b2", "b3"])])
        .gap(Gap::new(14.0))
        .match_tag("column");
    render("table_facets", &solve(outer, explained(Some(430.0), None)));
}

#[test]
fn three_facet_levels() {
    let panels = |a: Id, b: Id, axis: f32| {
        let second = leaf(60.0, 40.0)
            .id(b)
            .slab(Side::Left, Slab::new().min_width(axis));
        auto_row([leaf(60.0, 40.0).id(a), second]).gap(Gap::new(10.0).tag("panel"))
    };
    let facet = |panels: Row<Id>, legend: Id, title: f32| {
        let legend = leaf(40.0, 30.0)
            .id(legend)
            .slab(Side::Left, Slab::new().min_width(title));
        auto_row([Layout::from(panels), legend.into()]).gap(Gap::new(15.0).tag("legend"))
    };
    let outer = auto_row([
        facet(panels("a0", "a1", 0.0), "legend a", 0.0),
        facet(panels("b0", "b1", 20.0), "legend b", 25.0),
    ])
    .gap(Gap::new(30.0).tag("facet"))
    .match_tag("panel")
    .match_tag("legend");
    render("three_facet_levels", &solve(outer, explained(None, None)));
}

#[test]
fn placement() {
    let cell = |id: Id, placement: Placement| leaf(60.0, 16.0).id(id).place(H, placement);
    let grid = Grid::new([Track::px(200.0)], vec![Track::px(30.0); 4])
        .row_gap(Gap::new(6.0))
        .child(
            GridSlot::new(0, 0),
            cell("fit start", Placement::fit(Position::Start)),
        )
        .child(
            GridSlot::new(0, 1),
            cell("fit center", Placement::fit(Position::Center)),
        )
        .child(GridSlot::new(0, 2), cell("stretch", Placement::default()))
        .child(
            GridSlot::new(0, 3),
            cell("stretch to 120, end", Placement::stretch(Position::End)).max_width(120.0),
        );
    render("placement", &solve(grid, explained(None, None)));
}

struct Header;

impl Measure<Id> for Header {
    fn height(&mut self, id: &Id, _: f32) -> Option<Measurement> {
        (*id == "thin slab").then(|| Measurement::always(4.0))
    }
}

#[test]
fn small_elements() {
    let row = auto_row([
        leaf(70.0, 50.0).id("a"),
        leaf(70.0, 50.0).id("b"),
        leaf(70.0, 50.0)
            .id("c")
            .slab(Side::Top, Slab::new().id("thin slab")),
        leaf(70.0, 50.0)
            .id("d")
            .overlay(Overlay::new(leaf(0.0, 0.0).id("dot")).rect(0.45, 0.45, 0.55, 0.55)),
    ])
    .gap(Gap::new(2.0).clearance(0.0));
    let grid = auto_column([
        Layout::from(row),
        // A name too long for its region, which a line fits across: it ends in an ellipsis.
        auto_row([
            leaf(110.0, 30.0).id("e"),
            leaf(110.0, 30.0).id("f"),
            leaf(54.0, 30.0).id("a name longer than its region"),
        ])
        .gap(Gap::new(6.0))
        .into(),
    ])
    .share(false)
    .gap(Gap::new(3.0));
    let solution = L::from(grid)
        .measure(&explained(None, None), &mut Header)
        .unwrap()
        .into_solution();
    render("small_elements", &solution);
}

/// The guides of a two-region facet: the README's example of collapse and merge.
struct Guides;

impl Measure<Id> for Guides {
    fn height(&mut self, id: &Id, _: f32) -> Option<Measurement> {
        (id.starts_with("x labels") || id.starts_with("header")).then(|| Measurement::always(16.0))
    }

    fn width(&mut self, id: &Id, _: f32) -> Option<Measurement> {
        id.starts_with("y labels")
            .then(|| Measurement::always(28.0))
    }
}

#[test]
fn shared_guides() {
    const PLOTS: [Id; 6] = ["plot 0", "plot 1", "plot 2", "plot 3", "plot 4", "plot 5"];
    const X_LABELS: [Id; 6] = [
        "x labels 0",
        "x labels 1",
        "x labels 2",
        "x labels 3",
        "x labels 4",
        "x labels 5",
    ];
    const X_TITLES: [Id; 6] = [
        "x title 0",
        "x title 1",
        "x title 2",
        "x title 3",
        "x title 4",
        "x title 5",
    ];
    const Y_LABELS: [Id; 6] = [
        "y labels 0",
        "y labels 1",
        "y labels 2",
        "y labels 3",
        "y labels 4",
        "y labels 5",
    ];
    const Y_TITLES: [Id; 6] = [
        "y title 0",
        "y title 1",
        "y title 2",
        "y title 3",
        "y title 4",
        "y title 5",
    ];
    const LEGENDS: [Id; 6] = [
        "legend 0", "legend 1", "legend 2", "legend 3", "legend 4", "legend 5",
    ];
    let panel = |p: usize, y: &'static str| {
        Leaf::new()
            .min_size(Size::new(120.0, 90.0))
            .id(PLOTS[p])
            .slab(Side::Bottom, Slab::new().min_height(6.0))
            .slab(
                Side::Bottom,
                Slab::new()
                    .id(X_LABELS[p])
                    .tag("x-labels")
                    .content_key("x-labels/x"),
            )
            .slab(
                Side::Bottom,
                Slab::new()
                    .min_size(Size::new(60.0, 14.0))
                    .id(X_TITLES[p])
                    .tag("x-title")
                    .content_key("x-title/Month")
                    .place(Placement::fit(Position::Center)),
            )
            .slab(Side::Left, Slab::new().min_width(6.0))
            .slab(
                Side::Left,
                Slab::new().id(Y_LABELS[p]).tag("y-labels").content_key(y),
            )
            .slab(
                Side::Left,
                Slab::new()
                    .min_size(Size::new(14.0, 60.0))
                    .id(Y_TITLES[p])
                    .tag("y-title")
                    .content_key("y-title/Sales")
                    .place(Placement::fit(Position::Center)),
            )
            .slab(
                Side::Right,
                Slab::new()
                    .min_size(Size::new(50.0, 60.0))
                    .id(LEGENDS[p])
                    .tag("legend")
                    .content_key("legend/channel")
                    .place(Placement::fit(Position::Start)),
            )
    };
    let region = |header: Id, y: &'static str, panels: [usize; 3]| {
        Row::new(panels.map(|p| panel(p, y)))
            .gap(Gap::new(8.0))
            .slab(Side::Top, Slab::new().id(header))
    };
    let figure = Column::new([
        region("header north", "y-labels/north", [0, 1, 2]),
        region("header south", "y-labels/south", [3, 4, 5]),
    ])
    .gap(Gap::new(8.0))
    .collapse_tag("x-labels")
    .collapse_tag("y-labels")
    .merge_tag("x-title")
    .merge_tag("y-title")
    .merge_tag("legend");
    let solution = L::from(figure)
        .measure(&explained(None, None), &mut Guides)
        .unwrap()
        .into_solution();
    render("shared_guides", &solution);
}

/// Measures a facet wrap's guides: one answer per content key.
struct WrapGuides;

impl Measure<Id> for WrapGuides {
    fn height(&mut self, id: &Id, _: f32) -> Option<Measurement> {
        id.starts_with("x labels")
            .then(|| Measurement::always(16.0))
    }

    fn width(&mut self, id: &Id, _: f32) -> Option<Measurement> {
        id.starts_with("y labels")
            .then(|| Measurement::always(26.0))
    }
}

#[test]
fn facet_wrap() {
    const PLOTS: [Id; 5] = ["panel 0", "panel 1", "panel 2", "panel 3", "panel 4"];
    const X: [Id; 5] = [
        "x labels 0",
        "x labels 1",
        "x labels 2",
        "x labels 3",
        "x labels 4",
    ];
    const Y: [Id; 5] = [
        "y labels 0",
        "y labels 1",
        "y labels 2",
        "y labels 3",
        "y labels 4",
    ];
    const TITLES: [Id; 5] = [
        "x title 0",
        "x title 1",
        "x title 2",
        "x title 3",
        "x title 4",
    ];
    let panel = |i: usize| {
        leaf(90.0, 60.0)
            .id(PLOTS[i])
            .slab(
                Side::Bottom,
                Slab::new().id(X[i]).tag("x-labels").content_key("x"),
            )
            .slab(
                Side::Bottom,
                Slab::new()
                    .min_size(Size::new(50.0, 14.0))
                    .id(TITLES[i])
                    .tag("x-title")
                    .content_key("Month")
                    .place(Placement::fit(Position::Center)),
            )
            .slab(
                Side::Left,
                Slab::new().id(Y[i]).tag("y-labels").content_key("y"),
            )
    };
    let wrap = Wrap::new((0..5).map(panel))
        .min_column(110.0)
        .gap(Gap::new(10.0))
        .collapse_tag("x-labels")
        .collapse_tag("y-labels")
        .merge_tag("x-title")
        .id("wrap");
    let solution = L::from(wrap)
        .measure(&explained(Some(420.0), None), &mut WrapGuides)
        .unwrap()
        .into_solution();
    render("facet_wrap", &solution);
}

#[test]
fn chart_grid_overlay() {
    let specs = chart::specs();
    let engine = chart::text_engine();
    let measured = chart::solve(&specs, &engine, true);
    let solution = measured.solution();
    let mut scene = chart::scene(&specs, solution, &engine);
    let layer = diagnostic_layer(solution, &|id| format!("{id:?}"), &engine).unwrap();
    let Some(SceneMark::Group(root)) = scene.marks.first_mut() else {
        panic!("the chart scene has a root group");
    };
    root.marks.push(layer.into());
    let svg = SvgRenderer::new()
        .render_scene_graph(&scene, &engine)
        .unwrap();
    assert_baseline(
        env!("CARGO_MANIFEST_DIR"),
        &svg_to_png(&svg, 2.0),
        "chart_grid_overlay",
    );
}

#[test]
fn canvas_padding() {
    let row = auto_row([
        leaf(120.0, 80.0)
            .id("a")
            .slab(Side::Left, Slab::new().min_width(30.0).id("y axis")),
        leaf(120.0, 80.0).id("b"),
    ])
    .gap(Gap::new(12.0));
    let options = SolveOptions {
        padding: avenger_layout::Edges::new(12.0, 8.0, 24.0, 16.0),
        ..explained(None, None)
    };
    render("canvas_padding", &solve(row, options));
}
