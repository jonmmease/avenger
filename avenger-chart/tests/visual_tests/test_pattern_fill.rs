use super::helpers::assert_scene_graph_visual_match_default;
use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::{
    Band, BandScaleExt, Cartesian, CartesianRectPositionChannels, LegendPosition, Plot, Rect,
    ScaleChannelConfig, Theme, col, lit,
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{PathTransform, StrokeCap, StrokeJoin, SymbolShape},
    value::ScalarOrArray,
};
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark,
        area::SceneAreaMark,
        group::SceneGroup,
        path::ScenePathMark,
        pattern::{
            PatternAnchor, PatternFill, PatternInk, PatternLayer, PatternReferenceFrame,
            PatternSymbol, StripeDash, StripePatternLayer, SymbolLattice2d, SymbolPaint,
            SymbolPatternLayer,
        },
        rect::SceneRectMark,
        symbol::SceneSymbolMark,
    },
    scene_graph::SceneGraph,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float32Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{DataFrame, SessionContext},
};
use lyon_path::{Path, geom::point};
use std::sync::Arc;

const BASELINE_CATEGORY: &str = "pattern_fill";
const STROKE: ColorOrGradient = ColorOrGradient::Color([0.08, 0.09, 0.1, 1.0]);

fn solid_pattern(layers: Vec<PatternLayer>, opacity: f32) -> PatternFill {
    PatternFill {
        anchor: PatternAnchor::Mark,
        ink: PatternInk::Solid {
            color: [0.0, 0.0, 0.0, 1.0],
            opacity,
        },
        layers,
    }
}

fn stripe(angle: f32, spacing: f32, stroke_width: f32) -> PatternLayer {
    PatternLayer::Stripe(StripePatternLayer::new(angle, spacing, stroke_width))
}

fn stripe_with_phase(angle: f32, spacing: f32, stroke_width: f32, phase: f32) -> PatternLayer {
    let mut layer = StripePatternLayer::new(angle, spacing, stroke_width);
    layer.phase = phase;
    PatternLayer::Stripe(layer)
}

fn dashed_stripe() -> PatternLayer {
    let mut layer = StripePatternLayer::new(45.0, 12.0, 2.0);
    layer.phase = 1.5;
    layer.dash = Some(StripeDash {
        length: 10.0,
        gap: 6.0,
        phase: 3.0,
    });
    PatternLayer::Stripe(layer)
}

fn symbol_layer(
    shape: &str,
    paint: SymbolPaint,
    u_angle: f32,
    v_angle: f32,
    u_phase: f32,
    v_phase: f32,
) -> PatternLayer {
    PatternLayer::Symbol(SymbolPatternLayer {
        lattice: SymbolLattice2d {
            u_spacing: 13.0,
            u_angle,
            v_spacing: 13.0,
            v_angle,
            u_phase,
            v_phase,
        },
        symbol: PatternSymbol {
            shape: shape.to_string(),
            size: 28.0,
            rotation: 18.0,
        },
        paint,
    })
}

fn rect_mark(
    x: Vec<f32>,
    y: Vec<f32>,
    width: Vec<f32>,
    height: Vec<f32>,
    fill: Vec<ColorOrGradient>,
    fill_pattern: Vec<Option<PatternFill>>,
) -> SceneRectMark {
    SceneRectMark {
        len: x.len() as u32,
        x: ScalarOrArray::new_array(x),
        y: ScalarOrArray::new_array(y),
        width: Some(ScalarOrArray::new_array(width)),
        height: Some(ScalarOrArray::new_array(height)),
        fill: ScalarOrArray::new_array(fill),
        fill_pattern: ScalarOrArray::new_array(fill_pattern),
        stroke: ScalarOrArray::new_scalar(STROKE),
        stroke_width: ScalarOrArray::new_scalar(1.0),
        ..Default::default()
    }
}

fn polygon_path(points: &[[f32; 2]]) -> Path {
    let mut builder = Path::builder().with_svg();
    builder.move_to(point(points[0][0], points[0][1]));
    for point_xy in points.iter().skip(1) {
        builder.line_to(point(point_xy[0], point_xy[1]));
    }
    builder.close();
    builder.build()
}

fn pattern_theme_data(ctx: &SessionContext) -> DataFrame {
    let product = StringArray::from(vec!["Alpha", "Beta", "Gamma", "Delta"]);
    let series = StringArray::from(vec!["A", "B", "C", "D"]);
    let value = Float32Array::from(vec![42.0, 64.0, 52.0, 74.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("product", DataType::Utf8, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("value", DataType::Float32, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(product) as ArrayRef,
            Arc::new(series) as ArrayRef,
            Arc::new(value) as ArrayRef,
        ],
    )
    .expect("pattern theme baseline data");

    ctx.read_batch(batch).expect("pattern theme dataframe")
}

#[tokio::test]
async fn pattern_stripe_geometry_matrix() {
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut width = Vec::new();
    let mut height = Vec::new();
    let mut fill = Vec::new();
    let mut patterns = Vec::new();

    for (row, spacing) in [8.0, 14.0].into_iter().enumerate() {
        for (col, angle) in [0.0, 45.0, 90.0, 135.0].into_iter().enumerate() {
            x.push(12.0 + col as f32 * 52.0);
            y.push(12.0 + row as f32 * 52.0);
            width.push(40.0);
            height.push(40.0);
            fill.push(ColorOrGradient::Color([0.80, 0.87, 0.93, 1.0]));
            patterns.push(Some(solid_pattern(
                vec![stripe_with_phase(angle, spacing, 2.0, row as f32 * 3.5)],
                0.55,
            )));
        }
    }

    let scene_graph = SceneGraph {
        width: 220.0,
        height: 116.0,
        origin: [0.0, 0.0],
        marks: vec![rect_mark(x, y, width, height, fill, patterns).into()],
    };

    assert_scene_graph_visual_match_default(
        &scene_graph,
        BASELINE_CATEGORY,
        "pattern_stripe_geometry_matrix",
    )
    .await;
}

#[tokio::test]
async fn pattern_anchor_and_dash_phase() {
    let plot_pattern = PatternFill {
        anchor: PatternAnchor::Plot,
        ink: PatternInk::Solid {
            color: [0.0, 0.0, 0.0, 1.0],
            opacity: 0.5,
        },
        layers: vec![dashed_stripe()],
    };
    let mark_pattern = PatternFill {
        anchor: PatternAnchor::Mark,
        ..plot_pattern.clone()
    };
    let crosshatch = solid_pattern(
        vec![stripe(35.0, 12.0, 1.8), stripe(125.0, 12.0, 1.8)],
        0.45,
    );

    let scene_graph = SceneGraph {
        width: 260.0,
        height: 118.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneGroup {
                pattern_reference_frame: Some(PatternReferenceFrame {
                    x: 12.0,
                    y: 12.0,
                    width: 144.0,
                    height: 36.0,
                }),
                marks: vec![
                    rect_mark(
                        vec![12.0, 60.0, 108.0],
                        vec![12.0, 12.0, 12.0],
                        vec![48.0, 48.0, 48.0],
                        vec![36.0, 36.0, 36.0],
                        vec![
                            ColorOrGradient::Color([0.77, 0.86, 0.96, 1.0]),
                            ColorOrGradient::Color([0.77, 0.86, 0.96, 1.0]),
                            ColorOrGradient::Color([0.77, 0.86, 0.96, 1.0]),
                        ],
                        vec![
                            Some(plot_pattern.clone()),
                            Some(plot_pattern.clone()),
                            Some(plot_pattern),
                        ],
                    )
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            rect_mark(
                vec![12.0, 60.0, 108.0, 174.0],
                vec![64.0, 64.0, 64.0, 44.0],
                vec![48.0, 48.0, 48.0, 54.0],
                vec![36.0, 36.0, 36.0, 54.0],
                vec![
                    ColorOrGradient::Color([0.96, 0.85, 0.77, 1.0]),
                    ColorOrGradient::Color([0.96, 0.85, 0.77, 1.0]),
                    ColorOrGradient::Color([0.96, 0.85, 0.77, 1.0]),
                    ColorOrGradient::Color([0.89, 0.92, 0.84, 1.0]),
                ],
                vec![
                    Some(mark_pattern.clone()),
                    Some(mark_pattern.clone()),
                    Some(mark_pattern),
                    Some(crosshatch),
                ],
            )
            .into(),
        ],
    };

    assert_scene_graph_visual_match_default(
        &scene_graph,
        BASELINE_CATEGORY,
        "pattern_anchor_and_dash_phase",
    )
    .await;
}

#[tokio::test]
async fn pattern_filled_mark_clipping() {
    let pattern = solid_pattern(vec![stripe(135.0, 9.0, 1.7)], 0.5);
    let path = polygon_path(&[
        [142.0, 22.0],
        [190.0, 12.0],
        [218.0, 44.0],
        [196.0, 78.0],
        [150.0, 70.0],
    ]);

    let scene_graph = SceneGraph {
        width: 246.0,
        height: 162.0,
        origin: [0.0, 0.0],
        marks: vec![
            rect_mark(
                vec![14.0],
                vec![18.0],
                vec![48.0],
                vec![48.0],
                vec![ColorOrGradient::Color([0.82, 0.88, 0.96, 1.0])],
                vec![Some(pattern.clone())],
            )
            .into(),
            SceneArcMark {
                x: ScalarOrArray::new_scalar(96.0),
                y: ScalarOrArray::new_scalar(44.0),
                start_angle: ScalarOrArray::new_scalar(0.25),
                end_angle: ScalarOrArray::new_scalar(std::f32::consts::PI * 1.78),
                outer_radius: ScalarOrArray::new_scalar(31.0),
                inner_radius: ScalarOrArray::new_scalar(11.0),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.92, 0.86, 0.74, 1.0])),
                fill_pattern: ScalarOrArray::new_scalar(Some(pattern.clone())),
                stroke: ScalarOrArray::new_scalar(STROKE),
                stroke_width: ScalarOrArray::new_scalar(1.0),
                ..Default::default()
            }
            .into(),
            ScenePathMark {
                path: ScalarOrArray::new_scalar(path),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.82, 0.92, 0.84, 1.0])),
                fill_pattern: ScalarOrArray::new_scalar(Some(pattern.clone())),
                stroke: ScalarOrArray::new_scalar(STROKE),
                stroke_width: Some(1.0),
                transform: ScalarOrArray::new_scalar(PathTransform::identity()),
                ..Default::default()
            }
            .into(),
            SceneSymbolMark {
                shapes: vec![SymbolShape::from_vega_str("diamond").unwrap()],
                shape_index: ScalarOrArray::new_scalar(0),
                x: ScalarOrArray::new_scalar(52.0),
                y: ScalarOrArray::new_scalar(122.0),
                size: ScalarOrArray::new_scalar(2200.0),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.93, 0.82, 0.9, 1.0])),
                fill_pattern: ScalarOrArray::new_scalar(Some(pattern.clone())),
                stroke: ScalarOrArray::new_scalar(STROKE),
                stroke_width: Some(1.0),
                ..Default::default()
            }
            .into(),
            SceneAreaMark {
                len: 5,
                x: ScalarOrArray::new_array(vec![132.0, 154.0, 176.0, 198.0, 220.0]),
                y: ScalarOrArray::new_array(vec![132.0, 106.0, 124.0, 96.0, 118.0]),
                y2: ScalarOrArray::new_array(vec![148.0, 150.0, 147.0, 151.0, 149.0]),
                fill: ColorOrGradient::Color([0.78, 0.91, 0.9, 1.0]),
                fill_pattern: Some(pattern),
                stroke: STROKE,
                stroke_width: 1.0,
                stroke_cap: StrokeCap::Butt,
                stroke_join: StrokeJoin::Round,
                ..Default::default()
            }
            .into(),
        ],
    };

    assert_scene_graph_visual_match_default(
        &scene_graph,
        BASELINE_CATEGORY,
        "pattern_filled_mark_clipping",
    )
    .await;
}

#[tokio::test]
async fn pattern_ink_opacity() {
    let auto = PatternFill {
        anchor: PatternAnchor::Mark,
        ink: PatternInk::AutoContrast { opacity: 0.42 },
        layers: vec![stripe(45.0, 10.0, 2.0)],
    };
    let explicit_blue = PatternFill {
        anchor: PatternAnchor::Mark,
        ink: PatternInk::Solid {
            color: [0.0, 0.21, 0.65, 1.0],
            opacity: 0.28,
        },
        layers: vec![stripe(90.0, 8.0, 2.0)],
    };
    let overlap = solid_pattern(
        vec![stripe(30.0, 10.0, 3.0), stripe(120.0, 10.0, 3.0)],
        0.34,
    );

    let scene_graph = SceneGraph {
        width: 220.0,
        height: 72.0,
        origin: [0.0, 0.0],
        marks: vec![
            rect_mark(
                vec![12.0, 64.0, 116.0, 168.0],
                vec![14.0, 14.0, 14.0, 14.0],
                vec![40.0, 40.0, 40.0, 40.0],
                vec![44.0, 44.0, 44.0, 44.0],
                vec![
                    ColorOrGradient::Color([0.96, 0.95, 0.9, 1.0]),
                    ColorOrGradient::Color([0.12, 0.14, 0.17, 1.0]),
                    ColorOrGradient::Color([0.85, 0.9, 0.97, 0.62]),
                    ColorOrGradient::Color([0.92, 0.86, 0.74, 1.0]),
                ],
                vec![
                    Some(auto.clone()),
                    Some(auto),
                    Some(explicit_blue),
                    Some(overlap),
                ],
            )
            .into(),
        ],
    };

    assert_scene_graph_visual_match_default(&scene_graph, BASELINE_CATEGORY, "pattern_ink_opacity")
        .await;
}

#[tokio::test]
async fn pattern_facet_plot_anchor() {
    let pattern = PatternFill {
        anchor: PatternAnchor::Plot,
        ink: PatternInk::Solid {
            color: [0.0, 0.0, 0.0, 1.0],
            opacity: 0.45,
        },
        layers: vec![stripe_with_phase(45.0, 11.0, 2.0, 2.0)],
    };

    let facet_rects = |offset_x: f32| {
        rect_mark(
            vec![offset_x + 8.0, offset_x + 52.0, offset_x + 96.0],
            vec![18.0, 18.0, 18.0],
            vec![44.0, 44.0, 44.0],
            vec![58.0, 34.0, 48.0],
            vec![
                ColorOrGradient::Color([0.82, 0.88, 0.96, 1.0]),
                ColorOrGradient::Color([0.92, 0.86, 0.74, 1.0]),
                ColorOrGradient::Color([0.82, 0.92, 0.84, 1.0]),
            ],
            vec![
                Some(pattern.clone()),
                Some(pattern.clone()),
                Some(pattern.clone()),
            ],
        )
    };

    let scene_graph = SceneGraph {
        width: 296.0,
        height: 96.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneGroup {
                pattern_reference_frame: Some(PatternReferenceFrame {
                    x: 8.0,
                    y: 12.0,
                    width: 132.0,
                    height: 70.0,
                }),
                marks: vec![facet_rects(0.0).into()],
                ..Default::default()
            }
            .into(),
            SceneGroup {
                pattern_reference_frame: Some(PatternReferenceFrame {
                    x: 156.0,
                    y: 12.0,
                    width: 132.0,
                    height: 70.0,
                }),
                marks: vec![facet_rects(148.0).into()],
                ..Default::default()
            }
            .into(),
        ],
    };

    assert_scene_graph_visual_match_default(
        &scene_graph,
        BASELINE_CATEGORY,
        "pattern_facet_plot_anchor",
    )
    .await;
}

#[tokio::test]
async fn pattern_theme_scale_legend() {
    let ctx = SessionContext::new();
    let df = pattern_theme_data(&ctx);
    let theme = Theme::from_css(
        r##"
        mark[type="rect"] {
            fill-discrete: #4477AA, #EE6677, #228833, #CCBB44;
            fill-pattern-discrete:
                {
                    anchor: mark;
                    ink: { type: solid; color: #111827; opacity: 0.18; };
                    layers: [
                        { type: stripe; angle: 45deg; spacing: 14px; stroke-width: 1.5px; }
                    ];
                },
                none,
                {
                    anchor: mark;
                    ink: { type: solid; color: #111827; opacity: 0.18; };
                    layers: [
                        { type: stripe; angle: 135deg; spacing: 14px; stroke-width: 1.5px; }
                    ];
                },
                {
                    anchor: mark;
                    ink: { type: solid; color: #111827; opacity: 0.16; };
                    layers: [
                        { type: stripe; angle: 0deg; spacing: 12px; stroke-width: 1.25px; },
                        { type: stripe; angle: 90deg; spacing: 12px; stroke-width: 1.25px; }
                    ];
                };
        }

        legend[type="rect"] {
            pattern-legend-size: 30px;
        }
        "##,
    )
    .expect("pattern theme CSS");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .canvas_size(500.0, 280.0)
        .theme(theme)
        .legend("fill", |legend| {
            legend.title("Series").position(LegendPosition::Right)
        })
        .mark(
            Rect::new()
                .x_with(col("product"), |channel| {
                    channel
                        .scale_with::<Band>(|scale| scale.padding_inner(0.16))
                        .band(0.0)
                        .axis(|axis| axis.title("Product").grid(false))
                })
                .x2_with(col(":x"), |channel| channel.band(1.0))
                .y_with(lit(0.0), |channel| {
                    channel
                        .scale(|scale| scale.domain((0.0, 80.0)))
                        .axis(|axis| axis.title("Value"))
                })
                .y2(col("value"))
                .fill(col("series"))
                .fill_pattern(col("series"))
                .stroke("#111827")
                .stroke_width(1.0),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile pattern theme plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        BASELINE_CATEGORY,
        "pattern_theme_scale_legend",
    )
    .await;
}

#[tokio::test]
async fn pattern_symbol_layer_lattice() {
    let filled = solid_pattern(
        vec![symbol_layer(
            "circle",
            SymbolPaint::Filled,
            0.0,
            90.0,
            0.0,
            0.0,
        )],
        0.48,
    );
    let open = solid_pattern(
        vec![symbol_layer(
            "square",
            SymbolPaint::Open { stroke_width: 1.1 },
            18.0,
            104.0,
            3.0,
            5.0,
        )],
        0.58,
    );
    let path = polygon_path(&[[136.0, 24.0], [198.0, 12.0], [222.0, 62.0], [168.0, 82.0]]);

    let scene_graph = SceneGraph {
        width: 240.0,
        height: 100.0,
        origin: [0.0, 0.0],
        marks: vec![
            rect_mark(
                vec![12.0],
                vec![18.0],
                vec![52.0],
                vec![58.0],
                vec![ColorOrGradient::Color([0.82, 0.88, 0.96, 1.0])],
                vec![Some(filled)],
            )
            .into(),
            SceneSymbolMark {
                shapes: vec![SymbolShape::from_vega_str("triangle-up").unwrap()],
                shape_index: ScalarOrArray::new_scalar(0),
                x: ScalarOrArray::new_scalar(100.0),
                y: ScalarOrArray::new_scalar(48.0),
                size: ScalarOrArray::new_scalar(2700.0),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.93, 0.82, 0.9, 1.0])),
                fill_pattern: ScalarOrArray::new_scalar(Some(open.clone())),
                stroke: ScalarOrArray::new_scalar(STROKE),
                stroke_width: Some(1.0),
                ..Default::default()
            }
            .into(),
            ScenePathMark {
                path: ScalarOrArray::new_scalar(path),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.82, 0.92, 0.84, 1.0])),
                fill_pattern: ScalarOrArray::new_scalar(Some(open)),
                stroke: ScalarOrArray::new_scalar(STROKE),
                stroke_width: Some(1.0),
                transform: ScalarOrArray::new_scalar(PathTransform::identity()),
                ..Default::default()
            }
            .into(),
        ],
    };

    assert_scene_graph_visual_match_default(
        &scene_graph,
        BASELINE_CATEGORY,
        "pattern_symbol_layer_lattice",
    )
    .await;
}

#[tokio::test]
async fn pattern_symbol_stripe_opacity_union() {
    let mixed = solid_pattern(
        vec![
            stripe(0.0, 10.0, 4.0),
            symbol_layer("square", SymbolPaint::Filled, 0.0, 90.0, 0.0, 0.0),
        ],
        0.32,
    );
    let stripe_only = solid_pattern(vec![stripe(0.0, 10.0, 4.0)], 0.32);
    let symbol_only = solid_pattern(
        vec![symbol_layer(
            "square",
            SymbolPaint::Filled,
            0.0,
            90.0,
            0.0,
            0.0,
        )],
        0.32,
    );

    let scene_graph = SceneGraph {
        width: 178.0,
        height: 74.0,
        origin: [0.0, 0.0],
        marks: vec![
            rect_mark(
                vec![12.0, 66.0, 120.0],
                vec![14.0, 14.0, 14.0],
                vec![44.0, 44.0, 44.0],
                vec![46.0, 46.0, 46.0],
                vec![
                    ColorOrGradient::Color([0.9, 0.86, 0.76, 1.0]),
                    ColorOrGradient::Color([0.9, 0.86, 0.76, 1.0]),
                    ColorOrGradient::Color([0.9, 0.86, 0.76, 1.0]),
                ],
                vec![Some(mixed), Some(stripe_only), Some(symbol_only)],
            )
            .into(),
        ],
    };

    assert_scene_graph_visual_match_default(
        &scene_graph,
        BASELINE_CATEGORY,
        "pattern_symbol_stripe_opacity_union",
    )
    .await;
}
