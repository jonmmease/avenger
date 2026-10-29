//! The rules of avenger-layout that the lowering of rows and columns depends on: the match of the
//! lengths of the plots of a shared position scale, and the declarations of the tracks of a table
//! that its rows share.

use avenger_layout::{Column, Layout, LayoutError, Leaf, Row, Size, SolveOptions, Track};

fn plot(id: &'static str, min_width: f32) -> Leaf<&'static str> {
    Leaf::new().id(id).min_size(Size::new(min_width, 10.0))
}

fn widths(
    layout: impl Into<Layout<&'static str>>,
    ids: &[&'static str],
) -> Result<Vec<f32>, LayoutError> {
    let solution = layout.into().solve(&SolveOptions::sized(400.0, 200.0))?;
    Ok(ids
        .iter()
        .map(|id| solution.region(id).unwrap().content.width)
        .collect())
}

/// A row of two plots whose tracks have the tag `x`.
fn tagged() -> Row<&'static str> {
    Row::new([plot("a", 150.0), plot("b", 10.0)]).tracks(Track::fr(1.0).tag("x"))
}

#[test]
fn a_match_makes_the_tagged_tracks_that_its_row_spans_one_width() {
    let third = || Row::new([plot("c", 10.0), plot("d", 10.0), plot("e", 10.0)]);
    let ids = ["a", "b", "c", "d", "e"];
    // Without the match, the first track holds the wide plot, and the others share the rest.
    let unmatched = Column::new([Layout::from(tagged()), third().into()]);
    assert_eq!(
        widths(unmatched, &ids).unwrap(),
        [150.0, 125.0, 150.0, 125.0, 125.0]
    );
    // The first row's match gives its two tracks one width. The rows share their columns, so the
    // second row's plots in those tracks take it too, and the third track, which the first row
    // does not span, takes the rest.
    let matched = Column::new([Layout::from(tagged().match_tag("x")), third().into()]);
    assert_eq!(
        widths(matched, &ids).unwrap(),
        [150.0, 150.0, 150.0, 150.0, 100.0]
    );
}

#[test]
fn the_rows_of_a_table_declare_each_track_alike() {
    let second = || Row::new([plot("c", 10.0), plot("d", 10.0)]);
    let ids = ["a", "b", "c", "d"];
    // Equal sizes and tags declare one track of the shared table.
    let alike = Column::new([
        Layout::from(tagged().match_tag("x")),
        second().tracks(Track::fr(1.0).tag("x")).into(),
    ]);
    assert_eq!(widths(alike, &ids).unwrap(), [200.0; 4]);
    // The same size without the tag is another declaration.
    let untagged = Column::new([
        Layout::from(tagged().match_tag("x")),
        second().tracks(Track::fr(1.0)).into(),
    ]);
    assert!(matches!(
        widths(untagged, &ids),
        Err(LayoutError::SharedTracksDiffer { .. })
    ));
}

#[test]
fn a_match_takes_tracks_of_one_size() {
    let weighted = Row::new([plot("a", 10.0), plot("b", 10.0)])
        .track(0, Track::fr(1.0).tag("x"))
        .track(1, Track::fr(2.0).tag("x"))
        .match_tag("x");
    assert!(matches!(
        widths(weighted, &["a", "b"]),
        Err(LayoutError::MatchedTracksDiffer { .. })
    ));
}
