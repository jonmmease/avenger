use avenger_layout::{
    EdgeDemand, Edges, GridError, GridItem, GridRequirements, GridShape, GridSlot, Layout,
    LayoutError, Side, Size, SolveOptions, SolvedTracks, Spacing, TrackSize,
};

#[test]
fn overflowing_slots_return_errors_instead_of_panicking() {
    for slot in [
        GridSlot {
            row: usize::MAX,
            column: 0,
            row_span: 2,
            column_span: 1,
        },
        GridSlot {
            row: 0,
            column: 1,
            row_span: 1,
            column_span: usize::MAX,
        },
    ] {
        let layout: Layout = Layout::grid(2, 2).cell_span(
            slot.row,
            slot.column,
            slot.row_span,
            slot.column_span,
            Layout::leaf(Size::new(10.0, 10.0)),
        );
        assert!(matches!(
            layout.solve(&SolveOptions::default()),
            Err(LayoutError::SlotOutOfBounds { .. })
        ));
        let item = GridItem {
            id: 0,
            slot,
            content_size: Size::new(10.0, 10.0),
            guide_edges: Edges::default(),
            legend_edges: Edges::default(),
            total_edges: Edges::default(),
        };
        assert!(matches!(
            GridRequirements::from_items(
                GridShape {
                    rows: 2,
                    columns: 2
                },
                Size::default(),
                &[item]
            ),
            Err(GridError::SlotOutOfBounds { .. })
        ));
        let tracks = SolvedTracks {
            column_starts: vec![0.0, 10.0],
            column_sizes: vec![10.0; 2],
            row_starts: vec![0.0, 10.0],
            row_sizes: vec![10.0; 2],
            ..Default::default()
        };
        assert_eq!(tracks.content_rect_for_slot(slot), None);
    }
}

#[test]
fn solve_rejects_non_finite_measurements_and_constraints() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let leaf = || Layout::<usize>::leaf(Size::new(10.0, 10.0));
        for layout in [
            Layout::leaf(Size::new(bad, 10.0)),
            leaf().margin(bad),
            leaf().strip(Side::Top, bad),
            leaf().guide(Side::Left, bad),
            leaf().legend(Side::Right, bad),
            leaf().demand(
                Side::Bottom,
                EdgeDemand {
                    guide: bad,
                    legend: 0.0,
                },
            ),
            leaf().content_min(Size::new(0.0, bad)),
            Layout::row([leaf()]).base_cell_size(Size::new(bad, 0.0)),
            Layout::row([leaf()]).min_gap(bad),
            Layout::row([leaf()]).row_spacing(Spacing {
                outer_end: bad,
                ..Default::default()
            }),
            Layout::row([leaf()]).columns([TrackSize::Fixed(bad)]),
            Layout::row([leaf()]).rows([TrackSize::Flex(bad)]),
        ] {
            let nested = Layout::column([layout]);
            assert!(matches!(
                nested.solve(&SolveOptions::default()),
                Err(LayoutError::NonFiniteInput { .. })
            ));
        }
        assert!(matches!(
            leaf().solve(&SolveOptions {
                width: Some(bad),
                height: None
            }),
            Err(LayoutError::NonFiniteInput { .. })
        ));
    }
}

#[test]
fn arithmetic_overflow_is_reported() {
    let layout: Layout = Layout::row([
        Layout::leaf(Size::new(f32::MAX, 10.0)),
        Layout::leaf(Size::new(f32::MAX, 10.0)),
    ]);
    assert!(matches!(
        layout.solve(&SolveOptions::default()),
        Err(LayoutError::CoordinateOverflow)
    ));
}
