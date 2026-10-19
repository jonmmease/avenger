use avenger_common::types::{FontStyle, TextSyntaxMode};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geo::paths::geometry_path;
use avenger_geo::projections::{EqualEarth, Equirectangular, Mercator};
use avenger_geo::{FitSummary, Graticule as Lines, Projection};
use avenger_guides::GuideSide;
use avenger_guides_geo::{Graticule, GraticuleConfig, MapFrame};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    text::{text_label, text_origin, text_style, SceneTextMark},
};
use avenger_typst_label::{LabelEngine, LabelOptions};
use geo_types::MultiPoint;
use std::sync::Arc;

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// An equirectangular map of longitudes `west` to `east` and latitudes `south` to `north`, at
/// 4 pixels a degree when it is `4 * (east - west)` wide.
fn regional(
    [west, south]: [f64; 2],
    [east, north]: [f64; 2],
    config: &GraticuleConfig,
) -> Graticule {
    let projection = Projection::new(Equirectangular);
    let corners = MultiPoint::from(vec![(west, south), (east, north)]);
    let summary = projection.summarize(&corners).unwrap();
    Graticule::new(&projection, &summary, config, &engine()).unwrap()
}

/// An Equal Earth world map.
fn world(config: &GraticuleConfig) -> Graticule {
    let projection = Projection::new(EqualEarth);
    let summary = projection.summarize_sphere().unwrap();
    Graticule::new(&projection, &summary, config, &engine()).unwrap()
}

fn rectangle(step: f64, label_sides: Vec<GuideSide>) -> GraticuleConfig {
    GraticuleConfig {
        graticule: Lines::default().with_step(step),
        frame: MapFrame::Rectangle,
        label_sides,
        ..Default::default()
    }
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

fn texts(labels: &[(String, f32, f32)]) -> Vec<&str> {
    labels.iter().map(|label| label.0.as_str()).collect()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn rectangle_maps_label_lines_where_they_cross_the_edge() {
    let config = rectangle(10.0, vec![GuideSide::Left, GuideSide::Bottom]);
    let graticule = regional([-25.0, -15.0], [25.0, 15.0], &config);
    assert!(
        close(graticule.aspect(), 5.0 / 3.0),
        "{}",
        graticule.aspect()
    );
    let [left, bottom] = labels(&graticule.marks([200.0, 120.0])).try_into().unwrap();
    // Parallels from north to south, a padding left of the map.
    assert_eq!(texts(&left), ["10°N", "0°", "10°S"]);
    for (label, y) in left.iter().zip([20.0, 60.0, 100.0]) {
        assert!(close(label.1, -4.0) && close(label.2, y), "{label:?}");
    }
    // Meridians from west to east, a padding below it.
    assert_eq!(texts(&bottom), ["20°W", "10°W", "0°", "10°E", "20°E"]);
    for (label, x) in bottom.iter().zip([20.0, 60.0, 100.0, 140.0, 180.0]) {
        assert!(close(label.1, x) && close(label.2, 124.0), "{label:?}");
    }
}

#[test]
fn outline_maps_label_lines_at_their_ends() {
    let config = GraticuleConfig {
        graticule: Lines::default().with_step(30.0),
        ..Default::default()
    };
    let graticule = world(&config);
    let size = [400.0, 400.0 / graticule.aspect()];
    let [left, bottom] = labels(&graticule.marks(size)).try_into().unwrap();
    // The equator ends at the outline's westernmost point, on the map's left edge.
    let equator = left.iter().find(|label| label.0 == "0°").unwrap();
    assert!(
        close(equator.1, -4.0) && close(equator.2, size[1] / 2.0),
        "{equator:?}"
    );
    // The parallels at 60° end inside the map's rectangle, at the curved outline.
    let north = left.iter().find(|label| label.0 == "60°N").unwrap();
    assert!(north.1 > 0.0, "{north:?}");
    assert!(!bottom.is_empty());
}

#[test]
fn lines_that_end_inside_the_frame_are_not_labeled() {
    // Minor parallels span longitudes -10 to 10, short of the frame's left edge.
    // The major parallel, the equator, spans the globe.
    let config = GraticuleConfig {
        graticule: Lines {
            extent_minor: [[-10.0, -80.0], [10.0, 80.0]],
            ..Lines::default().with_step(10.0)
        },
        ..rectangle(10.0, vec![GuideSide::Left])
    };
    let graticule = regional([-25.0, -15.0], [25.0, 15.0], &config);
    let [left] = labels(&graticule.marks([200.0, 120.0])).try_into().unwrap();
    assert_eq!(texts(&left), ["0°"]);
}

#[test]
fn overlapping_labels_are_thinned_evenly() {
    // Meridians a degree, or 4 pixels, apart are narrower than their labels.
    let config = rectangle(1.0, vec![GuideSide::Bottom]);
    let graticule = regional([-25.0, -15.0], [25.0, 15.0], &config);
    let [bottom] = labels(&graticule.marks([200.0, 120.0])).try_into().unwrap();
    assert!(bottom.len() > 2 && bottom.len() < 25, "{}", bottom.len());
    let gaps: Vec<f32> = bottom
        .windows(2)
        .map(|pair| pair[1].1 - pair[0].1)
        .collect();
    assert!(gaps.iter().all(|gap| close(*gap, gaps[0])), "{gaps:?}");
}

#[test]
fn labels_that_overlap_an_earlier_side_are_left_out() {
    // The meridian at 20°W meets the bottom edge 2 pixels from the left corner, and the
    // parallel at 10°S meets the left edge 0.4 pixels above it, so their labels overlap there.
    let corner = |label_sides: Vec<GuideSide>| {
        let graticule = regional([-20.5, -10.1], [25.0, 15.0], &rectangle(10.0, label_sides));
        labels(&graticule.marks([182.0, 100.4]))
    };
    let [left, bottom] = corner(vec![GuideSide::Left, GuideSide::Bottom])
        .try_into()
        .unwrap();
    assert!(texts(&left).contains(&"10°S"), "{left:?}");
    assert!(!texts(&bottom).contains(&"20°W"), "{bottom:?}");
    let [bottom, left] = corner(vec![GuideSide::Bottom, GuideSide::Left])
        .try_into()
        .unwrap();
    assert!(texts(&bottom).contains(&"20°W"), "{bottom:?}");
    assert!(!texts(&left).contains(&"10°S"), "{left:?}");
}

/// The thickness and overhang of the labels that `part` draws on `side` at `length`.
fn drawn_reach(graticule: &Graticule, side: GuideSide, length: f32) -> (f32, [f32; 2]) {
    part_reach(graticule.part(side, length), side, length)
}

/// How far a side's part reaches past the map's edge and past the ends of a side of `length`.
fn part_reach(part: Option<SceneGroup>, side: GuideSide, length: f32) -> (f32, [f32; 2]) {
    let engine = engine();
    let (mut across, mut start, mut end) = (0.0_f32, 0.0_f32, 0.0_f32);
    let Some(part) = part else {
        return (0.0, [0.0; 2]);
    };
    for mark in &part.marks {
        let SceneMark::Text(mark) = mark else {
            continue;
        };
        let options = LabelOptions {
            text: text_style(
                mark.font.first().unwrap(),
                *mark.font_size.first().unwrap(),
                *mark.font_weight.first().unwrap(),
                FontStyle::Normal,
                *mark.color.first().unwrap(),
            ),
            ..Default::default()
        };
        let align = *mark.align.first().unwrap();
        let baseline = *mark.baseline.first().unwrap();
        for (text, x, y) in text_labels(mark) {
            let bounds = engine
                .bounds(&text_label(
                    &text,
                    TextSyntaxMode::TypstMarkup,
                    options.clone(),
                ))
                .unwrap();
            let [left, top] = text_origin(&bounds, [x, y], align, baseline);
            for corner in [[left, top], [left + bounds.width, top + bounds.height]] {
                let (along, outward) = side.along_across(corner);
                across = across.max(outward);
                start = start.max(-along);
                end = end.max(along - length);
            }
        }
    }
    (across, [start, end])
}

#[test]
fn extents_are_the_reach_of_the_drawn_labels() {
    let sides = vec![
        GuideSide::Left,
        GuideSide::Bottom,
        GuideSide::Right,
        GuideSide::Top,
    ];
    let maps = [
        world(&GraticuleConfig {
            graticule: Lines::default().with_step(30.0),
            label_sides: sides.clone(),
            ..Default::default()
        }),
        regional([-25.0, -15.0], [25.0, 15.0], &rectangle(5.0, sides.clone())),
    ];
    for graticule in &maps {
        for length in [40.0, 75.0, 120.0, 200.0, 333.0, 517.0] {
            for &side in &sides {
                let extent = graticule.extent(side, length);
                let (thickness, overhang) = drawn_reach(graticule, side, length);
                assert!(
                    close(extent.thickness, thickness)
                        && close(extent.overhang[0], overhang[0])
                        && close(extent.overhang[1], overhang[1]),
                    "{side:?} at {length}: {extent:?} vs {thickness}, {overhang:?}"
                );
                assert_eq!(extent.holds, length..=length);
            }
        }
    }
    // Outside labels give the rectangle's sides their thickness.
    let extent = maps[1].extent(GuideSide::Left, 120.0);
    assert!(extent.thickness > 4.0, "{extent:?}");
}

#[test]
fn a_view_through_the_fitted_projector_draws_the_fitted_graticule() {
    let sides = vec![GuideSide::Left, GuideSide::Bottom];
    let graticule = regional(
        [-25.0, -15.0],
        [25.0, 15.0],
        &rectangle(10.0, sides.clone()),
    );
    let size = [200.0, 120.0];
    let view = graticule.view(&graticule.projector(size), size);
    assert_eq!(labels(&view.marks()), labels(&graticule.marks(size)));
    for (side, length) in sides.into_iter().zip([120.0, 200.0]) {
        assert_eq!(
            view.part(side).map(|part| labels(&part)),
            graticule.part(side, length).map(|part| labels(&part)),
            "{side:?}"
        );
    }
    assert_eq!(
        format!("{:?}", view.lines()),
        format!("{:?}", graticule.lines(size))
    );
}

/// Turned and zoomed views label lines where they cross the map's edges, within the view
/// extent that a layout keeps while the view changes.
#[test]
fn views_label_lines_within_the_view_extent() {
    let sides = vec![
        GuideSide::Left,
        GuideSide::Bottom,
        GuideSide::Right,
        GuideSide::Top,
    ];
    let graticule = world(&GraticuleConfig {
        graticule: Lines::default().with_step(10.0),
        label_sides: sides.clone(),
        ..Default::default()
    });
    let size = [300.0, 200.0];
    for (rotate, scale) in [
        ([0.0, 0.0, 0.0], 60.0),
        ([120.0, 0.0, 0.0], 300.0),
        ([-40.0, 10.0, 0.0], 900.0),
    ] {
        let projector = Projection::new(EqualEarth)
            .with_rotate(rotate)
            .with_view(scale, [150.0, 100.0])
            .with_clip_extent(Some([[0.0, 0.0], [300.0, 200.0]]))
            .build()
            .unwrap();
        let view = graticule.view(&projector, size);
        let mut placed = 0;
        for &side in &sides {
            let length = if side.is_vertical() { size[1] } else { size[0] };
            let extent = graticule.view_extent(side);
            assert_eq!(extent.holds, 0.0..=f32::INFINITY);
            let part = view.part(side);
            placed += part.as_ref().map_or(0, |part| labels(part).concat().len());
            let (thickness, overhang) = part_reach(part, side, length);
            assert!(
                thickness <= extent.thickness + 1e-3
                    && overhang[0] <= extent.overhang[0] + 1e-3
                    && overhang[1] <= extent.overhang[1] + 1e-3,
                "{side:?} at {rotate:?}, {scale}: {thickness}, {overhang:?} vs {extent:?}"
            );
        }
        assert!(placed > 0, "{rotate:?}, {scale}");
    }
}

/// An outline labels its lines at their ends, though Mercator's bounds clip it to a square: the
/// minor meridians end at 80° and have labels too.
#[test]
fn outline_maps_label_line_ends_inside_the_projection_bounds() {
    let projection = Projection::new(Mercator);
    let summary = projection.summarize_sphere().unwrap();
    let config = GraticuleConfig {
        graticule: Lines::default().with_step(30.0),
        label_sides: vec![GuideSide::Bottom],
        ..Default::default()
    };
    let graticule = Graticule::new(&projection, &summary, &config, &engine()).unwrap();
    let [bottom] = labels(&graticule.marks([400.0, 400.0])).try_into().unwrap();
    assert!(texts(&bottom).contains(&"30°E"), "{:?}", texts(&bottom));
}

/// A view labels lines on the map's rectangle only, so a projector that doesn't clip to it
/// places no label past the view extent.
#[test]
fn views_label_only_the_map_edges() {
    let graticule = world(&GraticuleConfig {
        graticule: Lines::default().with_step(10.0),
        label_sides: vec![GuideSide::Left, GuideSide::Bottom],
        ..Default::default()
    });
    let size = [300.0, 200.0];
    let unclipped = Projection::new(EqualEarth)
        .with_view(900.0, [150.0, 100.0])
        .build()
        .unwrap();
    let view = graticule.view(&unclipped, size);
    for (side, length) in [(GuideSide::Left, size[1]), (GuideSide::Bottom, size[0])] {
        let extent = graticule.view_extent(side);
        let (thickness, overhang) = part_reach(view.part(side), side, length);
        assert!(
            thickness <= extent.thickness + 1e-3
                && overhang[0] <= extent.overhang[0] + 1e-3
                && overhang[1] <= extent.overhang[1] + 1e-3,
            "{side:?}: {thickness}, {overhang:?} vs {extent:?}"
        );
    }
}

#[test]
fn the_projector_lines_and_labels_agree() {
    let config = rectangle(10.0, vec![GuideSide::Bottom]);
    let graticule = regional([-25.0, -15.0], [25.0, 15.0], &config);
    let size = [300.0, 180.0];
    let projector = graticule.projector(size);
    assert_eq!(projector.clip_extent(), Some([[0.0, 0.0], [300.0, 180.0]]));
    let [bottom] = labels(&graticule.marks(size)).try_into().unwrap();
    for (label, lon) in bottom.iter().zip([-20.0, -10.0, 0.0, 10.0, 20.0]) {
        let (x, _) = projector.project_point(lon, 0.0).unwrap();
        assert!(close(label.1, x as f32), "{label:?} vs {x}");
    }
    // The lines are clipped to the map, and reach each of its edges.
    let lines = graticule.lines(size);
    let SceneMark::Path(path) = &lines.marks[0] else {
        panic!("the lines are a path mark");
    };
    let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for path in path.path_iter() {
        for event in path.iter() {
            for point in [event.from(), event.to()] {
                min = [min[0].min(point.x), min[1].min(point.y)];
                max = [max[0].max(point.x), max[1].max(point.y)];
            }
        }
    }
    for (bound, edge) in min.into_iter().chain(max).zip([0.0, 0.0, 300.0, 180.0]) {
        assert!(close(bound, edge), "{min:?} {max:?}");
    }
}

#[test]
fn fitted_maps_fill_their_size_at_the_aspect() {
    let projection = Projection::new(EqualEarth);
    let summary = projection.summarize_sphere().unwrap();
    let graticule = world(&GraticuleConfig::default());
    assert!(close(graticule.aspect(), summary.aspect().unwrap() as f32));
    let size = [500.0, 500.0 / graticule.aspect()];
    let sphere = geometry_path(&graticule.projector(size).project_sphere());
    let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for event in sphere.iter() {
        let point = event.to();
        min = [min[0].min(point.x), min[1].min(point.y)];
        max = [max[0].max(point.x), max[1].max(point.y)];
    }
    for (bound, edge) in min.into_iter().chain(max).zip([0.0, 0.0, size[0], size[1]]) {
        assert!((bound - edge).abs() < 1e-2, "{min:?} {max:?} in {size:?}");
    }
}

#[test]
fn labels_format_their_degrees() {
    let config = GraticuleConfig {
        label_format: ".1f".into(),
        ..rectangle(10.0, vec![GuideSide::Left])
    };
    let graticule = regional([-25.0, -15.0], [25.0, 15.0], &config);
    let [left] = labels(&graticule.marks([200.0, 120.0])).try_into().unwrap();
    // Labels are Typst markup, which escapes the decimal point.
    assert_eq!(texts(&left), [r"10\.0°N", r"0\.0°", r"10\.0°S"]);
}

#[test]
fn maps_without_length_have_no_labels() {
    let config = rectangle(10.0, vec![GuideSide::Left, GuideSide::Bottom]);
    let graticule = regional([-25.0, -15.0], [25.0, 15.0], &config);
    for side in [GuideSide::Left, GuideSide::Bottom] {
        assert_eq!(graticule.extent(side, 0.0).thickness, 0.0);
        assert!(graticule.part(side, 0.0).is_none());
    }
    assert!(graticule.marks([0.0, 0.0]).marks.is_empty());
    // A side without labels is empty at every length.
    let extent = graticule.extent(GuideSide::Top, 120.0);
    assert_eq!(extent.thickness, 0.0);
    assert_eq!(extent.holds, 0.0..=f32::INFINITY);
}

#[test]
fn the_constructor_checks_its_inputs() {
    let engine = engine();
    let projection = Projection::new(EqualEarth);
    let summary = projection.summarize_sphere().unwrap();
    let config = GraticuleConfig::default();
    let clipped = projection
        .clone()
        .with_clip_extent(Some([[0.0, 0.0], [10.0, 10.0]]));
    assert!(Graticule::new(&clipped, &summary, &config, &engine).is_err());
    assert!(Graticule::new(&projection, &FitSummary::default(), &config, &engine).is_err());
    let repeated = GraticuleConfig {
        label_sides: vec![GuideSide::Left, GuideSide::Left],
        ..Default::default()
    };
    assert!(Graticule::new(&projection, &summary, &repeated, &engine).is_err());
}
