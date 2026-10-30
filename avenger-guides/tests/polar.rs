//! Polar axes: their angles, where their ticks and labels stand, their label halos, and a sheet
//! of polar charts compared with a baseline.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

use std::sync::Arc;

use arrow::array::{ArrayRef, StringArray};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::{
    axis::{
        AxisStyle, PolarAxis, PolarAxisConfig, PolarAxisKind, PolarFrame, PolarLabel,
        PolarLabelOrient,
    },
    error::AvengerGuidesError,
};
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scales::scales::{band::BandScale, linear::LinearScale, ConfiguredScale};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    text::{SceneTextMark, TextHalo},
};
use avenger_svg::SvgRenderer;
use avenger_typst_label::LabelEngine;

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const SKILLS: [&str; 6] = ["Speed", "Power", "Range", "Armor", "Agility", "Stealth"];

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

fn frame() -> PolarFrame {
    PolarFrame {
        center: [150.0, 150.0],
        inner_radius: 0.0,
        outer_radius: 100.0,
        corners: Vec::new(),
    }
}

fn config(kind: PolarAxisKind, start: f32, end: f32) -> PolarAxisConfig {
    PolarAxisConfig {
        kind,
        start_angle: start,
        end_angle: end,
        tick_format: ",.0f".to_string(),
        style: AxisStyle {
            tick_length: 4.0,
            tick_count: Some(4.0),
            ..AxisStyle::default()
        },
        ..Default::default()
    }
}

fn percent() -> ConfiguredScale {
    LinearScale::configured((0.0, 100.0), (0.0, 1.0))
}

fn bands(categories: &[&str]) -> ConfiguredScale {
    let domain: ArrayRef = Arc::new(StringArray::from(categories.to_vec()));
    BandScale::configured(domain, (0.0, 1.0))
}

/// The text mark of an axis's labels.
fn labels(group: &SceneGroup) -> &SceneTextMark {
    group
        .marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Text(text) if text.name == "labels" => Some(text.as_ref()),
            _ => None,
        })
        .expect("the axis has labels")
}

fn texts(mark: &SceneTextMark) -> Vec<String> {
    mark.text_iter().cloned().collect()
}

fn centers(mark: &SceneTextMark) -> Vec<[f32; 2]> {
    mark.x_iter()
        .zip(mark.y_iter())
        .map(|(x, y)| [*x, *y])
        .collect()
}

#[test]
fn angles_run_forward_at_most_a_turn() {
    let engine = engine();
    for (start, end) in [(90.0, 45.0), (0.0, 0.0), (0.0, 400.0), (f32::NAN, 90.0)] {
        let config = config(PolarAxisKind::Theta, start, end);
        let error = PolarAxis::continuous(percent(), &config, &engine).unwrap_err();
        assert!(
            matches!(error, AvengerGuidesError::PolarAngles { .. }),
            "{error:?}"
        );
    }
    for (start, end) in [(0.0, 360.0), (-90.0, 90.0), (30.0, 150.0)] {
        let config = config(PolarAxisKind::Theta, start, end);
        assert!(PolarAxis::continuous(percent(), &config, &engine).is_ok());
    }
}

/// Over a whole circle, the first band centers at the start angle, and a continuous axis drops
/// the tick at its end, which is its start.
#[test]
fn a_whole_circle_starts_its_bands_at_the_start_and_closes_on_one_tick() {
    let engine = engine();
    let frame = frame();
    let config = config(PolarAxisKind::Theta, 0.0, 360.0);
    let banded = PolarAxis::band(bands(&["N", "E", "S", "W"]), &config, &engine).unwrap();
    let angles = banded.tick_angles(&frame);
    assert_eq!(angles.len(), 4, "{angles:?}");
    for (angle, expected) in angles.iter().zip([0.0, 90.0, 180.0, 270.0]) {
        assert!((angle - expected).abs() < 1e-3, "{angles:?}");
    }

    let continuous = PolarAxis::continuous(percent(), &config, &engine).unwrap();
    let angles = continuous.tick_angles(&frame);
    assert!((angles[0] - 0.0).abs() < 1e-3, "{angles:?}");
    assert!(angles.last().unwrap() < &359.0, "{angles:?}");
    let shown = texts(labels(&continuous.marks(&frame)));
    assert!(
        shown.contains(&"0".to_string()) && !shown.contains(&"100".to_string()),
        "{shown:?}"
    );
}

/// A radius axis over a whole circle draws its labels on the clockwise side of its spoke, and
/// over part of one, outside the start edge.
#[test]
fn radius_labels_stand_beside_their_spoke() {
    let engine = engine();
    let frame = frame();
    // The spoke at 90 degrees points right, and its clockwise side is below it.
    let spoke = PolarAxisConfig {
        angle: Some(90.0),
        ..config(PolarAxisKind::Radius, 0.0, 360.0)
    };
    // The start edge of a half circle from -90 degrees points left, and its outside is below.
    let edge = config(PolarAxisKind::Radius, -90.0, 90.0);
    for config in [spoke, edge] {
        let radius = PolarAxis::continuous(percent(), &config, &engine).unwrap();
        let centers = centers(labels(&radius.marks(&frame)));
        assert!(!centers.is_empty());
        for [_, y] in centers {
            assert!(y > frame.center[1], "{y}");
        }
    }
}

#[test]
fn radial_and_tangential_labels_stay_upright() {
    let engine = engine();
    for orient in [PolarLabelOrient::Radial, PolarLabelOrient::Tangential] {
        let config = PolarAxisConfig {
            label_orient: orient,
            ..config(PolarAxisKind::Theta, 0.0, 360.0)
        };
        let axis = PolarAxis::band(bands(&MONTHS), &config, &engine).unwrap();
        let group = axis.marks(&frame());
        let mark = labels(&group);
        assert_eq!(mark.len, 12);
        for angle in mark.angle_iter() {
            assert!(*angle > -90.0 && *angle <= 90.0, "{orient:?}: {angle}");
        }
    }
}

/// Labels that overlap thin by stride, and around a whole circle the stride divides the number of
/// labels, so that the pattern stays even past the start.
#[test]
fn overlapping_labels_thin_evenly_around_the_circle() {
    let engine = engine();
    let small = PolarFrame {
        outer_radius: 20.0,
        ..frame()
    };
    let every_kth = |shown: &[String], all: &[String]| {
        assert!(
            !shown.is_empty() && all.len().is_multiple_of(shown.len()),
            "{shown:?}"
        );
        let stride = all.len() / shown.len();
        assert!(stride > 1, "{shown:?}");
        let expected: Vec<String> = all.iter().step_by(stride).cloned().collect();
        assert_eq!(shown, expected);
    };

    // Twenty ticks on request, of which the one at 100 is the one at 0.
    let twenty = PolarAxisConfig {
        style: AxisStyle {
            tick_count: Some(20.0),
            ..AxisStyle::default()
        },
        ..config(PolarAxisKind::Theta, 0.0, 360.0)
    };
    let axis = PolarAxis::continuous(percent(), &twenty, &engine).unwrap();
    let all: Vec<String> = (0..20).map(|tick| (tick * 5).to_string()).collect();
    every_kth(&texts(labels(&axis.marks(&small))), &all);

    let axis = PolarAxis::band(
        bands(&MONTHS),
        &config(PolarAxisKind::Theta, 0.0, 360.0),
        &engine,
    )
    .unwrap();
    let all: Vec<String> = MONTHS.iter().map(|month| month.to_string()).collect();
    every_kth(&texts(labels(&axis.marks(&small))), &all);
}

/// A continuous axis that fits its tick count takes coarser ticks where its labels would overlap
/// around the rim, and labels each of them.
#[test]
fn a_fitted_axis_takes_coarser_ticks_around_the_circle() {
    let engine = engine();
    let config = PolarAxisConfig {
        style: AxisStyle {
            tick_count: None,
            ..AxisStyle::default()
        },
        ..config(PolarAxisKind::Theta, 0.0, 360.0)
    };
    let axis = PolarAxis::continuous(percent(), &config, &engine).unwrap();
    let shown_at = |outer_radius: f32| {
        let frame = PolarFrame {
            outer_radius,
            ..frame()
        };
        let shown = labels(&axis.marks(&frame)).len as usize;
        assert_eq!(shown, axis.tick_angles(&frame).len(), "{outer_radius}");
        shown
    };
    assert!(shown_at(25.0) < shown_at(150.0));
}

/// A halo goes on the label text, widens the axis's reach by its width, and keeps the labels of
/// two axes far enough apart that neither halo covers the other label.
#[test]
fn halos_cover_the_labels_and_widen_the_reach() {
    let engine = engine();
    let frame = frame();
    let halo = TextHalo {
        color: [1.0, 1.0, 1.0, 0.75].into(),
        width: 3.0,
    };
    let theta = config(PolarAxisKind::Theta, 0.0, 360.0);
    let haloed = PolarAxisConfig {
        label_halo: Some(halo.clone()),
        ..theta.clone()
    };
    let compass = bands(&["N", "E", "S", "W"]);
    let plain = PolarAxis::band(compass.clone(), &theta, &engine).unwrap();
    let with_halo = PolarAxis::band(compass, &haloed, &engine).unwrap();
    assert_eq!(labels(&with_halo.marks(&frame)).halo, Some(halo.clone()));
    assert_eq!(labels(&plain.marks(&frame)).halo, None);
    let [before, after] = [plain.reach(&frame), with_halo.reach(&frame)];
    for side in 0..4 {
        assert!(
            (after[side] - before[side] - 3.0).abs() < 1e-3,
            "{before:?} {after:?}"
        );
    }

    // The label of the radius axis at the rim sits beside the theta label at its spoke. A
    // label clear of it without halos hides when a halo would cover the theta label.
    let theta = PolarAxis::band(bands(&SKILLS), &theta, &engine).unwrap();
    let radius = config(PolarAxisKind::Radius, 0.0, 360.0);
    let haloed = PolarAxisConfig {
        label_halo: Some(halo),
        ..radius.clone()
    };
    let shown = |config: &PolarAxisConfig| {
        let axis = PolarAxis::continuous(percent(), config, &engine).unwrap();
        texts(labels(&axis.marks_avoiding(&frame, &[&theta])))
    };
    assert!(shown(&radius).contains(&"100".to_string()));
    assert!(!shown(&haloed).contains(&"100".to_string()));
}

/// An array of halo colors gives each tick label its color, also when labels hide.
#[test]
fn halo_colors_follow_their_ticks() {
    let engine = engine();
    let frame = frame();
    let colors: Vec<[f32; 4]> = (0..6)
        .map(|tick| [tick as f32 / 10.0, 0.0, 0.0, 1.0])
        .collect();
    let radius = PolarAxisConfig {
        label_halo: Some(TextHalo {
            color: colors.clone().into(),
            width: 2.0,
        }),
        ..config(PolarAxisKind::Radius, 0.0, 360.0)
    };
    let theta = PolarAxis::band(
        bands(&SKILLS),
        &config(PolarAxisKind::Theta, 0.0, 360.0),
        &engine,
    )
    .unwrap();
    let radius = PolarAxis::continuous(percent(), &radius, &engine).unwrap();
    let group = radius.marks_avoiding(&frame, &[&theta]);
    let mark = labels(&group);
    let shown = texts(mark);
    assert_eq!(shown, ["0", "20", "40", "60", "80"]);
    let halo = mark.halo.as_ref().unwrap();
    let expected: Vec<[f32; 4]> = shown
        .iter()
        .map(|text| colors[text.parse::<usize>().unwrap() / 20])
        .collect();
    assert_eq!(halo.color.as_vec(shown.len(), None), expected);
}

#[test]
fn polar_axes_visual_regression() {
    let engine = engine();
    let scene = avenger_render_fixtures::polar::polar_axes(&engine);
    let svg = SvgRenderer::new()
        .render_scene_graph(&scene, &engine)
        .unwrap();
    let png = svg_to_png(&svg, 2.0);
    assert_baseline(env!("CARGO_MANIFEST_DIR"), &png, "polar-axes");
}

/// A polar label stands off its anchor with its nearest edge at the anchor: beside it across a
/// side, beyond its corner along a diagonal, and turned with its text.
#[test]
fn a_polar_label_stands_off_its_anchor() {
    let close = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
    let right = PolarLabel::standing_off([10.0, 20.0], [1.0, 0.0], [40.0, 10.0], 0.0);
    assert!(close(right.center, [30.0, 20.0]));
    assert!(close(right.bounds()[0], [10.0, 15.0]));
    // Along a diagonal, the corner nearest the anchor lies on the line across the diagonal
    // through the anchor, and the center on the diagonal.
    let half = std::f32::consts::FRAC_1_SQRT_2;
    let corner = PolarLabel::standing_off([0.0, 0.0], [half, half], [40.0, 10.0], 0.0);
    let [[left, top], _] = corner.bounds();
    assert!(((left + top) * half).abs() < 1e-4);
    assert!((corner.center[0] - corner.center[1]).abs() < 1e-4);
    // Turned a quarter, the box is as wide as it was tall.
    let turned = PolarLabel::standing_off([0.0, 0.0], [1.0, 0.0], [40.0, 10.0], 90.0);
    assert!(close(turned.center, [5.0, 0.0]));
    // Along a zero vector, the box centers on its anchor.
    let centered = PolarLabel::standing_off([3.0, 4.0], [0.0, 0.0], [40.0, 10.0], 30.0);
    assert!(close(centered.center, [3.0, 4.0]));
}
