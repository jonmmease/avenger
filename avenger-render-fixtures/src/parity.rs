//! Shared inputs for SVG, WGPU, and browser parity checks.
use avenger_annotation::leader::{make_text_leaders, LeaderStyle, LeaderTarget};
use avenger_color::{ColorOrGradient as C, Gradient, GradientStop, LinearGradient, RadialGradient};
use avenger_common::{
    types::{StrokeCap, StrokeJoin, SymbolShape},
    value::ScalarOrArray as S,
};
use avenger_scenegraph::{
    marks::{
        area::SceneAreaMark,
        group::{Clip, SceneGroup},
        image::SceneImageMark,
        line::SceneLineMark,
        mark::SceneMark,
        path::ScenePathMark,
        pattern::*,
        rect::SceneRectMark,
        rule::SceneRuleMark,
        symbol::SceneSymbolMark,
        text::SceneTextMark,
        trail::SceneTrailMark,
    },
    scene_graph::SceneGraph,
};
use lyon_path::{math::point, Path};

pub struct Case {
    pub name: String,
    pub scene: SceneGraph,
    /// Colors at scene points.
    pub samples: Vec<([u32; 2], [u8; 4])>,
    /// Whether resvg draws the case's SVG, so that other backends compare against it. resvg
    /// ignores a radial gradient's starting radius.
    pub resvg_reference: bool,
}

impl Case {
    fn new(name: impl Into<String>, marks: Vec<SceneMark>) -> Self {
        Self {
            name: name.into(),
            scene: SceneGraph {
                width: 420.0,
                height: 240.0,
                origin: [0.0; 2],
                marks,
            },
            samples: Vec::new(),
            resvg_reference: true,
        }
    }

    fn sized(mut self, width: f32, height: f32) -> Self {
        self.scene.width = width;
        self.scene.height = height;
        self
    }

    fn with_samples(mut self, samples: Vec<([u32; 2], [u8; 4])>) -> Self {
        self.samples = samples;
        self
    }

    /// Asserts that `image`, the case drawn at two pixels per unit, shows the samples within
    /// `tolerance` per channel, and that under 1% of its pixels differ from `reference` by more
    /// than 20, which allows for edge antialiasing.
    pub fn assert_image(
        &self,
        image: &image::RgbaImage,
        reference: Option<&image::RgbaImage>,
        tolerance: u8,
    ) {
        if let Some(reference) = reference {
            let large = image
                .pixels()
                .zip(reference.pixels())
                .filter(|(a, b)| a.0.into_iter().zip(b.0).any(|(a, b)| a.abs_diff(b) > 20))
                .count();
            assert!(
                large < image.pixels().len() / 100,
                "{}: {large} pixels differ beyond edge tolerance",
                self.name
            );
        }
        for (point, expected) in &self.samples {
            let actual = image.get_pixel(point[0] * 2, point[1] * 2).0;
            assert!(
                actual
                    .into_iter()
                    .zip(*expected)
                    .all(|(a, b)| a.abs_diff(b) <= tolerance),
                "{} {point:?}: {actual:?} != {expected:?}",
                self.name
            );
        }
    }
}

fn compound(opposite: bool, overlap: bool) -> Path {
    let mut builder = Path::builder();
    for (i, [x, y, w, h]) in [
        [30.0, 30.0, 280.0, 180.0],
        if overlap {
            [160.0, 70.0, 220.0, 100.0]
        } else {
            [110.0, 70.0, 120.0, 100.0]
        },
    ]
    .into_iter()
    .enumerate()
    {
        let mut corners = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        if i == 1 && opposite {
            corners.reverse();
        }
        builder.begin(point(corners[0][0], corners[0][1]));
        for p in &corners[1..] {
            builder.line_to(point(p[0], p[1]));
        }
        builder.close();
    }
    builder.build()
}

fn stops() -> Vec<GradientStop> {
    vec![
        GradientStop {
            offset: 0.0,
            color: [1.0, 0.0, 0.0, 1.0],
        },
        GradientStop {
            offset: 1.0,
            color: [0.0, 0.0, 1.0, 1.0],
        },
    ]
}

fn linear() -> Gradient {
    Gradient::LinearGradient(LinearGradient {
        x0: 0.0,
        y0: 0.0,
        x1: 1.0,
        y1: 1.0,
        stops: stops(),
    })
}
fn rect() -> SceneRectMark {
    SceneRectMark {
        x: 30.0.into(),
        y: 30.0.into(),
        width: Some(360.0.into()),
        height: Some(180.0.into()),
        fill: C::Color([0.2, 0.5, 0.8, 1.0]).into(),
        ..Default::default()
    }
}
fn pattern(opacity: f32) -> PatternFill {
    PatternFill {
        anchor: PatternAnchor::Mark,
        ink: PatternInk::Solid {
            color: [0.0; 4],
            opacity,
        },
        layers: vec![StripeLayer::new(45.0, 16.0, 6.0).into()],
    }
}

pub fn cases() -> Vec<Case> {
    let blue = [51, 128, 204, 255];
    let white = [255; 4];
    let mut cases = Vec::new();
    // Fills and path clips use the nonzero rule: an inner square with the outer square's winding
    // stays filled, and one with the opposite winding is a hole.
    for (label, opposite, overlap) in [
        ("nested", false, false),
        ("hole", true, false),
        ("overlap", false, true),
    ] {
        let center = if opposite { white } else { blue };
        cases.push(
            Case::new(
                format!("fill-{label}"),
                vec![ScenePathMark {
                    path: compound(opposite, overlap).into(),
                    fill: C::Color([0.2, 0.5, 0.8, 1.0]).into(),
                    ..Default::default()
                }
                .into()],
            )
            .with_samples(vec![([190, 120], center), ([50, 50], blue)]),
        );
    }
    cases.push(
        Case::new(
            "clip-nested",
            vec![SceneGroup {
                clip: Clip::Path(compound(false, false)),
                marks: vec![rect().into()],
                ..Default::default()
            }
            .into()],
        )
        .with_samples(vec![([190, 120], blue)]),
    );
    let mut xor = pattern(0.5);
    xor.ink = PatternInk::Solid {
        color: [0.0, 0.0, 0.0, 1.0],
        opacity: 0.5,
    };
    xor.layers.push(
        PatternLayer::from(StripeLayer::new(-45.0, 27.0, 12.0))
            .with_operation(PatternLayerOperation::Xor),
    );
    cases.push(Case::new(
        "pattern-host",
        vec![ScenePathMark {
            path: compound(false, false).into(),
            fill: C::Color([0.2, 0.5, 0.8, 1.0]).into(),
            fill_pattern: Some(xor).into(),
            ..Default::default()
        }
        .into()],
    ));
    for count in [1, 100] {
        let path = compound(false, false)
            .transformed(&avenger_common::types::PathTransform::scale(0.005, 0.005));
        let mut xs = vec![1000.0; count];
        xs[0] = 0.0;
        cases.push(
            Case::new(
                format!("symbol-{count}"),
                vec![SceneSymbolMark {
                    len: count as u32,
                    shapes: vec![SymbolShape::Path(path)],
                    x: S::new_array(xs),
                    size: 40000.0.into(),
                    fill: C::Color([0.2, 0.5, 0.8, 1.0]).into(),
                    stroke_width: None,
                    ..Default::default()
                }
                .into()],
            )
            .with_samples(vec![([190, 120], blue)]),
        );
    }
    // An item with a non-finite coordinate draws nothing, and the scene's other items still draw.
    let fill = C::Color([0.2, 0.5, 0.8, 1.0]);
    let nan = f32::NAN;
    let red = avenger_image::RgbaImage {
        width: 1,
        height: 1,
        data: vec![255, 0, 0, 255],
    };
    cases.push(
        Case::new(
            "non-finite",
            vec![
                SceneSymbolMark {
                    len: 2,
                    x: S::new_array(vec![60.0, nan]),
                    y: 60.0.into(),
                    size: 1600.0.into(),
                    fill: fill.clone().into(),
                    ..Default::default()
                }
                .into(),
                SceneRectMark {
                    len: 2,
                    x: S::new_array(vec![nan, 130.0]),
                    y: 40.0.into(),
                    width: Some(40.0.into()),
                    height: Some(40.0.into()),
                    fill: fill.clone().into(),
                    ..Default::default()
                }
                .into(),
                SceneImageMark {
                    len: 2,
                    image: red.into(),
                    x: S::new_array(vec![210.0, nan]),
                    y: 40.0.into(),
                    width: 40.0.into(),
                    height: 40.0.into(),
                    aspect: false,
                    ..Default::default()
                }
                .into(),
                SceneTextMark {
                    text: "label".to_string().into(),
                    x: nan.into(),
                    y: 60.0.into(),
                    ..Default::default()
                }
                .into(),
                SceneTrailMark {
                    len: 3,
                    x: S::new_array(vec![300.0, 350.0, 400.0]),
                    y: S::new_array(vec![60.0, nan, 60.0]),
                    size: 10.0.into(),
                    stroke: fill.clone(),
                    ..Default::default()
                }
                .into(),
                SceneLineMark {
                    len: 3,
                    x: S::new_array(vec![30.0, 200.0, 390.0]),
                    y: S::new_array(vec![150.0, f32::INFINITY, 150.0]),
                    stroke_width: 10.0,
                    stroke: fill.clone(),
                    ..Default::default()
                }
                .into(),
                SceneAreaMark {
                    len: 3,
                    x: S::new_array(vec![30.0, 200.0, 390.0]),
                    y: S::new_array(vec![190.0, nan, 190.0]),
                    y2: 230.0.into(),
                    fill,
                    ..Default::default()
                }
                .into(),
            ],
        )
        .with_samples(vec![
            ([60, 60], blue),
            ([150, 60], blue),
            ([230, 60], [255, 0, 0, 255]),
            ([320, 60], white),
            ([100, 150], white),
            ([100, 210], white),
        ]),
    );
    cases.push(
        Case::new(
            "clip-empty",
            vec![SceneGroup {
                clip: Clip::Rect {
                    x: 30.0,
                    y: 30.0,
                    width: 0.0,
                    height: 180.0,
                },
                marks: vec![rect().into()],
                ..Default::default()
            }
            .into()],
        )
        .with_samples(vec![([210, 120], white)]),
    );
    // Two crossing stripe layers, with a sample where both cover, where each covers alone, and
    // where neither does. Their ink's opacity applies once.
    let ink = [128, 128, 128, 255];
    for (operation, samples) in [
        (PatternLayerOperation::Add, [ink, ink, ink, white]),
        (PatternLayerOperation::Subtract, [white, ink, white, white]),
        (PatternLayerOperation::Xor, [white, ink, ink, white]),
    ] {
        let mark = SceneRectMark {
            width: Some(100.0.into()),
            height: Some(100.0.into()),
            fill: C::Color([1.0; 4]).into(),
            fill_pattern: Some(PatternFill {
                anchor: PatternAnchor::Mark,
                ink: PatternInk::Solid {
                    color: [0.0, 0.0, 0.0, 1.0],
                    opacity: 0.5,
                },
                layers: vec![
                    StripeLayer::new(0.0, 20.0, 8.0).into(),
                    PatternLayer::from(StripeLayer::new(90.0, 20.0, 8.0)).with_operation(operation),
                ],
            })
            .into(),
            ..Default::default()
        };
        let points = [[40, 40], [50, 40], [40, 50], [50, 50]];
        cases.push(
            Case::new(format!("pattern-{operation:?}"), vec![mark.into()])
                .sized(100.0, 100.0)
                .with_samples(points.into_iter().zip(samples).collect()),
        );
    }
    // Squares 40 units wide on lattices close enough for neighbors to overlap, so a layer's
    // instances must unite before its operation applies.
    for (paint, spacing, ink_point, gap_point) in [
        (SymbolPaint::Filled, 30.0, [65, 50], [50, 20]),
        (
            SymbolPaint::Open { stroke_width: 8.0 },
            38.0,
            [69, 50],
            [50, 50],
        ),
    ] {
        for operation in [
            PatternLayerOperation::Add,
            PatternLayerOperation::Subtract,
            PatternLayerOperation::Xor,
        ] {
            let mut layers = Vec::new();
            if operation != PatternLayerOperation::Add {
                layers.push(StripeLayer::new(0.0, 1000.0, 1000.0).into());
            }
            let symbols = SymbolLayer {
                lattice: SymbolLattice2d {
                    u_spacing: spacing,
                    v_spacing: 100.0,
                    u_angle: 0.0,
                    v_angle: 90.0,
                    u_phase: 50.0,
                    v_phase: 50.0,
                },
                symbol: PatternSymbol {
                    shape: PatternSymbolShape::Square,
                    size: 1600.0,
                    rotation: 0.0,
                },
                paint: paint.clone(),
            };
            layers.push(PatternLayer::from(symbols).with_operation(operation));
            let mark = SceneRectMark {
                width: Some(100.0.into()),
                height: Some(100.0.into()),
                fill: C::Color([1.0; 4]).into(),
                fill_pattern: Some(PatternFill {
                    anchor: PatternAnchor::Mark,
                    ink: PatternInk::Solid {
                        color: [0.0, 0.0, 0.0, 1.0],
                        opacity: 0.5,
                    },
                    layers,
                })
                .into(),
                ..Default::default()
            };
            let add = operation == PatternLayerOperation::Add;
            let [inked, gap] = if add { [ink, white] } else { [white, ink] };
            let paint = if paint == SymbolPaint::Filled {
                "filled"
            } else {
                "open"
            };
            cases.push(
                Case::new(
                    format!("pattern-union-{paint}-{operation:?}"),
                    vec![mark.into()],
                )
                .sized(100.0, 100.0)
                .with_samples(vec![(ink_point, inked), (gap_point, gap)]),
            );
        }
    }
    for (name, xs, ys, widths, defined) in [
        (
            "trail",
            vec![50., 160., 270., 370.],
            vec![150., 80., 160., 60.],
            vec![10., 100., 20., 70.],
            vec![true; 4],
        ),
        (
            "trail-cap",
            vec![80., 300.],
            vec![120., 120.],
            vec![80., 80.],
            vec![true; 2],
        ),
        (
            "trail-crossing",
            vec![60., 320., 60., 320.],
            vec![60., 180., 180., 60.],
            vec![70., 90., 30., 70.],
            vec![true; 4],
        ),
        (
            "trail-break",
            vec![60., 160., 210., 300.],
            vec![120.; 4],
            vec![40.; 4],
            vec![true, true, false, true],
        ),
    ] {
        let gray = [128, 128, 128, 255];
        let samples = match name {
            "trail-cap" => vec![([50, 120], gray), ([330, 120], gray), ([30, 120], white)],
            "trail-break" => vec![([210, 120], white), ([300, 120], gray)],
            "trail-crossing" => vec![([190, 120], gray)],
            _ => vec![],
        };
        cases.push(
            Case::new(
                name,
                vec![SceneTrailMark {
                    len: xs.len() as u32,
                    x: S::new_array(xs),
                    y: S::new_array(ys),
                    size: S::new_array(widths),
                    defined: S::new_array(defined),
                    stroke: C::Color([0.0, 0.0, 0.0, 0.5]),
                    ..Default::default()
                }
                .into()],
            )
            .with_samples(samples),
        );
    }
    let radial = Gradient::RadialGradient(RadialGradient {
        x0: 0.5,
        y0: 0.5,
        x1: 0.5,
        y1: 0.5,
        r0: 0.0,
        r1: 0.5,
        stops: stops(),
    });
    // Trails place gradients on their centerline's bounds, and symbols on their nominal square,
    // whatever their shape and rotation, including when an invisible pattern draws over them.
    for (kind, gradient) in [("linear", linear()), ("radial", radial)] {
        cases.push(Case::new(
            format!("trail-{kind}"),
            vec![SceneTrailMark {
                len: 4,
                x: S::new_array(vec![50., 160., 270., 370.]),
                y: S::new_array(vec![150., 80., 160., 60.]),
                size: S::new_array(vec![10., 100., 20., 70.]),
                stroke: C::GradientIndex(0),
                gradients: vec![gradient.clone()],
                ..Default::default()
            }
            .into()],
        ));
        for patterned in [false, true] {
            cases.push(Case::new(
                format!("symbol-{kind}{}", if patterned { "-pattern" } else { "" }),
                vec![SceneGroup {
                    origin: [10.0, 5.0],
                    marks: vec![SceneSymbolMark {
                        len: 2,
                        shapes: vec![
                            SymbolShape::from_vega_str("star").unwrap(),
                            SymbolShape::from_vega_str("square").unwrap(),
                        ],
                        shape_index: S::new_array(vec![0, 1]),
                        x: S::new_array(vec![120., 290.]),
                        y: 115.0.into(),
                        size: S::new_array(vec![9000., 16000.]),
                        angle: S::new_array(vec![0., 45.]),
                        fill: C::GradientIndex(0).into(),
                        gradients: vec![gradient.clone()],
                        fill_pattern: if patterned { Some(pattern(0.0)) } else { None }.into(),
                        ..Default::default()
                    }
                    .into()],
                    ..Default::default()
                }
                .into()],
            ));
        }
    }
    for (name, p0, p1, r0, r1) in [
        ("radial-offset-inner", [0.3, 0.5], [0.5, 0.5], 0.2, 0.5),
        ("radial-concentric", [0.5, 0.5], [0.5, 0.5], 0.2, 0.5),
        ("radial-zero", [0.3, 0.5], [0.5, 0.5], 0., 0.5),
        ("radial-tangent", [0.25, 0.5], [0.5, 0.5], 0.25, 0.5),
        ("radial-separated", [0.9, 0.5], [0.3, 0.5], 0.1, 0.25),
        ("radial-reversed", [0.5, 0.5], [0.3, 0.5], 0.5, 0.2),
        ("radial-identical", [0.5, 0.5], [0.5, 0.5], 0.3, 0.3),
    ] {
        let g = RadialGradient {
            x0: p0[0],
            y0: p0[1],
            x1: p1[0],
            y1: p1[1],
            r0,
            r1,
            stops: stops(),
        };
        let mut mark = rect();
        mark.fill = C::GradientIndex(0).into();
        mark.gradients = vec![Gradient::RadialGradient(g.clone())];
        // Axis-aligned circle intersections give these parameters directly.
        let mut samples = match name {
            "radial-offset-inner" => vec![
                ([100, 120], [255, 0, 0, 255]),
                ([210, 120], [255, 0, 0, 255]),
                ([300, 120], [127, 0, 128, 255]),
                ([380, 35], [0, 0, 255, 255]),
            ],
            "radial-concentric" => vec![([318, 120], [170, 0, 85, 255])],
            "radial-zero" => vec![([264, 120], [127, 0, 128, 255])],
            "radial-tangent" | "radial-reversed" => vec![([300, 120], [127, 0, 128, 255])],
            "radial-separated" => vec![([282, 120], [85, 0, 170, 255]), ([210, 200], white)],
            "radial-identical" => vec![([210, 120], white)],
            _ => unreachable!(),
        };
        for point in [
            [80, 60],
            [210, 60],
            [340, 60],
            [80, 180],
            [210, 180],
            [340, 180],
        ] {
            let xy = [
                (point[0] as f64 + 0.25 - 30.0) / 360.0,
                (point[1] as f64 + 0.25 + 60.0) / 360.0,
            ];
            samples.push((point, radial_sample(&g, xy)));
        }
        let mut case = Case::new(name, vec![mark.into()]).with_samples(samples);
        case.resvg_reference = false;
        cases.push(case);
    }
    // Nearly identical circles, whose precision the backends keep at small and large scales.
    for (label, left, top, side) in [("small", 20., 20., 80.), ("large", 500., 400., 720.)] {
        let g = RadialGradient {
            x0: 0.501,
            y0: 0.5,
            x1: 0.502,
            y1: 0.5,
            r0: 0.1001,
            r1: 0.1008,
            stops: stops(),
        };
        let samples = [
            [0.15, 0.2],
            [0.5, 0.2],
            [0.85, 0.2],
            [0.5, 0.5],
            [0.85, 0.8],
        ]
        .into_iter()
        .map(|[u, v]: [f32; 2]| {
            let point = [(left + side * u) as u32, (top + side * v) as u32];
            let xy = [
                (point[0] as f64 + 0.25 - left as f64) / side as f64,
                (point[1] as f64 + 0.25 - top as f64) / side as f64,
            ];
            (point, radial_sample(&g, xy))
        })
        .collect();
        let mut case = Case::new(
            format!("radial-precision-{label}"),
            vec![SceneRectMark {
                x: left.into(),
                y: top.into(),
                width: Some(side.into()),
                height: Some(side.into()),
                fill: C::GradientIndex(0).into(),
                gradients: vec![Gradient::RadialGradient(g)],
                ..Default::default()
            }
            .into()],
        )
        .sized(left + side + 20., top + side + 20.)
        .with_samples(samples);
        case.resvg_reference = false;
        cases.push(case);
    }
    // Gradients with no stops paint nothing, and constant gradients paint their last stop.
    let translucent_blue = [128, 128, 255, 255];
    let one_stop = vec![GradientStop {
        offset: 0.4,
        color: [0.0, 0.0, 1.0, 0.5],
    }];
    let two_stops = vec![
        GradientStop {
            offset: 0.0,
            color: [1.0, 0.0, 0.0, 1.0],
        },
        GradientStop {
            offset: 1.0,
            color: [0.0, 0.0, 1.0, 0.5],
        },
    ];
    let linear_at = |x1, stops| {
        Gradient::LinearGradient(LinearGradient {
            x0: 0.5,
            y0: 0.5,
            x1,
            y1: 0.5,
            stops,
        })
    };
    let radial_with = |stops| {
        Gradient::RadialGradient(RadialGradient {
            x0: 0.5,
            y0: 0.5,
            x1: 0.5,
            y1: 0.5,
            r0: 0.0,
            r1: 0.5,
            stops,
        })
    };
    for (name, gradient, expected) in [
        ("linear-no-stops", linear_at(1.0, vec![]), white),
        (
            "linear-one-stop",
            linear_at(1.0, one_stop.clone()),
            translucent_blue,
        ),
        (
            "linear-coincident",
            linear_at(0.5, two_stops),
            translucent_blue,
        ),
        ("radial-no-stops", radial_with(vec![]), white),
        ("radial-one-stop", radial_with(one_stop), translucent_blue),
    ] {
        let mut mark = rect();
        mark.gradients = vec![gradient];
        mark.fill = C::GradientIndex(0).into();
        cases.push(
            Case::new(format!("gradient-{name}"), vec![mark.into()])
                .with_samples(vec![([210, 120], expected)]),
        );
    }
    for (name, xs) in [
        ("miter", vec![80., 110., 140.]),
        ("miter-bevel", vec![100., 110., 120.]),
    ] {
        cases.push(Case::new(
            name,
            vec![SceneLineMark {
                len: 3,
                x: S::new_array(xs),
                y: S::new_array(vec![210., 120., 210.]),
                stroke_width: 12.,
                stroke_join: StrokeJoin::Miter,
                ..Default::default()
            }
            .into()],
        ));
    }
    // A red pixel beside a transparent blue one, which smoothing mustn't mix with blue.
    for (name, smooth) in [("image-nearest", false), ("image-smooth", true)] {
        let case = Case::new(
            name,
            vec![SceneImageMark {
                image: avenger_image::RgbaImage {
                    width: 2,
                    height: 1,
                    data: vec![255, 0, 0, 255, 0, 0, 255, 0],
                }
                .into(),
                x: 30.0.into(),
                y: 30.0.into(),
                width: 360.0.into(),
                height: 180.0.into(),
                smooth,
                aspect: false,
                ..Default::default()
            }
            .into()],
        );
        cases.push(if smooth {
            case.with_samples(vec![([210, 120], [255, 128, 128, 255])])
        } else {
            case
        });
    }
    // Isolated samples and zero-length dashes keep their round and square caps.
    let ink = [140, 153, 229, 255];
    for cap in [StrokeCap::Butt, StrokeCap::Round, StrokeCap::Square] {
        let capped = if cap == StrokeCap::Butt { white } else { ink };
        for dashed in [false, true] {
            cases.push(
                Case::new(
                    format!("isolated-{cap:?}{}", if dashed { "-dashed" } else { "" }),
                    vec![SceneLineMark {
                        len: 6,
                        x: S::new_array(vec![30., 80., 140., 210., 310., 350.]),
                        y: S::new_array(vec![30., 140., 80., 180., 60., 140.]),
                        defined: S::new_array(vec![true, false, true, true, false, true]),
                        stroke: C::Color([0.1, 0.2, 0.8, 0.5]),
                        stroke_width: 24.,
                        stroke_cap: cap,
                        stroke_join: StrokeJoin::Round,
                        stroke_dash: dashed.then(|| vec![14., 7., 3.]),
                        ..Default::default()
                    }
                    .into()],
                )
                .with_samples(vec![([30, 30], capped), ([350, 140], capped)]),
            );
        }
        let square = if cap == StrokeCap::Square { ink } else { white };
        cases.push(
            Case::new(
                format!("zero-dash-{cap:?}"),
                vec![SceneRuleMark {
                    x: 60.0.into(),
                    y: 60.0.into(),
                    x2: 200.0.into(),
                    y2: 200.0.into(),
                    stroke: C::Color([0.1, 0.2, 0.8, 0.5]).into(),
                    stroke_width: 24.0.into(),
                    stroke_cap: cap.into(),
                    stroke_dash: Some(S::new_scalar(vec![0., 30.])),
                    ..Default::default()
                }
                .into()],
            )
            .with_samples(vec![([60, 60], capped), ([74, 60], square)]),
        );
    }
    // Translucent paint applies once where a dashed line crosses itself.
    for (name, stroke, expected) in [
        ("translucent", C::Color([0.1, 0.2, 0.8, 0.5]), ink),
        ("gradient", C::GradientIndex(0), [191, 128, 191, 255]),
        ("opaque", C::Color([0.1, 0.2, 0.8, 1.0]), [26, 51, 204, 255]),
    ] {
        let mark = SceneLineMark {
            len: 4,
            x: S::new_array(vec![60., 340., 60., 340.]),
            y: S::new_array(vec![40., 200., 200., 40.]),
            stroke,
            gradients: vec![Gradient::LinearGradient(LinearGradient {
                x0: 0.,
                y0: 0.,
                x1: 1.,
                y1: 0.,
                stops: vec![
                    GradientStop {
                        offset: 0.,
                        color: [1., 0., 0., 0.5],
                    },
                    GradientStop {
                        offset: 1.,
                        color: [0., 0., 1., 0.5],
                    },
                ],
            })],
            stroke_width: 24.,
            stroke_cap: StrokeCap::Square,
            stroke_dash: Some(vec![1000., 1.]),
            ..Default::default()
        };
        cases.push(
            Case::new(format!("dash-crossing-{name}"), vec![mark.into()])
                .with_samples(vec![([200, 120], expected)]),
        );
    }
    // Overlapping dashes of one rule apply translucent paint once, and separate rules twice.
    let rule = SceneRuleMark {
        x: 60.0.into(),
        y: 40.0.into(),
        x2: 340.0.into(),
        y2: 200.0.into(),
        stroke: C::Color([0.1, 0.2, 0.8, 0.5]).into(),
        stroke_width: 24.0.into(),
        stroke_cap: StrokeCap::Square.into(),
        stroke_dash: Some(S::new_scalar(vec![14., 7.])),
        ..Default::default()
    };
    cases.push(
        Case::new("dash-overlap", vec![rule.clone().into()]).with_samples(vec![([75, 49], ink)]),
    );
    cases.push(
        Case::new(
            "dash-overlap-separate",
            vec![rule.clone().into(), rule.clone().into()],
        )
        .with_samples(vec![([75, 49], [83, 102, 217, 255])]),
    );
    let opaque_rule = SceneRuleMark {
        stroke: C::Color([0.1, 0.2, 0.8, 1.0]).into(),
        ..rule
    };
    cases.push(
        Case::new(
            "dash-overlap-opaque",
            vec![
                opaque_rule.clone().into(),
                SceneRuleMark {
                    y: 220.0.into(),
                    y2: 220.0.into(),
                    stroke_width: 6.0.into(),
                    stroke_dash: Some(S::new_scalar(vec![20., 20.])),
                    ..opaque_rule
                }
                .into(),
            ],
        )
        .with_samples(vec![
            ([75, 49], [26, 51, 204, 255]),
            ([70, 220], [26, 51, 204, 255]),
            ([90, 220], white),
        ]),
    );
    for patterned in [false, true] {
        cases.push(
            Case::new(
                format!("dash-area{}", if patterned { "-pattern" } else { "" }),
                vec![SceneAreaMark {
                    len: 2,
                    x: S::new_array(vec![60., 340.]),
                    y: 40.0.into(),
                    y2: 200.0.into(),
                    fill: C::Color([0.0; 4]),
                    fill_pattern: patterned.then(|| pattern(0.0)),
                    stroke: C::Color([0.1, 0.2, 0.8, 0.5]),
                    stroke_width: 24.,
                    stroke_cap: StrokeCap::Square,
                    stroke_dash: Some(vec![0., 7., 14., 7.]),
                    ..Default::default()
                }
                .into()],
            )
            .with_samples(vec![([75, 40], ink)]),
        );
    }
    let label = SceneTextMark {
        text: "label".to_string().into(),
        x: 340.0.into(),
        y: 120.0.into(),
        color: [0.0; 4].into(),
        ..Default::default()
    };
    let style = LeaderStyle {
        stroke: [0.1, 0.2, 0.8, 0.5],
        stroke_width: 24.0,
        stroke_cap: StrokeCap::Square,
        stroke_dash: Some(vec![0., 7., 14., 7.]),
        ..Default::default()
    };
    let engine = avenger_typst_label::bundled_label_engine();
    let target = LeaderTarget {
        position: [60.0, 120.0],
        radius: 0.0,
    };
    let leaders =
        make_text_leaders(&label, &[Some(target)], &style, &engine).expect("the label lays out");
    cases.push(
        Case::new("dash-leader", vec![leaders.into(), label.into()])
            .with_samples(vec![([200, 120], ink)]),
    );
    // Adjacent rule variants reinstall identical symbol geometry through the renderer cache.
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

// Solve |p - (c0 + t*dc)|² = (r0 + t*dr)² in f64 for independent interior probes.
fn radial_sample(g: &RadialGradient, p: [f64; 2]) -> [u8; 4] {
    let dx = g.x1 as f64 - g.x0 as f64;
    let dy = g.y1 as f64 - g.y0 as f64;
    let dr = g.r1 as f64 - g.r0 as f64;
    let px = p[0] - g.x0 as f64;
    let py = p[1] - g.y0 as f64;
    let a = dx * dx + dy * dy - dr * dr;
    let b = -2.0 * (px * dx + py * dy + g.r0 as f64 * dr);
    let c = px * px + py * py - (g.r0 as f64).powi(2);
    let roots = if a == 0.0 {
        vec![-c / b]
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            return [255; 4];
        }
        vec![
            (-b - disc.sqrt()) / (2.0 * a),
            (-b + disc.sqrt()) / (2.0 * a),
        ]
    };
    match roots
        .into_iter()
        .filter(|t| t.is_finite() && g.r0 as f64 + t * dr >= 0.0)
        .max_by(f64::total_cmp)
    {
        Some(t) => {
            let t = t.clamp(0.0, 1.0);
            [
                ((1.0 - t) * 255.0).round() as u8,
                0,
                (t * 255.0).round() as u8,
                255,
            ]
        }
        None => [255; 4],
    }
}
