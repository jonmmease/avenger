//! Axes and colorbars with crowded labels, thinned by each option, compared with a baseline.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

use std::sync::Arc;

use arrow::array::{ArrayRef, StringArray};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{FontWeight, TextAlign, TextBaseline},
    value::ScalarOrArray,
};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::{
    axis::{Axis, AxisConfig, AxisStyle, LabelThinning},
    legend::{Colorbar, ColorbarConfig},
    GuideSide,
};
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scales::scales::{band::BandScale, linear::LinearScale};
use avenger_scenegraph::{
    marks::{group::SceneGroup, mark::SceneMark, rect::SceneRectMark, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_svg::SvgRenderer;
use avenger_typst_label::LabelEngine;

const OPTIONS: [(LabelThinning, &str); 4] = [
    (LabelThinning::None, "None"),
    (LabelThinning::Greedy, "Greedy"),
    (LabelThinning::Stride, "Stride"),
    (LabelThinning::StrideWithEnds, "Stride with ends"),
];
const LEFT: f32 = 130.0;
const COLUMN: f32 = 250.0;
const LENGTH: f32 = 210.0;

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

fn text(content: &str, x: f32, y: f32, weight: FontWeight) -> SceneMark {
    SceneTextMark {
        len: 1,
        text: content.into(),
        x: x.into(),
        y: y.into(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Top.into(),
        font_size: 12.0.into(),
        font_weight: weight.into(),
        ..Default::default()
    }
    .into()
}

fn config(side: GuideSide, thinning: LabelThinning, tick_count: Option<f64>) -> AxisConfig {
    AxisConfig {
        side,
        tick_format: ",.0f".to_string(),
        tick_label: None,
        style: AxisStyle {
            tick_count,
            label_thinning: thinning,
            title_visible: false,
            ..AxisStyle::default()
        },
    }
}

/// Draws a row's guide with an option's thinning, from the top left of its cell.
type Draw = Box<dyn Fn(LabelThinning, &LabelEngine) -> SceneGroup>;

/// A row of guides, one for each option.
struct Row {
    title: &'static str,
    height: f32,
    draw: Draw,
}

#[test]
fn label_thinning_options() {
    let engine = engine();
    let months: ArrayRef = Arc::new(StringArray::from(vec![
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
    ]));
    let rows = [
        Row {
            title: "Linear, 20 ticks",
            height: 50.0,
            draw: Box::new(|thinning, engine| {
                let scale = LinearScale::configured((0.0, 1_000_000.0), (0.0, 1.0));
                let config = config(GuideSide::Bottom, thinning, Some(20.0));
                let axis = Axis::continuous(&scale, "", &config, engine).unwrap();
                axis.marks([LENGTH, 0.0])
            }),
        },
        Row {
            title: "Months",
            height: 50.0,
            draw: Box::new(move |thinning, engine| {
                let scale = BandScale::configured(months.clone(), (0.0, 1.0));
                let config = config(GuideSide::Bottom, thinning, None);
                let axis = Axis::band(&scale, "", &config, engine).unwrap();
                axis.marks([LENGTH, 0.0])
            }),
        },
        Row {
            title: "Vertical, 30 ticks",
            height: 180.0,
            draw: Box::new(|thinning, engine| {
                let scale = LinearScale::configured((0.0, 100.0), (0.0, 1.0));
                let config = config(GuideSide::Left, thinning, Some(30.0));
                let mut group = Axis::continuous(&scale, "", &config, engine)
                    .unwrap()
                    .marks([0.0, 160.0]);
                group.origin = [40.0, 8.0];
                group
            }),
        },
        Row {
            title: "Colorbar, 20 ticks",
            height: 70.0,
            draw: Box::new(|thinning, engine| {
                let defaults = ColorbarConfig::default();
                let config = ColorbarConfig {
                    axis: AxisConfig {
                        side: GuideSide::Bottom,
                        style: AxisStyle {
                            tick_count: Some(20.0),
                            label_thinning: thinning,
                            ..defaults.axis.style.clone()
                        },
                        ..config(GuideSide::Bottom, thinning, Some(20.0))
                    },
                    ..defaults
                };
                let scale =
                    LinearScale::configured_color((0.0, 1000.0), vec!["white", "steelblue"]);
                Colorbar::new(&scale, "", &config, engine)
                    .unwrap()
                    .marks(LENGTH)
                    .group
            }),
        },
    ];

    let mut marks = vec![SceneMark::Rect(SceneRectMark {
        width: Some((LEFT + COLUMN * 4.0).into()),
        height: Some(1000.0.into()),
        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0; 4])),
        ..Default::default()
    })];
    for (column, (_, title)) in OPTIONS.iter().enumerate() {
        marks.push(text(
            title,
            LEFT + column as f32 * COLUMN,
            10.0,
            FontWeight::BOLD,
        ));
    }
    let mut y = 40.0;
    for row in &rows {
        marks.push(text(row.title, 10.0, y, FontWeight::default()));
        for (column, (thinning, _)) in OPTIONS.iter().enumerate() {
            let drawn = (row.draw)(*thinning, &engine);
            marks.push(
                SceneGroup {
                    origin: [LEFT + column as f32 * COLUMN, y],
                    marks: vec![drawn.into()],
                    ..Default::default()
                }
                .into(),
            );
        }
        y += row.height + 20.0;
    }
    let size = [LEFT + COLUMN * 4.0, y];
    if let SceneMark::Rect(background) = &mut marks[0] {
        background.height = Some(size[1].into());
    }
    let scene = SceneGraph {
        marks,
        width: size[0],
        height: size[1],
        origin: [0.0, 0.0],
    };
    let svg = SvgRenderer::new()
        .render_scene_graph(&scene, &engine)
        .unwrap();
    assert_baseline(
        env!("CARGO_MANIFEST_DIR"),
        &svg_to_png(&svg, 2.0),
        "label-thinning",
    );
}
