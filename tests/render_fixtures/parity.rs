//! Shared inputs for SVG, WGPU, and browser parity checks.
use avenger_color::{ColorOrGradient as C, Gradient, GradientStop, LinearGradient, RadialGradient};
use avenger_common::{
    types::{FillRule, StrokeJoin, SymbolShape},
    value::ScalarOrArray as S,
};
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        image::{SceneImageMark, SceneImageSource},
        line::SceneLineMark,
        mark::SceneMark,
        path::ScenePathMark,
        pattern::*,
        rect::SceneRectMark,
        symbol::SceneSymbolMark,
        trail::SceneTrailMark,
    },
    scene_graph::SceneGraph,
};
use lyon_path::{math::point, Path};

pub struct Case {
    pub name: String,
    pub scene: SceneGraph,
    pub samples: Vec<([u32; 2], [u8; 4])>,
    pub browser_only: bool,
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
            browser_only: false,
        }
    }
}

pub fn compound(opposite: bool, overlap: bool) -> Path {
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
        layers: vec![PatternLayer::Stripe(StripePatternLayer::new(
            45.0, 16.0, 6.0,
        ))],
    }
}

pub fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        for (label, opposite, overlap) in [
            ("nested", false, false),
            ("hole", true, false),
            ("overlap", false, true),
        ] {
            let path = compound(opposite, overlap);
            let filled = rule == FillRule::NonZero && !opposite;
            let center = if filled {
                [51, 128, 204, 255]
            } else {
                [255; 4]
            };
            let mut case = Case::new(
                format!("fill-{label}-{rule:?}"),
                vec![ScenePathMark {
                    path: path.clone().into(),
                    fill_rule: rule,
                    fill: C::Color([0.2, 0.5, 0.8, 1.0]).into(),
                    ..Default::default()
                }
                .into()],
            );
            case.samples = vec![([190, 120], center), ([50, 50], [51, 128, 204, 255])];
            cases.push(case);
            let mut case = Case::new(
                format!("clip-{label}-{rule:?}"),
                vec![SceneGroup {
                    clip: Clip::Path {
                        path: path.clone(),
                        fill_rule: rule,
                    },
                    marks: vec![rect().into()],
                    ..Default::default()
                }
                .into()],
            );
            case.samples.push(([190, 120], center));
            cases.push(case);
            if !overlap {
                let mut ink = pattern(0.5);
                ink.ink = PatternInk::Solid {
                    color: [0.0, 0.0, 0.0, 1.0],
                    opacity: 0.5,
                };
                ink.layers.push(PatternLayer::Stripe(StripePatternLayer {
                    operation: PatternLayerOperation::Xor,
                    ..StripePatternLayer::new(-45.0, 27.0, 12.0)
                }));
                cases.push(Case::new(
                    format!("pattern-{label}-{rule:?}"),
                    vec![ScenePathMark {
                        path: path.into(),
                        fill_rule: rule,
                        fill: C::Color([0.2, 0.5, 0.8, 1.0]).into(),
                        fill_pattern: Some(ink).into(),
                        ..Default::default()
                    }
                    .into()],
                ));
            }
        }
        for count in [1, 100] {
            let path = compound(false, false)
                .transformed(&avenger_common::types::PathTransform::scale(0.005, 0.005));
            let mut xs = vec![1000.0; count];
            xs[0] = 0.0;
            let mut case = Case::new(
                format!("symbol-{count}-{rule:?}"),
                vec![SceneSymbolMark {
                    len: count as u32,
                    shapes: vec![SymbolShape::Path(path)],
                    fill_rule: rule,
                    x: S::new_array(xs),
                    size: 40000.0.into(),
                    fill: C::Color([0.2, 0.5, 0.8, 1.0]).into(),
                    stroke_width: None,
                    ..Default::default()
                }
                .into()],
            );
            case.samples.push((
                [190, 120],
                if rule == FillRule::NonZero {
                    [51, 128, 204, 255]
                } else {
                    [255; 4]
                },
            ));
            cases.push(case);
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
            "trail-reversal",
            vec![60., 320., 60.],
            vec![120., 120., 120.],
            vec![40., 100., 60.],
            vec![true; 3],
        ),
        (
            "trail-break",
            vec![60., 160., 210., 300.],
            vec![120.; 4],
            vec![40.; 4],
            vec![true, true, false, true],
        ),
    ] {
        let mut case = Case::new(
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
        );
        if name == "trail-cap" {
            case.samples = vec![
                ([50, 120], [128, 128, 128, 255]),
                ([330, 120], [128, 128, 128, 255]),
                ([30, 120], [255; 4]),
            ];
        }
        if name == "trail-break" {
            case.samples = vec![([210, 120], [255; 4]), ([300, 120], [128, 128, 128, 255])];
        }
        if name == "trail-crossing" || name == "trail-reversal" {
            case.samples.push(([190, 120], [128, 128, 128, 255]));
        }
        cases.push(case);
    }
    cases.push(Case::new(
        "trail-gradient",
        vec![SceneTrailMark {
            len: 4,
            x: S::new_array(vec![50., 160., 270., 370.]),
            y: S::new_array(vec![150., 80., 160., 60.]),
            size: S::new_array(vec![10., 100., 20., 70.]),
            stroke: C::GradientIndex(0),
            gradients: vec![linear()],
            ..Default::default()
        }
        .into()],
    ));
    for (name, shape, angle) in [
        ("star-gradient", "star", 0.0),
        ("symbol-gradient", "square", 45.0),
    ] {
        for patterned in [false, true] {
            cases.push(Case::new(
                format!("{name}-{patterned}"),
                vec![SceneGroup {
                    origin: [10.0, 5.0],
                    marks: vec![SceneSymbolMark {
                        len: 2,
                        shapes: vec![SymbolShape::from_vega_str(shape).unwrap()],
                        x: S::new_array(vec![120., 290.]),
                        y: 115.0.into(),
                        size: S::new_array(vec![9000., 16000.]),
                        angle: angle.into(),
                        fill: C::GradientIndex(0).into(),
                        gradients: vec![linear()],
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
    // Reuse the bounds cases with a native radial paint, including an invisible pattern.
    let radial = Gradient::RadialGradient(RadialGradient {
        x0: 0.5,
        y0: 0.5,
        x1: 0.5,
        y1: 0.5,
        r0: 0.0,
        r1: 0.5,
        stops: stops(),
    });
    let mut radial_bounds = Vec::new();
    for case in &cases {
        if case.name == "trail-gradient" || case.name.starts_with("star-gradient-") {
            let mut scene = case.scene.clone();
            match &mut scene.marks[0] {
                SceneMark::Trail(mark) => mark.gradients = vec![radial.clone()],
                SceneMark::Group(group) => {
                    let SceneMark::Symbol(mark) = &mut group.marks[0] else {
                        unreachable!()
                    };
                    mark.gradients = vec![radial.clone()];
                }
                _ => unreachable!(),
            }
            radial_bounds.push(Case::new(format!("{}-radial", case.name), scene.marks));
        }
    }
    cases.extend(radial_bounds);
    for (name, p0, p1, r0, r1) in [
        ("radial-offset-inner", [0.3, 0.5], [0.5, 0.5], 0.2, 0.5),
        ("radial-concentric", [0.5, 0.5], [0.5, 0.5], 0.2, 0.5),
        ("radial-zero", [0.3, 0.5], [0.5, 0.5], 0., 0.5),
        ("radial-tangent", [0.25, 0.5], [0.5, 0.5], 0.25, 0.5),
        ("radial-separated", [0.9, 0.5], [0.3, 0.5], 0.1, 0.25),
        ("radial-reversed", [0.5, 0.5], [0.3, 0.5], 0.5, 0.2),
        ("radial-identical", [0.5, 0.5], [0.5, 0.5], 0.3, 0.3),
        (
            "radial-precision",
            [0.501, 0.5],
            [0.502, 0.5],
            0.1001,
            0.1008,
        ),
    ] {
        let mut mark = rect();
        mark.fill = C::GradientIndex(0).into();
        mark.gradients = vec![Gradient::RadialGradient(RadialGradient {
            x0: p0[0],
            y0: p0[1],
            x1: p1[0],
            y1: p1[1],
            r0,
            r1,
            stops: stops(),
        })];
        let mut case = Case::new(name, vec![mark.into()]);
        case.browser_only = true;
        // Axis-aligned circle intersections give these parameters directly.
        case.samples = match name {
            "radial-offset-inner" => vec![
                ([100, 120], [255, 0, 0, 255]),
                ([210, 120], [255, 0, 0, 255]),
                ([300, 120], [127, 0, 128, 255]),
                ([380, 35], [0, 0, 255, 255]),
            ],
            "radial-concentric" => vec![([318, 120], [170, 0, 85, 255])],
            "radial-zero" => vec![([264, 120], [127, 0, 128, 255])],
            "radial-tangent" | "radial-reversed" => vec![([300, 120], [127, 0, 128, 255])],
            "radial-separated" => vec![([282, 120], [85, 0, 170, 255]), ([210, 200], [255; 4])],
            "radial-identical" => vec![([210, 120], [255; 4])],
            "radial-precision" => vec![
                ([170, 120], [255, 0, 0, 255]),
                ([175, 120], [0, 0, 255, 255]),
            ],
            _ => unreachable!(),
        };
        let SceneMark::Rect(mark) = &case.scene.marks[0] else {
            unreachable!()
        };
        let Gradient::RadialGradient(g) = &mark.gradients[0] else {
            unreachable!()
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
            case.samples.push((point, radial_sample(g, xy)));
        }
        cases.push(case);
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
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let mut mark = rect();
        mark.fill = C::Color([1.0; 4]).into();
        mark.fill_pattern = Some(PatternFill {
            anchor: PatternAnchor::Mark,
            ink: PatternInk::Solid {
                color: [0.0, 0.0, 0.0, 1.0],
                opacity: 1.0,
            },
            layers: vec![PatternLayer::Symbol(SymbolPatternLayer {
                operation: PatternLayerOperation::Add,
                lattice: SymbolLattice2d {
                    u_spacing: 1000.0,
                    v_spacing: 1000.0,
                    u_angle: 0.0,
                    v_angle: 90.0,
                    u_phase: 180.0,
                    v_phase: 90.0,
                },
                symbol: PatternSymbol {
                    shape: "M-.5,-.5 H.5 V.5 H-.5 Z M-.2,-.2 H.2 V.2 H-.2 Z".into(),
                    size: 10000.0,
                    rotation: 0.0,
                    fill_rule: rule,
                },
                paint: SymbolPaint::Filled,
            })],
        })
        .into();
        let mut case = Case::new(format!("pattern-symbol-{rule:?}"), vec![mark.into()]);
        case.samples.push((
            [210, 120],
            if rule == FillRule::NonZero {
                [0, 0, 0, 255]
            } else {
                [255; 4]
            },
        ));
        cases.push(case);
    }
    for (name, shape, covered) in [
        ("pattern-miter", "M-.2,-1 L0,0 L.2,-1", true),
        ("pattern-bevel", "M-.1,-1 L0,0 L.1,-1", false),
    ] {
        let mut mark = rect();
        mark.fill = C::Color([1.0; 4]).into();
        mark.fill_pattern = Some(PatternFill {
            anchor: PatternAnchor::Mark,
            ink: PatternInk::Solid {
                color: [0.0, 0.0, 0.0, 1.0],
                opacity: 1.0,
            },
            layers: vec![PatternLayer::Symbol(SymbolPatternLayer {
                operation: PatternLayerOperation::Add,
                lattice: SymbolLattice2d {
                    u_spacing: 1000.0,
                    v_spacing: 1000.0,
                    u_angle: 0.0,
                    v_angle: 90.0,
                    u_phase: 180.0,
                    v_phase: 90.0,
                },
                symbol: PatternSymbol {
                    shape: shape.into(),
                    size: 10000.0,
                    rotation: 0.0,
                    fill_rule: FillRule::EvenOdd,
                },
                paint: SymbolPaint::Open { stroke_width: 12.0 },
            })],
        })
        .into();
        let mut case = Case::new(name, vec![mark.into()]);
        case.samples
            .push(([210, 140], if covered { [0, 0, 0, 255] } else { [255; 4] }));
        cases.push(case);
    }
    for smooth in [false, true] {
        let mut case = Case::new(
            format!("image-{smooth}"),
            vec![SceneImageMark {
                image: SceneImageSource::inline(avenger_image::RgbaImage {
                    width: 2,
                    height: 1,
                    data: vec![255, 0, 0, 255, 0, 0, 255, 0],
                })
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
        if smooth {
            case.samples = vec![([210, 120], [255, 128, 128, 255])];
        }
        cases.push(case);
    }
    for radial in [false, true] {
        for count in [0, 1, 2] {
            let stops = if count == 0 {
                vec![]
            } else if count == 1 {
                vec![GradientStop {
                    offset: 0.4,
                    color: [0.0, 0.0, 1.0, 0.5],
                }]
            } else {
                vec![
                    GradientStop {
                        offset: 0.0,
                        color: [1.0, 0.0, 0.0, 1.0],
                    },
                    GradientStop {
                        offset: 1.0,
                        color: [0.0, 0.0, 1.0, 0.5],
                    },
                ]
            };
            if radial && count == 2 {
                continue;
            }
            let gradient = if radial {
                Gradient::RadialGradient(RadialGradient {
                    x0: 0.5,
                    y0: 0.5,
                    x1: 0.5,
                    y1: 0.5,
                    r0: 0.0,
                    r1: 0.5,
                    stops,
                })
            } else {
                Gradient::LinearGradient(LinearGradient {
                    x0: 0.5,
                    y0: 0.5,
                    x1: if count == 2 { 0.5 } else { 1.0 },
                    y1: 0.5,
                    stops,
                })
            };
            for stroke in [false, true] {
                let mut mark = rect();
                mark.gradients = vec![gradient.clone()];
                mark.fill = if stroke {
                    C::Color([0.0; 4])
                } else {
                    C::GradientIndex(0)
                }
                .into();
                if stroke {
                    mark.stroke = C::GradientIndex(0).into();
                    mark.stroke_width = 20.0.into();
                }
                let mut case = Case::new(
                    format!("gradient-{radial}-{count}-{stroke}"),
                    vec![mark.into()],
                );
                case.samples.push((
                    [210, if stroke { 30 } else { 120 }],
                    if count == 0 {
                        [255; 4]
                    } else {
                        [128, 128, 255, 255]
                    },
                ));
                cases.push(case);
            }
        }
    }
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
        let mut case = Case::new(
            format!("radial-precision-{label}"),
            vec![SceneRectMark {
                x: left.into(),
                y: top.into(),
                width: Some(side.into()),
                height: Some(side.into()),
                fill: C::GradientIndex(0).into(),
                gradients: vec![Gradient::RadialGradient(g.clone())],
                ..Default::default()
            }
            .into()],
        );
        case.scene.width = left + side + 20.;
        case.scene.height = top + side + 20.;
        case.browser_only = true;
        for [u, v] in [
            [0.15, 0.2],
            [0.5, 0.2],
            [0.85, 0.2],
            [0.5, 0.5],
            [0.85, 0.8],
        ] {
            let point = [(left + side * u) as u32, (top + side * v) as u32];
            let xy = [
                (point[0] as f64 + 0.25 - left as f64) / side as f64,
                (point[1] as f64 + 0.25 - top as f64) / side as f64,
            ];
            case.samples.push((point, radial_sample(&g, xy)));
        }
        cases.push(case);
    }
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
