use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geo::projections::{EqualEarth, Equirectangular};
use avenger_geo::{Graticule, Projection, Projector};
use avenger_guides_geo::{make_graticule_marks, GraticuleConfig, MapSide};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark, text::SceneTextMark};
use avenger_typst_label::LabelEngine;
use std::sync::Arc;

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// Equirectangular at 4 pixels a degree, centered on (100, 100), clipped to
/// longitudes -25 to 25 and latitudes -15 to 15.
fn framed() -> Projector {
    Projection::new(Equirectangular)
        .with_view(4.0 * 180.0 / std::f64::consts::PI, [100.0, 100.0])
        .with_clip_extent(Some([[0.0, 40.0], [200.0, 160.0]]))
        .build()
        .unwrap()
}

/// Each text mark's labels with their positions.
fn labels(group: &SceneGroup) -> Vec<Vec<(String, f32, f32)>> {
    group
        .marks
        .iter()
        .filter_map(|mark| match mark {
            SceneMark::Text(text) => Some(text_labels(text)),
            _ => None,
        })
        .collect()
}

fn text_labels(mark: &SceneTextMark) -> Vec<(String, f32, f32)> {
    mark.text_iter()
        .zip(mark.x_iter())
        .zip(mark.y_iter())
        .map(|((text, x), y)| (text.clone(), *x, *y))
        .collect()
}

#[test]
fn framed_maps_label_lines_where_they_cross_the_frame() {
    let config = GraticuleConfig {
        graticule: Graticule::default().with_step(10.0),
        ..Default::default()
    };
    let group = make_graticule_marks(&framed(), &config, &engine()).unwrap();
    let [left, bottom] = labels(&group).try_into().unwrap();
    let close = |(_, x, y): &(String, f32, f32), [ex, ey]: [f32; 2]| {
        (x - ex).abs() < 1e-3 && (y - ey).abs() < 1e-3
    };
    let texts = |labels: &[(String, f32, f32)]| -> Vec<String> {
        labels.iter().map(|label| label.0.clone()).collect()
    };
    // Parallels from north to south, a padding left of the frame.
    assert_eq!(texts(&left), ["10°N", "0°", "10°S"]);
    for (label, y) in left.iter().zip([60.0, 100.0, 140.0]) {
        assert!(close(label, [-4.0, y]), "{label:?}");
    }
    // Meridians from west to east, a padding below it.
    assert_eq!(texts(&bottom), ["20°W", "10°W", "0°", "10°E", "20°E"]);
    for (label, x) in bottom.iter().zip([20.0, 60.0, 100.0, 140.0, 180.0]) {
        assert!(close(label, [x, 164.0]), "{label:?}");
    }
}

#[test]
fn lines_that_end_inside_the_frame_are_not_labeled() {
    // Minor parallels span longitudes -10 to 10, short of the frame's left edge.
    // The major parallel, the equator, spans the globe.
    let config = GraticuleConfig {
        graticule: Graticule {
            extent_minor: [[-10.0, -80.0], [10.0, 80.0]],
            ..Graticule::default().with_step(10.0)
        },
        label_sides: vec![MapSide::Left],
        ..Default::default()
    };
    let group = make_graticule_marks(&framed(), &config, &engine()).unwrap();
    let [left] = labels(&group).try_into().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].0, "0°");
}

#[test]
fn overlapping_labels_are_thinned_evenly() {
    // Meridians a degree, or 4 pixels, apart are narrower than their labels.
    let config = GraticuleConfig {
        graticule: Graticule::default().with_step(1.0),
        label_sides: vec![MapSide::Bottom],
        ..Default::default()
    };
    let group = make_graticule_marks(&framed(), &config, &engine()).unwrap();
    let [bottom] = labels(&group).try_into().unwrap();
    assert!(bottom.len() > 2 && bottom.len() < 25, "{}", bottom.len());
    let gaps: Vec<f32> = bottom
        .windows(2)
        .map(|pair| pair[1].1 - pair[0].1)
        .collect();
    assert!(
        gaps.iter().all(|gap| (gap - gaps[0]).abs() < 1e-3),
        "{gaps:?}"
    );
}

#[test]
fn world_maps_label_lines_at_the_outline() {
    let mut projection = Projection::new(EqualEarth);
    let summary = projection.summarize_sphere().unwrap();
    projection
        .fit([[0.0, 0.0], [400.0, 200.0]], &summary)
        .unwrap();
    let projector = projection.build().unwrap();
    let config = GraticuleConfig {
        graticule: Graticule::default().with_step(30.0),
        ..Default::default()
    };
    let group = make_graticule_marks(&projector, &config, &engine()).unwrap();
    let [left, bottom] = labels(&group).try_into().unwrap();
    // The equator ends at the outline's westernmost point.
    let equator = left.iter().find(|label| label.0 == "0°").unwrap();
    assert!(
        (equator.1 + 4.0).abs() < 1e-3 && (equator.2 - 100.0).abs() < 1e-3,
        "{equator:?}"
    );
    assert!(!bottom.is_empty());
}
