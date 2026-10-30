//! Tree-level behavior, each case checking rectangles with exact values.

use std::collections::HashMap;

use avenger_layout::{
    Cell, Column, Direction, Distribute, Edges, Explanation, Flow, Gap, Grid, GridSlot, GridTracks,
    Layout, LayoutError, Leaf, MatchedParts, Measure, Measurement, Overflow, Overlay, PathStep,
    Placement, Position, Rect, Region, RegionKind, RepresentationKind, Row, Side, Size, Slab,
    SlabStack, Solution, SolveOptions, Tag, Track, Warning, Wrap,
};

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

fn region(solution: &Solution<Id>, id: Id) -> &Region<Id> {
    solution
        .region(&id)
        .unwrap_or_else(|| panic!("no region {id}"))
}

fn content(solution: &Solution<Id>, id: Id) -> Rect {
    region(solution, id).content
}

fn slot(solution: &Solution<Id>, id: Id) -> Rect {
    region(solution, id).slot
}

fn tracks(solution: &Solution<Id>, id: Id) -> &GridTracks {
    let kind = &region(solution, id).kind;
    kind.tracks().unwrap_or_else(|| panic!("{id} is {kind:?}"))
}

fn widths(solution: &Solution<Id>, id: Id) -> Vec<f32> {
    tracks(solution, id)
        .columns
        .iter()
        .map(|t| t.length)
        .collect()
}

fn heights(solution: &Solution<Id>, id: Id) -> Vec<f32> {
    tracks(solution, id).rows.iter().map(|t| t.length).collect()
}

/// Solves without a canvas size, so the layout takes the size it needs.
fn no_canvas(layout: impl Into<L>) -> Solution<Id> {
    layout.into().solve(&SolveOptions::default()).unwrap()
}

fn sized(layout: impl Into<L>, width: f32, height: f32) -> Solution<Id> {
    layout
        .into()
        .solve(&SolveOptions::sized(width, height))
        .unwrap()
}

fn width_only(width: f32) -> SolveOptions {
    SolveOptions {
        width: Some(width),
        ..SolveOptions::default()
    }
}

/// A [`Measure`] from two functions, which counts the questions it gets per id.
struct Measurer {
    height: fn(Id, f32) -> Option<Measurement>,
    width: fn(Id, f32) -> Option<Measurement>,
    asked: HashMap<(Id, char), usize>,
}

impl Measurer {
    fn new(
        height: fn(Id, f32) -> Option<Measurement>,
        width: fn(Id, f32) -> Option<Measurement>,
    ) -> Self {
        Self {
            height,
            width,
            asked: HashMap::new(),
        }
    }

    fn heights(height: fn(Id, f32) -> Option<Measurement>) -> Self {
        Self::new(height, |_, _| None)
    }

    fn widths(width: fn(Id, f32) -> Option<Measurement>) -> Self {
        Self::new(|_, _| None, width)
    }

    fn asked(&self, id: Id, direction: char) -> usize {
        self.asked.get(&(id, direction)).copied().unwrap_or(0)
    }

    /// Questions that got an answer.
    fn answered(&self) -> usize {
        self.asked.values().sum()
    }
}

impl Measure<Id> for Measurer {
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        let answer = (self.height)(id, width);
        if answer.is_some() {
            *self.asked.entry((id, 'h')).or_default() += 1;
        }
        answer
    }

    fn width(&mut self, id: &Id, height: f32) -> Option<Measurement> {
        let answer = (self.width)(id, height);
        if answer.is_some() {
            *self.asked.entry((id, 'w')).or_default() += 1;
        }
        answer
    }
}

// --- the core model ---

#[test]
fn spans_empty_cells_auto_tracks_and_slack() {
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
        .child(GridSlot::new(0, 1).span(2, 1), leaf(150.0, 10.0).id("c"))
        .child(GridSlot::new(2, 1), leaf(30.0, 30.0).id("d"))
        .id("grid");
    let solution = no_canvas(grid);
    // The span needs 150 but gets 40 + 10 + 60, so each Auto column it spans grows by 20.
    assert_eq!(widths(&solution, "grid"), [60.0, 80.0, 30.0]);
    assert_eq!(heights(&solution, "grid"), [30.0, 30.0]);
    assert_eq!(solution.size(), Size::new(190.0, 70.0));
    assert_eq!(slot(&solution, "a"), Rect::new(0.0, 0.0, 60.0, 30.0));
    assert_eq!(content(&solution, "a"), Rect::new(0.0, 5.0, 40.0, 20.0));
    assert_eq!(content(&solution, "b"), Rect::new(70.0, 0.0, 80.0, 30.0));
    assert_eq!(content(&solution, "c"), Rect::new(0.0, 40.0, 150.0, 30.0));
    assert_eq!(content(&solution, "d"), Rect::new(160.0, 40.0, 30.0, 30.0));
}

fn stacked_axes(policy: Overflow) -> L {
    let plot = |id: Id, axis: Id, width: f32| {
        leaf(100.0, 50.0)
            .id(id)
            .slab(Side::Left, Slab::new().min_width(width).id(axis))
            .overflow_direction(H, policy)
    };
    auto_column([plot("a", "a-axis", 30.0), plot("b", "b-axis", 50.0)]).into()
}

#[test]
fn propagated_overflow_lines_up_plot_edges() {
    let solution = no_canvas(stacked_axes(Overflow::Propagate));
    assert_eq!(solution.size(), Size::new(150.0, 100.0));
    assert_eq!(content(&solution, "a"), Rect::new(50.0, 0.0, 100.0, 50.0));
    assert_eq!(content(&solution, "b"), Rect::new(50.0, 50.0, 100.0, 50.0));
    assert_eq!(slot(&solution, "a-axis"), Rect::new(20.0, 0.0, 30.0, 50.0));
    assert_eq!(slot(&solution, "b-axis"), Rect::new(0.0, 50.0, 50.0, 50.0));
}

#[test]
fn contained_overflow_lines_up_boxes() {
    let solution = no_canvas(stacked_axes(Overflow::Contain));
    assert_eq!(solution.size(), Size::new(150.0, 100.0));
    assert_eq!(slot(&solution, "a"), Rect::new(0.0, 0.0, 150.0, 50.0));
    assert_eq!(content(&solution, "a"), Rect::new(30.0, 0.0, 120.0, 50.0));
    assert_eq!(content(&solution, "b"), Rect::new(50.0, 50.0, 100.0, 50.0));
    assert_eq!(slot(&solution, "a-axis"), Rect::new(0.0, 0.0, 30.0, 50.0));
}

#[test]
fn flushed_overflow_reserves_no_space() {
    let solution = no_canvas(stacked_axes(Overflow::Flush));
    assert_eq!(solution.size(), Size::new(100.0, 100.0));
    assert_eq!(content(&solution, "a"), Rect::new(0.0, 0.0, 100.0, 50.0));
    assert_eq!(
        slot(&solution, "b-axis"),
        Rect::new(-50.0, 50.0, 50.0, 50.0)
    );
}

fn facet_wrap(dangling: Overflow) -> Grid<Id> {
    let axis = |id: Id| Slab::new().min_height(20.0).id(id);
    let panel = |id: Id| leaf(100.0, 60.0).id(id);
    Grid::new(auto(3), auto(2))
        .column_gap(Gap::new(10.0))
        .row_gap(Gap::new(10.0))
        .child(GridSlot::new(0, 0), panel("p0"))
        .child(GridSlot::new(1, 0), panel("p1"))
        .child(
            GridSlot::new(2, 0),
            panel("p2")
                .slab(Side::Bottom, axis("p2-axis"))
                .overflow(Side::Bottom, dangling),
        )
        .child(
            GridSlot::new(0, 1),
            panel("p3").slab(Side::Bottom, axis("p3-axis")),
        )
        .child(
            GridSlot::new(1, 1),
            panel("p4").slab(Side::Bottom, axis("p4-axis")),
        )
        .id("wrap")
}

#[test]
fn a_dangling_axis_flushes_into_the_empty_cell_below() {
    let solution = no_canvas(facet_wrap(Overflow::Flush));
    let wrap = tracks(&solution, "wrap");
    assert_eq!(
        wrap.rows[1].start - (wrap.rows[0].start + wrap.rows[0].length),
        10.0
    );
    assert_eq!(content(&solution, "p3"), Rect::new(0.0, 70.0, 100.0, 60.0));
    // The slab is still placed, reaching through the gap into the empty cell.
    assert_eq!(
        slot(&solution, "p2-axis"),
        Rect::new(220.0, 60.0, 100.0, 20.0)
    );
    assert!(matches!(
        region(&solution, "p2-axis").kind,
        RegionKind::Slab {
            side: Side::Bottom,
            ..
        }
    ));
    assert_eq!(solution.size(), Size::new(320.0, 150.0));

    let propagated = no_canvas(facet_wrap(Overflow::Propagate));
    assert_eq!(
        content(&propagated, "p3"),
        Rect::new(0.0, 80.0, 100.0, 60.0)
    );
}

#[test]
fn the_innermost_grid_sets_each_gap_for_three_facet_levels() {
    let panel = |id: Id| leaf(10.0, 10.0).id(id);
    let level3 = |a: Id, b: Id, with_axis: bool| {
        let mut second = panel(b);
        if with_axis {
            second = second.slab(Side::Left, Slab::new().min_width(8.0));
        }
        auto_row([panel(a), second]).gap(Gap::new(5.0).clearance(2.0))
    };
    let level2 = |first: Row<Id>, second: Row<Id>| auto_row([first, second]).gap(Gap::new(20.0));
    let outer = auto_row([
        level2(level3("c0", "c1", true), level3("c2", "c3", false)),
        level2(level3("c4", "c5", false), level3("c6", "c7", false)),
    ])
    .gap(Gap::new(40.0));
    let solution = no_canvas(outer);
    let starts: Vec<f32> = (0..8)
        .map(|i| {
            content(
                &solution,
                ["c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7"][i],
            )
            .x
        })
        .collect();
    // Gaps: the 8 px axis plus 2 px clearance, 20, 5, 40, 5, 20, 5.
    assert_eq!(starts, [0.0, 20.0, 50.0, 65.0, 115.0, 130.0, 160.0, 175.0]);
    assert_eq!(solution.size().width, 185.0);
}

#[test]
fn auto_px_and_fr_tracks_in_a_marginal_plot() {
    let grid = Grid::new(
        [Track::fr_max(1.0, 300.0), Track::auto()],
        [Track::px(80.0), Track::fr(1.0)],
    )
    .child(GridSlot::new(0, 0), leaf(0.0, 0.0).id("x-histogram"))
    .child(GridSlot::new(0, 1), leaf(0.0, 0.0).id("scatter"))
    .child(GridSlot::new(1, 1), leaf(60.0, 0.0).id("y-histogram"));
    let solution = sized(grid, 500.0, 400.0);
    assert_eq!(
        content(&solution, "x-histogram"),
        Rect::new(0.0, 0.0, 300.0, 80.0)
    );
    assert_eq!(
        content(&solution, "scatter"),
        Rect::new(0.0, 80.0, 300.0, 320.0)
    );
    assert_eq!(
        content(&solution, "y-histogram"),
        Rect::new(300.0, 80.0, 60.0, 320.0)
    );
}

fn panels_and_legend(matched: bool) -> Grid<Id> {
    let panel = Track::fr(1.0).tag("panel");
    let mut grid = Grid::new([panel.clone(), panel, Track::auto()], auto(1))
        .child(GridSlot::new(0, 0), leaf(50.0, 10.0))
        .child(GridSlot::new(1, 0), leaf(120.0, 10.0))
        .child(GridSlot::new(2, 0), leaf(40.0, 10.0))
        .id("grid");
    if matched {
        grid = grid.match_tag("panel");
    }
    grid
}

#[test]
fn matched_tracks_rise_together_while_a_legend_column_stays_auto() {
    assert_eq!(
        widths(&no_canvas(panels_and_legend(true)), "grid"),
        [120.0, 120.0, 40.0]
    );
    assert_eq!(
        widths(&no_canvas(panels_and_legend(false)), "grid"),
        [50.0, 120.0, 40.0]
    );
}

fn rows_with_tagged_gaps(matched: bool) -> Column<Id> {
    let row = |id: Id, axis: f32| {
        let second = leaf(50.0, 10.0).slab(Side::Left, Slab::new().min_width(axis));
        auto_row([leaf(50.0, 10.0), second])
            .gap(Gap::new(10.0).tag("panel-gap"))
            .id(id)
    };
    // Each row is a separate table, so only the match ties their gaps together.
    let mut outer = auto_column([row("a", 0.0), row("b", 25.0)]).share(false);
    if matched {
        outer = outer.match_tag("panel-gap");
    }
    outer
}

#[test]
fn matched_gaps_take_the_largest_gap_across_separate_grids() {
    let gap = |solution: &Solution<Id>, id| {
        let columns = &tracks(solution, id).columns;
        columns[1].start - (columns[0].start + columns[0].length)
    };
    let matched = no_canvas(rows_with_tagged_gaps(true));
    assert_eq!((gap(&matched, "a"), gap(&matched, "b")), (25.0, 25.0));
    let unmatched = no_canvas(rows_with_tagged_gaps(false));
    assert_eq!((gap(&unmatched, "a"), gap(&unmatched, "b")), (10.0, 25.0));
}

#[test]
fn facets_with_different_counts_align_as_a_prefix_of_shared_tracks() {
    let facet = |ids: &[Id]| {
        Row::new(ids.iter().map(|&id| leaf(20.0, 20.0).id(id)))
            .tracks(Track::fr(1.0).tag("column"))
            .gap(Gap::new(10.0))
    };
    let outer = auto_column([facet(&["a0", "a1", "a2"]), facet(&["b0", "b1", "b2", "b3"])])
        .match_tag("column");
    let solution = L::solve(&outer.into(), &width_only(430.0)).unwrap();
    for (a, b) in [("a0", "b0"), ("a1", "b1"), ("a2", "b2")] {
        assert_eq!(content(&solution, a).x, content(&solution, b).x);
        assert_eq!(content(&solution, a).width, 100.0);
    }
    assert_eq!(
        content(&solution, "b3"),
        Rect::new(330.0, 20.0, 100.0, 20.0)
    );
}

#[test]
fn row_gaps_of_a_row_facet_ignore_headers_and_axes_in_other_rows() {
    let header = Slab::new().min_height(20.0);
    let axis = Slab::new().min_height(25.0);
    let row = |index: usize, ids: [Id; 3]| {
        auto_row(ids.map(|id| {
            let mut panel = leaf(100.0, 50.0).id(id);
            if index == 0 {
                panel = panel.slab(Side::Top, header.clone());
            }
            if index == 2 {
                panel = panel.slab(Side::Bottom, axis.clone());
            }
            panel
        }))
    };
    let outer = auto_column([
        row(0, ["a0", "a1", "a2"]),
        row(1, ["b0", "b1", "b2"]),
        row(2, ["c0", "c1", "c2"]),
    ])
    .gap(Gap::new(10.0));
    let solution = no_canvas(outer);
    assert_eq!(content(&solution, "a0").y, 20.0);
    assert_eq!(content(&solution, "b0").y, 80.0);
    assert_eq!(content(&solution, "c0").y, 140.0);
    assert_eq!(solution.size().height, 215.0);
}

#[test]
fn three_facet_levels_with_legends_match_panel_and_legend_gaps_separately() {
    let panels = |a: Id, b: Id, axis: f32| {
        let second = leaf(50.0, 20.0)
            .id(b)
            .slab(Side::Left, Slab::new().min_width(axis));
        auto_row([leaf(50.0, 20.0).id(a), second]).gap(Gap::new(10.0).tag("panel"))
    };
    let facet = |panels: Row<Id>, legend: Id, title: f32| {
        let legend = leaf(40.0, 20.0)
            .id(legend)
            .slab(Side::Left, Slab::new().min_width(title));
        auto_row([Layout::from(panels), legend.into()]).gap(Gap::new(15.0).tag("legend"))
    };
    let outer = auto_row([
        facet(panels("a0", "a1", 0.0), "a-legend", 0.0),
        facet(panels("b0", "b1", 20.0), "b-legend", 25.0),
    ])
    .gap(Gap::new(30.0).tag("facet"))
    .match_tag("panel")
    .match_tag("legend")
    .match_tag("facet");
    let solution = no_canvas(outer);
    let x = |id| content(&solution, id).x;
    assert_eq!([x("a0"), x("a1"), x("a-legend")], [0.0, 70.0, 145.0]);
    assert_eq!([x("b0"), x("b1"), x("b-legend")], [215.0, 285.0, 360.0]);
    assert_eq!(solution.size().width, 400.0);
}

#[test]
fn placement_sizes_and_positions_a_node_in_its_slot() {
    let cell = |id: Id, placement: Placement| leaf(30.0, 10.0).id(id).place(H, placement);
    let grid = Grid::new([Track::px(100.0)], vec![Track::px(20.0); 4])
        .child(
            GridSlot::new(0, 0),
            cell("start", Placement::fit(Position::Start)),
        )
        .child(
            GridSlot::new(0, 1),
            cell("center", Placement::fit(Position::Center)),
        )
        .child(GridSlot::new(0, 2), cell("stretch", Placement::default()))
        .child(
            GridSlot::new(0, 3),
            cell("stretch-max", Placement::stretch(Position::End)).max_width(60.0),
        );
    let solution = no_canvas(grid);
    let x_range = |id| {
        let rect = content(&solution, id);
        (rect.x, rect.right())
    };
    assert_eq!(x_range("start"), (0.0, 30.0));
    assert_eq!(x_range("center"), (35.0, 65.0));
    assert_eq!(x_range("stretch"), (0.0, 100.0));
    assert_eq!(x_range("stretch-max"), (40.0, 100.0));
    // A maximum height stops a Stretch leaf short of its slot, at its position.
    let short = Grid::new([Track::px(100.0)], [Track::px(80.0)]).child(
        GridSlot::new(0, 0),
        leaf(10.0, 10.0)
            .id("short")
            .max_height(30.0)
            .place(V, Placement::stretch(Position::Center)),
    );
    assert_eq!(
        content(&no_canvas(short), "short"),
        Rect::new(0.0, 25.0, 100.0, 30.0)
    );
}

#[test]
fn a_slab_places_its_content_along_its_side() {
    let plot = leaf(200.0, 100.0).id("plot").slab(
        Side::Top,
        Slab::new()
            .min_size(Size::new(60.0, 20.0))
            .id("title")
            .place(Placement::fit(Position::Center)),
    );
    let solution = no_canvas(plot);
    assert_eq!(slot(&solution, "title"), Rect::new(0.0, 0.0, 200.0, 20.0));
    assert_eq!(
        content(&solution, "title"),
        Rect::new(70.0, 0.0, 60.0, 20.0)
    );
    // Regions can also be found by their path from the root.
    let by_path = solution.at_path(&[PathStep::Slab(Side::Top, 0)]).unwrap();
    assert_eq!(by_path.id, Some("title"));
}

#[test]
fn an_aspect_leaf_fits_inside_its_space() {
    let centered = |id: Id, ratio: f32| {
        leaf(0.0, 0.0)
            .id(id)
            .aspect(ratio)
            .place(H, Placement::stretch(Position::Center))
            .place(V, Placement::stretch(Position::Center))
    };
    let grid = Grid::new([Track::px(200.0)], vec![Track::px(200.0); 2])
        .child(GridSlot::new(0, 0), centered("wide", 2.0))
        .child(GridSlot::new(0, 1), centered("tall", 0.5));
    let solution = no_canvas(grid);
    assert_eq!(
        content(&solution, "wide"),
        Rect::new(0.0, 50.0, 200.0, 100.0)
    );
    assert_eq!(
        content(&solution, "tall"),
        Rect::new(50.0, 200.0, 100.0, 200.0)
    );
}

// --- tables ---

fn cell(solution: &Solution<Id>, id: Id) -> Option<Cell> {
    region(solution, id).cell.clone()
}

#[test]
fn three_nested_levels_align_without_counting_tracks() {
    let panel = |id: Id, width: f32, height: f32| leaf(width, height).id(id);
    let region = |ids: [Id; 4], first_width: f32, low_height: f32| {
        Column::new([
            Row::new([panel(ids[0], first_width, 10.0), panel(ids[1], 20.0, 10.0)])
                .gap(Gap::new(4.0)),
            Row::new([panel(ids[2], 20.0, low_height), panel(ids[3], 20.0, 10.0)])
                .gap(Gap::new(4.0)),
        ])
        .gap(Gap::new(6.0))
    };
    let figure = Row::new([
        region(["a0", "a1", "a2", "a3"], 20.0, 30.0),
        region(["b0", "b1", "b2", "b3"], 40.0, 10.0),
    ])
    .gap(Gap::new(10.0));
    let solution = no_canvas(figure);
    let x = |id| content(&solution, id).x;
    assert_eq!(
        [x("a0"), x("a1"), x("b0"), x("b1")],
        [0.0, 24.0, 54.0, 98.0]
    );
    // The tall panel in the first region makes the second row of both regions 30 tall, and
    // the wide panel in the second region widens the panel below it.
    assert_eq!(content(&solution, "b2"), Rect::new(54.0, 16.0, 40.0, 30.0));
    assert_eq!(solution.size(), Size::new(118.0, 46.0));
}

#[test]
fn a_row_in_a_row_adds_its_children_to_the_columns() {
    let row = Row::new([
        Layout::from(
            Row::new([leaf(10.0, 10.0).id("a"), leaf(20.0, 10.0).id("b")]).gap(Gap::new(2.0)),
        ),
        leaf(30.0, 10.0).id("c").into(),
    ])
    .tracks(Track::auto())
    .gap(Gap::new(8.0))
    .id("outer");
    let solution = no_canvas(row);
    assert_eq!(widths(&solution, "outer"), [10.0, 20.0, 30.0]);
    let x = |id| content(&solution, id).x;
    assert_eq!([x("a"), x("b"), x("c")], [0.0, 12.0, 40.0]);
}

#[test]
fn a_leaf_in_a_column_spans_every_column_and_a_short_row_leaves_cells_empty() {
    let panel = |id: Id| leaf(30.0, 10.0).id(id);
    let column = Column::new([
        Layout::from(Row::new([panel("a"), panel("b"), panel("c")])),
        Row::new([panel("d")]).id("short").into(),
        leaf(90.0, 10.0).id("wide").into(),
    ]);
    let solution = no_canvas(column);
    assert_eq!(content(&solution, "wide"), Rect::new(0.0, 20.0, 90.0, 10.0));
    // The short row covers only its own column.
    assert_eq!(slot(&solution, "short"), Rect::new(0.0, 10.0, 30.0, 10.0));
    assert_eq!(
        cell(&solution, "wide").map(|cell| cell.slot),
        Some(GridSlot::new(0, 2).span(3, 1))
    );
}

#[test]
fn a_span_runs_a_child_under_two_of_three_panels() {
    let panel = |id: Id| leaf(40.0, 10.0).id(id);
    let column = Column::new([
        Row::new([panel("a"), panel("b"), panel("c")]).gap(Gap::new(10.0)),
        Row::new([
            leaf(0.0, 10.0).id("legend").span(2),
            leaf(0.0, 10.0).id("note"),
        ])
        .gap(Gap::new(10.0)),
    ]);
    let solution = no_canvas(column);
    assert_eq!(
        content(&solution, "legend"),
        Rect::new(0.0, 10.0, 90.0, 10.0)
    );
    assert_eq!(
        content(&solution, "note"),
        Rect::new(100.0, 10.0, 40.0, 10.0)
    );
}

#[test]
fn a_span_widens_a_joined_sequences_tracks() {
    let row = Row::new([
        Layout::from(
            Column::new([leaf(0.0, 10.0).id("a"), leaf(0.0, 10.0).id("b")])
                .span(2)
                .id("column"),
        ),
        leaf(0.0, 10.0).id("c").into(),
    ]);
    let solution = sized(row, 300.0, 100.0);
    assert_eq!(content(&solution, "a").width, 200.0);
    assert_eq!(content(&solution, "b").width, 200.0);
    assert_eq!(tracks(&solution, "column").columns.len(), 2);
    assert_eq!(content(&solution, "c").x, 200.0);
}

#[test]
fn share_false_separates_children_whose_own_sequences_still_share() {
    let facet = |ids: [Id; 4], first: Size| {
        Column::new([
            Row::new([
                leaf(first.width, first.height).id(ids[0]),
                leaf(20.0, 10.0).id(ids[1]),
            ]),
            Row::new([leaf(20.0, 10.0).id(ids[2]), leaf(20.0, 10.0).id(ids[3])]),
        ])
    };
    let page = Row::new([
        facet(["a0", "a1", "a2", "a3"], Size::new(50.0, 10.0)),
        facet(["b0", "b1", "b2", "b3"], Size::new(20.0, 30.0)),
    ])
    .share(false)
    .gap(Gap::new(10.0));
    let solution = no_canvas(page);
    // Each facet aligns its own panels.
    assert_eq!(content(&solution, "a2").width, 50.0);
    // The facets do not align with each other.
    assert_eq!(content(&solution, "b0"), Rect::new(80.0, 0.0, 20.0, 30.0));
    // Each facet stretches its own rows to the page's height: the first facet's rows share it
    // equally, and the second keeps its tall first row.
    assert_eq!(content(&solution, "a2").y, 20.0);
    assert_eq!(content(&solution, "b2").y, 30.0);
}

#[test]
fn a_sequence_in_a_grid_cell_starts_its_own_table() {
    let grid = Grid::new(auto(1), auto(2))
        .child(
            GridSlot::new(0, 0),
            Row::new([leaf(50.0, 10.0).id("a"), leaf(10.0, 10.0).id("b")]),
        )
        .child(
            GridSlot::new(0, 1),
            Row::new([leaf(10.0, 10.0).id("c"), leaf(10.0, 10.0).id("d")]),
        );
    let solution = no_canvas(grid);
    // The second row shares its cell's width equally instead of lining up with the first.
    assert_eq!(content(&solution, "b").x, 50.0);
    assert_eq!(content(&solution, "d").x, 30.0);
    assert_eq!(
        cell(&solution, "d"),
        Some(Cell {
            table: vec![PathStep::Child(1)],
            slot: GridSlot::new(1, 0),
        })
    );
}

#[test]
fn an_undeclared_track_takes_a_siblings_size_or_else_one_fr() {
    let pair = |a: Id, b: Id| Row::new([leaf(0.0, 10.0).id(a), leaf(0.0, 10.0).id(b)]);
    let column = Column::new([pair("a", "b").track(0, Track::fr(3.0)), pair("c", "d")]).id("table");
    let solution = L::solve(&column.into(), &width_only(400.0)).unwrap();
    assert_eq!(widths(&solution, "table"), [300.0, 100.0]);
    assert_eq!(content(&solution, "c").width, 300.0);
}

#[test]
fn cells_report_slots_in_the_whole_table() {
    let panel = |id: Id| leaf(10.0, 10.0).id(id);
    let region = |ids: [Id; 4]| {
        Column::new([
            Row::new([panel(ids[0]), panel(ids[1])]),
            Row::new([panel(ids[2]), panel(ids[3])]),
        ])
    };
    let figure = Row::new([
        region(["a", "b", "c", "d"]),
        region(["e", "f", "g", "h"]).id("east"),
    ]);
    let grid = Grid::new(auto(2), auto(2))
        .child(GridSlot::new(0, 0), figure)
        .child(
            GridSlot::new(1, 0).span(1, 2),
            panel("tall")
                .slab(Side::Top, Slab::new().min_height(5.0).id("header"))
                .overlay(Overlay::new(leaf(0.0, 0.0).id("inset"))),
        )
        .id("page");
    let solution = no_canvas(grid);
    let figure_path = vec![PathStep::Child(0)];
    assert_eq!(
        cell(&solution, "h"),
        Some(Cell {
            table: figure_path.clone(),
            slot: GridSlot::new(3, 1),
        })
    );
    assert_eq!(
        cell(&solution, "east"),
        Some(Cell {
            table: figure_path,
            slot: GridSlot::new(2, 0).span(2, 2),
        })
    );
    assert_eq!(
        cell(&solution, "tall"),
        Some(Cell {
            table: Vec::new(),
            slot: GridSlot::new(1, 0).span(1, 2),
        })
    );
    for id in ["page", "header", "inset"] {
        assert_eq!(cell(&solution, id), None, "{id}");
    }
}

// --- collapse and merge ---

/// A slab with `tag` and content key `key`, `height` thick.
fn shared(id: Id, tag: &str, key: &str, height: f32) -> Slab<Id> {
    Slab::new()
        .min_height(height)
        .id(id)
        .tag(tag)
        .content_key(key)
}

fn shared_width(id: Id, tag: &str, key: &str) -> Slab<Id> {
    Slab::new().min_width(12.0).id(id).tag(tag).content_key(key)
}

fn represented_by(solution: &Solution<Id>, id: Id) -> Option<Vec<PathStep>> {
    match &region(solution, id).kind {
        RegionKind::Slab { represented_by, .. } => represented_by.clone(),
        kind => panic!("{id} is {kind:?}"),
    }
}

fn path_of(solution: &Solution<Id>, id: Id) -> Vec<PathStep> {
    region(solution, id).path.clone()
}

fn labeled(id: Id, label: Id, key: &str) -> Leaf<Id> {
    leaf(30.0, 20.0)
        .id(id)
        .slab(Side::Bottom, shared(label, "x-labels", key, 10.0))
}

#[test]
fn a_column_collapses_to_its_bottom_slab() {
    let column = |collapse: bool| {
        let column = Column::new([
            labeled("p0", "l0", "x"),
            labeled("p1", "l1", "x"),
            labeled("p2", "l2", "x"),
        ]);
        if collapse {
            column.collapse_tag("x-labels")
        } else {
            column
        }
    };
    let solution = no_canvas(column(true));
    assert_eq!(slot(&solution, "l2"), Rect::new(0.0, 60.0, 30.0, 10.0));
    for id in ["l0", "l1"] {
        assert_eq!(slot(&solution, id).height, 0.0, "{id}");
        assert_eq!(
            represented_by(&solution, id),
            Some(path_of(&solution, "l2"))
        );
        assert_eq!(solution.representative(&id), solution.region(&"l2"));
    }
    assert_eq!(represented_by(&solution, "l2"), None);
    assert_eq!(solution.size().height, 70.0);
    assert_eq!(no_canvas(column(false)).size().height, 90.0);
}

#[test]
fn a_slab_that_shows_another_key_or_none_ends_a_run() {
    let solution = no_canvas(
        Column::new([
            labeled("p0", "l0", "a"),
            labeled("p1", "l1", "b"),
            labeled("p2", "l2", "b"),
        ])
        .collapse_tag("x-labels"),
    );
    assert_eq!(represented_by(&solution, "l0"), None);
    assert_eq!(
        represented_by(&solution, "l1"),
        Some(path_of(&solution, "l2"))
    );
    assert_eq!(slot(&solution, "l0").height, 10.0);

    let unkeyed = leaf(30.0, 20.0).id("p1").slab(
        Side::Bottom,
        Slab::new().min_height(10.0).id("l1").tag("x-labels"),
    );
    let solution = no_canvas(
        Column::new([labeled("p0", "l0", "b"), unkeyed, labeled("p2", "l2", "b")])
            .collapse_tag("x-labels"),
    );
    for id in ["l0", "l1", "l2"] {
        assert_eq!(represented_by(&solution, id), None, "{id}");
    }
}

#[test]
fn runs_pass_over_spacers_and_empty_cells_and_stop_at_spanning_plots() {
    let solution = no_canvas(
        Column::new([
            Row::new([labeled("a", "la", "x"), labeled("b", "lb", "x")]),
            Row::new([leaf(30.0, 20.0).id("spacer")]),
            Row::new([labeled("c", "lc", "x"), labeled("d", "ld", "x")]),
        ])
        .collapse_tag("x-labels"),
    );
    // The spacer has no slab, and the second column has an empty cell in the middle row.
    assert_eq!(
        represented_by(&solution, "la"),
        Some(path_of(&solution, "lc"))
    );
    assert_eq!(
        represented_by(&solution, "lb"),
        Some(path_of(&solution, "ld"))
    );

    let solution = no_canvas(
        Column::new([
            Layout::from(Row::new([labeled("a", "la", "x"), labeled("b", "lb", "x")])),
            leaf(60.0, 20.0).id("wide").into(),
            Row::new([labeled("c", "lc", "x"), labeled("d", "ld", "x")]).into(),
        ])
        .collapse_tag("x-labels"),
    );
    assert_eq!(represented_by(&solution, "la"), None);
    assert_eq!(represented_by(&solution, "lb"), None);
}

#[test]
fn a_run_stops_at_a_panel_with_another_guide() {
    // The middle panel draws a guide of another tag on the left, such as an axis of another
    // scale, so the third panel's title does not pass it.
    let panel = |id: Id, title: Id, tag: &str, key: &str| {
        leaf(40.0, 30.0)
            .id(id)
            .slab(Side::Left, shared_width(title, tag, key))
    };
    let solution = no_canvas(
        Row::new([
            panel("a", "ta", "title", "y"),
            panel("b", "tb", "other", "z"),
            panel("c", "tc", "title", "y"),
        ])
        .collapse_tag("title"),
    );
    for id in ["ta", "tc"] {
        assert_eq!(represented_by(&solution, id), None, "{id}");
        assert_eq!(slot(&solution, id).width, 12.0, "{id}");
    }
}

#[test]
fn rows_of_separate_tables_do_not_collapse_across_each_other() {
    let figure = Column::new([
        Row::new([labeled("a", "la", "x"), labeled("b", "lb", "x")]),
        Row::new([labeled("c", "lc", "x"), labeled("d", "ld", "x")]),
    ])
    .share(false)
    .collapse_tag("x-labels");
    let separate = no_canvas(figure);
    for id in ["la", "lb", "lc", "ld"] {
        assert_eq!(represented_by(&separate, id), None, "{id}");
    }
}

/// A row of two panels with a header above it that labels both.
fn headed_row(panels: [Id; 2], header: Id, key: &str) -> Row<Id> {
    auto_row(panels.map(|id| leaf(30.0, 20.0).id(id)))
        .gap(Gap::new(10.0))
        .slab(Side::Top, shared(header, "header", key, 12.0))
}

#[test]
fn headers_on_the_rows_of_a_grid_collapse_to_the_top_row() {
    let solution = no_canvas(
        auto_column([
            headed_row(["a", "b"], "h0", "x"),
            headed_row(["c", "d"], "h1", "x"),
            headed_row(["e", "f"], "h2", "x"),
        ])
        .gap(Gap::new(10.0))
        .collapse_tag("header"),
    );
    // The rows' own panels lie between their headers, and they do not end the run.
    assert_eq!(slot(&solution, "h0"), Rect::new(0.0, 0.0, 70.0, 12.0));
    for id in ["h1", "h2"] {
        assert_eq!(
            represented_by(&solution, id),
            Some(path_of(&solution, "h0"))
        );
        assert_eq!(slot(&solution, id).height, 0.0, "{id}");
    }
    assert_eq!(content(&solution, "c").y, 42.0);
    assert_eq!(solution.size().height, 92.0);
}

#[test]
fn headers_beside_the_columns_of_a_grid_collapse_past_their_panels() {
    let column = |panels: [Id; 2], header: Id| {
        auto_column(panels.map(|id| leaf(30.0, 20.0).id(id)))
            .gap(Gap::new(10.0))
            .slab(
                Side::Right,
                Slab::new()
                    .min_width(12.0)
                    .id(header)
                    .tag("header")
                    .content_key("y"),
            )
    };
    let solution = no_canvas(
        auto_row([column(["a", "b"], "h0"), column(["c", "d"], "h1")])
            .gap(Gap::new(10.0))
            .collapse_tag("header"),
    );
    assert_eq!(
        represented_by(&solution, "h0"),
        Some(path_of(&solution, "h1"))
    );
    assert_eq!(slot(&solution, "h0").width, 0.0);
    assert_eq!(slot(&solution, "h1"), Rect::new(70.0, 0.0, 12.0, 50.0));
    assert_eq!(content(&solution, "c").x, 40.0);
}

#[test]
fn a_header_run_stops_at_a_plot_that_crosses_its_lane() {
    let solution = no_canvas(
        auto_column([
            Layout::from(headed_row(["a", "b"], "h0", "x")),
            auto_row([leaf(30.0, 20.0).id("narrow")]).into(),
            headed_row(["c", "d"], "h1", "x").into(),
        ])
        .collapse_tag("header"),
    );
    for id in ["h0", "h1"] {
        assert_eq!(represented_by(&solution, id), None, "{id}");
        assert_eq!(slot(&solution, id).height, 12.0, "{id}");
    }
}

const PANELS: [Id; 5] = ["p0", "p1", "p2", "p3", "p4"];
const TITLES: [Id; 5] = ["t0", "t1", "t2", "t3", "t4"];

/// Five panels in rows of three and two, each with a centered x title under it.
fn ragged_wrap() -> Column<Id> {
    let panel = |i: usize| {
        let mut panel = leaf(40.0, 20.0).id(PANELS[i]).slab(
            Side::Bottom,
            Slab::new()
                .min_size(Size::new(30.0, 12.0))
                .id(TITLES[i])
                .tag("x-title")
                .content_key("Month")
                .place(Placement::fit(Position::Center)),
        );
        if i.is_multiple_of(3) {
            panel = panel.slab(Side::Left, Slab::new().min_width(20.0));
        }
        panel
    };
    Column::new([
        Row::new([panel(0), panel(1), panel(2)]),
        Row::new([panel(3), panel(4)]),
    ])
    .merge_tag("x-title")
    .id("wrap")
}

#[test]
fn a_ragged_wrap_merges_one_title_centered_on_the_plots() {
    let solution = no_canvas(ragged_wrap());
    // The runs keep the titles of p3, p4 and p2, and the merge stands for every title. The
    // first of them in tree order, p0's, gives its id to the merged slab.
    let merged = vec![PathStep::Merged(Side::Bottom, 0)];
    assert_eq!(path_of(&solution, "t0"), merged);
    assert_eq!(
        solution.at_path(&[
            PathStep::Child(0),
            PathStep::Child(0),
            PathStep::Slab(Side::Bottom, 0)
        ]),
        None
    );
    // The plots run from 20 to 140, after the y axes, and the title centers on them.
    assert_eq!(slot(&solution, "t0"), Rect::new(20.0, 40.0, 120.0, 12.0));
    assert_eq!(content(&solution, "t0"), Rect::new(65.0, 40.0, 30.0, 12.0));
    for id in ["t1", "t2", "t3", "t4"] {
        assert_eq!(represented_by(&solution, id), Some(merged.clone()), "{id}");
        assert_eq!(slot(&solution, id).height, 0.0, "{id}");
    }
    assert_eq!(solution.size(), Size::new(140.0, 52.0));
    let merges: Vec<_> = solution
        .explanation()
        .representations
        .iter()
        .filter(|representation| representation.kind == RepresentationKind::Merge)
        .collect();
    assert_eq!(merges.len(), 1);
    assert_eq!(merges[0].tag, Tag::from("x-title"));
    assert_eq!(merges[0].side, Side::Bottom);
    assert_eq!(merges[0].representative, merged);
    // Its members are the titles that the runs keep.
    let title = |row, column| {
        vec![
            PathStep::Child(row),
            PathStep::Child(column),
            PathStep::Slab(Side::Bottom, 0),
        ]
    };
    assert_eq!(merges[0].members, [title(0, 2), title(1, 0), title(1, 1)]);
}

#[test]
fn alternating_titles_do_not_merge() {
    let panel = |id: Id, title: Id, key: &str| {
        leaf(40.0, 20.0)
            .id(id)
            .slab(Side::Top, shared(title, "title", key, 12.0))
    };
    let row = Row::new([
        panel("a", "ta", "first"),
        panel("b", "tb", "second"),
        panel("c", "tc", "first"),
    ])
    .merge_tag("title");
    let solution = no_canvas(row);
    for id in ["ta", "tb", "tc"] {
        assert_eq!(represented_by(&solution, id), None, "{id}");
        assert_eq!(slot(&solution, id).height, 12.0, "{id}");
    }
    assert!(solution.explanation().representations.is_empty());
}

#[test]
fn titles_away_from_the_side_do_not_merge() {
    // Each column of panels has its own left title, as when each column has its own y scale.
    let panel = |id: Id, title: Id, column: usize| {
        leaf(40.0, 30.0).id(id).slab(
            Side::Left,
            Slab::new()
                .min_width(12.0)
                .id(title)
                .tag(["first", "second"][column])
                .content_key(["first", "second"][column]),
        )
    };
    let grid = Column::new([
        Row::new([panel("a0", "t0", 0), panel("a1", "t1", 1)]),
        Row::new([panel("b0", "t2", 0), panel("b1", "t3", 1)]),
    ])
    .merge_tag("first")
    .merge_tag("second");
    let solution = no_canvas(grid);
    // The first column's titles merge onto the grid's left side.
    assert_eq!(path_of(&solution, "t0"), [PathStep::Merged(Side::Left, 0)]);
    assert_eq!(content(&solution, "t0"), Rect::new(0.0, 0.0, 12.0, 60.0));
    // The second column's titles stay beside their panels instead of standing beyond the first
    // column's on the grid's left side.
    assert_eq!(represented_by(&solution, "t3"), None);
    assert_eq!(content(&solution, "t1"), Rect::new(52.0, 0.0, 12.0, 30.0));
    assert_eq!(content(&solution, "t3"), Rect::new(52.0, 30.0, 12.0, 30.0));
}

#[test]
fn a_legend_merges_past_panels_that_show_it() {
    // A tall panel beside a column of two, each with the same legend on its right, as when the
    // panels share one color scale.
    let panel = |id: Id, legend: Id| {
        leaf(40.0, 30.0).id(id).slab(
            Side::Right,
            Slab::new()
                .min_width(20.0)
                .id(legend)
                .tag("legend")
                .content_key("color"),
        )
    };
    let children: [L; 2] = [
        panel("a", "ga").into(),
        Column::new([panel("b", "gb"), panel("c", "gc")]).into(),
    ];
    let solution = no_canvas(Row::new(children).merge_tag("legend"));
    // The tall panel's legend passes the column's panels, which show it too.
    let merged = vec![PathStep::Merged(Side::Right, 0)];
    assert_eq!(path_of(&solution, "ga"), merged);
    for id in ["gb", "gc"] {
        assert_eq!(represented_by(&solution, id), Some(merged.clone()), "{id}");
    }
}

#[test]
fn a_title_merges_past_an_empty_cell() {
    let panel = |id: Id, title: Id| {
        leaf(40.0, 30.0)
            .id(id)
            .slab(Side::Left, shared_width(title, "title", "y"))
    };
    let rows: [L; 2] = [
        Row::new([panel("a", "ta"), panel("b", "tb")]).into(),
        Row::new([leaf(40.0, 30.0).id("empty"), panel("d", "td")]).into(),
    ];
    let solution = no_canvas(Column::new(rows).merge_tag("title"));
    // The empty cell draws nothing on its left, so the second row's title passes it.
    let merged = vec![PathStep::Merged(Side::Left, 0)];
    assert_eq!(path_of(&solution, "ta"), merged);
    for id in ["tb", "td"] {
        assert_eq!(represented_by(&solution, id), Some(merged.clone()), "{id}");
    }
}

/// Two regions of two panels, each panel with a legend on its right, at the top of its slab.
fn legends(keys: [&str; 2]) -> Column<Id> {
    let panel = |id: Id, legend: Id, key: &str| {
        leaf(40.0, 30.0).id(id).slab(
            Side::Right,
            Slab::new()
                .min_size(Size::new(20.0, 10.0))
                .id(legend)
                .tag("legend")
                .content_key(key)
                .place(Placement::fit(Position::Start)),
        )
    };
    Column::new([
        Row::new([panel("a0", "g0", keys[0]), panel("a1", "g1", keys[0])])
            .merge_tag("legend")
            .id("north"),
        Row::new([panel("b0", "g2", keys[1]), panel("b1", "g3", keys[1])])
            .merge_tag("legend")
            .id("south"),
    ])
    .merge_tag("legend")
}

#[test]
fn a_region_keyed_legend_merges_onto_its_region() {
    let solution = no_canvas(legends(["north", "south"]));
    assert_eq!(
        path_of(&solution, "g0"),
        [PathStep::Child(0), PathStep::Merged(Side::Right, 0)]
    );
    assert_eq!(
        path_of(&solution, "g2"),
        [PathStep::Child(1), PathStep::Merged(Side::Right, 0)]
    );
    // Each legend sits at the top of its region's plots.
    assert_eq!(content(&solution, "g0"), Rect::new(80.0, 0.0, 20.0, 10.0));
    assert_eq!(content(&solution, "g2"), Rect::new(80.0, 30.0, 20.0, 10.0));
    assert_eq!(solution.size(), Size::new(100.0, 60.0));
}

#[test]
fn a_figure_keyed_legend_merges_onto_the_figure() {
    let solution = no_canvas(legends(["figure", "figure"]));
    let merged = vec![PathStep::Merged(Side::Right, 0)];
    assert_eq!(path_of(&solution, "g0"), merged);
    assert_eq!(content(&solution, "g0"), Rect::new(80.0, 0.0, 20.0, 10.0));
    assert_eq!(slot(&solution, "g0"), Rect::new(80.0, 0.0, 20.0, 60.0));
    for id in ["g1", "g2", "g3"] {
        assert_eq!(represented_by(&solution, id), Some(merged.clone()), "{id}");
    }
    // The regions' own merges are replaced by the figure's.
    assert!(
        solution
            .regions()
            .filter(|region| matches!(region.path.last(), Some(PathStep::Merged(..))))
            .count()
            == 1
    );
}

/// Records the width that each id's height was asked at. Titles are 50 wide unwrapped.
#[derive(Default)]
struct Widths {
    asked: Vec<(Id, f32)>,
}

impl Measure<Id> for Widths {
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        let answer = match id.chars().next() {
            Some('l') => Measurement::always(12.0),
            Some('t') => Measurement::from(14.0),
            _ => return None,
        };
        self.asked.push((id, width));
        Some(answer)
    }

    fn max_content(&mut self, id: &Id, direction: Direction) -> Option<f32> {
        (id.starts_with('t') && direction == H).then_some(50.0)
    }
}

/// Two rows of two panels, with measured x labels that collapse per column and measured x
/// titles that merge.
fn measured_facet() -> L {
    let panel = |i: usize| {
        let ids = [
            ("p0", "l0", "t0"),
            ("p1", "l1", "t1"),
            ("p2", "l2", "t2"),
            ("p3", "l3", "t3"),
        ];
        let (plot, labels, title) = ids[i];
        leaf(60.0, 40.0)
            .id(plot)
            .slab(
                Side::Bottom,
                Slab::new()
                    .id(labels)
                    .tag("x-labels")
                    .content_key(["x0", "x1"][i % 2]),
            )
            .slab(
                Side::Bottom,
                Slab::new()
                    .id(title)
                    .tag("x-title")
                    .content_key("Month")
                    .place(Placement::fit(Position::Center)),
            )
    };
    Column::new([
        Row::new([panel(0), panel(1)]).gap(Gap::new(10.0)),
        Row::new([panel(2), panel(3)]).gap(Gap::new(10.0)),
    ])
    .collapse_tag("x-labels")
    .merge_tag("x-title")
    .into()
}

#[test]
fn collapsed_slabs_are_never_measured_and_a_merge_is_measured_once_at_its_content_length() {
    let mut measure = Widths::default();
    let measured = measured_facet()
        .measure(&SolveOptions::default(), &mut measure)
        .unwrap();
    let mut asked: Vec<Id> = measure.asked.iter().map(|(id, _)| *id).collect();
    asked.sort_unstable();
    assert_eq!(asked, ["l2", "l3", "t0"]);
    // The merged title runs along both columns and the gap between them, and its content takes
    // its unwrapped width there, which is the width it is measured at.
    let title_width = measure.asked.iter().find(|(id, _)| *id == "t0").unwrap().1;
    assert_eq!(title_width, 50.0);
    let solution = measured.solution();
    assert_eq!(slot(solution, "t0"), Rect::new(0.0, 92.0, 130.0, 14.0));
    assert_eq!(content(solution, "t0"), Rect::new(40.0, 92.0, 50.0, 14.0));
    assert_eq!(slot(solution, "l0").height, 0.0);
    // A resize keeps the runs and the merge.
    let resized = measured.resize(Size::new(300.0, 200.0)).unwrap();
    assert_eq!(path_of(&resized, "t0"), [PathStep::Merged(Side::Bottom, 0)]);
    assert_eq!(
        represented_by(&resized, "l0"),
        Some(path_of(&resized, "l2"))
    );
}

#[test]
fn a_slab_is_measured_at_its_content_length() {
    let mut measure = Widths::default();
    let plot = leaf(400.0, 100.0).slab(Side::Top, Slab::new().id("l").max_length(120.0));
    let solution = L::from(plot)
        .measure(&SolveOptions::default(), &mut measure)
        .unwrap()
        .into_solution();
    assert_eq!(measure.asked, [("l", 120.0)]);
    assert_eq!(content(&solution, "l"), Rect::new(0.0, 0.0, 120.0, 12.0));
}

#[test]
fn matching_skips_collapsed_slabs_and_holds_kept_slabs_across_tables() {
    let table = |ids: [Id; 4], key: &str, height: f32| {
        Column::new([
            leaf(30.0, 20.0)
                .id(ids[0])
                .slab(Side::Bottom, shared(ids[1], "x-labels", key, height)),
            leaf(30.0, 20.0)
                .id(ids[2])
                .slab(Side::Bottom, shared(ids[3], "x-labels", key, height)),
        ])
        .collapse_tag("x-labels")
    };
    let page = Row::new([
        table(["a0", "la0", "a1", "la1"], "a", 10.0),
        table(["b0", "lb0", "b1", "lb1"], "b", 30.0),
    ])
    .share(false)
    .match_tag("x-labels");
    let solution = no_canvas(page);
    assert_eq!(slot(&solution, "la1").height, 30.0);
    assert_eq!(slot(&solution, "lb1").height, 30.0);
    assert_eq!(slot(&solution, "la0").height, 0.0);
    assert_eq!(slot(&solution, "lb0").height, 0.0);
}

/// Six panels whose x labels share one content key and no tag.
fn six_panels(key: &'static str) -> L {
    const PLOTS: [Id; 6] = ["q0", "q1", "q2", "q3", "q4", "q5"];
    const LABELS: [Id; 6] = ["m0", "m1", "m2", "m3", "m4", "m5"];
    Row::new((0..6).map(|i| {
        leaf(50.0, 30.0)
            .id(PLOTS[i])
            .slab(Side::Bottom, Slab::new().id(LABELS[i]).content_key(key))
    }))
    .into()
}

#[test]
fn slabs_with_one_content_key_share_their_answers() {
    let mut measure = Measurer::heights(|id, _| {
        id.starts_with('m')
            .then(|| Measurement::over(12.0, 0.0..=200.0))
    });
    let measured = six_panels("x")
        .measure(&width_only(300.0), &mut measure)
        .unwrap();
    assert_eq!(measure.answered(), 1);
    // The first slab in tree order asks.
    assert_eq!(measure.asked("m0", 'h'), 1);
    assert!(
        measured
            .solution()
            .regions()
            .filter(|region| matches!(region.kind, RegionKind::Slab { .. }))
            .all(|region| region.slot.height == 12.0)
    );
    // The same key reuses the answer, and a new key asks again.
    let reused = six_panels("x")
        .measure_reusing(&measured, &width_only(320.0), &mut measure)
        .unwrap();
    assert_eq!(measure.answered(), 1);
    six_panels("y")
        .measure_reusing(&reused, &width_only(320.0), &mut measure)
        .unwrap();
    assert_eq!(measure.answered(), 2);
}

// --- padding and container sizes ---

#[test]
fn a_padded_card_keeps_its_title_and_content_inside_its_padding() {
    let card = Column::new([leaf(100.0, 50.0).id("chart")])
        .padding(Edges::all(10.0))
        .overflow_direction(H, Overflow::Contain)
        .overflow_direction(V, Overflow::Contain)
        .slab(Side::Top, Slab::new().min_height(20.0).id("title"))
        .id("card");
    let page = Grid::new(auto(2), auto(1))
        .column_gap(Gap::new(8.0))
        .child(GridSlot::new(0, 0), card)
        .child(GridSlot::new(1, 0), leaf(40.0, 40.0).id("next"));
    let solution = no_canvas(page);
    assert_eq!(slot(&solution, "card"), Rect::new(0.0, 0.0, 120.0, 90.0));
    assert_eq!(slot(&solution, "title"), Rect::new(10.0, 10.0, 100.0, 20.0));
    assert_eq!(
        content(&solution, "chart"),
        Rect::new(10.0, 30.0, 100.0, 50.0)
    );
    assert_eq!(content(&solution, "next").x, 128.0);

    // On a side that propagates, the slab hangs outside the padding.
    let plot = leaf(100.0, 50.0)
        .id("plot")
        .padding(Edges::new(5.0, 0.0, 0.0, 0.0))
        .slab(Side::Left, Slab::new().min_width(20.0).id("axis"));
    let row = Row::new([leaf(30.0, 50.0).id("before"), plot]).tracks(Track::auto());
    let solution = no_canvas(row);
    assert_eq!(slot(&solution, "plot"), Rect::new(50.0, 0.0, 105.0, 50.0));
    assert_eq!(content(&solution, "plot").x, 55.0);
    assert_eq!(slot(&solution, "axis"), Rect::new(30.0, 0.0, 20.0, 50.0));
}

#[test]
fn a_container_minimum_grows_its_tracks() {
    let row = Row::new([leaf(20.0, 10.0).id("a"), leaf(20.0, 10.0).id("b")])
        .min_width(200.0)
        .id("row");
    let solution = no_canvas(row);
    assert_eq!(widths(&solution, "row"), [100.0, 100.0]);
    assert_eq!(solution.size().width, 200.0);
}

#[test]
fn a_container_maximum_is_placed_in_a_larger_slot() {
    let row = Row::new([leaf(0.0, 10.0).id("a"), leaf(0.0, 10.0).id("b")])
        .max_width(100.0)
        .place(H, Placement::stretch(Position::Center))
        .id("row");
    let grid = Grid::new([Track::fr(1.0)], auto(1)).child(GridSlot::new(0, 0), row);
    let solution = L::solve(&grid.into(), &width_only(300.0)).unwrap();
    assert_eq!(slot(&solution, "row").width, 300.0);
    assert_eq!(
        content(&solution, "row"),
        Rect::new(100.0, 0.0, 100.0, 10.0)
    );
    assert_eq!(content(&solution, "b"), Rect::new(150.0, 0.0, 50.0, 10.0));
}

#[test]
fn a_joined_sequences_minimum_spreads_over_its_tracks() {
    let small = |id: Id| leaf(10.0, 10.0).id(id);
    let column = Column::new([
        Row::new([small("a"), small("b")]).min_width(100.0),
        Row::new([small("c"), small("d"), small("e")]),
    ])
    .id("table");
    let solution = no_canvas(column);
    assert_eq!(widths(&solution, "table"), [50.0, 50.0, 10.0]);
    assert_eq!(content(&solution, "e").x, 100.0);
}

#[test]
fn a_tall_legend_slab_makes_its_plots_row_taller() {
    let row = Row::new([
        leaf(100.0, 50.0).id("a").slab(
            Side::Right,
            Slab::new().min_size(Size::new(40.0, 80.0)).id("legend"),
        ),
        leaf(100.0, 50.0).id("b"),
    ]);
    let solution = no_canvas(row);
    assert_eq!(content(&solution, "a").height, 80.0);
    assert_eq!(content(&solution, "b").height, 80.0);
    assert_eq!(slot(&solution, "legend"), Rect::new(100.0, 0.0, 40.0, 80.0));
}

// --- max-content sizes ---

/// A text leaf whose unwrapped width is 150 beside a plot.
fn text_and_plot(text: Track) -> Row<Id> {
    Row::new([
        leaf(40.0, 20.0)
            .max_content(Size::new(150.0, 30.0))
            .id("text"),
        leaf(100.0, 20.0).id("plot"),
    ])
    .track(0, text)
    .id("row")
}

#[test]
fn an_auto_text_column_takes_its_max_content_width_when_there_is_room() {
    let at = |width: f32| {
        let solution = L::solve(&text_and_plot(Track::auto()).into(), &width_only(width)).unwrap();
        widths(&solution, "row")
    };
    assert_eq!(at(400.0), [150.0, 250.0]);
    // Between its base size and its growth limit, the text column takes what the plot leaves.
    assert_eq!(at(200.0), [100.0, 100.0]);
    assert_eq!(at(140.0), [40.0, 100.0]);
    // A fit-content limit caps the column's growth.
    let capped = L::solve(
        &text_and_plot(Track::fit_content(90.0)).into(),
        &width_only(400.0),
    )
    .unwrap();
    assert_eq!(widths(&capped, "row"), [90.0, 310.0]);
}

#[test]
fn an_fr_column_takes_its_max_content_width_without_a_canvas() {
    let row = |text: Track| L::from(text_and_plot(text).track(1, Track::fr(1.0)));
    // Equal weights give both columns the text's unwrapped width.
    assert_eq!(
        widths(&no_canvas(row(Track::fr(1.0))), "row"),
        [150.0, 150.0]
    );
    // With twice the weight, the text column alone needs the unit, and the plot keeps its
    // minimum.
    assert_eq!(
        widths(&no_canvas(row(Track::fr(2.0))), "row"),
        [150.0, 100.0]
    );
    // With a canvas, the weights share the width as before.
    let solution = L::solve(&row(Track::fr(1.0)), &width_only(260.0)).unwrap();
    assert_eq!(widths(&solution, "row"), [130.0, 130.0]);
}

/// Band counts of three facet panels, and the clip padding that each panel's band scale keeps
/// in pixels.
const BANDS: [f32; 3] = [2.0, 5.0, 3.0];
const CLIP: f32 = 16.0;

/// The step of each panel's band scale: its plot's width less the clip padding, per band.
fn band_steps(solution: &Solution<Id>) -> Vec<f32> {
    ["p0", "p1", "p2"]
        .iter()
        .zip(BANDS)
        .map(|(&id, bands)| (content(solution, id).width - CLIP) / bands)
        .collect()
}

#[test]
fn fr_bases_give_band_panels_equal_steps() {
    let mut row = Row::new(["p0", "p1", "p2"].map(|id| leaf(20.0, 50.0).id(id)))
        .gap(Gap::new(10.0))
        .id("row");
    for (column, bands) in BANDS.into_iter().enumerate() {
        row = row.track(column, Track::fr_basis(bands, CLIP));
    }
    // 568 less two gaps and three bases leaves 500 for 10 bands.
    let solution = L::solve(&row.into(), &width_only(568.0)).unwrap();
    assert_eq!(widths(&solution, "row"), [116.0, 266.0, 166.0]);
    assert_eq!(band_steps(&solution), [50.0; 3]);
}

#[test]
fn a_boundary_inside_a_spanned_panel_takes_the_panels_gap() {
    // Each panel spans a px column for its clip padding and an fr column for its bands.
    let panel = |id: Id, bands: f32| {
        Row::new([leaf(20.0, 50.0).id(id).span(2)])
            .track(0, Track::px(CLIP))
            .track(1, Track::fr(bands))
            .gap(Gap::new(0.0))
    };
    let row = Row::new([panel("p0", 2.0), panel("p1", 5.0), panel("p2", 3.0)])
        .gap(Gap::new(10.0))
        .id("row");
    let solution = L::solve(&row.into(), &width_only(568.0)).unwrap();
    // Only the boundaries between panels take the row's gap of 10.
    assert_eq!(
        widths(&solution, "row"),
        [16.0, 100.0, 16.0, 250.0, 16.0, 150.0]
    );
    assert_eq!(content(&solution, "p2").x, 402.0);
    assert_eq!(band_steps(&solution), [50.0; 3]);
}

#[test]
fn a_fit_title_takes_its_max_content_length_without_widening_its_plot() {
    let plot = || -> L {
        leaf(20.0, 50.0)
            .id("plot")
            .slab(
                Side::Top,
                Slab::new()
                    .min_size(Size::new(40.0, 12.0))
                    .max_content(160.0)
                    .id("title")
                    .place(Placement::fit(Position::Center)),
            )
            .into()
    };
    let at = |width: f32| plot().solve(&width_only(width)).unwrap();
    assert_eq!(
        content(&at(300.0), "title"),
        Rect::new(70.0, 0.0, 160.0, 12.0)
    );
    assert_eq!(
        content(&at(120.0), "title"),
        Rect::new(0.0, 0.0, 120.0, 12.0)
    );
    assert_eq!(content(&at(30.0), "title").width, 40.0);
    // Only the title's minimum length raises the plot's minimum.
    assert_eq!(no_canvas(plot()).size().width, 40.0);
}

#[test]
fn a_spanning_items_max_content_reaches_the_auto_tracks_it_spans() {
    let column = Column::new([
        Layout::from(
            Row::new([leaf(10.0, 10.0).id("a"), leaf(10.0, 10.0).id("b")]).tracks(Track::auto()),
        ),
        leaf(10.0, 10.0)
            .max_content(Size::new(100.0, 10.0))
            .id("wide")
            .into(),
    ])
    .id("table");
    assert_eq!(widths(&no_canvas(column), "table"), [50.0, 50.0]);
}

/// Two matched Auto columns, one of them with a max-content width of 80.
fn matched_row() -> Row<Id> {
    Row::new([
        leaf(10.0, 10.0).max_content(Size::new(80.0, 10.0)).id("a"),
        leaf(10.0, 10.0).id("b"),
    ])
    .tracks(Track::auto().tag("c"))
    .match_tag("c")
    .id("row")
}

#[test]
fn matched_tracks_share_the_largest_growth_limit() {
    let solution = L::from(matched_row()).solve(&explained_size()).unwrap();
    assert_eq!(widths(&solution, "row"), [80.0, 80.0]);
    let sizes = solution.explanation().sizes.as_ref().unwrap();
    let columns = sizes
        .owners
        .iter()
        .find(|owner| owner.path.is_empty() && owner.direction == H)
        .unwrap();
    assert_eq!(
        columns
            .tracks
            .iter()
            .map(|track| (track.base_size, track.growth_limit))
            .collect::<Vec<_>>(),
        [(10.0, 80.0), (10.0, 80.0)]
    );
}

#[test]
fn a_solve_without_a_canvas_takes_max_content_sizes_and_min_size_reports_minimums() {
    let layout = L::from(text_and_plot(Track::auto()));
    let measured = layout.measure(&SolveOptions::default(), &mut ()).unwrap();
    assert_eq!(measured.solution().size(), Size::new(250.0, 30.0));
    assert_eq!(measured.min_size().unwrap(), Size::new(140.0, 20.0));
}

/// Counts the max-content questions per id and direction.
#[derive(Default)]
struct MaxContent {
    asked: HashMap<(Id, Direction), usize>,
}

impl Measure<Id> for MaxContent {
    fn width(&mut self, id: &Id, height: f32) -> Option<Measurement> {
        (*id == "y").then(|| Measurement::from(if height > 100.0 { 40.0 } else { 30.0 }))
    }

    fn max_content(&mut self, id: &Id, direction: Direction) -> Option<f32> {
        *self.asked.entry((id, direction)).or_default() += 1;
        Some(match direction {
            H => 300.0,
            V => 200.0,
        })
    }
}

#[test]
fn max_content_is_asked_once_per_leaf_and_direction() {
    let mut measure = MaxContent::default();
    let measured = y_axis_chart()
        .measure(&explained(400.0, 300.0), &mut measure)
        .unwrap();
    assert_eq!(measured.solution().explanation().solves, 2);
    // A left slab is asked only along its side.
    let expected = HashMap::from([(("plot", H), 1), (("plot", V), 1), (("y", V), 1)]);
    assert_eq!(measure.asked, expected);
    y_axis_chart()
        .measure_reusing(&measured, &explained(500.0, 300.0), &mut measure)
        .unwrap();
    assert_eq!(measure.asked, expected);
}

#[test]
fn spanning_max_content_passes_a_capped_track_to_the_others() {
    let grid = Grid::new([Track::fit_content(20.0), Track::auto()], [Track::auto()])
        .child(
            GridSlot::new(0, 0).span(2, 1),
            Leaf::new().id("text").max_content(Size::new(100.0, 10.0)),
        )
        .distribute(H, Distribute::Start)
        .id("grid");
    // The fit-content track stops at 20, and the Auto track takes the other 80.
    let sized = L::solve(&grid.clone().into(), &SolveOptions::sized(200.0, 100.0)).unwrap();
    assert_eq!(widths(&sized, "grid"), [20.0, 80.0]);
    assert_eq!(content(&sized, "text").width, 100.0);
    assert_eq!(no_canvas(grid).size().width, 100.0);
}

// --- Wrap ---

const WRAPPED: [Id; 6] = ["w0", "w1", "w2", "w3", "w4", "w5"];
const X_LABELS: [Id; 6] = ["x0", "x1", "x2", "x3", "x4", "x5"];
const Y_LABELS: [Id; 6] = ["y0", "y1", "y2", "y3", "y4", "y5"];

fn wrap_of(count: usize) -> Wrap<Id> {
    Wrap::new((0..count).map(|i| leaf(50.0, 20.0).id(WRAPPED[i])))
        .min_column(80.0)
        .gap(Gap::new(10.0))
        .id("wrap")
}

fn wrap_columns(solution: &Solution<Id>) -> usize {
    match &region(solution, "wrap").kind {
        RegionKind::Wrap { columns, .. } => *columns,
        kind => panic!("the wrap is {kind:?}"),
    }
}

/// A Wrap's column count, or a Flow's line count.
fn arrangement(solution: &Solution<Id>) -> usize {
    match &region(solution, "wrap").kind {
        RegionKind::Wrap { columns, .. } => *columns,
        RegionKind::Flow { tracks } => tracks.rows.len(),
        kind => panic!("the wrap is {kind:?}"),
    }
}

fn empty_cells(solution: &Solution<Id>) -> Vec<Rect> {
    match &region(solution, "wrap").kind {
        RegionKind::Wrap { empty, .. } => empty.clone(),
        kind => panic!("the wrap is {kind:?}"),
    }
}

fn at_width(layout: impl Into<L>, width: f32) -> Solution<Id> {
    layout.into().solve(&width_only(width)).unwrap()
}

#[test]
fn a_wraps_count_follows_its_width() {
    let columns = |width| wrap_columns(&at_width(wrap_of(6), width));
    assert_eq!(
        [
            columns(300.0),
            columns(200.0),
            columns(100.0),
            columns(60.0)
        ],
        [3, 2, 1, 1]
    );
    let solution = at_width(wrap_of(6), 300.0);
    assert_eq!(widths(&solution, "wrap"), [280.0 / 3.0; 3]);
    // Below the column minimum, one column keeps the minimum and overflows.
    assert_eq!(widths(&at_width(wrap_of(6), 60.0), "wrap"), [80.0]);
    assert_eq!(wrap_columns(&at_width(wrap_of(6).max_columns(2), 300.0)), 2);
    // Few children give fewer, wider columns.
    let few = at_width(wrap_of(2), 300.0);
    assert_eq!(widths(&few, "wrap"), [145.0, 145.0]);
}

/// A panel with x labels under it and y labels left of it, keyed as given.
fn guided(i: usize, y_key: &str) -> Leaf<Id> {
    leaf(50.0, 20.0)
        .id(WRAPPED[i])
        .slab(Side::Bottom, shared(X_LABELS[i], "x-labels", "x", 10.0))
        .slab(
            Side::Left,
            Slab::new()
                .min_width(20.0)
                .id(Y_LABELS[i])
                .tag("y-labels")
                .content_key(y_key),
        )
}

#[test]
fn a_wrap_solves_as_the_column_of_rows_for_its_count() {
    let wrap = Wrap::new((0..6).map(|i| guided(i, "y")))
        .min_column(80.0)
        .gap(Gap::new(10.0))
        .id("wrap");
    let rows = Column::new([
        Row::new((0..3).map(|i| guided(i, "y"))).gap(Gap::new(10.0)),
        Row::new((3..6).map(|i| guided(i, "y"))).gap(Gap::new(10.0)),
    ])
    .gap(Gap::new(10.0));
    let wrapped = at_width(wrap, 330.0);
    let written = at_width(rows, 330.0);
    assert_eq!(wrap_columns(&wrapped), 3);
    for id in WRAPPED {
        assert_eq!(content(&wrapped, id), content(&written, id), "{id}");
    }
    assert_eq!(wrapped.size(), written.size());
}

#[test]
fn y_labels_collapse_along_rows_only_when_every_panel_shows_one_key() {
    let wrap = |keys: [&'static str; 6]| {
        Wrap::new((0..6).map(|i| guided(i, keys[i])))
            .min_column(80.0)
            .gap(Gap::new(10.0))
            .collapse_tag("y-labels")
            .id("wrap")
    };
    let shared_key = ["y"; 6];
    let kept = |solution: &Solution<Id>| -> Vec<Id> {
        Y_LABELS
            .into_iter()
            .filter(|&id| represented_by(solution, id).is_none())
            .collect()
    };
    let three = at_width(wrap(shared_key), 330.0);
    assert_eq!(wrap_columns(&three), 3);
    assert_eq!(kept(&three), ["y0", "y3"]);
    let two = at_width(wrap(shared_key), 230.0);
    assert_eq!(wrap_columns(&two), 2);
    assert_eq!(kept(&two), ["y0", "y2", "y4"]);
    // With a key per panel, every panel keeps its labels at every count.
    let separate = at_width(wrap(["a", "b", "c", "d", "e", "f"]), 330.0);
    assert_eq!(kept(&separate), Y_LABELS);
}

fn five_with_x_labels() -> Wrap<Id> {
    Wrap::new((0..5).map(|i| {
        leaf(50.0, 20.0)
            .id(WRAPPED[i])
            .slab(Side::Bottom, shared(X_LABELS[i], "x-labels", "x", 15.0))
    }))
    .min_column(80.0)
    .gap(Gap::new(10.0))
    .collapse_tag("x-labels")
    .id("wrap")
}

#[test]
fn the_lowest_panel_hangs_its_x_labels_into_an_empty_cell_that_the_wrap_reports() {
    let solution = at_width(five_with_x_labels(), 280.0);
    assert_eq!(wrap_columns(&solution), 3);
    assert_eq!(
        represented_by(&solution, "x0"),
        Some(path_of(&solution, "x3"))
    );
    assert_eq!(
        represented_by(&solution, "x1"),
        Some(path_of(&solution, "x4"))
    );
    assert_eq!(represented_by(&solution, "x2"), None);
    // The first row's labels are not drawn and the third column's hang into the empty cell, so
    // the gap between the rows is the minimum.
    let rows = &tracks(&solution, "wrap").rows;
    assert_eq!(rows[1].start - (rows[0].start + rows[0].length), 10.0);
    let panel = content(&solution, "w2");
    assert_eq!(
        slot(&solution, "x2"),
        Rect::new(panel.x, panel.bottom(), panel.width, 15.0)
    );
    assert!(slot(&solution, "x2").bottom() > rows[1].start);
    let columns = &tracks(&solution, "wrap").columns;
    assert_eq!(
        empty_cells(&solution),
        [Rect::new(
            columns[2].start,
            rows[1].start,
            columns[2].length,
            rows[1].length
        )]
    );
    assert_eq!(
        cell(&solution, "w4").map(|cell| cell.slot),
        Some(GridSlot::new(1, 1))
    );
    let two = at_width(five_with_x_labels(), 180.0);
    assert_eq!(wrap_columns(&two), 2);
    assert_eq!(empty_cells(&two).len(), 1);
    assert_eq!(
        cell(&two, "w4").map(|cell| cell.slot),
        Some(GridSlot::new(0, 2))
    );
    assert_eq!(cell(&two, "w4").map(|cell| cell.table), Some(Vec::new()));
}

#[test]
fn a_resize_changes_the_count_without_measuring() {
    let wrap = || -> L {
        Wrap::new((0..6).map(|i| {
            leaf(50.0, 20.0).id(WRAPPED[i]).slab(
                Side::Bottom,
                Slab::new().id(X_LABELS[i]).tag("x-labels").content_key("x"),
            )
        }))
        .min_column(80.0)
        .gap(Gap::new(10.0))
        .collapse_tag("x-labels")
        .id("wrap")
        .into()
    };
    let mut measure = Measurer::heights(|id, _| {
        id.starts_with('x')
            .then(|| Measurement::over(14.0, 0.0..=1000.0))
    });
    let measured = wrap().measure(&width_only(330.0), &mut measure).unwrap();
    assert_eq!(wrap_columns(measured.solution()), 3);
    // The labels share one key, so they are asked once.
    assert_eq!(measure.answered(), 1);
    let resized = measured.resize(Size::new(230.0, 200.0)).unwrap();
    assert_eq!(wrap_columns(&resized), 2);
    assert!(resized.stale().is_empty());
    assert_eq!(
        represented_by(&resized, "x2"),
        Some(path_of(&resized, "x4"))
    );
    wrap()
        .measure_reusing(&measured, &width_only(230.0), &mut measure)
        .unwrap();
    assert_eq!(measure.answered(), 1);
}

#[test]
fn a_wrap_without_a_canvas_puts_every_child_on_one_row() {
    let measured = L::from(wrap_of(6))
        .measure(&SolveOptions::default(), &mut ())
        .unwrap();
    assert_eq!(wrap_columns(measured.solution()), 6);
    assert_eq!(measured.solution().size().width, 6.0 * 80.0 + 50.0);
    assert_eq!(measured.min_size().unwrap().width, 80.0);
    // A measured width that grows makes the Wrap narrower, and the solves end.
    let growing = Wrap::new((0..4).map(|i| {
        leaf(50.0, 40.0)
            .id(WRAPPED[i])
            .slab(Side::Left, Slab::new().id(Y_LABELS[i]))
    }))
    .gap(Gap::new(10.0))
    .id("wrap");
    let mut measure = Measurer::widths(|id, height| {
        id.starts_with('y')
            .then(|| Measurement::from(if height > 30.0 { 30.0 } else { 20.0 }))
    });
    let measured = L::from(growing)
        .measure(&explained(300.0, 200.0), &mut measure)
        .unwrap();
    assert!(!measured.solution().explanation().stopped_at_limit);
}

#[test]
fn a_row_of_wraps_without_a_canvas_puts_each_wraps_children_on_one_row() {
    let wrap = |id: Id, panels: &[Id]| {
        Wrap::new(panels.iter().map(|&panel| leaf(100.0, 60.0).id(panel)))
            .gap(Gap::new(10.0))
            .id(id)
    };
    let row: L = Row::new([
        wrap("left", &["l0", "l1", "l2", "l3", "l4"]),
        wrap("right", &["r0", "r1", "r2"]),
    ])
    .gap(Gap::new(30.0))
    .into();
    let columns = |solution: &Solution<Id>, id: Id| match &region(solution, id).kind {
        RegionKind::Wrap { columns, .. } => *columns,
        kind => panic!("{id} is {kind:?}"),
    };
    let measured = row.measure(&SolveOptions::default(), &mut ()).unwrap();
    let open = measured.solution();
    assert_eq!((columns(open, "left"), columns(open, "right")), (5, 3));
    // Equal weights give the second Wrap the first one's width, with wider columns.
    assert_eq!(open.size(), Size::new(540.0 + 30.0 + 540.0, 60.0));
    // The smallest canvas takes one column each.
    assert_eq!(measured.min_size().unwrap().width, 100.0 + 30.0 + 100.0);
}

#[test]
fn a_wrap_keeps_its_trailing_overflow_at_every_count() {
    let wrap = |count: usize| {
        Wrap::new((0..count).map(|i| {
            leaf(50.0, 40.0)
                .id(WRAPPED[i])
                .slab(Side::Right, Slab::new().min_width(30.0).id(Y_LABELS[i]))
        }))
        .id("wrap")
    };
    let solution = sized(wrap(4), 300.0, 200.0);
    assert_eq!(wrap_columns(&solution), 3);
    for id in &Y_LABELS[..4] {
        assert!(slot(&solution, id).right() <= 300.0, "{id}");
    }
    let open = no_canvas(wrap(3));
    assert_eq!(open.size().width, slot(&open, "y2").right());
}

#[test]
fn merged_slabs_in_a_wrap_never_collapse_or_narrow_its_gaps() {
    let cell = |i: usize| {
        Row::new([leaf(60.0, 40.0).id(WRAPPED[i]).slab(
            Side::Left,
            Slab::new()
                .min_width(30.0)
                .id(Y_LABELS[i])
                .tag("y-labels")
                .content_key("y"),
        )])
        .merge_tag("y-labels")
    };
    let wrap = Wrap::new((0..3).map(cell))
        .collapse_tag("y-labels")
        .id("wrap");
    let solution = sized(wrap, 400.0, 100.0);
    for i in 1..3 {
        // A region keeps its exact width, so its right edge, its left edge plus its width, can
        // pass its neighbor's left edge in the last bit.
        assert!(
            slot(&solution, Y_LABELS[i]).x >= content(&solution, WRAPPED[i - 1]).right() - 1e-4,
            "{i}"
        );
    }
}

#[test]
fn a_wrap_merges_only_when_every_child_shows_the_slab() {
    let titled = |i: usize| {
        leaf(60.0, 40.0).id(WRAPPED[i]).slab(
            Side::Bottom,
            Slab::new()
                .min_size(Size::new(30.0, 12.0))
                .id(TITLES[i])
                .tag("x-title")
                .content_key("Month"),
        )
    };
    let wrap = Wrap::new([
        titled(0),
        titled(1),
        titled(2),
        leaf(60.0, 40.0).id("spacer"),
        titled(4),
    ])
    .max_columns(4)
    .merge_tag("x-title")
    .id("wrap");
    for width in [400.0, 70.0] {
        let solution = sized(wrap.clone(), width, 300.0);
        assert!(
            solution
                .regions()
                .all(|region| !matches!(region.path.last(), Some(PathStep::Merged(..)))),
            "{width}"
        );
        // The titles still collapse down each column, and the first keeps its own region.
        assert_eq!(
            path_of(&solution, "t0"),
            [PathStep::Child(0), PathStep::Slab(Side::Bottom, 0)],
            "{width}"
        );
    }
}

/// A Wrap of two pairs of panels, each panel with a left title with the tag and the key that
/// `titles` gives for its place in the pair, or none.
fn wrapped_pairs(titles: [Option<&'static str>; 2]) -> Wrap<Id> {
    let panel = |i: usize| {
        let panel = leaf(40.0, 30.0).id(WRAPPED[i]);
        match titles[i % 2] {
            Some(title) => panel.slab(
                Side::Left,
                Slab::new()
                    .min_width(12.0)
                    .id(Y_LABELS[i])
                    .tag(title)
                    .content_key(title),
            ),
            None => panel,
        }
    };
    Wrap::new((0..2).map(|pair| Row::new([panel(2 * pair), panel(2 * pair + 1)])))
        .merge_tag("first")
        .merge_tag("second")
        .id("wrap")
}

#[test]
fn a_wrap_merges_a_title_only_past_panels_that_show_it_or_nothing() {
    for width in [400.0, 120.0] {
        // A merged first title would pass the second panel of the pair before it.
        let solution = sized(wrapped_pairs([Some("first"), Some("second")]), width, 300.0);
        for id in Y_LABELS.into_iter().take(4) {
            assert_eq!(represented_by(&solution, id), None, "{width} {id}");
            assert_eq!(slot(&solution, id).width, 12.0, "{width} {id}");
        }
        // When every panel shows the title, or nothing on its left, it merges onto the Wrap's
        // left side.
        for titles in [[Some("first"), Some("first")], [Some("first"), None]] {
            let solution = sized(wrapped_pairs(titles), width, 300.0);
            assert_eq!(
                path_of(&solution, "y0"),
                [PathStep::Merged(Side::Left, 0)],
                "{width} {titles:?}"
            );
        }
    }
}

#[test]
fn widths_measured_at_a_discarded_count_do_not_carry_over() {
    let wrap: L = Wrap::new((0..4).map(|i| Leaf::new().id(WRAPPED[i])))
        .id("wrap")
        .into();
    let mut measure = Measurer::widths(|id, height| {
        id.starts_with('w')
            .then(|| Measurement::from(if height < 150.0 { 120.0 } else { 40.0 }))
    });
    let measured = wrap
        .measure(&SolveOptions::sized(440.0, 400.0), &mut measure)
        .unwrap();
    assert_eq!(wrap_columns(measured.solution()), 4);
    assert_eq!(widths(measured.solution(), "wrap"), [110.0; 4]);
}

#[test]
fn a_match_group_holds_wrap_column_gaps() {
    let pair = |gap: f32, id: Id| {
        Wrap::new([leaf(0.0, 10.0), leaf(0.0, 10.0)])
            .min_column(50.0)
            .gap(Gap::new(gap).tag("g"))
            .id(id)
    };
    let column = Column::new([pair(10.0, "a"), pair(40.0, "b")]).match_tag("g");
    let solution = L::from(column)
        .solve(&SolveOptions::sized(300.0, 200.0))
        .unwrap();
    let gap = |id| {
        let columns = &tracks(&solution, id).columns;
        columns[1].start - (columns[0].start + columns[0].length)
    };
    assert_eq!([gap("a"), gap("b")], [40.0, 40.0]);
}

// --- Flow ---

const CHIPS: [Id; 4] = ["c0", "c1", "c2", "c3"];

fn chips(distribute: Distribute) -> Flow<Id> {
    Flow::new(
        [60.0, 80.0, 40.0, 100.0]
            .into_iter()
            .enumerate()
            .map(|(i, width)| {
                leaf(20.0, 16.0)
                    .max_content(Size::new(width, 16.0))
                    .id(CHIPS[i])
                    .slab(Side::Bottom, shared(X_LABELS[i], "x-labels", "x", 4.0))
            }),
    )
    .gap(Gap::new(10.0))
    .distribute(H, distribute)
    .id("wrap")
}

#[test]
fn a_flow_breaks_lines_at_its_width() {
    let solution = at_width(chips(Distribute::Start), 200.0);
    let line = |id| cell(&solution, id).map(|cell| cell.slot.row);
    assert_eq!(CHIPS.map(line), [Some(0), Some(0), Some(0), Some(1)]);
    let x = |id| content(&solution, id).x;
    assert_eq!(CHIPS.map(x), [0.0, 70.0, 160.0, 0.0]);
    assert_eq!(content(&solution, "c1").width, 80.0);
    assert!(content(&solution, "c3").y > content(&solution, "c0").bottom());
    // Without a canvas, every child shares one line.
    let open = no_canvas(chips(Distribute::Start));
    assert!(
        CHIPS
            .into_iter()
            .all(|id| cell(&open, id).unwrap().slot.row == 0)
    );
    assert_eq!(open.size().width, 310.0);
}

#[test]
fn distribute_places_each_lines_leftover_space() {
    let at = |distribute| at_width(chips(distribute), 200.0);
    assert_eq!(content(&at(Distribute::End), "c3").x, 100.0);
    assert_eq!(content(&at(Distribute::Center), "c3").x, 50.0);
    // The full first line has no leftover space to place.
    assert_eq!(content(&at(Distribute::End), "c0").x, 0.0);
    assert_eq!(content(&at(Distribute::Stretch), "c3").width, 200.0);
}

#[test]
fn nothing_collapses_or_merges_in_a_flow() {
    let around = Column::new([chips(Distribute::Start)])
        .collapse_tag("x-labels")
        .merge_tag("x-labels");
    let solution = at_width(around, 200.0);
    assert!(solution.explanation().representations.is_empty());
    for id in &X_LABELS[..4] {
        assert_eq!(represented_by(&solution, id), None, "{id}");
        assert_eq!(slot(&solution, id).height, 4.0, "{id}");
    }
}

#[test]
fn a_flows_lines_hold_the_slabs_that_face_each_other() {
    let chip = |slab: Slab<Id>, side: Side| leaf(100.0, 20.0).slab(side, slab);
    let flow = Flow::new([
        chip(Slab::new().id("below").min_height(10.0), Side::Bottom),
        chip(Slab::new().id("above").min_height(20.0), Side::Top),
    ])
    .rows(Track::auto());
    let solution = L::from(flow).solve(&width_only(100.0)).unwrap();
    // Each chip fills a line, and the gap between the lines holds both slabs.
    assert_eq!(slot(&solution, "below"), Rect::new(0.0, 20.0, 100.0, 10.0));
    assert_eq!(slot(&solution, "above"), Rect::new(0.0, 30.0, 100.0, 20.0));
}

// --- measuring ---

fn header_chart(width: f32) -> L {
    Grid::new([Track::px(width)], auto(1))
        .child(
            GridSlot::new(0, 0),
            leaf(0.0, 100.0)
                .id("plot")
                .slab(Side::Top, Slab::new().id("header")),
        )
        .into()
}

/// A header that wraps to two lines below 150 px.
fn header_height(id: Id, width: f32) -> Option<Measurement> {
    (id == "header").then(|| {
        if width < 150.0 {
            Measurement::over(40.0, 0.0..=149.0)
        } else {
            Measurement::over(20.0, 150.0..=f32::INFINITY)
        }
    })
}

fn y_axis_chart() -> L {
    leaf(0.0, 0.0)
        .id("plot")
        .slab(Side::Left, Slab::new().id("y"))
        .into()
}

fn explained(width: f32, height: f32) -> SolveOptions {
    SolveOptions {
        explain: true,
        ..SolveOptions::sized(width, height)
    }
}

/// Options that record the explanation of each track and gap, without a canvas.
fn explained_size() -> SolveOptions {
    SolveOptions {
        explain: true,
        ..SolveOptions::default()
    }
}

#[test]
fn a_y_axis_measured_at_its_plot_height_settles_in_two_solves() {
    let mut measure = Measurer::widths(|id, height| {
        (id == "y").then(|| Measurement::from(if height > 200.0 { 40.0 } else { 30.0 }))
    });
    let measured = y_axis_chart()
        .measure(&explained(400.0, 300.0), &mut measure)
        .unwrap();
    let solution = measured.solution();
    assert_eq!(
        content(solution, "plot"),
        Rect::new(40.0, 0.0, 360.0, 300.0)
    );
    let explanation = solution.explanation();
    assert_eq!(explanation.solves, 2);
    assert!(!explanation.stopped_at_limit);
    assert_eq!(explanation.raised_widths.len(), 1);
}

#[test]
fn an_alternating_width_stops_once_the_larger_value_is_in_place() {
    let layout: L = leaf(0.0, 0.0)
        .id("plot")
        .slab(Side::Left, Slab::new().id("y"))
        .slab(Side::Bottom, Slab::new().id("x"))
        .into();
    let mut measure = Measurer::new(
        // Labels rotate below 350 px.
        |id, width| (id == "x").then(|| Measurement::from(if width > 350.0 { 20.0 } else { 60.0 })),
        // A tall plot gets another tick with a wider label.
        |id, height| {
            (id == "y").then(|| Measurement::from(if height > 260.0 { 50.0 } else { 30.0 }))
        },
    );
    let measured = layout
        .measure(&explained(400.0, 300.0), &mut measure)
        .unwrap();
    let solution = measured.solution();
    assert_eq!(
        content(solution, "plot"),
        Rect::new(50.0, 0.0, 350.0, 240.0)
    );
    assert_eq!(solution.explanation().solves, 2);
}

#[test]
fn a_width_that_keeps_growing_stops_at_the_limit() {
    let layout: L = leaf(10.0, 0.0).id("text").into();
    let mut measure = Measurer::new(
        |_, width| Some(Measurement::from(width + 1.0)),
        |_, height| Some(Measurement::from(height + 1.0)),
    );
    let measured = layout.measure(&explained_size(), &mut measure).unwrap();
    let solution = measured.solution();
    assert_eq!(content(solution, "text"), Rect::new(0.0, 0.0, 16.0, 17.0));
    let explanation = solution.explanation();
    assert_eq!(explanation.solves, 4);
    assert!(explanation.stopped_at_limit);
}

#[test]
fn a_width_that_converges_settles_within_the_default_solves() {
    // A map keeps its aspect, and its labels reach past its left edge by less as it grows, so
    // each solve raises their width by a fiftieth of the last raise.
    let layout: L = leaf(0.0, 0.0)
        .id("map")
        .aspect(2.0)
        .slab(Side::Left, Slab::new().id("labels"))
        .into();
    let mut measure = Measurer::widths(|id, height| {
        (id == "labels").then(|| Measurement::from(20.0 + 0.04 * (200.0 - height)))
    });
    let measured = layout
        .measure(&explained(400.0, 300.0), &mut measure)
        .unwrap();
    let solution = measured.solution();
    assert!(!solution.explanation().stopped_at_limit);
    assert!(solution.warnings().is_empty(), "{:?}", solution.warnings());
    // The labels settle at 20 / 0.98, a thousandth of a unit short of it at most.
    let map = content(solution, "map");
    assert!((map.x - 20.0 / 0.98).abs() < 1e-3, "{map:?}");
}

fn legend_chart() -> L {
    Grid::new([Track::fr(1.0), Track::auto()], auto(1))
        .child(GridSlot::new(0, 0), leaf(0.0, 120.0).id("plot"))
        .child(GridSlot::new(1, 0), leaf(0.0, 20.0).id("legend"))
        .into()
}

#[test]
fn a_side_legend_wraps_into_columns_without_making_its_row_taller() {
    // Ten 20 px entries in 50 px columns, as many columns as the height needs.
    let mut measure = Measurer::widths(|id, height| {
        (id == "legend").then(|| {
            let per_column = (height / 20.0).floor().max(1.0);
            Measurement::from((10.0 / per_column).ceil() * 50.0)
        })
    });
    let measured = legend_chart()
        .measure(&width_only(400.0), &mut measure)
        .unwrap();
    let solution = measured.solution();
    assert_eq!(
        content(solution, "legend"),
        Rect::new(300.0, 0.0, 100.0, 120.0)
    );
    assert_eq!(content(solution, "plot"), Rect::new(0.0, 0.0, 300.0, 120.0));
}

// --- overhang ---

/// X labels 12 high that reach 4 px past their plot's start and 20 past its end.
fn overhanging_labels(id: Id, _: f32) -> Option<Measurement> {
    id.starts_with('x')
        .then(|| Measurement::always(12.0).with_overhang([4.0, 20.0]))
}

fn measured_solution(layout: impl Into<L>, measure: &mut impl Measure<Id>) -> Solution<Id> {
    layout
        .into()
        .measure(&SolveOptions::default(), measure)
        .unwrap()
        .into_solution()
}

/// A 100 x 50 plot with measured x labels.
fn plot_with_labels(id: Id, labels: Id) -> Leaf<Id> {
    leaf(100.0, 50.0)
        .id(id)
        .slab(Side::Bottom, Slab::new().id(labels))
}

#[test]
fn an_overhang_widens_the_gap_and_the_padding_beside_its_plot() {
    let row =
        auto_row([plot_with_labels("a", "xa"), plot_with_labels("b", "xb")]).gap(Gap::new(10.0));
    let solution = measured_solution(row, &mut Measurer::heights(overhanging_labels));
    // The gap holds a's last label and b's first one, and the root holds the outer ones.
    assert_eq!(content(&solution, "a").x, 4.0);
    assert_eq!(
        content(&solution, "b").x - content(&solution, "a").right(),
        24.0
    );
    assert_eq!(solution.size(), Size::new(248.0, 62.0));
    // The width pass ran before the labels were measured, so a second solve counts them.
    let explanation = solution.explanation();
    assert_eq!(explanation.solves, 2);
    assert_eq!(explanation.raised_overhangs.len(), 2);
    assert_eq!(explanation.raised_overhangs[0].to, [4.0, 20.0]);
}

#[test]
fn a_y_axis_overhang_widens_the_row_gap_above_its_plot() {
    let plot = |id: Id, labels: Id| {
        leaf(100.0, 50.0)
            .id(id)
            .slab(Side::Left, Slab::new().id(labels))
    };
    let column = Column::new([plot("a", "ya"), plot("b", "yb")])
        .tracks(Track::auto())
        .gap(Gap::new(4.0));
    let mut measure = Measurer::widths(|id, _| {
        id.starts_with('y')
            .then(|| Measurement::always(30.0).with_overhang([6.0, 6.0]))
    });
    let solution = measured_solution(column, &mut measure);
    assert_eq!(content(&solution, "a"), Rect::new(30.0, 6.0, 100.0, 50.0));
    assert_eq!(
        content(&solution, "b").y - content(&solution, "a").bottom(),
        12.0
    );
    assert_eq!(solution.size(), Size::new(130.0, 124.0));
    // The width and the overhang grew in the same solve, so one more solve counts both.
    assert_eq!(solution.explanation().solves, 2);
}

#[test]
fn a_contained_overhang_stays_in_its_slot_and_a_flushed_one_takes_no_space() {
    let at = |policy| {
        let row = auto_row([
            plot_with_labels("a", "xa").overflow(Side::Right, policy),
            leaf(100.0, 50.0).id("b"),
        ])
        .gap(Gap::new(10.0));
        measured_solution(row, &mut Measurer::heights(overhanging_labels))
    };
    let contained = at(Overflow::Contain);
    assert_eq!(
        slot(&contained, "a").right() - content(&contained, "a").right(),
        20.0
    );
    assert_eq!(
        slot(&contained, "b").x - slot(&contained, "a").right(),
        10.0
    );
    let flushed = at(Overflow::Flush);
    assert_eq!(
        content(&flushed, "b").x - content(&flushed, "a").right(),
        10.0
    );
}

#[test]
fn an_overhang_counts_past_the_slabs_beside_it_only_from_a_full_slab() {
    let width = |labels: Slab<Id>| {
        let plot = leaf(100.0, 50.0)
            .slab(Side::Right, Slab::new().min_width(30.0))
            .slab(Side::Bottom, labels.id("xa"));
        measured_solution(plot, &mut Measurer::heights(overhanging_labels))
            .size()
            .width
    };
    // The labels run under the plot, beside the right slab, so the larger of the two counts.
    assert_eq!(width(Slab::new()), 4.0 + 100.0 + 30.0);
    // Full labels run under the right slab, and reach past it.
    assert_eq!(width(Slab::new().full()), 4.0 + 100.0 + 30.0 + 20.0);
}

#[test]
fn a_collapsed_slab_adds_no_overhang() {
    let labels = |id: Id| Slab::new().id(id).tag("x-labels").content_key("x");
    let grid = Grid::new(auto(2), auto(2))
        .child(
            GridSlot::new(0, 0),
            leaf(100.0, 50.0).id("a").slab(Side::Bottom, labels("xa")),
        )
        .child(
            GridSlot::new(0, 1),
            leaf(100.0, 50.0).id("b").slab(Side::Bottom, labels("xb")),
        )
        .child(
            GridSlot::new(1, 0),
            leaf(100.0, 50.0)
                .id("c")
                .slab(Side::Left, Slab::new().min_width(15.0)),
        )
        .collapse_tag("x-labels");
    let solution = measured_solution(grid, &mut Measurer::heights(overhanging_labels));
    // b draws the labels, so a's row holds only c's slab, and b's row only b's overhang.
    assert_eq!(
        represented_by(&solution, "xa"),
        Some(path_of(&solution, "xb"))
    );
    assert_eq!(
        content(&solution, "c").x - content(&solution, "a").right(),
        20.0
    );
}

#[test]
fn a_slab_on_a_sequence_in_its_parents_table_counts_its_overhang() {
    let column = Column::new([
        auto_row([leaf(100.0, 50.0).id("a"), leaf(100.0, 50.0).id("b")])
            .slab(Side::Bottom, Slab::new().id("xr")),
        auto_row([leaf(100.0, 50.0).id("c"), leaf(100.0, 50.0).id("d")]),
    ]);
    let solution = measured_solution(column, &mut Measurer::heights(overhanging_labels));
    assert_eq!(content(&solution, "a").x, 4.0);
    assert_eq!(solution.size().width, 4.0 + 200.0 + 20.0);
}

#[test]
fn an_aspect_fit_measures_a_title_again_at_its_final_width() {
    // A title that wraps to two lines below 200 px.
    let mut measure = Measurer::heights(|id, width| {
        (id == "title").then(|| {
            if width >= 200.0 {
                Measurement::over(20.0, 200.0..=f32::INFINITY)
            } else {
                Measurement::over(40.0, 0.0..=200.0f32.next_down())
            }
        })
    });
    let map = Leaf::new()
        .aspect(1.0)
        .id("map")
        .slab(Side::Top, Slab::new().id("title"));
    let solution = L::from(map)
        .measure(&SolveOptions::sized(400.0, 100.0), &mut measure)
        .unwrap()
        .into_solution();
    // At its first width the title is one line, but the fitted map leaves it 80 px wide, so a
    // second solve gives it two lines, and the map fits under them.
    assert_eq!(slot(&solution, "title"), Rect::new(0.0, 0.0, 60.0, 40.0));
    assert_eq!(content(&solution, "map"), Rect::new(0.0, 40.0, 60.0, 60.0));
    assert_eq!(solution.explanation().solves, 2);
}

// --- resizing and reuse ---

/// Canvas widths that move a Wrap's children to more columns or lines and back.
const REFLOW_WIDTHS: [f32; 10] = [
    400.0, 230.0, 150.0, 400.0, 90.0, 230.0, 330.0, 150.0, 120.0, 400.0,
];

/// Labels that are taller under narrow panels and reach farther past their panel's end, with the
/// widths each answer holds for.
fn x_label_height(id: Id, width: f32) -> Option<Measurement> {
    id.starts_with('x').then(|| {
        if width < 70.0 {
            Measurement::over(24.0, 0.0..=70.0f32.next_down()).with_overhang([2.0, 9.0])
        } else {
            Measurement::over(12.0, 70.0..=f32::INFINITY).with_overhang([0.0, 5.0])
        }
    })
}

#[test]
fn remeasure_and_resize_reflow_wraps_as_fresh_measures_do() {
    let columns = Wrap::new((0..6).map(|i| guided(i, "y")))
        .min_column(60.0)
        .gap(Gap::new(10.0))
        .collapse_tag("x-labels")
        .collapse_tag("y-labels")
        .id("wrap");
    for layout in [L::from(columns), L::from(chips(Distribute::Start))] {
        let size = |width| Size::new(width, 400.0);
        let mut measure = Measurer::heights(x_label_height);
        let mut measured = layout
            .measure(&SolveOptions::sized(REFLOW_WIDTHS[0], 400.0), &mut measure)
            .unwrap();
        let mut counts = vec![arrangement(measured.solution())];
        let mut compared = Vec::new();
        for width in REFLOW_WIDTHS {
            let fresh = layout
                .measure(
                    &SolveOptions::sized(width, 400.0),
                    &mut Measurer::heights(x_label_height),
                )
                .unwrap();
            let expected: Vec<_> = fresh.solution().regions().collect();
            let resized = measured.resize(size(width)).unwrap();
            if resized.stale().is_empty() {
                assert_eq!(resized.regions().collect::<Vec<_>>(), expected, "{width}");
                compared.push(arrangement(&resized));
            }
            let remeasured = measured.remeasure(size(width), &mut measure).unwrap();
            assert_eq!(
                remeasured.regions().collect::<Vec<_>>(),
                expected,
                "{width}"
            );
            counts.push(arrangement(remeasured));
        }
        // Resizes without stale answers match fresh measures at more than one arrangement.
        compared.sort_unstable();
        compared.dedup();
        assert!(compared.len() >= 2, "{compared:?}");
        counts.sort_unstable();
        counts.dedup();
        assert!(counts.len() >= 3, "{counts:?}");
        // The labels share one key, so each range is asked once.
        assert_eq!(measure.answered(), 2);
    }
}

#[test]
fn a_failed_remeasure_keeps_its_solution() {
    // Each answer holds on one side of 120 px, and w4 gives an invalid answer past it.
    let mut measure = Measurer::heights(|id, width| {
        Some(if width <= 120.0 {
            Measurement::over(20.0, 0.0..=120.0)
        } else if id == "w4" {
            Measurement::from(f32::NAN)
        } else {
            Measurement::over(30.0, 120.0f32.next_up()..=f32::INFINITY)
        })
    });
    let mut measured = L::from(wrap_of(6))
        .measure(&SolveOptions::sized(330.0, 200.0), &mut measure)
        .unwrap();
    let before = measured.solution().clone();
    assert!(matches!(
        measured.remeasure(Size::new(150.0, 200.0), &mut measure),
        Err(LayoutError::InvalidMeasurement { .. })
    ));
    // The measure failed after the leaves before w4 answered at the new width.
    assert_eq!(measure.asked("w0", 'h'), 2);
    assert_eq!(measure.asked("w4", 'h'), 2);
    assert_eq!(
        measured.solution().regions().collect::<Vec<_>>(),
        before.regions().collect::<Vec<_>>()
    );
    assert_eq!(measured.options().width, Some(330.0));
    let again = measured.remeasure(Size::new(330.0, 200.0), &mut measure);
    assert_eq!(
        again.unwrap().regions().collect::<Vec<_>>(),
        before.regions().collect::<Vec<_>>()
    );
}

/// Whether two rectangles share more than an edge.
fn overlapping(a: Rect, b: Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

#[test]
fn contained_collapsing_slabs_count_in_every_column_width() {
    // Each panel contains a y axis that collapses along the rows, and the wider panels need
    // 100 with it, so every count's columns are at least 100.
    let wrap = || -> L {
        Wrap::new((0..4).map(|i| {
            leaf(if i % 2 == 1 { 80.0 } else { 50.0 }, 20.0)
                .id(WRAPPED[i])
                .overflow(Side::Left, Overflow::Contain)
                .slab(
                    Side::Left,
                    Slab::new()
                        .min_width(20.0)
                        .id(Y_LABELS[i])
                        .tag("y-labels")
                        .content_key("y"),
                )
        }))
        .gap(Gap::new(10.0))
        .collapse_tag("y-labels")
        .id("wrap")
        .into()
    };
    let panels = |solution: &Solution<Id>| -> Vec<Rect> {
        WRAPPED[..4].iter().map(|&id| slot(solution, id)).collect()
    };
    for (width, columns) in [(200.0, 1), (250.0, 2), (300.0, 2), (400.0, 3)] {
        let solution = at_width(wrap(), width);
        assert_eq!(wrap_columns(&solution), columns, "{width}");
        let panels = panels(&solution);
        for (i, a) in panels.iter().enumerate() {
            assert!(
                panels[i + 1..].iter().all(|b| !overlapping(*a, *b)),
                "{width}"
            );
        }
    }
    // A resize from more columns takes the count that a fresh solve takes.
    let measured = wrap().measure(&width_only(400.0), &mut ()).unwrap();
    let resized = measured.resize(Size::new(200.0, 300.0)).unwrap();
    assert_eq!(wrap_columns(&resized), 1);
}

#[test]
fn merged_slabs_keep_their_places_at_every_count() {
    const TITLES: [Id; 4] = ["t0", "t1", "t2", "t3"];
    const NOTES: [Id; 4] = ["n0", "n1", "n2", "n3"];
    // Each panel has a title and a note that the Wrap merges, and the third panel stacks them
    // the other way round.
    let wrap = Wrap::new((0..4).map(|i| {
        let title = Slab::new()
            .min_height(5.0)
            .id(TITLES[i])
            .tag("title")
            .content_key("t");
        let note = Slab::new()
            .min_height(7.0)
            .id(NOTES[i])
            .tag("note")
            .content_key("n");
        let panel = leaf(50.0, 20.0).id(WRAPPED[i]);
        if i == 2 {
            panel.slab(Side::Bottom, note).slab(Side::Bottom, title)
        } else {
            panel.slab(Side::Bottom, title).slab(Side::Bottom, note)
        }
    }))
    .min_column(60.0)
    .merge_tag("title")
    .merge_tag("note")
    .id("wrap");
    // The merged slabs go under the last row, across the Wrap, in the first panel's order.
    let title = vec![PathStep::Merged(Side::Bottom, 0)];
    let note = vec![PathStep::Merged(Side::Bottom, 1)];
    for (width, columns, bottom) in [(60.0, 1, 80.0), (130.0, 2, 40.0), (250.0, 4, 20.0)] {
        let solution = at_width(wrap.clone(), width);
        assert_eq!(wrap_columns(&solution), columns, "{width}");
        assert_eq!(path_of(&solution, "t0"), title, "{width}");
        assert_eq!(path_of(&solution, "n0"), note, "{width}");
        assert_eq!(
            slot(&solution, "t0"),
            Rect::new(0.0, bottom, width, 5.0),
            "{width}"
        );
        assert_eq!(
            slot(&solution, "n0"),
            Rect::new(0.0, bottom + 5.0, width, 7.0),
            "{width}"
        );
        for i in 1..4 {
            for (id, merged) in [(TITLES[i], &title), (NOTES[i], &note)] {
                assert_eq!(
                    represented_by(&solution, id).as_ref(),
                    Some(merged),
                    "{width} {id}"
                );
                assert_eq!(slot(&solution, id).height, 0.0, "{width} {id}");
            }
        }
        assert_eq!(solution.size(), Size::new(width, bottom + 12.0));
    }
}

#[test]
fn an_fr_maximum_bounds_its_max_content_share() {
    let row = Row::new([
        leaf(10.0, 10.0).max_content(Size::new(200.0, 10.0)).id("a"),
        leaf(10.0, 10.0).id("b"),
    ])
    .track(0, Track::fr_max(1.0, 50.0))
    .track(1, Track::fr(1.0))
    .id("row");
    // The first column stops at 50, so the unit is 50 and not the 200 it cannot reach.
    assert_eq!(widths(&no_canvas(row), "row"), [50.0, 50.0]);
}

#[test]
fn snapping_keeps_distributed_gaps_inside_the_canvas() {
    let grid = Grid::new(vec![Track::px(30.0); 3], [Track::px(10.0)])
        .child(GridSlot::new(0, 0), leaf(0.0, 0.0).id("a"))
        .child(GridSlot::new(1, 0), leaf(0.0, 0.0).id("b"))
        .child(GridSlot::new(2, 0), leaf(0.0, 0.0).id("c"))
        .distribute(H, Distribute::SpaceBetween)
        .id("grid");
    let options = SolveOptions {
        pixel_ratio: Some(1.0),
        ..SolveOptions::sized(101.0, 10.0)
    };
    let solution = L::from(grid).solve(&options).unwrap();
    // Each gap takes 5 of the 5.5 that it gains, and the last pixel is left over.
    assert_eq!(slot(&solution, "c"), Rect::new(70.0, 0.0, 30.0, 10.0));
    assert!(solution.warnings().is_empty());
}

#[test]
fn a_resize_marks_a_forgotten_max_content_answer_stale() {
    struct Text(f32);
    impl Measure<Id> for Text {
        fn max_content(&mut self, _: &Id, direction: Direction) -> Option<f32> {
            (direction == H).then_some(self.0)
        }
    }
    let layout = L::from(
        Leaf::new()
            .id("text")
            .place(H, Placement::fit(Position::Start)),
    );
    let size = Size::new(300.0, 100.0);
    let mut measured = layout
        .measure(
            &SolveOptions::sized(size.width, size.height),
            &mut Text(100.0),
        )
        .unwrap();
    measured.forget(&"text");
    // The resize can only take the forgotten answer, so it lists the text as stale.
    let resized = measured.resize(size).unwrap();
    assert_eq!(content(&resized, "text").width, 100.0);
    assert_eq!(resized.stale(), ["text"]);
    let remeasured = measured.remeasure(size, &mut Text(200.0)).unwrap();
    assert_eq!(content(remeasured, "text").width, 200.0);
}

#[test]
fn a_resize_without_stale_answers_matches_remeasure() {
    // A y axis that widens as its square plot grows taller.
    fn y_axis(id: Id, height: f32) -> Option<Measurement> {
        (id == "y").then(|| {
            if height <= 200.0 {
                Measurement::over(30.0, 0.0..=200.0)
            } else if height <= 300.0 {
                Measurement::over(40.0, 200.0f32.next_up()..=300.0)
            } else {
                Measurement::over(50.0, 300.0f32.next_up()..=f32::INFINITY)
            }
        })
    }
    let square = || -> L {
        Grid::new([Track::fr(1.0)], [Track::aspect(0, 1.0)])
            .child(
                GridSlot::new(0, 0),
                leaf(10.0, 10.0)
                    .id("plot")
                    .slab(Side::Left, Slab::new().id("y")),
            )
            .into()
    };
    let mut measure = Measurer::widths(y_axis);
    let mut measured = square()
        .measure(&SolveOptions::sized(260.0, 1000.0), &mut measure)
        .unwrap();
    // Widths only grow within a solve, so at 330 the axis first measures 50 at the plot's full
    // height and keeps it. Without that answer, a resize to 330 is stale.
    let at = |width| Size::new(width, 1000.0);
    assert_eq!(measured.resize(at(330.0)).unwrap().stale(), ["y"]);
    measured.remeasure(at(330.0), &mut measure).unwrap();
    let asked = measure.answered();
    for width in [260.0, 330.0] {
        let resized = measured.resize(at(width)).unwrap();
        assert!(resized.stale().is_empty(), "{width}");
        let mut again = measured.clone();
        let remeasured = again.remeasure(at(width), &mut measure).unwrap();
        assert_eq!(
            resized.regions().collect::<Vec<_>>(),
            remeasured.regions().collect::<Vec<_>>(),
            "{width}"
        );
    }
    assert_eq!(measure.answered(), asked);
}

#[test]
fn a_recount_keeps_the_solves_and_raised_widths_that_it_follows() {
    // The panels measure 120 wide, so the first solve's three columns become two.
    let mut measure =
        Measurer::widths(|id, _| id.starts_with('w').then(|| Measurement::always(120.0)));
    let measured = L::from(wrap_of(3))
        .measure(&width_only(300.0), &mut measure)
        .unwrap();
    let solution = measured.solution();
    assert_eq!(wrap_columns(solution), 2);
    assert_eq!(solution.explanation().solves, 2);
    assert_eq!(solution.explanation().raised_widths.len(), 3);
}

#[test]
fn an_aspect_fit_without_fr_columns_caps_nothing() {
    let grid = Grid::new([Track::px(200.0)], [Track::aspect(0, 1.0)])
        .child(GridSlot::new(0, 0), leaf(0.0, 0.0).id("plot"))
        .fit_aspect();
    let solution = sized(grid, 200.0, 100.0);
    assert!(solution.explanation().aspect_fits.is_empty());
    assert_eq!(content(&solution, "plot").height, 200.0);
}

#[test]
fn min_size_is_the_smallest_canvas_that_keeps_every_minimum() {
    let layout: L = auto_row([
        leaf(100.0, 50.0)
            .id("a")
            .slab(Side::Left, Slab::new().id("y")),
        leaf(100.0, 50.0).id("b"),
    ])
    .into();
    let mut measure = Measurer::widths(|id, _| (id == "y").then(|| Measurement::always(30.0)));
    let measured = layout
        .measure(&SolveOptions::default(), &mut measure)
        .unwrap();
    let min = measured.min_size().unwrap();
    assert_eq!(min, Size::new(230.0, 50.0));
    let fits = measured.resize(min).unwrap();
    assert!(
        fits.regions()
            .all(|region| region.content.right() <= min.width)
    );
    assert_eq!(content(&fits, "a").width, 100.0);
    let narrow = measured
        .resize(Size::new(min.width - 1.0, min.height))
        .unwrap();
    assert!(content(&narrow, "b").right() > narrow.size().width);
}

#[test]
fn a_drag_back_over_measured_widths_asks_nothing() {
    let mut measure = Measurer::heights(header_height);
    let mut measured = header_chart(100.0)
        .measure(&SolveOptions::default(), &mut measure)
        .unwrap();
    for width in [200.0, 120.0, 100.0, 180.0] {
        measured = header_chart(width)
            .measure_reusing(&measured, &SolveOptions::default(), &mut measure)
            .unwrap();
    }
    assert_eq!(measure.asked("header", 'h'), 2);
}

#[test]
fn an_always_answer_is_asked_once_until_forgotten() {
    let mut measure = Measurer::widths(|id, _| (id == "legend").then(|| Measurement::always(80.0)));
    let mut measured = legend_chart()
        .measure(&width_only(400.0), &mut measure)
        .unwrap();
    measured.resize(Size::new(600.0, 300.0)).unwrap();
    for width in [300.0, 500.0] {
        measured = legend_chart()
            .measure_reusing(&measured, &width_only(width), &mut measure)
            .unwrap();
    }
    assert_eq!(measure.asked("legend", 'w'), 1);
    measured.forget(&"legend");
    legend_chart()
        .measure_reusing(&measured, &width_only(400.0), &mut measure)
        .unwrap();
    assert_eq!(measure.asked("legend", 'w'), 2);
}

#[test]
fn answers_survive_a_shape_change_and_back() {
    let with_header = || header_chart(100.0);
    let without: L = leaf(10.0, 10.0).id("other").into();
    // The other tree measures one leaf, so it keeps the answers of one absent leaf.
    let mut measure = Measurer::heights(|id, width| match id {
        "other" => Some(Measurement::always(10.0)),
        _ => header_height(id, width),
    });
    let first = with_header()
        .measure(&SolveOptions::default(), &mut measure)
        .unwrap();
    let changed = without
        .measure_reusing(&first, &SolveOptions::default(), &mut measure)
        .unwrap();
    with_header()
        .measure_reusing(&changed, &SolveOptions::default(), &mut measure)
        .unwrap();
    assert_eq!(measure.asked("header", 'h'), 1);
}

#[test]
fn stale_lists_answers_that_no_longer_hold() {
    let layout: L = Grid::new([Track::fr(1.0)], auto(1))
        .child(
            GridSlot::new(0, 0),
            leaf(0.0, 100.0)
                .id("plot")
                .slab(Side::Top, Slab::new().id("header")),
        )
        .into();
    let mut measure = Measurer::heights(header_height);
    let measured = layout.measure(&width_only(100.0), &mut measure).unwrap();
    assert!(measured.solution().stale().is_empty());
    assert!(
        measured
            .resize(Size::new(120.0, 140.0))
            .unwrap()
            .stale()
            .is_empty()
    );
    assert_eq!(
        measured.resize(Size::new(200.0, 140.0)).unwrap().stale(),
        ["header"]
    );
}

// --- cases ported from v1 ---

#[test]
fn spans_respect_matched_columns() {
    let column = Track::auto().tag("c");
    let grid = Grid::new(vec![column; 3], auto(1))
        .match_tag("c")
        .child(GridSlot::new(0, 0).span(2, 1), leaf(100.0, 10.0))
        .child(GridSlot::new(2, 0), leaf(10.0, 10.0))
        .id("grid");
    assert_eq!(widths(&no_canvas(grid), "grid"), [50.0, 50.0, 50.0]);
}

#[test]
fn overlapping_spans_keep_both_directions_matched() {
    let grid = Grid::new(
        vec![Track::auto().tag("c"); 3],
        vec![Track::auto().tag("r"); 3],
    )
    .match_tag("c")
    .match_tag("r")
    .child(GridSlot::new(0, 0).span(2, 2), leaf(100.0, 80.0))
    .child(GridSlot::new(1, 1).span(2, 2), leaf(100.0, 80.0))
    .id("grid");
    let solution = no_canvas(grid);
    assert_eq!(widths(&solution, "grid"), [50.0; 3]);
    assert_eq!(heights(&solution, "grid"), [40.0; 3]);
}

#[test]
fn px_tracks_stay_rigid_and_large_content_overflows() {
    let grid = Grid::new([Track::px(50.0), Track::auto()], auto(1))
        .child(GridSlot::new(0, 0), leaf(80.0, 10.0).id("big"))
        .child(GridSlot::new(1, 0), leaf(20.0, 10.0).id("next"))
        .id("grid");
    let solution = no_canvas(grid);
    assert_eq!(widths(&solution, "grid"), [50.0, 20.0]);
    assert_eq!(content(&solution, "big").width, 80.0);
    assert_eq!(content(&solution, "next").x, 50.0);
}

#[test]
fn a_rows_distribute_places_its_leftover_space() {
    let row = auto_row([
        leaf(10.0, 10.0).id("a"),
        leaf(10.0, 10.0).id("b"),
        leaf(10.0, 10.0).id("c"),
    ])
    .distribute(H, Distribute::SpaceBetween);
    let solution = L::solve(&row.into(), &width_only(100.0)).unwrap();
    assert_eq!(
        ["a", "b", "c"].map(|id| content(&solution, id).x),
        [0.0, 45.0, 90.0]
    );
}

// --- the additions ---

#[test]
fn full_slabs_span_the_corners_and_top_and_bottom_take_them() {
    let full = |width: f32, height: f32| Slab::new().min_size(Size::new(width, height)).full();
    let child = leaf(100.0, 60.0)
        .id("child")
        .slab(Side::Top, Slab::new().min_height(10.0))
        .slab(Side::Right, Slab::new().min_width(20.0))
        .slab(Side::Bottom, Slab::new().min_height(30.0))
        .slab(Side::Left, Slab::new().min_width(40.0));
    let parent = auto_row([child])
        .slab(Side::Top, full(0.0, 5.0).id("top"))
        .slab(Side::Right, full(6.0, 0.0).id("right"))
        .slab(Side::Bottom, full(0.0, 7.0).id("bottom"))
        .slab(Side::Left, full(8.0, 0.0).id("left"));
    let solution = no_canvas(parent);
    assert_eq!(
        content(&solution, "child"),
        Rect::new(48.0, 15.0, 100.0, 60.0)
    );
    assert_eq!(slot(&solution, "top"), Rect::new(0.0, 0.0, 174.0, 5.0));
    assert_eq!(slot(&solution, "bottom"), Rect::new(0.0, 105.0, 174.0, 7.0));
    assert_eq!(slot(&solution, "left"), Rect::new(0.0, 5.0, 8.0, 100.0));
    assert_eq!(slot(&solution, "right"), Rect::new(168.0, 5.0, 6.0, 100.0));
}

#[test]
fn matched_headers_stay_level_and_matched_axes_align_titles() {
    let header = |id: Id| Slab::new().id(id).tag("header");
    let row = auto_row([
        leaf(100.0, 50.0)
            .id("a")
            .slab(Side::Top, header("a-header")),
        leaf(100.0, 50.0)
            .id("b")
            .slab(Side::Top, header("b-header")),
    ])
    .match_tag("header");
    let mut measure = Measurer::heights(|id, _| match id {
        "a-header" => Some(Measurement::always(20.0)),
        "b-header" => Some(Measurement::always(40.0)),
        _ => None,
    });
    let measured = L::from(row)
        .measure(&SolveOptions::default(), &mut measure)
        .unwrap();
    let solution = measured.solution();
    assert_eq!(slot(solution, "a-header").height, 40.0);
    assert_eq!(content(solution, "a").y, 40.0);

    let plot = |id: Id, title: Id, axis: f32| {
        leaf(100.0, 50.0)
            .id(id)
            .slab(Side::Left, Slab::new().min_width(axis).tag("y-axis"))
            .slab(Side::Left, Slab::new().min_width(15.0).id(title))
    };
    let column =
        auto_column([plot("p", "p-title", 30.0), plot("q", "q-title", 50.0)]).match_tag("y-axis");
    let solution = no_canvas(column);
    assert_eq!(slot(&solution, "p-title").x, 0.0);
    assert_eq!(slot(&solution, "q-title").x, 0.0);
    assert_eq!(content(&solution, "p").x, 65.0);
}

#[test]
fn overlays_change_no_sizes_and_ignore_ancestor_matches() {
    let header = |height: f32| Slab::new().min_height(height).tag("header");
    let build = |with_overlay: bool| {
        let mut first = leaf(100.0, 50.0).id("a").slab(Side::Top, header(20.0));
        if with_overlay {
            let inset = leaf(0.0, 0.0)
                .id("inset")
                .slab(Side::Top, header(50.0).id("inset-header"));
            first = first.overlay(Overlay::new(inset).rect(0.5, 0.0, 1.0, 0.5));
        }
        auto_row([
            first,
            leaf(100.0, 50.0).id("b").slab(Side::Top, header(30.0)),
        ])
        .match_tag("header")
    };
    let plain = no_canvas(build(false));
    let overlaid = no_canvas(build(true));
    for id in ["a", "b"] {
        assert_eq!(content(&plain, id), content(&overlaid, id));
    }
    assert_eq!(content(&overlaid, "a"), Rect::new(0.0, 30.0, 100.0, 50.0));
    // The overlay sits in the right half of the top half of a's content, and its own header
    // keeps its 50 px instead of the match.
    assert_eq!(
        slot(&overlaid, "inset-header"),
        Rect::new(50.0, 30.0, 50.0, 50.0)
    );
    let inset = region(&overlaid, "inset");
    assert_eq!(inset.slot, Rect::new(50.0, 30.0, 50.0, 25.0));
    // The overlay comes after its host's subtree in drawing order.
    let order: Vec<Id> = overlaid.regions().filter_map(|r| r.id).collect();
    let position = |id| order.iter().position(|&o| o == id).unwrap();
    assert!(position("inset") > position("a"));
}

fn square_maps(fit: bool) -> Grid<Id> {
    let mut grid = Grid::new(vec![Track::fr(1.0); 2], vec![Track::aspect(0, 1.0); 2])
        .column_gap(Gap::new(20.0))
        .row_gap(Gap::new(20.0))
        .id("maps");
    for (i, id) in ["m0", "m1", "m2", "m3"].into_iter().enumerate() {
        grid = grid.child(GridSlot::new(i % 2, i / 2), leaf(0.0, 0.0).id(id));
    }
    if fit {
        grid = grid.fit_aspect();
    }
    grid
}

#[test]
fn aspect_rows_take_their_height_from_a_column() {
    let measured = L::from(square_maps(false))
        .measure(&width_only(420.0), &mut ())
        .unwrap();
    let solution = measured.solution();
    assert_eq!(heights(solution, "maps"), [200.0, 200.0]);
    assert_eq!(solution.size(), Size::new(420.0, 420.0));
    assert_eq!(
        content(solution, "m3"),
        Rect::new(220.0, 220.0, 200.0, 200.0)
    );
    let resized = measured.resize(Size::new(620.0, 620.0)).unwrap();
    assert_eq!(heights(&resized, "maps"), [300.0, 300.0]);
}

#[test]
fn fit_aspect_caps_fr_columns_to_fit_a_fixed_height() {
    let options = explained(420.0, 320.0);
    let fitted = L::from(square_maps(true)).solve(&options).unwrap();
    assert_eq!(widths(&fitted, "maps"), [150.0, 150.0]);
    assert_eq!(heights(&fitted, "maps"), [150.0, 150.0]);
    let fits = &fitted.explanation().aspect_fits;
    assert_eq!(fits.len(), 1);
    assert_eq!(fits[0].cap, 150.0);
    assert_eq!(fits[0].remaining, 0.0);

    let overflowing = L::from(square_maps(false)).solve(&options).unwrap();
    assert_eq!(heights(&overflowing, "maps"), [200.0, 200.0]);
    assert_eq!(slot(&overflowing, "maps").height, 320.0);
    assert_eq!(content(&overflowing, "m3").bottom(), 420.0);
}

// --- pixel snapping ---

fn snapped(width: f32, ratio: Option<f32>) -> SolveOptions {
    SolveOptions {
        pixel_ratio: ratio,
        ..width_only(width)
    }
}

#[test]
fn snapping_keeps_uniform_columns_equal() {
    let column = Track::fr(1.0).tag("column");
    let mut grid = Grid::new(vec![column; 3], auto(1))
        .match_tag("column")
        .id("grid");
    for column in 0..3 {
        grid = grid.child(GridSlot::new(column, 0), leaf(0.0, 10.0));
    }
    let grid = L::from(grid);
    let at = |ratio| grid.solve(&snapped(101.0, ratio)).unwrap();
    assert_eq!(widths(&at(Some(1.0)), "grid"), [33.0; 3]);
    assert_eq!(widths(&at(Some(2.0)), "grid"), [33.5; 3]);
    let starts: Vec<f32> = tracks(&at(Some(2.0)), "grid")
        .columns
        .iter()
        .map(|t| t.start)
        .collect();
    assert_eq!(starts, [0.0, 33.5, 67.0]);
    let unsnapped = widths(&at(None), "grid");
    assert!(unsnapped.iter().all(|&w| (w - 101.0 / 3.0).abs() < 1e-4));
}

#[test]
fn snapping_keeps_aligned_edges_aligned() {
    let facet =
        |ids: &[Id]| Row::new(ids.iter().map(|&id| leaf(0.0, 20.0).id(id))).gap(Gap::new(7.3));
    let outer = auto_column([facet(&["a0", "a1"]), facet(&["b0", "b1", "b2"])]);
    let solution = L::from(outer).solve(&snapped(250.0, Some(1.0))).unwrap();
    for (a, b) in [("a0", "b0"), ("a1", "b1")] {
        assert_eq!(content(&solution, a).x, content(&solution, b).x);
        assert_eq!(content(&solution, a).right(), content(&solution, b).right());
    }
    assert!(
        solution
            .regions()
            .all(|r| r.content.x.fract() == 0.0 && r.content.width.fract() == 0.0)
    );
}

#[test]
fn snapping_rounds_measured_slab_thickness_up() {
    let chart = || -> L {
        leaf(100.0, 50.0)
            .id("plot")
            .slab(Side::Top, Slab::new().id("header"))
            .into()
    };
    let thickness = |ratio| {
        let mut measure =
            Measurer::heights(|id, _| (id == "header").then(|| Measurement::always(19.2)));
        let options = SolveOptions {
            pixel_ratio: ratio,
            ..SolveOptions::default()
        };
        let measured = chart().measure(&options, &mut measure).unwrap();
        slot(measured.solution(), "header").height
    };
    assert_eq!(thickness(Some(1.0)), 20.0);
    assert_eq!(thickness(Some(2.0)), 19.5);
    assert_eq!(thickness(None), 19.2);
}

#[test]
fn a_wrap_counts_its_columns_at_their_snapped_sizes() {
    let wrap = Wrap::new([leaf(0.0, 10.0).id("a"), leaf(0.0, 10.0).id("b")])
        .min_column(10.2)
        .gap(Gap::new(0.2))
        .id("wrap");
    let options = SolveOptions {
        pixel_ratio: Some(1.0),
        ..SolveOptions::sized(21.0, 20.0)
    };
    let solution = L::from(wrap).solve(&options).unwrap();
    // Two columns would snap to 11 + 1 + 11 = 23 px, past the 21 px canvas.
    assert_eq!(wrap_columns(&solution), 1);
    assert!(content(&solution, "b").right() <= 21.0);
}

// --- reading a solution ---

#[test]
fn the_report_names_what_decided_each_size() {
    let row = auto_row([
        leaf(50.0, 10.0).id("a"),
        leaf(50.0, 10.0)
            .id("b")
            .slab(Side::Left, Slab::new().min_width(20.0)),
        leaf(50.0, 10.0).id("c"),
    ])
    .gap(Gap::new(10.0).clearance(4.0).tag("panel"))
    .match_tag("panel")
    .id("row");
    let solution = L::from(row).solve(&explained_size()).unwrap();
    let report = solution.report();
    assert!(
        report.contains("gap 0: at 50 for 24, the overflow of root / child 0 and root / child 1 plus the clearance of root"),
        "{report}"
    );
    assert!(
        report.contains("gap 1: at 124 for 24, the minimum of root, raised from 10 by match panel"),
        "{report}"
    );

    let tracks = L::from(matched_row()).solve(&explained_size()).unwrap();
    let report = tracks.report();
    assert!(report.contains("base size 10, growth limit 80"), "{report}");

    let merge = no_canvas(ragged_wrap()).report();
    assert!(
        merge.contains("merge x-title (Month) on Bottom: root / Bottom merged slab 0 for"),
        "{merge}"
    );

    let plain = L::from(auto_row([leaf(10.0, 10.0)]))
        .solve(&SolveOptions::default())
        .unwrap();
    assert!(plain.report().contains("were not recorded"));
}

#[test]
fn explaining_changes_no_rectangle() {
    let y_axes = auto_column([
        leaf(100.0, 50.0).slab(Side::Left, Slab::new().min_width(30.0).tag("y-axis")),
        leaf(100.0, 50.0).slab(Side::Left, Slab::new().min_width(50.0).tag("y-axis")),
    ])
    .match_tag("y-axis");
    let gapped =
        |gap: f32| auto_row([leaf(20.0, 10.0), leaf(20.0, 10.0)]).gap(Gap::new(gap).tag("g"));
    let pair = |gap: f32| {
        Wrap::new([leaf(0.0, 10.0), leaf(0.0, 10.0)])
            .min_column(50.0)
            .gap(Gap::new(gap).tag("g"))
    };
    let layouts: [L; 5] = [
        matched_row().into(),
        y_axes.into(),
        auto_column([gapped(10.0), gapped(30.0)])
            .match_tag("g")
            .into(),
        Column::new([pair(10.0), pair(40.0)]).match_tag("g").into(),
        ragged_wrap().into(),
    ];
    let mut parts = Vec::new();
    for (index, layout) in layouts.iter().enumerate() {
        let options = SolveOptions::sized(300.0, 200.0);
        let plain = layout.solve(&options).unwrap();
        let explained = layout
            .solve(&SolveOptions {
                explain: true,
                ..options
            })
            .unwrap();
        assert_eq!(
            plain.regions().collect::<Vec<_>>(),
            explained.regions().collect::<Vec<_>>(),
            "{index}"
        );
        // Only the sizes' explanation depends on explaining.
        let sizes = explained.explanation().sizes.clone().unwrap();
        assert_eq!(
            plain.explanation(),
            &Explanation {
                sizes: None,
                ..explained.explanation().clone()
            },
            "{index}"
        );
        parts.extend(sizes.match_groups.iter().map(|group| group.parts));
    }
    for kind in [
        MatchedParts::Tracks,
        MatchedParts::Gaps,
        MatchedParts::Slabs,
    ] {
        assert!(parts.contains(&kind), "{kind:?} in {parts:?}");
    }
}

#[test]
fn explain_at_finds_the_region_track_and_gap_under_a_point() {
    let grid = Grid::new(vec![Track::px(100.0); 2], auto(1))
        .column_gap(Gap::new(20.0))
        .child(GridSlot::new(0, 0), leaf(0.0, 40.0).id("left"))
        .child(GridSlot::new(1, 0), leaf(0.0, 40.0).id("right"))
        .id("grid");
    let solution = L::from(grid).solve(&explained_size()).unwrap();
    let in_track = solution.explain_at([150.0, 20.0]);
    assert_eq!(
        in_track
            .regions
            .iter()
            .filter_map(|r| r.id)
            .collect::<Vec<_>>(),
        ["grid", "right"]
    );
    assert_eq!(in_track.tracks.len(), 2);
    assert!(
        in_track
            .tracks
            .iter()
            .any(|t| t.owner.direction == H && t.index == 1)
    );
    assert!(in_track.gaps.is_empty());
    let in_gap = solution.explain_at([110.0, 20.0]);
    assert_eq!(in_gap.gaps.len(), 1);
    assert_eq!(in_gap.gaps[0].gap.length, 20.0);
}

#[test]
fn warnings_flag_flush_overhangs_that_reach_another_region() {
    let short = |overhang: f32| -> L {
        auto_column([
            leaf(100.0, 30.0)
                .id("top")
                .slab(Side::Bottom, Slab::new().min_height(overhang).id("axis"))
                .overflow(Side::Bottom, Overflow::Flush),
            leaf(100.0, 30.0).id("bottom"),
        ])
        .gap(Gap::new(10.0))
        .into()
    };
    assert!(no_canvas(short(8.0)).warnings().is_empty());
    let warnings = no_canvas(short(25.0)).warnings();
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, avenger_layout::Warning::Overlap { .. })),
        "{warnings:?}"
    );
}

// --- canvas padding ---

#[test]
fn canvas_padding_is_reserved_around_the_root() {
    let row = || -> L {
        auto_row([leaf(100.0, 50.0).id("a"), leaf(100.0, 50.0).id("b")])
            .slab(Side::Left, Slab::new().min_width(30.0).id("axis"))
            .overflow(Side::Left, Overflow::Propagate)
            .into()
    };
    let padding = avenger_layout::Edges::new(10.0, 5.0, 20.0, 15.0);
    let open = row()
        .solve(&SolveOptions {
            padding,
            ..SolveOptions::default()
        })
        .unwrap();
    // The padding sits outside the root's slab, and the canvas includes it.
    assert_eq!(open.size(), Size::new(260.0, 70.0));
    assert_eq!(slot(&open, "axis").x, 10.0);
    assert_eq!(content(&open, "a"), Rect::new(40.0, 5.0, 100.0, 50.0));

    let options = SolveOptions {
        padding,
        ..SolveOptions::sized(400.0, 100.0)
    };
    let measured = row().measure(&options, &mut ()).unwrap();
    let solution = measured.solution();
    assert_eq!(solution.size(), Size::new(400.0, 100.0));
    assert_eq!(content(solution, "b").right(), 380.0);
    assert_eq!(content(solution, "b").bottom(), 85.0);
    // Resizing and the smallest canvas keep the padding.
    let resized = measured.resize(Size::new(300.0, 100.0)).unwrap();
    assert_eq!(content(&resized, "b").right(), 280.0);
    assert_eq!(measured.min_size().unwrap(), Size::new(260.0, 70.0));
}

#[test]
fn a_root_larger_than_its_canvas_keeps_its_minimum() {
    let root = Leaf::new()
        .min_size(Size::new(100.0, 50.0))
        .padding(Edges::all(20.0))
        .id("root");
    let solution = L::from(root)
        .solve(&SolveOptions::sized(10.0, 10.0))
        .unwrap();
    // The root keeps the canvas as its slot and overflows it from the start.
    assert_eq!(slot(&solution, "root"), Rect::new(0.0, 0.0, 10.0, 10.0));
    assert_eq!(
        content(&solution, "root"),
        Rect::new(20.0, 20.0, 100.0, 50.0)
    );
    assert!(
        solution
            .warnings()
            .iter()
            .any(|warning| matches!(warning, Warning::PastCanvas { .. }))
    );
}

// --- slab stacks ---

/// A member of `width` declared thickness and `length` max-content length.
fn member(id: Id, width: f32, length: f32) -> Slab<Id> {
    Slab::new().id(id).min_width(width).max_content(length)
}

/// Each member's span along the stack, as `[start, end]` from the region's top.
fn spans(solution: &Solution<Id>, ids: &[Id]) -> Vec<[f32; 2]> {
    ids.iter()
        .map(|&id| {
            let slot = slot(solution, id);
            [slot.y, slot.bottom()]
        })
        .collect()
}

#[test]
fn a_stack_is_one_band_as_thick_as_its_thickest_member() {
    let plot = leaf(100.0, 200.0)
        .id("plot")
        .slab(Side::Right, Slab::new().id("axis").min_width(10.0))
        .slab_stack(
            Side::Right,
            SlabStack::new([
                Slab::new()
                    .id("a")
                    .min_width(30.0)
                    .max_length(40.0)
                    .place(Placement::stretch(Position::End)),
                Slab::new().id("b").min_width(50.0),
            ])
            .id("stack"),
        )
        .slab(Side::Right, Slab::new().id("title").min_width(8.0));
    let solution = no_canvas(plot);
    assert_eq!(slot(&solution, "axis"), Rect::new(100.0, 0.0, 10.0, 200.0));
    assert_eq!(slot(&solution, "stack"), Rect::new(110.0, 0.0, 50.0, 200.0));
    assert_eq!(slot(&solution, "title"), Rect::new(160.0, 0.0, 8.0, 200.0));
    // Each member spans the band, and undeclared tracks share the length equally.
    assert_eq!(slot(&solution, "a"), Rect::new(110.0, 0.0, 50.0, 100.0));
    assert_eq!(slot(&solution, "b"), Rect::new(110.0, 100.0, 50.0, 100.0));
    // A member's content takes its maximum and placement in its segment.
    assert_eq!(content(&solution, "a"), Rect::new(110.0, 60.0, 50.0, 40.0));
    assert!(matches!(
        region(&solution, "stack").kind,
        RegionKind::Stack {
            side: Side::Right,
            ..
        }
    ));
    assert!(matches!(
        region(&solution, "a").kind,
        RegionKind::Slab {
            side: Side::Right,
            ..
        }
    ));
    assert_eq!(
        path_of(&solution, "stack"),
        [PathStep::Slab(Side::Right, 1)]
    );
    assert_eq!(
        path_of(&solution, "b"),
        [PathStep::Slab(Side::Right, 1), PathStep::Member(1)]
    );
}

#[test]
fn a_stacks_tracks_divide_its_length_by_the_track_rules() {
    let ids = ["px", "auto", "fit", "one", "two"];
    let plot = leaf(100.0, 310.0).slab_stack(
        Side::Right,
        SlabStack::new([
            member("px", 10.0, 0.0),
            member("auto", 10.0, 40.0),
            member("fit", 10.0, 80.0),
            member("one", 10.0, 0.0),
            member("two", 10.0, 0.0),
        ])
        .track(0, Track::px(50.0))
        .track(1, Track::auto())
        .track(2, Track::fit_content(30.0))
        .track(4, Track::fr(2.0))
        .gap(Gap::new(10.0)),
    );
    let solution = no_canvas(plot);
    // The Auto track grows to its max-content length, the fit-content track to its limit, and
    // the Fr tracks share the remaining 150 by weight.
    assert_eq!(
        spans(&solution, &ids),
        [
            [0.0, 50.0],
            [60.0, 100.0],
            [110.0, 140.0],
            [150.0, 200.0],
            [210.0, 310.0]
        ]
    );
    let capped = leaf(100.0, 200.0).slab_stack(
        Side::Right,
        SlabStack::new([
            member("basis", 10.0, 0.0),
            member("max", 10.0, 0.0),
            member("rest", 10.0, 0.0),
        ])
        .track(0, Track::fr_basis(1.0, 20.0))
        .track(1, Track::fr_max(1.0, 30.0)),
    );
    let solution = no_canvas(capped);
    assert_eq!(
        spans(&solution, &["basis", "max", "rest"]),
        [[0.0, 95.0], [95.0, 125.0], [125.0, 200.0]]
    );
}

#[test]
fn auto_members_stay_together_at_the_start_unless_the_stack_distributes_them() {
    let stack = |distribute: Option<Distribute>| {
        let stack = SlabStack::new([member("a", 10.0, 40.0), member("b", 10.0, 30.0)])
            .tracks(Track::auto())
            .gap(Gap::new(10.0));
        let stack = match distribute {
            Some(distribute) => stack.distribute(distribute),
            None => stack,
        };
        spans(
            &no_canvas(leaf(100.0, 200.0).slab_stack(Side::Right, stack)),
            &["a", "b"],
        )
    };
    assert_eq!(stack(None), [[0.0, 40.0], [50.0, 80.0]]);
    assert_eq!(
        stack(Some(Distribute::End)),
        [[120.0, 160.0], [170.0, 200.0]]
    );
    assert_eq!(
        stack(Some(Distribute::Center)),
        [[60.0, 100.0], [110.0, 140.0]]
    );
}

#[test]
fn an_auto_member_shrinks_on_a_short_host_unless_its_minimum_raises_the_host() {
    let stack = |first: Slab<Id>| {
        SlabStack::new([first, member("b", 10.0, 40.0)])
            .tracks(Track::auto())
            .gap(Gap::new(10.0))
    };
    // Both members want 40 but share the 40 that the gap leaves.
    let short =
        no_canvas(leaf(100.0, 50.0).slab_stack(Side::Right, stack(member("a", 10.0, 40.0))));
    assert_eq!(spans(&short, &["a", "b"]), [[0.0, 20.0], [30.0, 50.0]]);
    // A minimum length keeps the first member at 40 and makes the host taller.
    let raised = no_canvas(
        leaf(100.0, 30.0)
            .id("plot")
            .slab_stack(Side::Right, stack(member("a", 10.0, 40.0).min_height(40.0))),
    );
    assert_eq!(content(&raised, "plot").height, 50.0);
    assert_eq!(spans(&raised, &["a", "b"]), [[0.0, 40.0], [50.0, 50.0]]);
}

/// Members whose labels pass their segments' ends: `a` by 3 and 6, `b` by 5 and 2.
fn overhanging_members(id: Id, _: f32) -> Option<Measurement> {
    match id {
        "a" => Some(Measurement::always(20.0).with_overhang([3.0, 6.0])),
        "b" => Some(Measurement::always(10.0).with_overhang([5.0, 2.0])),
        _ => None,
    }
}

#[test]
fn overhang_between_members_widens_their_gap_and_at_the_ends_follows_the_host() {
    let plot = |policy: Overflow| {
        leaf(100.0, 200.0)
            .id("plot")
            .overflow(Side::Bottom, policy)
            .slab_stack(
                Side::Right,
                SlabStack::new([Slab::new().id("a"), Slab::new().id("b")]).gap(Gap::new(4.0)),
            )
    };
    let page = |policy: Overflow| {
        let column = Column::new([L::from(plot(policy)), leaf(100.0, 10.0).id("below").into()]);
        measured_solution(column, &mut Measurer::widths(overhanging_members))
    };
    let solution = page(Overflow::Propagate);
    // The gap holds a's 6 below it and b's 5 above, more than its minimum of 4.
    assert_eq!(spans(&solution, &["a", "b"]), [[3.0, 97.5], [108.5, 203.0]]);
    // The first member's start overhang and the last one's end overhang pass the stack's ends.
    assert_eq!(content(&solution, "plot").y, 3.0);
    assert_eq!(content(&solution, "below").y, 203.0 + 2.0);
    assert_eq!(slot(&solution, "a").width, 20.0);
    // A host that contains its bottom holds the end overhang inside its box.
    let contained = page(Overflow::Contain);
    assert_eq!(
        content(&contained, "below").y,
        content(&contained, "plot").bottom() + 2.0
    );
    assert_eq!(
        slot(&contained, "b").bottom(),
        content(&contained, "plot").bottom()
    );
}

#[test]
fn a_full_stack_runs_across_the_corners() {
    let plot = leaf(100.0, 200.0)
        .slab(Side::Bottom, Slab::new().min_height(20.0))
        .slab_stack(
            Side::Right,
            SlabStack::new([member("a", 10.0, 0.0), member("b", 10.0, 0.0)])
                .id("stack")
                .full(),
        );
    let solution = no_canvas(plot);
    assert_eq!(slot(&solution, "stack"), Rect::new(100.0, 0.0, 10.0, 220.0));
    assert_eq!(
        spans(&solution, &["a", "b"]),
        [[0.0, 110.0], [110.0, 220.0]]
    );
    assert!(matches!(
        region(&solution, "a").kind,
        RegionKind::Slab {
            extent: avenger_layout::Extent::Full,
            ..
        }
    ));
}

#[test]
fn members_of_stacks_with_one_content_key_share_their_answers() {
    let panel = |id: Id, members: [Id; 2], key: Option<&str>| {
        let stack = SlabStack::new(members.map(|member| Slab::new().id(member)));
        let stack = match key {
            Some(key) => stack.content_key(key),
            None => stack,
        };
        leaf(100.0, 100.0).id(id).slab_stack(Side::Right, stack)
    };
    let widths = |id: Id, _: f32| match id.as_bytes()[0] {
        b'a' => Some(Measurement::always(20.0)),
        b'b' => Some(Measurement::always(30.0)),
        _ => None,
    };
    let row = |key: Option<&'static str>| {
        Row::new([
            panel("p0", ["a0", "b0"], key),
            panel("p1", ["a1", "b1"], key),
        ])
    };
    let mut measure = Measurer::widths(widths);
    let solution = measured_solution(row(Some("legends")), &mut measure);
    // The first stack's members ask, and the second stack's members reuse their answers.
    assert_eq!(measure.answered(), 2);
    assert_eq!(measure.asked("a0", 'w') + measure.asked("b0", 'w'), 2);
    assert_eq!(slot(&solution, "a1").width, 30.0);
    // Without a key, every member asks.
    let mut measure = Measurer::widths(widths);
    measured_solution(row(None), &mut measure);
    assert_eq!(measure.answered(), 4);
}

#[test]
fn an_empty_stack_takes_no_space() {
    let plot = leaf(100.0, 50.0)
        .slab_stack(Side::Right, SlabStack::new([]).id("empty"))
        .slab(Side::Right, Slab::new().id("title").min_width(8.0));
    let solution = no_canvas(plot);
    assert_eq!(slot(&solution, "empty").width, 0.0);
    assert_eq!(slot(&solution, "title").x, 100.0);
    assert_eq!(solution.size().width, 108.0);
}

/// The ids of legend panel `index`: the panel, its legend, its colorbar and its stack.
const LEGEND_PANELS: [[Id; 4]; 3] = [
    ["p0", "a0", "b0", "stack0"],
    ["p1", "a1", "b1", "stack1"],
    ["p2", "a2", "b2", "stack2"],
];

/// Panel `index` with a legend stack on its right: an Auto legend 20 long and a filling
/// colorbar, with tag `legends` and content key `key`.
fn legend_panel(index: usize, key: &'static str) -> Leaf<Id> {
    let [panel, legend, colorbar, stack] = LEGEND_PANELS[index];
    leaf(100.0, 100.0).id(panel).slab_stack(
        Side::Right,
        SlabStack::new([member(legend, 10.0, 20.0), member(colorbar, 15.0, 0.0)])
            .id(stack)
            .track(0, Track::auto())
            .tag("legends")
            .content_key(key),
    )
}

fn stack_represented_by(solution: &Solution<Id>, id: Id) -> Option<Vec<PathStep>> {
    match &region(solution, id).kind {
        RegionKind::Stack { represented_by, .. } => represented_by.clone(),
        kind => panic!("{id} is {kind:?}"),
    }
}

#[test]
fn stacks_along_a_row_collapse_to_the_outermost_one() {
    let row = Row::new((0..3).map(|index| legend_panel(index, "k")))
        .gap(Gap::new(10.0))
        .collapse_tag("legends");
    let solution = no_canvas(row);
    // The rightmost stack is drawn, and the others take no space.
    assert_eq!(stack_represented_by(&solution, "stack2"), None);
    for index in ["stack0", "stack1"] {
        assert_eq!(
            stack_represented_by(&solution, index),
            Some(path_of(&solution, "stack2"))
        );
        assert_eq!(slot(&solution, index).width, 0.0);
    }
    assert_eq!(content(&solution, "p1").x, 110.0);
    // Each member resolves to the drawn stack's member at its position.
    assert_eq!(solution.representative(&"a0").unwrap().id, Some("a2"));
    assert_eq!(solution.representative(&"b1").unwrap().id, Some("b2"));
    assert_eq!(
        solution.representative(&"stack0").unwrap().id,
        Some("stack2")
    );
    assert_eq!(
        represented_by(&solution, "b0"),
        Some(path_of(&solution, "b2"))
    );
}

#[test]
fn stacks_merge_into_one_stack_along_the_whole_container() {
    let column = Column::new([legend_panel(0, "k"), legend_panel(1, "k")])
        .gap(Gap::new(10.0))
        .merge_tag("legends");
    let solution = no_canvas(column);
    // The merged stack takes the first stack's ids and runs along both panels and their gap.
    let merged = region(&solution, "stack0");
    assert_eq!(merged.path, [PathStep::Merged(Side::Right, 0)]);
    assert_eq!(merged.slot, Rect::new(100.0, 0.0, 15.0, 210.0));
    // Its Fr member fills what the Auto legend leaves.
    assert_eq!(
        spans(&solution, &["a0", "b0"]),
        [[0.0, 20.0], [20.0, 210.0]]
    );
    assert_eq!(
        stack_represented_by(&solution, "stack1"),
        Some(merged.path.clone())
    );
    assert_eq!(solution.representative(&"b1").unwrap().id, Some("b0"));
    assert_eq!(slot(&solution, "stack1").width, 0.0);
}

#[test]
fn stacks_in_a_wrap_collapse_beside_each_other_only_when_every_child_shows_one_key() {
    let wrap = |last: &'static str| {
        Wrap::new([
            legend_panel(0, "k"),
            legend_panel(1, "k"),
            legend_panel(2, last),
        ])
        .gap(Gap::new(10.0))
        .collapse_tag("legends")
    };
    let solve = |last| sized(wrap(last), 600.0, 200.0);
    let uniform = solve("k");
    assert_eq!(
        stack_represented_by(&uniform, "stack0"),
        Some(path_of(&uniform, "stack2"))
    );
    let mixed = solve("other");
    for id in ["stack0", "stack1", "stack2"] {
        assert_eq!(stack_represented_by(&mixed, id), None, "{id}");
    }
}

#[test]
fn the_explanation_names_what_set_each_members_length_and_gap() {
    let plot = leaf(100.0, 200.0).id("plot").slab_stack(
        Side::Right,
        SlabStack::new([Slab::new().id("a").min_height(30.0), Slab::new().id("b")])
            .id("stack")
            .track(0, Track::auto())
            .gap(Gap::new(4.0)),
    );
    let solution = L::from(plot)
        .measure(
            &explained_size(),
            &mut Measurer::widths(overhanging_members),
        )
        .unwrap()
        .into_solution();
    let sizes = solution.explanation().sizes.as_ref().unwrap();
    let owner = sizes
        .owners
        .iter()
        .find(|owner| owner.path == path_of(&solution, "stack"))
        .expect("the stack's tracks");
    assert_eq!(owner.direction, V);
    // The legend's minimum sets its Auto track, and the colorbar takes the rest.
    assert_eq!(owner.tracks[0].set_by, Some(path_of(&solution, "a")));
    assert_eq!(
        [owner.tracks[0].length, owner.tracks[1].length],
        [30.0, 200.0 - 30.0 - 11.0]
    );
    // a's 6 below it and b's 5 above it decide the gap.
    assert_eq!(owner.gaps[0].length, 11.0);
    assert_eq!(
        owner.gaps[0].term,
        avenger_layout::GapTerm::Overflow {
            trailing: Some(path_of(&solution, "a")),
            leading: Some(path_of(&solution, "b")),
        }
    );
    // A point in the gap reports it.
    let gap_y = owner.gaps[0].start + 5.0;
    let at = solution.explain_at([content(&solution, "a").x + 5.0, gap_y]);
    assert!(at.gaps.iter().any(|gap| gap.owner.path == owner.path));
}

/// A legend that records the lengths it is asked at and takes 63 pixels with no limit.
struct Legend {
    asked: Vec<f32>,
}

impl Measure<Id> for Legend {
    fn width(&mut self, id: &Id, height: f32) -> Option<Measurement> {
        (*id == "legend").then(|| {
            self.asked.push(height);
            Measurement::from(40.0)
        })
    }

    fn max_content(&mut self, id: &Id, direction: Direction) -> Option<f32> {
        (*id == "legend" && direction == V).then_some(63.0)
    }
}

#[test]
fn a_slab_sized_to_its_max_content_length_gets_exactly_that_length() {
    // The plot stands below a slab of a fractional thickness, so the legend beside it starts at
    // a fractional position, where `start + 63 - start` in `f32` is not 63.
    let layout: L = leaf(100.0, 100.0)
        .id("plot")
        .slab(Side::Top, Slab::new().min_height(47.82918))
        .slab_stack(
            Side::Right,
            SlabStack::new([Slab::new().id("legend")]).track(0, Track::auto()),
        )
        .into();
    let mut legend = Legend { asked: Vec::new() };
    let measured = layout
        .measure(&SolveOptions::sized(300.0, 300.0), &mut legend)
        .unwrap();
    let region = content(measured.solution(), "legend");
    assert_ne!(region.y.fract(), 0.0);
    assert_eq!(region.height, 63.0);
    assert!(!legend.asked.is_empty());
    assert!(
        legend.asked.iter().all(|&height| height == 63.0),
        "{:?}",
        legend.asked
    );
}
