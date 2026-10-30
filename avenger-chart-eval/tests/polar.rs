//! Polar plots evaluated and rendered at two sizes and compared with image baselines: pies, a half
//! donut, a sunburst, a rose chart, a radial bar chart, and a radar chart.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::{proto, Chart, Encode, Scale};
use avenger_chart_eval::{ChartEvaluator, EvaluatedChart, PlotCoord, Theme};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scenegraph::marks::{mark::SceneMark, text::SceneTextMark};
use avenger_svg::SvgRenderer;
use avenger_typst_label::{EngineOptions, LabelEngine};

fn engine() -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: avenger_render_fixtures::scene::fonts(),
    })
    .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// Evaluates `chart` at `size`, renders it, and compares it with the baseline `name`.
fn render(chart: &Chart, size: [f32; 2], name: &str) -> EvaluatedChart {
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(chart, &engine, &Theme::default()).unwrap();
    let evaluated = evaluator.evaluate(size).unwrap();
    let svg = SvgRenderer::new()
        .render_scene_graph(&evaluated.scene, &engine)
        .unwrap();
    assert_baseline(env!("CARGO_MANIFEST_DIR"), &svg_to_png(&svg, 2.0), name);
    evaluated
}

/// Renders `chart` at 320 by 240 and at 640 by 360, writing both images before a failure of
/// either.
fn render_both(chart: &Chart, name: &str) {
    render_sizes(chart, &[[320.0, 240.0], [640.0, 360.0]], name);
}

/// Renders `chart` at each of `sizes`, writing every image before a failure of any.
fn render_sizes(chart: &Chart, sizes: &[[f32; 2]], name: &str) {
    let results: Vec<_> = sizes
        .iter()
        .map(|size: &[f32; 2]| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                render(chart, *size, &format!("{name}-{}x{}", size[0], size[1]));
            }))
        })
        .collect();
    for result in results {
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}

fn evaluate(chart: &Chart, size: [f32; 2]) -> EvaluatedChart {
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(chart, &engine, &Theme::default()).unwrap();
    evaluator.evaluate(size).unwrap()
}

fn strings(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

fn numbers(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

/// The slices of a pie of `counts` of `names`: the share of the total where each starts, where it
/// ends, and its middle.
fn slices(names: &[&str], counts: &[f64]) -> RecordBatch {
    let total: f64 = counts.iter().sum();
    let mut start = Vec::new();
    let mut end = Vec::new();
    let mut sum = 0.0;
    for count in counts {
        start.push(sum / total);
        sum += count;
        end.push(sum / total);
    }
    let middle: Vec<f64> = start.iter().zip(&end).map(|(a, b)| (a + b) / 2.0).collect();
    RecordBatch::try_from_iter([
        ("fruit", strings(names)),
        ("count", numbers(counts)),
        ("start", numbers(&start)),
        ("end", numbers(&end)),
        ("middle", numbers(&middle)),
    ])
    .unwrap()
}

const FRUITS: [&str; 5] = ["Apples", "Bananas", "Cherries", "Dates", "Figs"];

fn fruits() -> RecordBatch {
    slices(&FRUITS, &[42.0, 28.0, 15.0, 10.0, 5.0])
}

/// The slices of `table` as rects in the part of a circle of `polar`, with a gap between them.
fn pie_slices(table: RecordBatch, polar: proto::Polar) -> Chart {
    Chart::unit(
        table,
        proto::RectMark {
            inset: Some(1.0),
            ..Default::default()
        },
    )
    .coord(polar)
    .theta("start")
    .theta2("end")
    .fill("fruit")
    .build()
    .unwrap()
}

/// A label for each slice of `table` just outside its middle, which runs as `orient` says and
/// stands outward from its point.
fn pie_labels(table: RecordBatch, polar: proto::Polar, orient: proto::PolarTextOrient) -> Chart {
    let text = proto::TextMark {
        orient: orient as i32,
        ..Default::default()
    };
    Chart::unit(table, text)
        .coord(polar)
        .theta("middle")
        .radius(Encode::constant(1.04))
        .text("fruit")
        .build()
        .unwrap()
}

/// A pie of the fruits in the part of a circle of `polar`, with a label outside each slice that
/// runs as `orient` says.
fn pie(polar: proto::Polar, orient: proto::PolarTextOrient) -> Chart {
    Chart::overlay([
        pie_slices(fruits(), polar),
        pie_labels(fruits(), polar, orient),
    ])
    .build()
    .unwrap()
}

/// The texts of the scene, with the origins of their groups.
fn texts(evaluated: &EvaluatedChart) -> Vec<(SceneTextMark, [f32; 2])> {
    fn walk(marks: &[SceneMark], origin: [f32; 2], out: &mut Vec<(SceneTextMark, [f32; 2])>) {
        for mark in marks {
            match mark {
                SceneMark::Text(text) => out.push((text.as_ref().clone(), origin)),
                SceneMark::Group(group) => walk(
                    &group.marks,
                    [origin[0] + group.origin[0], origin[1] + group.origin[1]],
                    out,
                ),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&evaluated.scene.marks, [0.0, 0.0], &mut out);
    out
}

#[test]
fn a_pie() {
    let chart = pie(proto::Polar::default(), proto::PolarTextOrient::Tangential);
    render_both(&chart, "polar-pie");
}

#[test]
fn a_half_donut_with_its_labels_outside() {
    let polar = proto::Polar {
        start_angle: Some(-90.0),
        end_angle: Some(90.0),
        inner_radius: Some(0.5),
    };
    let chart = pie(polar, proto::PolarTextOrient::Unspecified);
    render_both(&chart, "polar-half-donut");
}

/// The labels outside a pie, straight or curved along a circle, stay inside its plot at each
/// size, because its circle shrinks to hold the outlines of their glyphs.
#[test]
fn the_labels_outside_a_pie_stay_inside_the_plot() {
    let engine = engine();
    for orient in [
        proto::PolarTextOrient::Unspecified,
        proto::PolarTextOrient::Tangential,
    ] {
        let chart = pie(proto::Polar::default(), orient);
        for size in [[320.0, 240.0], [640.0, 360.0]] {
            let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
            let evaluated = evaluator.evaluate(size).unwrap();
            assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
            let plot = evaluated.plots[0].rect;
            let labels: Vec<_> = texts(&evaluated)
                .into_iter()
                .filter(|(text, _)| text.name.ends_with("/1"))
                .collect();
            assert_eq!(labels.len(), 1);
            let (text, origin) = &labels[0];
            for (index, label) in text.labels().enumerate() {
                let bounds = engine.bounds(&label.label).unwrap();
                let placement = label.placement(&bounds, *origin);
                let outline = engine.outline(&label.label).unwrap();
                for point in outline.contours.iter().flatten() {
                    let [x, y] = placement.to_scene(*point);
                    let inside = x >= plot[0] - 0.5
                        && y >= plot[1] - 0.5
                        && x <= plot[0] + plot[2] + 0.5
                        && y <= plot[1] + plot[3] + 0.5;
                    assert!(
                        inside,
                        "{} of {orient:?} reaches {:?} outside {plot:?} at {size:?}",
                        FRUITS[index],
                        [x, y]
                    );
                }
            }
        }
    }
}

/// The nodes of a hierarchy of three levels: the share of the total where each starts and ends,
/// its depth, and the branch it belongs to.
fn hierarchy() -> RecordBatch {
    // (name, branch, depth, start, end)
    let nodes: [(&str, &str, f64, f64, f64); 12] = [
        ("All", "All", 0.0, 0.0, 1.0),
        ("Fruit", "Fruit", 1.0, 0.0, 0.55),
        ("Vegetables", "Vegetables", 1.0, 0.55, 0.85),
        ("Grains", "Grains", 1.0, 0.85, 1.0),
        ("Apples", "Fruit", 2.0, 0.0, 0.25),
        ("Pears", "Fruit", 2.0, 0.25, 0.4),
        ("Plums", "Fruit", 2.0, 0.4, 0.55),
        ("Carrots", "Vegetables", 2.0, 0.55, 0.7),
        ("Leeks", "Vegetables", 2.0, 0.7, 0.85),
        ("Rice", "Grains", 2.0, 0.85, 0.93),
        ("Oats", "Grains", 2.0, 0.93, 1.0),
        ("Barley", "Grains", 2.0, 1.0, 1.0),
    ];
    RecordBatch::try_from_iter([
        ("name", strings(&nodes.map(|node| node.0)) as ArrayRef),
        ("branch", strings(&nodes.map(|node| node.1))),
        ("depth", numbers(&nodes.map(|node| node.2))),
        ("outer", numbers(&nodes.map(|node| node.2 + 1.0))),
        ("start", numbers(&nodes.map(|node| node.3))),
        ("end", numbers(&nodes.map(|node| node.4))),
    ])
    .unwrap()
}

/// Sides refer to the circle, so on every side of it a text lies alike: its glyphs outward or
/// inward from the radius of its point, and clockwise or counterclockwise from its angle,
/// straight, radial, or curved along the circle.
#[test]
fn sides_place_texts_alike_on_every_side() {
    let engine = engine();
    let angles = [30.0, 150.0, 210.0, 330.0];
    let table = RecordBatch::try_from_iter([
        ("name", strings(&["Sided"; 4])),
        ("angle", numbers(&angles)),
    ])
    .unwrap();
    for orient in [
        proto::PolarTextOrient::Unspecified,
        proto::PolarTextOrient::Radial,
        proto::PolarTextOrient::Tangential,
    ] {
        for radius_side in [proto::RadiusSide::Outward, proto::RadiusSide::Inward] {
            for theta_side in [
                proto::ThetaSide::Clockwise,
                proto::ThetaSide::Counterclockwise,
            ] {
                let text = proto::TextMark {
                    orient: orient as i32,
                    radius_side: radius_side as i32,
                    theta_side: theta_side as i32,
                    ..Default::default()
                };
                let chart = Chart::unit(table.clone(), text)
                    .coord(proto::Polar::default())
                    .theta(Encode::values("angle"))
                    .radius(Encode::constant(0.6))
                    .text("name")
                    .build()
                    .unwrap();
                let evaluated = evaluate(&chart, [320.0, 240.0]);
                let (_, _, center, outer) = polar_coord(&evaluated, 0);
                let circle = 0.6 * outer;
                let (mark, origin) = &texts(&evaluated)[0];
                let center = [origin[0] + center[0], origin[1] + center[1]];
                for (row, label) in mark.labels().enumerate() {
                    let bounds = engine.bounds(&label.label).unwrap();
                    let placement = label.placement(&bounds, *origin);
                    let outline = engine.outline(&label.label).unwrap();
                    let (sin, cos) = (angles[row] as f32).to_radians().sin_cos();
                    let [out, clockwise] = [[sin, -cos], [cos, sin]];
                    for point in outline.contours.iter().flatten() {
                        let [x, y] = placement.to_scene(*point);
                        let [dx, dy] = [x - center[0], y - center[1]];
                        // How far out the glyph lies and how far clockwise of the angle: along
                        // the circle for a curved text, and along the spoke and its normal for
                        // a straight one.
                        let (out, around) = match orient {
                            proto::PolarTextOrient::Tangential => {
                                let turn = dx.atan2(-dy).to_degrees() - angles[row] as f32;
                                let turn = (turn + 180.0).rem_euclid(360.0) - 180.0;
                                (dx.hypot(dy) - circle, turn)
                            }
                            _ => (
                                dx * out[0] + dy * out[1] - circle,
                                dx * clockwise[0] + dy * clockwise[1],
                            ),
                        };
                        let case = format!(
                            "{orient:?} {radius_side:?} {theta_side:?} at {}: {out} out, \
                             {around} around",
                            angles[row]
                        );
                        match radius_side {
                            proto::RadiusSide::Outward => assert!(out > -0.5, "{case}"),
                            _ => assert!(out < 0.5, "{case}"),
                        }
                        match theta_side {
                            proto::ThetaSide::Clockwise => assert!(around > -0.5, "{case}"),
                            _ => assert!(around < 0.5, "{case}"),
                        }
                    }
                }
            }
        }
    }
}

/// A label for each of `nodes`, a name with the shares where it starts and ends, at the middle
/// of its angles and at `radius`.
fn node_labels(nodes: &[(&str, f64, f64)], radius: f64) -> RecordBatch {
    let middle: Vec<f64> = nodes.iter().map(|(_, a, b)| (a + b) / 2.0).collect();
    RecordBatch::try_from_iter([
        (
            "name",
            strings(&nodes.iter().map(|node| node.0).collect::<Vec<_>>()),
        ),
        ("middle", numbers(&middle)),
        ("radius", numbers(&vec![radius; nodes.len()])),
    ])
    .unwrap()
}

/// A sunburst of three levels, labeled at its center, along the arc of each branch in its ring,
/// and along the spoke of each leaf outside the rim. The branch labels curve along the middle of
/// their ring, the leaf labels stand off the rim, and the label at the center centers on it.
#[test]
fn a_sunburst_of_three_levels() {
    let polar = proto::Polar::default();
    let nodes = Chart::unit(
        hierarchy(),
        proto::RectMark {
            inset: Some(1.0),
            ..Default::default()
        },
    )
    .coord(polar)
    .theta("start")
    .theta2("end")
    .radius("depth")
    .radius2("outer")
    .fill("branch")
    .build()
    .unwrap();
    let branches = Chart::unit(
        node_labels(
            &[
                ("Fruit", 0.0, 0.55),
                ("Vegetables", 0.55, 0.85),
                ("Grains", 0.85, 1.0),
            ],
            1.5,
        ),
        proto::TextMark {
            orient: proto::PolarTextOrient::Tangential as i32,
            radius_side: proto::RadiusSide::Middle as i32,
            ..Default::default()
        },
    )
    .coord(polar)
    .theta("middle")
    .radius("radius")
    .text("name")
    .color(Encode::constant("white"))
    .build()
    .unwrap();
    let leaves = Chart::unit(
        node_labels(
            &[
                ("Apples", 0.0, 0.25),
                ("Pears", 0.25, 0.4),
                ("Plums", 0.4, 0.55),
                ("Carrots", 0.55, 0.7),
                ("Leeks", 0.7, 0.85),
                ("Rice", 0.85, 0.93),
                ("Oats", 0.93, 1.0),
            ],
            1.02,
        ),
        proto::TextMark {
            orient: proto::PolarTextOrient::Radial as i32,
            ..Default::default()
        },
    )
    .coord(polar)
    .theta("middle")
    .radius(Encode::values("radius"))
    .text("name")
    .build()
    .unwrap();
    let root = Chart::unit(
        RecordBatch::try_from_iter([("name", strings(&["All"]))]).unwrap(),
        proto::TextMark::default(),
    )
    .coord(polar)
    .text("name")
    .color(Encode::constant("white"))
    .build()
    .unwrap();
    let chart = Chart::overlay([nodes, branches, leaves, root])
        .build()
        .unwrap();
    render_both(&chart, "polar-sunburst");
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The rainfall of each month, in millimeters.
fn rainfall() -> RecordBatch {
    RecordBatch::try_from_iter([
        ("month", strings(&MONTHS)),
        (
            "rain",
            numbers(&[
                78.0, 60.0, 55.0, 48.0, 50.0, 42.0, 35.0, 40.0, 52.0, 70.0, 85.0, 90.0,
            ]),
        ),
    ])
    .unwrap()
}

/// An axis of a polar plot.
fn axis() -> Scale {
    Scale::new().guide(proto::Axis::default())
}

/// A rose chart whose month labels curve along a circle outside the rim.
#[test]
fn a_rose_chart() {
    let curved = Scale::new().guide(proto::Axis {
        label_turn: Some(proto::axis::LabelTurn::LabelOrient(
            proto::PolarTextOrient::Tangential as i32,
        )),
        ..Default::default()
    });
    let chart = Chart::unit(rainfall(), proto::BarMark::default())
        .coord(proto::Polar::default())
        .theta(Encode::column("month").scale(curved.domain(strings(&MONTHS))))
        .radius(Encode::column("rain").scale(axis()))
        .build()
        .unwrap();
    render_both(&chart, "polar-rose");
}

/// The record rainfall of each month, in millimeters, which is more than its rainfall.
fn records() -> RecordBatch {
    RecordBatch::try_from_iter([
        ("month", strings(&MONTHS)),
        (
            "record",
            numbers(&[
                120.0, 95.0, 88.0, 80.0, 76.0, 70.0, 62.0, 66.0, 90.0, 115.0, 130.0, 140.0,
            ]),
        ),
    ])
    .unwrap()
}

/// A rose chart of rainfall overlaid with a symbol at each month's record, which share `theta`
/// and `radius`, with the axis of `theta` declared by the bars and the axis of `radius` by the
/// symbols, or by both with `radius_axis_on_bars`.
fn rose_with_records(record_theta: Scale, radius_axis_on_bars: Option<proto::Axis>) -> Chart {
    let months = || strings(&MONTHS);
    let bars_radius = match radius_axis_on_bars {
        Some(axis) => Encode::column("rain").scale(Scale::new().guide(axis)),
        None => Encode::column("rain"),
    };
    let bars = Chart::unit(rainfall(), proto::BarMark::default())
        .coord(proto::Polar::default())
        .theta(Encode::column("month").scale(axis().domain(months())))
        .radius(bars_radius)
        .build()
        .unwrap();
    let marks = Chart::unit(records(), proto::SymbolMark::default())
        .coord(proto::Polar::default())
        .theta(Encode::column("month").scale(record_theta.domain(months())))
        .radius(Encode::column("record").scale(axis()))
        .color(Encode::constant("#333333"))
        .build()
        .unwrap();
    Chart::overlay([bars, marks]).build().unwrap()
}

/// The children of an overlay share `radius` over all of their values, so the records past the
/// rainfall stay inside the circle, and each child draws the axis that it declares. The children
/// may both declare one axis alike, but not differently.
#[test]
fn a_rose_overlaid_with_its_records() {
    let chart = rose_with_records(Scale::new(), None);
    render_both(&chart, "polar-rose-records");
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
    let (_, radius, _, outer) = polar_coord(&evaluated, 0);
    let at = |value: f64| {
        radius
            .as_ref()
            .unwrap()
            .scale_to_numeric(&numbers(&[value]))
            .unwrap()
            .as_vec(1, None)[0]
    };
    assert!(at(90.0) < at(140.0), "{} {}", at(90.0), at(140.0));
    assert!(at(140.0) <= outer + 1e-3, "{} past {outer}", at(140.0));

    // The axis of `theta` labels the months, and the axis of `radius` the rainfall.
    let labels: Vec<Vec<String>> = texts(&evaluated)
        .into_iter()
        .filter(|(text, _)| text.name == "labels")
        .map(|(text, _)| text.text.as_vec(text.len as usize, None))
        .collect();
    assert_eq!(labels.len(), 2, "{labels:?}");
    assert!(labels
        .iter()
        .any(|labels| labels.contains(&"Jan".to_string())));
    assert!(labels
        .iter()
        .any(|labels| labels.contains(&"100".to_string())));

    // Both children may declare the axis of `radius` alike, but not with labels that run two
    // ways. A child that leaves an option unset takes the other's.
    let alike = rose_with_records(Scale::new(), Some(proto::Axis::default()));
    let evaluated = evaluate(&alike, [320.0, 240.0]);
    let count = texts(&evaluated)
        .iter()
        .filter(|(text, _)| text.name == "labels")
        .count();
    assert_eq!(count, 2);
    let turned = |orient: proto::PolarTextOrient| proto::Axis {
        label_turn: Some(proto::axis::LabelTurn::LabelOrient(orient as i32)),
        ..Default::default()
    };
    let bars = Chart::unit(rainfall(), proto::BarMark::default())
        .coord(proto::Polar::default())
        .theta(Encode::column("month").scale(axis().domain(strings(&MONTHS))))
        .radius(
            Encode::column("rain")
                .scale(Scale::new().guide(turned(proto::PolarTextOrient::Tangential))),
        )
        .build()
        .unwrap();
    let marks = Chart::unit(records(), proto::SymbolMark::default())
        .coord(proto::Polar::default())
        .theta(Encode::column("month").scale(Scale::new().domain(strings(&MONTHS))))
        .radius(
            Encode::column("record")
                .scale(Scale::new().guide(turned(proto::PolarTextOrient::Radial))),
        )
        .build()
        .unwrap();
    let error = Chart::overlay([bars, marks])
        .build()
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("set `guide.axis.label_turn` to different values"),
        "{error}"
    );
}

#[test]
fn a_radial_bar_chart() {
    let table = RecordBatch::try_from_iter([
        ("fruit", strings(&FRUITS)),
        ("count", numbers(&[42.0, 28.0, 15.0, 10.0, 5.0])),
    ])
    .unwrap();
    let chart = Chart::unit(
        table,
        proto::BarMark {
            orient: proto::Orient::Horizontal as i32,
            ..Default::default()
        },
    )
    .coord(proto::Polar {
        end_angle: Some(270.0),
        inner_radius: Some(0.25),
        ..Default::default()
    })
    .radius(Encode::column("fruit").scale(axis().domain(strings(&FRUITS))))
    .theta(Encode::column("count").scale(axis()))
    .color("fruit")
    .build()
    .unwrap();
    render_both(&chart, "polar-radial-bars");
}

const SKILLS: [&str; 6] = ["Speed", "Power", "Range", "Armor", "Agility", "Stealth"];

/// The skills of two players, with the first skill of each again at its end, so a line through
/// them closes.
fn skills(scores: [[f64; 6]; 2]) -> RecordBatch {
    let mut skill = Vec::new();
    let mut score = Vec::new();
    let mut player = Vec::new();
    for (index, scores) in scores.iter().enumerate() {
        for row in 0..=SKILLS.len() {
            skill.push(SKILLS[row % SKILLS.len()]);
            score.push(scores[row % SKILLS.len()]);
            player.push(["Ada", "Bo"][index]);
        }
    }
    RecordBatch::try_from_iter([
        ("skill", strings(&skill)),
        ("score", numbers(&score)),
        ("player", strings(&player)),
    ])
    .unwrap()
}

/// A radar chart of `table`, with straight edges, its web, its rim, and its spokes, and its
/// `radius` through `radius`.
fn radar(table: RecordBatch, radius: Scale) -> Chart {
    let canvas = proto::Route::Canvas as i32;
    let polar = proto::Polar::default();
    let theta =
        Encode::column("skill").scale(Scale::new().domain(strings(&SKILLS)).guide(proto::Axis {
            route: canvas,
            ..Default::default()
        }));
    let radius = Encode::column("score").scale(radius.guide(proto::Axis {
        route: canvas,
        ..Default::default()
    }));
    let area = Chart::unit(
        table.clone(),
        proto::AreaMark {
            route: canvas,
            ..Default::default()
        },
    )
    .coord(polar)
    .theta(theta.clone())
    .radius(radius.clone())
    .color("player")
    .fill_opacity(Encode::constant(0.25))
    .build()
    .unwrap();
    let line = Chart::unit(table, proto::LineMark { route: canvas })
        .coord(polar)
        .theta(theta)
        .radius(radius)
        .color("player")
        .build()
        .unwrap();
    Chart::overlay([area, line]).build().unwrap()
}

#[test]
fn a_radar_chart() {
    let table = skills([
        [80.0, 65.0, 90.0, 40.0, 70.0, 55.0],
        [50.0, 85.0, 45.0, 75.0, 60.0, 80.0],
    ]);
    render_both(&radar(table, Scale::new()), "polar-radar");
}

/// The plot coordinate system of `plot`, which is polar: its scales, its center, and its outer
/// radius.
fn polar_coord(
    evaluated: &EvaluatedChart,
    plot: usize,
) -> (
    Option<avenger_scales::scales::ConfiguredScale>,
    Option<avenger_scales::scales::ConfiguredScale>,
    [f32; 2],
    f32,
) {
    match &evaluated.plots[plot].coord {
        PlotCoord::Polar {
            theta,
            radius,
            center,
            outer_radius,
            ..
        } => (theta.clone(), radius.clone(), *center, *outer_radius),
        other => panic!("plot {plot} is not polar: {other:?}"),
    }
}

/// The pies of three regions in a facet row, which share `theta`, so each pie of shares of its
/// total is whole.
#[test]
fn a_facet_row_of_pies_shares_theta() {
    let regions = [
        ("North", [12.0, 30.0, 8.0, 5.0, 5.0]),
        ("South", [40.0, 10.0, 25.0, 15.0, 10.0]),
        ("West", [20.0, 20.0, 20.0, 20.0, 20.0]),
    ];
    let pies = regions.map(|(region, counts)| {
        Chart::unit(
            slices(&FRUITS, &counts),
            proto::RectMark {
                inset: Some(1.0),
                ..Default::default()
            },
        )
        .coord(proto::Polar::default())
        .partition("region", region)
        .theta("start")
        .theta2("end")
        .fill("fruit")
        .build()
        .unwrap()
    });
    let chart = Chart::row(pies).facet("region").build().unwrap();
    render_both(&chart, "polar-facet-pies");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    let coords: Vec<_> = (0..3).map(|plot| polar_coord(&evaluated, plot)).collect();
    for (theta, _, _, outer) in &coords {
        let theta = theta.as_ref().unwrap();
        let ends = theta
            .scale_to_numeric(&numbers(&[0.0, 1.0]))
            .unwrap()
            .as_vec(2, None);
        assert!(
            ends[0].abs() < 1e-3 && (ends[1] - 360.0).abs() < 1e-3,
            "{ends:?}"
        );
        assert!((outer - coords[0].3).abs() < 1e-3);
    }
}

/// Two radar charts in a row of tracks of weights 1 and 2 share `radius`, so the smaller circle
/// sets the outer radius of both, and a value lies at one distance from each center.
#[test]
fn radar_charts_that_share_radius_draw_a_value_at_one_distance() {
    let track = |weight: f32| proto::Track {
        size: Some(proto::track::Size::Share(proto::Share {
            weight,
            max: None,
        })),
    };
    let first = skills([
        [80.0, 65.0, 90.0, 40.0, 70.0, 55.0],
        [50.0, 85.0, 45.0, 75.0, 60.0, 80.0],
    ]);
    let second = skills([
        [30.0, 45.0, 20.0, 40.0, 35.0, 25.0],
        [20.0, 25.0, 45.0, 35.0, 30.0, 40.0],
    ]);
    let chart = Chart::row([radar(first, Scale::new()), radar(second, Scale::new())])
        .share(proto::Channel::Radius)
        .layout(proto::ConcatLayout {
            tracks: vec![track(1.0), track(2.0)],
            ..Default::default()
        })
        .build()
        .unwrap();
    render_sizes(
        &chart,
        &[[640.0, 360.0], [960.0, 480.0]],
        "polar-shared-radius",
    );
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    let [first, second] = [0, 1].map(|plot| polar_coord(&evaluated, plot));
    assert!(evaluated.plots[1].rect[2] > evaluated.plots[0].rect[2] + 50.0);
    assert!(
        (first.3 - second.3).abs() < 1e-3,
        "{} and {}",
        first.3,
        second.3
    );
    let distance = |coord: &(_, Option<avenger_scales::scales::ConfiguredScale>, _, _)| {
        coord
            .1
            .as_ref()
            .unwrap()
            .scale_to_numeric(&numbers(&[50.0]))
            .unwrap()
            .as_vec(1, None)[0]
    };
    assert!((distance(&first) - distance(&second)).abs() < 1e-3);
}

/// The vertices of the line mark `name` of the scene, in the frame of its plot.
fn line_vertices(evaluated: &EvaluatedChart, name: &str) -> Vec<[f32; 2]> {
    fn find<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneMark> {
        marks.iter().find_map(|mark| match mark {
            SceneMark::Group(group) => find(&group.marks, name),
            mark if mark.mark_name() == name => Some(mark),
            _ => None,
        })
    }
    let Some(SceneMark::Line(line)) = find(&evaluated.scene.marks, name) else {
        panic!("the scene has no line `{name}`");
    };
    let len = line.len as usize;
    line.x
        .as_vec(len, None)
        .into_iter()
        .zip(line.y.as_vec(len, None))
        .map(|(x, y)| [x, y])
        .collect()
}

/// A rule without `theta` is a ring at its radius, and a rule without `radius` is a spoke at its
/// angle across the radii of the plot.
#[test]
fn a_rule_without_theta_is_a_ring_and_without_radius_a_spoke() {
    let polar = proto::Polar {
        inner_radius: Some(0.25),
        ..Default::default()
    };
    let rings = Chart::unit(
        RecordBatch::try_from_iter([("score", numbers(&[50.0]))]).unwrap(),
        proto::RuleMark::default(),
    )
    .coord(polar)
    .radius(Encode::column("score").scale(Scale::new().domain(numbers(&[0.0, 100.0]))))
    .build()
    .unwrap();
    let spokes = Chart::unit(
        RecordBatch::try_from_iter([("angle", numbers(&[90.0]))]).unwrap(),
        proto::RuleMark::default(),
    )
    .coord(polar)
    .theta(Encode::values("angle"))
    .build()
    .unwrap();
    let chart = Chart::overlay([rings, spokes]).build().unwrap();
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let (_, radius, center, outer) = polar_coord(&evaluated, 0);
    let fifty = radius
        .unwrap()
        .scale_to_numeric(&numbers(&[50.0]))
        .unwrap()
        .as_vec(1, None)[0];
    let distance = |[x, y]: [f32; 2]| (x - center[0]).hypot(y - center[1]);

    let ring = line_vertices(&evaluated, "root/0/0");
    assert!(ring.len() > 100);
    for vertex in &ring {
        assert!((distance(*vertex) - fifty).abs() < 1e-2);
    }
    let (first, last) = (ring[0], ring[ring.len() - 1]);
    assert!((first[0] - last[0]).abs() < 1e-2 && (first[1] - last[1]).abs() < 1e-2);

    let spoke = line_vertices(&evaluated, "root/1/0");
    assert_eq!(spoke.len(), 2);
    for vertex in &spoke {
        assert!((vertex[1] - center[1]).abs() < 1e-3 && vertex[0] > center[0]);
    }
    assert!((distance(spoke[0]) - 0.25 * outer).abs() < 1e-2);
    assert!((distance(spoke[1]) - outer).abs() < 1e-2);
}
