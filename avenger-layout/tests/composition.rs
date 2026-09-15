use avenger_layout::{
    ChromeLayer, EdgeDemand, Layout, LayoutError, LayoutSolution, Rect, RegionDetail, Side, Size,
    SolveOptions,
};

type L = Layout<&'static str, &'static str>;
fn leaf(w: f32) -> L {
    L::leaf(Size::new(w, 10.0))
}
fn widths(s: &LayoutSolution<&str>, id: &str) -> Vec<f32> {
    match &s.region(&id).unwrap().detail {
        RegionDetail::Grid { tracks } => tracks.column_sizes.clone(),
        _ => panic!("grid expected"),
    }
}

#[test]
fn span_growth_is_retained_when_stretching() {
    let grid = L::grid(1, 2).cell_span(0, 0, 1, 2, leaf(100.0)).id("g");
    let s = grid
        .solve(&SolveOptions {
            width: Some(200.0),
            height: None,
        })
        .unwrap();
    assert_eq!(widths(&s, "g"), vec![100.0, 100.0]);
}

#[test]
fn spans_respect_uniform_columns() {
    let grid = L::grid(1, 3)
        .cell_span(0, 0, 1, 2, leaf(100.0))
        .cell(0, 2, leaf(10.0))
        .uniform_columns()
        .id("g");
    let s = grid.solve(&SolveOptions::default()).unwrap();
    assert_eq!(widths(&s, "g"), vec![50.0, 50.0, 50.0]);
}

#[test]
fn shared_tracks_include_span_requirements() {
    let a = L::row([leaf(10.0), leaf(10.0)]).share("k").id("a");
    let b = L::grid(1, 2)
        .cell_span(0, 0, 1, 2, leaf(100.0))
        .share("k")
        .id("b");
    let s = L::column([a, b]).solve(&SolveOptions::default()).unwrap();
    assert_eq!(widths(&s, "a"), widths(&s, "b"));
    assert_eq!(widths(&s, "a"), [50.0; 2]);
}

#[test]
fn ragged_uniform_tracks_stay_shared_after_allocation() {
    let a = L::row([leaf(100.0), leaf(100.0)])
        .uniform_columns()
        .share("k")
        .id("a");
    let b = L::row([leaf(100.0), leaf(100.0), leaf(100.0)])
        .uniform_columns()
        .share("k")
        .id("b");
    let s = L::column([a, b])
        .solve(&SolveOptions {
            width: Some(600.0),
            height: None,
        })
        .unwrap();
    assert_eq!(widths(&s, "a"), [200.0; 2]);
    assert_eq!(widths(&s, "b"), [200.0; 3]);
}

#[test]
fn nested_share_constraints_propagate() {
    let inner = L::row([leaf(10.0)]).share("inner");
    let a = L::row([inner]).share("outer").id("a");
    let b = L::row([leaf(20.0)]).share("outer").id("b");
    let c = L::row([leaf(100.0)]).share("inner");
    let root = L::column([a, b, c]);
    let s = root.solve(&SolveOptions::default()).unwrap();
    assert_eq!(widths(&s, "a"), [100.0]);
    assert_eq!(widths(&s, "a"), widths(&s, "b"));
}

#[test]
fn parent_guide_stacks_outside_child_demand() {
    let mut child = L::leaf(Size::new(100.0, 60.0)).id("child");
    for (side, guide) in [
        (Side::Top, 10.0),
        (Side::Right, 20.0),
        (Side::Bottom, 30.0),
        (Side::Left, 40.0),
    ] {
        child = child.demand(side, EdgeDemand { guide, legend: 0.0 });
    }
    let solved = L::row([child])
        .guide(Side::Top, 5.0)
        .guide(Side::Right, 6.0)
        .guide(Side::Bottom, 7.0)
        .guide(Side::Left, 8.0)
        .id("parent")
        .solve(&SolveOptions::default())
        .unwrap();
    assert_eq!(
        solved.region(&"child").unwrap().content,
        Rect::new(48.0, 15.0, 100.0, 60.0)
    );
    let parent = solved.region(&"parent").unwrap();
    // Top and bottom own the corners; all four guides sit outside child overflow.
    for (side, expected) in [
        (Side::Top, Rect::new(0.0, 0.0, 174.0, 5.0)),
        (Side::Bottom, Rect::new(0.0, 105.0, 174.0, 7.0)),
        (Side::Left, Rect::new(0.0, 5.0, 8.0, 100.0)),
        (Side::Right, Rect::new(168.0, 5.0, 6.0, 100.0)),
    ] {
        let guide = parent
            .slabs
            .iter()
            .find(|slab| slab.side == side && slab.layer == ChromeLayer::Guide)
            .unwrap();
        assert_eq!(guide.rect, expected, "{side:?}");
    }
}

#[test]
fn uniform_share_coordinates_interior_edge_gaps() {
    let row = |gap| {
        L::row([
            leaf(100.0),
            leaf(100.0).demand(
                Side::Left,
                EdgeDemand {
                    guide: gap,
                    legend: 0.0,
                },
            ),
        ])
        .uniform_columns()
        .share("k")
    };
    let s = L::column([row(20.0).id("a"), row(40.0).id("b")])
        .solve(&SolveOptions::default())
        .unwrap();
    let starts = |id| match &s.region(&id).unwrap().detail {
        RegionDetail::Grid { tracks } => tracks.column_starts.clone(),
        _ => panic!(),
    };
    assert_eq!(starts("a"), [0.0, 140.0]);
    assert_eq!(starts("a"), starts("b"));
}

#[test]
fn overlapping_spans_keep_both_axes_uniform() {
    let solved = L::grid(3, 3)
        .cell_span(0, 0, 2, 2, L::leaf(Size::new(100.0, 80.0)))
        .cell_span(1, 1, 2, 2, L::leaf(Size::new(100.0, 80.0)))
        .uniform_columns()
        .uniform_rows()
        .id("g")
        .solve(&SolveOptions::default())
        .unwrap();
    let RegionDetail::Grid { tracks } = &solved.region(&"g").unwrap().detail else {
        panic!("grid expected")
    };
    assert_eq!(tracks.column_sizes, [50.0; 3]);
    assert_eq!(tracks.row_sizes, [40.0; 3]);
}

#[test]
fn nested_shared_grids_allocate_from_final_parent_slots() {
    let inner = || L::row([leaf(10.0)]).share("inner");
    let outer = |child| L::row([child]).share("outer");
    let root = L::column([
        outer(inner().id("a")).id("parent-a"),
        outer(leaf(20.0)).id("parent-b"),
        inner().id("b"),
    ]);
    for width in [None, Some(200.0)] {
        let solved = root
            .solve(&SolveOptions {
                width,
                height: None,
            })
            .unwrap();
        let expected = width.unwrap_or(20.0);
        for id in ["a", "b", "parent-a", "parent-b"] {
            assert_eq!(widths(&solved, id), [expected], "{id} with width {width:?}");
        }
    }
}

#[test]
fn circular_sharing_dependencies_return_an_error() {
    for root in [
        L::row([L::row([leaf(10.0)]).share("a")]).share("a"),
        L::column([
            L::row([L::row([leaf(10.0)]).share("b")]).share("a"),
            L::row([L::row([leaf(10.0)]).share("a")]).share("b"),
        ]),
    ] {
        assert!(matches!(
            root.solve(&SolveOptions::default()),
            Err(LayoutError::CyclicSharing)
        ));
    }
}
