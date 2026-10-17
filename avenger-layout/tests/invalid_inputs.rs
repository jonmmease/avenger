use avenger_layout::{
    Column, Direction, Distribute, Edges, Gap, Grid, GridSlot, Layout, LayoutError, Leaf, Measure,
    Measurement, Overflow, Overlay, Placement, Position, Row, Side, Size, Slab, SolveOptions,
    Track, Wrap,
};

type L = Layout<&'static str>;

fn leaf() -> Leaf<&'static str> {
    Leaf::new().min_size(Size::new(10.0, 10.0))
}

fn solve(layout: impl Into<L>) -> Result<(), LayoutError> {
    layout.into().solve(&SolveOptions::default()).map(|_| ())
}

fn auto(count: usize) -> Vec<Track> {
    vec![Track::auto(); count]
}

#[test]
fn slots_outside_the_grid_are_rejected() {
    for slot in [
        GridSlot::new(usize::MAX, 0).span(2, 1),
        GridSlot::new(1, 0).span(usize::MAX, 1),
        GridSlot::new(0, 0).span(0, 1),
        GridSlot::new(0, 2),
    ] {
        let grid = Grid::new(auto(2), auto(2)).child(slot, leaf());
        assert!(
            matches!(solve(grid), Err(LayoutError::SlotOutOfBounds { .. })),
            "{slot:?}"
        );
    }
}

#[test]
fn non_finite_inputs_are_rejected() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let in_grid = |child: Layout<&'static str>| Column::new([child]);
        let cases: Vec<L> = vec![
            Leaf::new().min_size(Size::new(bad, 10.0)).into(),
            leaf().slab(Side::Top, Slab::new().min_height(bad)).into(),
            Grid::new([Track::px(bad)], auto(1)).into(),
            Grid::new([Track::fr(bad)], auto(1)).into(),
            Grid::new([Track::fr_max(1.0, bad)], auto(1)).into(),
            Grid::new([Track::fr_basis(1.0, bad)], auto(1)).into(),
            Column::new([leaf()]).gap(Gap::new(bad)).into(),
            Column::new([leaf()])
                .gap(Gap::new(0.0).clearance(bad))
                .into(),
            Grid::new(auto(1), auto(1)).column_gap(Gap::new(bad)).into(),
            Row::new([leaf()]).track(0, Track::px(bad)).into(),
            Row::new([leaf()]).tracks(Track::fr(bad)).into(),
            leaf()
                .overlay(Overlay::new(leaf()).rect(0.0, 0.0, bad, 1.0))
                .into(),
            Grid::new(auto(1), [Track::aspect(0, bad)]).into(),
        ];
        for case in cases {
            assert!(
                matches!(
                    solve(in_grid(case.clone())),
                    Err(LayoutError::NonFiniteInput { .. })
                ),
                "{bad}: {case:?}"
            );
        }
        let canvas = SolveOptions {
            width: Some(bad),
            ..SolveOptions::default()
        };
        assert!(matches!(
            L::from(leaf()).solve(&canvas),
            Err(LayoutError::NonFiniteInput { .. })
        ));
        let padding = SolveOptions {
            padding: avenger_layout::Edges::new(bad, 0.0, 0.0, 0.0),
            ..SolveOptions::default()
        };
        assert!(matches!(
            L::from(leaf()).solve(&padding),
            Err(LayoutError::NonFiniteInput { .. })
        ));
        let ratio = SolveOptions {
            pixel_ratio: Some(bad),
            ..SolveOptions::default()
        };
        assert!(matches!(
            L::from(leaf()).solve(&ratio),
            Err(LayoutError::NonFiniteInput { .. })
        ));
    }
}

#[test]
fn out_of_range_values_are_rejected() {
    let cases: Vec<L> = vec![
        Leaf::new().min_size(Size::new(-1.0, 10.0)).into(),
        leaf().aspect(0.0).into(),
        Grid::new([Track::px(-5.0)], auto(1)).into(),
        Grid::new([Track::fr(-1.0)], auto(1)).into(),
        Row::new([leaf().span(0)]).into(),
        leaf().padding(Edges::new(0.0, -1.0, 0.0, 0.0)).into(),
        Wrap::new([leaf()]).max_columns(0).into(),
        Wrap::new([leaf()]).min_column(-1.0).into(),
    ];
    for case in cases {
        assert!(
            matches!(solve(case.clone()), Err(LayoutError::InvalidValue { .. })),
            "{case:?}"
        );
    }
    for options in [
        SolveOptions {
            pixel_ratio: Some(0.0),
            ..SolveOptions::default()
        },
        SolveOptions {
            padding: avenger_layout::Edges::new(0.0, -1.0, 0.0, 0.0),
            ..SolveOptions::default()
        },
        SolveOptions {
            height: Some(-10.0),
            ..SolveOptions::default()
        },
    ] {
        assert!(matches!(
            L::from(leaf()).solve(&options),
            Err(LayoutError::InvalidValue { .. })
        ));
    }
}

#[test]
fn a_maximum_below_the_minimum_is_rejected() {
    let cases: Vec<L> = vec![
        leaf().max_width(5.0).into(),
        Row::new([leaf()]).min_width(50.0).max_width(20.0).into(),
        leaf()
            .slab(
                Side::Left,
                Slab::new().min_size(Size::new(10.0, 20.0)).max_length(5.0),
            )
            .into(),
    ];
    for case in cases {
        assert!(matches!(solve(case), Err(LayoutError::MaxBelowMin { .. })));
    }
}

#[test]
fn duplicate_ids_are_rejected() {
    let leaves = Row::new([leaf().id("a"), leaf().id("a")]);
    assert!(matches!(
        solve(leaves),
        Err(LayoutError::DuplicateId { .. })
    ));
    let slab = leaf()
        .id("a")
        .slab(Side::Bottom, Slab::new().min_height(10.0).id("a"));
    assert!(matches!(solve(slab), Err(LayoutError::DuplicateId { .. })));
}

#[test]
fn a_track_past_the_sequence_is_rejected() {
    let row = Row::new([leaf(), leaf().span(2)]).track(3, Track::auto());
    assert!(matches!(
        solve(row),
        Err(LayoutError::TrackOutOfRange { track: 3, .. })
    ));
    let last = Row::new([leaf(), leaf().span(2)]).track(2, Track::auto());
    assert!(solve(last).is_ok());
}

#[test]
fn a_span_outside_a_sequence_is_rejected() {
    let cases: Vec<L> = vec![
        leaf().span(2).into(),
        Grid::new(auto(2), auto(1))
            .child(GridSlot::new(0, 0), leaf().span(2))
            .into(),
        leaf().overlay(Overlay::new(leaf().span(1))).into(),
        Wrap::new([leaf().span(2)]).into(),
    ];
    for case in cases {
        assert!(
            matches!(
                solve(case.clone()),
                Err(LayoutError::SpanOutsideSequence { .. })
            ),
            "{case:?}"
        );
    }
}

#[test]
fn sequences_in_one_table_must_declare_tracks_alike() {
    let pair = || Row::new([leaf(), leaf()]);
    let facet = |first: Track, second: Track| {
        Column::new([pair().track(0, first), pair().track(0, second)])
    };
    assert!(matches!(
        solve(facet(Track::fr(2.0), Track::fr(1.0))),
        Err(LayoutError::SharedTracksDiffer {
            direction: Direction::Horizontal,
            track: 0,
            ..
        })
    ));
    assert!(matches!(
        solve(facet(Track::auto().tag("a"), Track::auto().tag("b"))),
        Err(LayoutError::SharedTracksDiffer { .. })
    ));
    // Equal declarations agree, and separate tables may differ.
    assert!(solve(facet(Track::fr(2.0), Track::fr(2.0))).is_ok());
    assert!(solve(facet(Track::fr(2.0), Track::fr(1.0)).share(false)).is_ok());
}

#[test]
fn a_sequence_in_its_parents_table_cannot_size_the_table() {
    let joined = |row: Row<&'static str>| Column::new([row, Row::new([leaf()])]);
    let row = || Row::new([leaf()]);
    let cases = [
        (
            row().distribute(Direction::Vertical, Distribute::Center),
            "distribute",
        ),
        (row().overflow(Side::Top, Overflow::Contain), "Contain"),
        (row().padding(Edges::all(4.0)), "padding"),
        (row().max_width(100.0), "a maximum"),
        (
            row().place(Direction::Horizontal, Placement::fit(Position::End)),
            "a placement",
        ),
    ];
    for (row, setting) in cases {
        assert!(
            matches!(
                solve(joined(row.clone())),
                Err(LayoutError::SharedSequenceSetting { setting: found, .. }) if found == setting
            ),
            "{setting}"
        );
        // A sequence with its own table can set it.
        assert!(solve(joined(row).share(false)).is_ok(), "{setting}");
    }
}

#[test]
fn slabs_that_stand_in_for_each_other_must_be_declared_alike() {
    let labels = |height: f32| {
        Slab::new()
            .min_height(height)
            .tag("x-labels")
            .content_key("x")
    };
    let column = |second: Slab<&'static str>| {
        Column::new([
            leaf().slab(Side::Bottom, labels(10.0)),
            leaf().slab(Side::Bottom, second),
        ])
        .collapse_tag("x-labels")
    };
    for different in [labels(12.0), labels(10.0).max_content(80.0)] {
        assert!(matches!(
            solve(column(different)),
            Err(LayoutError::RepresentativesDiffer { .. })
        ));
    }
    // Another key, another tag or no rule leaves the slabs independent.
    assert!(solve(column(labels(12.0).content_key("y"))).is_ok());
    assert!(solve(column(labels(12.0).tag("other"))).is_ok());
    assert!(
        solve(Column::new([
            leaf().slab(Side::Bottom, labels(10.0)),
            leaf().slab(Side::Bottom, labels(12.0)),
        ]))
        .is_ok()
    );
}

#[test]
fn a_host_with_two_slabs_of_a_collapsed_tag_on_one_side_is_rejected() {
    let labels = || Slab::new().tag("x-labels").content_key("x");
    let doubled = leaf()
        .slab(Side::Bottom, labels())
        .slab(Side::Bottom, labels());
    assert!(matches!(
        solve(Column::new([doubled.clone()]).merge_tag("x-labels")),
        Err(LayoutError::RepeatedTag {
            side: Side::Bottom,
            ..
        })
    ));
    assert!(solve(Column::new([doubled])).is_ok());
}

#[test]
fn invalid_aspect_tracks_are_rejected() {
    let cases: Vec<L> = vec![
        Grid::new([Track::aspect(0, 1.0)], auto(1)).into(),
        Grid::new(auto(1), [Track::aspect(3, 1.0)]).into(),
        Grid::new(auto(1), [Track::aspect(0, 0.0)]).into(),
        Column::new([leaf()]).tracks(Track::aspect(0, 1.0)).into(),
        Wrap::new([leaf()]).rows(Track::aspect(0, 1.0)).into(),
    ];
    for case in cases {
        assert!(
            matches!(solve(case.clone()), Err(LayoutError::InvalidAspect { .. })),
            "{case:?}"
        );
    }
}

#[test]
fn matched_tracks_must_be_declared_alike() {
    let grid = Grid::new(
        [Track::fr(1.0).tag("panel"), Track::fr(2.0).tag("panel")],
        auto(1),
    )
    .match_tag("panel");
    assert!(matches!(
        solve(grid),
        Err(LayoutError::MatchedTracksDiffer { .. })
    ));
    let unmatched = Grid::new(
        [Track::fr(1.0).tag("panel"), Track::fr(2.0).tag("panel")],
        auto(1),
    );
    assert!(solve(unmatched).is_ok());
}

#[test]
fn overlay_fractions_must_form_a_rectangle_in_the_frame() {
    for rect in [
        [0.5, 0.0, 0.5, 1.0],
        [0.0, 0.8, 1.0, 0.2],
        [-0.1, 0.0, 1.0, 1.0],
        [0.0, 0.0, 1.0, 1.5],
    ] {
        let [left, top, right, bottom] = rect;
        let layout = leaf().overlay(Overlay::new(leaf()).rect(left, top, right, bottom));
        assert!(
            matches!(solve(layout), Err(LayoutError::InvalidOverlayRect { .. })),
            "{rect:?}"
        );
    }
}

struct Answers(Measurement);

impl Measure<&'static str> for Answers {
    fn height(&mut self, _: &&'static str, _: f32) -> Option<Measurement> {
        Some(self.0.clone())
    }
}

#[test]
fn spans_past_a_tables_tracks_are_rejected() {
    for row in [
        Row::new([leaf().span(usize::MAX), leaf()]),
        Row::new([leaf().span(1 << 21)]),
    ] {
        assert!(
            matches!(
                solve(row),
                Err(LayoutError::InvalidValue {
                    field: "track count",
                    ..
                })
            ),
            "spans that add up past the tracks a table holds are an error, not a panic"
        );
    }
}

#[test]
fn invalid_measurements_are_rejected() {
    let layout = L::from(Leaf::new().min_size(Size::new(100.0, 0.0)).id("text"));
    let options = SolveOptions::default();
    for measurement in [
        Measurement::from(f32::NAN),
        Measurement::from(-1.0),
        Measurement::over(10.0, 50.0..=20.0),
        // Only a slab has an overhang.
        Measurement::from(10.0).with_overhang([0.0, 4.0]),
    ] {
        assert!(matches!(
            layout.measure(&options, &mut Answers(measurement)),
            Err(LayoutError::InvalidMeasurement { .. })
        ));
    }
    let slab = L::from(leaf().slab(Side::Bottom, Slab::new().id("labels")));
    for overhang in [[-1.0, 0.0], [0.0, f32::INFINITY]] {
        let measurement = Measurement::from(10.0).with_overhang(overhang);
        assert!(matches!(
            slab.measure(&options, &mut Answers(measurement)),
            Err(LayoutError::InvalidMeasurement { .. })
        ));
    }
    assert!(matches!(
        layout.measure(&options, &mut Answers(Measurement::over(10.0, 0.0..=50.0))),
        Err(LayoutError::MeasurementOutsideRange { value: 100.0, .. })
    ));
}

#[test]
fn arithmetic_overflow_is_reported() {
    let layout = Row::new([
        Leaf::<&'static str>::new().min_size(Size::new(f32::MAX, 10.0)),
        Leaf::new().min_size(Size::new(f32::MAX, 10.0)),
    ]);
    assert!(matches!(
        solve(layout),
        Err(LayoutError::CoordinateOverflow)
    ));
}
