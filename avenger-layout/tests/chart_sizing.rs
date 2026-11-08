//! The sizing that charts lean on: a fixed length fixes its track across a table, the solve's
//! padding surrounds the content, and a solve without a given size takes the content's natural
//! size with its padding.

use avenger_layout::{Column, Edges, Layout, Leaf, Row, Size, SolveOptions, Track};

type Id = &'static str;

fn padded(padding: f32) -> Edges {
    Edges {
        top: padding,
        right: padding,
        bottom: padding,
        left: padding,
    }
}

/// Two rows of a column share one table, so a track that one row fixes at 120 is 120 in both, and
/// the other track takes the rest.
#[test]
fn a_fixed_track_has_its_length_in_every_row_of_its_table() {
    let row = |left: Id, right: Id| {
        Row::new([Leaf::new().id(left), Leaf::new().id(right)]).track(0, Track::px(120.0))
    };
    let layout: Layout<Id> = Column::new([row("a", "b"), row("c", "d")]).into();
    let solution = layout.solve(&SolveOptions::sized(600.0, 400.0)).unwrap();
    for id in ["a", "c"] {
        let width = solution.region(&id).unwrap().content.width;
        assert!((width - 120.0).abs() < 0.5, "{id}: {width}");
    }
    for id in ["b", "d"] {
        let width = solution.region(&id).unwrap().content.width;
        assert!(width > 400.0, "{id}: {width}");
    }
}

/// The padding of a solve of a given size moves the content in, and the size stays the given one.
#[test]
fn the_padding_surrounds_the_content_of_a_given_size() {
    let layout: Layout<Id> = Leaf::new().id("plot").into();
    let options = SolveOptions {
        padding: padded(30.0),
        ..SolveOptions::sized(400.0, 300.0)
    };
    let solution = layout.solve(&options).unwrap();
    let content = solution.region(&"plot").unwrap().content;
    assert_eq!([content.x, content.y], [30.0, 30.0]);
    assert_eq!([content.width, content.height], [340.0, 240.0]);
    assert_eq!(solution.size(), Size::new(400.0, 300.0));
}

/// A solve without a size takes the content's max-content size with its padding, and a given width
/// stretches the content in its dimension only.
#[test]
fn a_solve_without_a_size_takes_the_natural_size_with_its_padding() {
    let layout = || -> Layout<Id> {
        Leaf::new()
            .id("plot")
            .max_content(Size::new(200.0, 150.0))
            .into()
    };
    let natural = layout()
        .solve(&SolveOptions {
            padding: padded(10.0),
            ..SolveOptions::default()
        })
        .unwrap();
    assert_eq!(natural.size(), Size::new(220.0, 170.0));
    let wide = layout()
        .solve(&SolveOptions {
            width: Some(500.0),
            padding: padded(10.0),
            ..SolveOptions::default()
        })
        .unwrap();
    assert_eq!(wide.size(), Size::new(500.0, 170.0));
    assert_eq!(wide.region(&"plot").unwrap().content.width, 480.0);
}
