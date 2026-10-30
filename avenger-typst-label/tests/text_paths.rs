//! Labels on paths: their text bent along a curve, placed by the path's origin.

mod common;

use std::sync::Arc;

use avenger_typst_label::{
    BoxAnchor, Curve, CurveItem, Label, LabelEngine, LabelOptions, LabelSource,
    PathAnchor, PathBaseline, PathItem, PathKind, PathOffset, PathOverflow, PdfItem,
    PdfText, Point, RunPlace, SvgItem, TextBounds, TextPath,
};
use kurbo::{ParamCurve as _, ParamCurveArclen as _, PathEl, Shape as _};

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options())
}

/// Where the origin of a bent label's path lies in its box.
fn path_origin(bounds: &TextBounds) -> Point {
    let BoxAnchor::Path(origin) = bounds.anchor else {
        panic!("a label on a path: {bounds:?}");
    };
    origin
}

/// The distance from a straight label's box's top down to its first baseline.
fn ascent(bounds: &TextBounds) -> f32 {
    let BoxAnchor::Lines { ascent, .. } = bounds.anchor else {
        panic!("a label in straight lines: {bounds:?}");
    };
    ascent
}

fn options() -> LabelOptions {
    let mut options = LabelOptions::default();
    options.text.font_family = "Lato".into();
    options.text.font_size = 14.0;
    options
}

fn text(source: &str, path: Option<TextPath>) -> Label<'_> {
    Label {
        source: LabelSource::Text(source),
        options: options(),
        path,
    }
}

/// A path of `curve` that the text's start follows from its start, on its baseline, kept upright.
fn along(curve: Curve) -> TextPath {
    TextPath {
        path: curve,
        offset: PathOffset::Fraction(0.0),
        anchor: PathAnchor::Start,
        baseline: PathBaseline::Alphabetic,
        shift: 0.0,
        upright: Some(0.0),
        overflow: PathOverflow::Extend,
    }
}

fn line(from: [f32; 2], to: [f32; 2]) -> Curve {
    Curve(vec![
        CurveItem::Move(Point::new(from[0], from[1])),
        CurveItem::Line(Point::new(to[0], to[1])),
    ])
}

/// The arc of `radius` about the origin from `start` to `end`, in degrees clockwise from the
/// top, as cubic segments.
fn arc(radius: f64, start: f64, end: f64) -> Curve {
    let arc = kurbo::Arc::new(
        (0.0, 0.0),
        (radius, radius),
        (start - 90.0).to_radians(),
        (end - start).to_radians(),
        0.0,
    );
    let point = |point: kurbo::Point| Point::new(point.x as f32, point.y as f32);
    let items = arc
        .path_elements(0.01)
        .map(|element| match element {
            PathEl::MoveTo(to) => CurveItem::Move(point(to)),
            PathEl::LineTo(to) => CurveItem::Line(point(to)),
            PathEl::QuadTo(control, to) => CurveItem::Quad(point(control), point(to)),
            PathEl::CurveTo(a, b, to) => CurveItem::Cubic(point(a), point(b), point(to)),
            PathEl::ClosePath => CurveItem::Close,
        })
        .collect();
    Curve(items)
}

/// The middle of the advance of a run's first glyph, on its baseline, in the label's box.
fn middle(run: &PdfText) -> Point {
    let glyph = &run.glyphs[0];
    run.transform
        .apply(Point::new(glyph.position.x + glyph.x_advance / 2.0, glyph.position.y))
}

/// The PDF runs of a label, with the middle of each run's first glyph on its baseline, in the
/// coordinates of the label's path.
fn pdf_runs(
    engine: &LabelEngine,
    label: &Label,
) -> (TextBounds, Vec<(PdfText, [f32; 2])>) {
    let (bounds, pdf) = engine.pdf(label).unwrap();
    let origin = path_origin(&bounds);
    let runs = pdf
        .items
        .into_iter()
        .filter_map(|item| match item {
            PdfItem::Text(run) => Some(run),
            PdfItem::Path(_) => None,
        })
        .map(|run| {
            let point = middle(&run);
            (run, [point.x - origin.x, point.y - origin.y])
        })
        .collect();
    (bounds, runs)
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

/// Along a straight path, every glyph lies where the straight label puts it, with the path's
/// origin at the label's position.
#[test]
fn straight_paths_place_glyphs_as_straight_labels_do() {
    let engine = engine();
    let (straight, pdf) = engine.pdf(&text("Hello", None)).unwrap();
    let PdfItem::Text(run) = &pdf.items[0] else {
        panic!("{:?}", pdf.items);
    };
    let expected: Vec<[f32; 2]> = run
        .glyphs
        .iter()
        .map(|glyph| {
            let point = run.transform.apply(Point::new(
                glyph.position.x + glyph.x_advance / 2.0,
                glyph.position.y,
            ));
            // A straight label sits with its first baseline's start at its position.
            [point.x, point.y - ascent(&straight)]
        })
        .collect();

    let (_, runs) =
        pdf_runs(&engine, &text("Hello", Some(along(line([0.0, 0.0], [200.0, 0.0])))));
    assert_eq!(runs.len(), 5);
    for ((_, start), expected) in runs.iter().zip(&expected) {
        assert!(
            close(start[0], expected[0], 1e-3) && close(start[1], expected[1], 1e-3),
            "{start:?} {expected:?}"
        );
    }
}

/// On an arc, each part of the box that `baseline` names lies on the arc, moved by the shift:
/// the middle of each glyph's advance lies on it, within the tolerance of flattening the arc,
/// and the glyphs keep their advances along their baseline.
#[test]
fn arcs_set_each_part_of_the_text_on_the_path() {
    let engine = engine();
    let straight = engine.bounds(&text("Typography", None)).unwrap();
    let ascent = ascent(&straight);
    let descent = straight.height - ascent;
    let radius = 100.0;
    for (baseline, shift, rise) in [
        (PathBaseline::Alphabetic, 0.0, 0.0),
        (PathBaseline::Bottom, 3.0, descent + 3.0),
        (PathBaseline::Top, 3.0, -ascent - 3.0),
        (PathBaseline::Middle, 3.0, straight.height / 2.0 - ascent + 3.0),
    ] {
        let path = TextPath { baseline, shift, ..along(arc(radius, -60.0, 60.0)) };
        let (_, runs) = pdf_runs(&engine, &text("Typography", Some(path)));
        assert_eq!(runs.len(), 10);
        let expected = radius as f32 + rise;
        for (run, start) in &runs {
            let distance = start[0].hypot(start[1]);
            assert!(
                close(distance, expected, 0.06),
                "{baseline:?} {}: {distance} {expected}",
                run.text
            );
        }
        // The angle between the middles of the first and the last glyph, at the baseline's
        // radius, is the straight distance between them.
        let angle = |[x, y]: [f32; 2]| x.atan2(-y);
        let advance = |run: &PdfText| run.glyphs[0].x_advance;
        let advance: f32 = (advance(&runs[0].0) + advance(&runs[9].0)) / 2.0
            + runs[1..9].iter().map(|(run, _)| advance(run)).sum::<f32>();
        let swept = (angle(runs[9].1) - angle(runs[0].1)) * expected;
        assert!(close(swept, advance, 0.1), "{baseline:?}: {swept} {advance}");
    }
}

/// `upright` reverses a label whose middle reads right to left once its angle turns it, and the
/// memos of labels that differ only in their angles on the same half of a circle are one.
#[test]
fn upright_labels_reverse_by_their_middles_and_share_memos() {
    let engine = engine();
    let at = |angle: f32| TextPath {
        anchor: PathAnchor::Middle,
        offset: PathOffset::Fraction(0.5),
        baseline: PathBaseline::Middle,
        upright: Some(angle),
        ..along(arc(100.0, -30.0, 30.0))
    };
    let outline = |angle: f32| engine.outline(&text("Pears", Some(at(angle)))).unwrap();
    assert!(Arc::ptr_eq(&outline(10.0), &outline(50.0)));
    assert!(Arc::ptr_eq(&outline(150.0), &outline(200.0)));
    assert!(!Arc::ptr_eq(&outline(10.0), &outline(150.0)));

    // Reversed, the text runs from the arc's end to its start, so it reads left to right once
    // the angle turns it upside down.
    let first_and_last = |angle: f32| {
        let (_, runs) = pdf_runs(&engine, &text("Pears", Some(at(angle))));
        (runs[0].1[0], runs[runs.len() - 1].1[0])
    };
    let (first, last) = first_and_last(10.0);
    assert!(first < last);
    let (first, last) = first_and_last(150.0);
    assert!(first > last);
    // Without upright, the angle changes nothing.
    let fixed = TextPath { upright: None, ..at(150.0) };
    let (_, runs) = pdf_runs(&engine, &text("Pears", Some(fixed)));
    assert!(runs[0].1[0] < runs[runs.len() - 1].1[0]);
}

/// Text longer than its path runs past the end with `Extend`, and lays out nothing with `Fit`.
#[test]
fn overflow_extends_past_the_ends_or_lays_out_nothing() {
    let engine = engine();
    let short = |overflow| TextPath { overflow, ..along(line([0.0, 0.0], [10.0, 0.0])) };
    let (bounds, runs) =
        pdf_runs(&engine, &text("Overrunning", Some(short(PathOverflow::Extend))));
    assert_eq!(runs.len(), 11);
    assert!(bounds.width > 40.0, "{bounds:?}");
    let (bounds, runs) =
        pdf_runs(&engine, &text("Overrunning", Some(short(PathOverflow::Fit))));
    assert!(runs.is_empty());
    assert_eq!((bounds.width, bounds.height), (0.0, 0.0));
    // Text that fits draws with `Fit` too.
    let long = TextPath {
        overflow: PathOverflow::Fit,
        ..along(line([0.0, 0.0], [200.0, 0.0]))
    };
    let (_, runs) = pdf_runs(&engine, &text("Overrunning", Some(long)));
    assert_eq!(runs.len(), 11);
}

/// The lines of a label follow concentric arcs, a line's distance apart.
#[test]
fn lines_follow_concentric_arcs() {
    let engine = engine();
    let mut two = options();
    two.newline_breaks = true;
    let compiled = engine.compile_text("One\nTwo", &two).unwrap();
    let pitch = compiled.metrics.lines[1].baseline - compiled.metrics.lines[0].baseline;
    let (_, runs) = pdf_runs(
        &engine,
        &Label {
            source: LabelSource::Text("One\nTwo"),
            options: two,
            path: Some(TextPath {
                baseline: PathBaseline::Top,
                ..along(arc(100.0, -60.0, 60.0))
            }),
        },
    );
    let radius = |text: &str| {
        let (_, start) = runs.iter().find(|(run, _)| run.text == text).unwrap();
        start[0].hypot(start[1])
    };
    assert!(
        close(radius("O") - radius("T"), pitch, 0.05),
        "{} {} {pitch}",
        radius("O"),
        radius("T")
    );
}

/// A bent label draws as one run of text along its line's baseline in SVG, from where its text
/// starts on the curve, and as one run for each cluster in PDF.
#[test]
fn bent_labels_draw_as_text_in_svg_and_pdf() {
    let engine = engine();
    let path = TextPath {
        baseline: PathBaseline::Middle,
        ..along(arc(80.0, -70.0, 70.0))
    };
    let label = text("Curved", Some(path));
    let (bounds, svg) = engine.svg(&label).unwrap();
    assert_eq!((svg.size.x, svg.size.y), (bounds.width, bounds.height));
    let [SvgItem::Text(run)] = svg.items.as_slice() else {
        panic!("{:?}", svg.items);
    };
    assert_eq!(run.text, "Curved");
    // The text starts at the path's start, a point along the curve, past the straight run that
    // extends it before its start.
    let RunPlace::Path(path) = &run.place else {
        panic!("the run follows a path: {run:?}");
    };
    assert!(close(path.start, 1.0, 1e-3), "{path:?}");
    let (_, runs) = pdf_runs(&engine, &label);
    let joined: String = runs.iter().map(|(run, _)| run.text.as_str()).collect();
    assert_eq!(joined, "Curved");
}

/// The length of `curve`, and the length of the chain of its segments' ends.
fn lengths(curve: &Curve) -> (f64, f64) {
    let point = |point: Point| kurbo::Point::new(f64::from(point.x), f64::from(point.y));
    let mut path = kurbo::BezPath::new();
    for item in &curve.0 {
        match *item {
            CurveItem::Move(to) => path.move_to(point(to)),
            CurveItem::Line(to) => path.line_to(point(to)),
            CurveItem::Quad(control, to) => path.quad_to(point(control), point(to)),
            CurveItem::Cubic(a, b, to) => path.curve_to(point(a), point(b), point(to)),
            CurveItem::Close => path.close_path(),
        }
    }
    let along = path.segments().map(|segment| segment.arclen(1e-6)).sum();
    let chain = path
        .segments()
        .map(|segment| (segment.end() - segment.start()).hypot())
        .sum();
    (along, chain)
}

/// A short line before an arc, as lyon's arc builder can add to reach the arc's start, doesn't
/// fold the baseline moved off the path, so the curve that SVG draws the text along stays on the
/// baseline's points instead of bulging away between them.
#[test]
fn a_short_line_before_an_arc_folds_no_baseline() {
    let engine = engine();
    let arc = arc(100.0, -40.0, 40.0);
    let CurveItem::Move(start) = arc.0[0] else { panic!("{arc:?}") };
    // 0.03 points back along the arc's direction at its start, 40 degrees before the top.
    let (sin, cos) = (-40.0_f32).to_radians().sin_cos();
    let mut led = vec![
        CurveItem::Move(Point::new(start.x - 0.03 * cos, start.y - 0.03 * sin)),
        CurveItem::Line(start),
    ];
    led.extend_from_slice(&arc.0[1..]);
    let path = TextPath {
        anchor: PathAnchor::Middle,
        offset: PathOffset::Fraction(0.5),
        baseline: PathBaseline::Middle,
        ..along(Curve(led))
    };
    let (_, svg) = engine.svg(&text("Along an arc", Some(path))).unwrap();
    let [SvgItem::Text(run)] = svg.items.as_slice() else {
        panic!("{:?}", svg.items);
    };
    let RunPlace::Path(path) = &run.place else {
        panic!("the run follows a path: {run:?}");
    };
    let (along, chain) = lengths(&path.curve);
    assert!(along - chain < 0.05, "{along} {chain}");
}

/// Glyphs turn at once at a path's corner and keep each side's direction, as on the rim of a
/// radar chart.
#[test]
fn glyphs_turn_at_once_at_corners() {
    let engine = engine();
    let corner = Curve(vec![
        CurveItem::Move(Point::new(0.0, 0.0)),
        CurveItem::Line(Point::new(100.0, 0.0)),
        CurveItem::Line(Point::new(100.0, 100.0)),
    ]);
    let path = TextPath {
        offset: PathOffset::Distance(60.0),
        ..along(corner)
    };
    let (_, runs) = pdf_runs(&engine, &text("Around the corner", Some(path)));
    let angles: Vec<f32> = runs
        .iter()
        .map(|(run, _)| run.transform.ky.atan2(run.transform.sx).to_degrees())
        .collect();
    assert!(
        angles
            .iter()
            .all(|angle| close(*angle, 0.0, 1e-3) || close(*angle, 90.0, 1e-3)),
        "{angles:?}"
    );
    assert!(
        angles.iter().any(|angle| close(*angle, 0.0, 1e-3))
            && angles.iter().any(|angle| close(*angle, 90.0, 1e-3))
    );
}

/// The points of an SVG path item, flattened, in the label's box.
fn points(item: &PathItem) -> Vec<kurbo::Point> {
    let point = |point: Point| {
        let point = item.transform.apply(point);
        kurbo::Point::new(f64::from(point.x), f64::from(point.y))
    };
    let mut path = kurbo::BezPath::new();
    for item in &item.path.0 {
        match *item {
            CurveItem::Move(to) => path.move_to(point(to)),
            CurveItem::Line(to) => path.line_to(point(to)),
            CurveItem::Quad(control, to) => path.quad_to(point(control), point(to)),
            CurveItem::Cubic(a, b, to) => path.curve_to(point(a), point(b), point(to)),
            CurveItem::Close => path.close_path(),
        }
    }
    let mut points = Vec::new();
    kurbo::flatten(&path, 0.01, |element| {
        if let PathEl::MoveTo(to) | PathEl::LineTo(to) = element {
            points.push(to);
        }
    });
    points
}

/// How far the points of the widest piece of a label's SVG lie from the origin of the label's
/// path, nearest and farthest. A piece is a shape, or the glyphs of one text item.
fn widest_piece(engine: &LabelEngine, label: &Label) -> [f64; 2] {
    let (bounds, svg) = engine.svg(label).unwrap();
    let origin = path_origin(&bounds);
    let mut pieces: Vec<(Option<usize>, Vec<kurbo::Point>)> = Vec::new();
    for item in &svg.items {
        let SvgItem::Path(path) = item else {
            continue;
        };
        let text = match &path.kind {
            PathKind::Glyph(glyph) => Some(glyph.text),
            PathKind::Shape => None,
        };
        match pieces.iter_mut().find(|(other, _)| text.is_some() && *other == text) {
            Some((_, piece)) => piece.extend(points(path)),
            None => pieces.push((text, points(path))),
        }
    }
    let widest = pieces
        .into_iter()
        .map(|(_, piece)| piece)
        .max_by(|a, b| {
            let width = |points: &Vec<kurbo::Point>| {
                let xs = points.iter().map(|point| point.x);
                xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min)
            };
            width(a).total_cmp(&width(b))
        })
        .expect("the label draws pieces");
    let radii = widest.iter().map(|point| {
        (point.x - f64::from(origin.x)).hypot(point.y - f64::from(origin.y))
    });
    [radii.clone().fold(f64::MAX, f64::min), radii.fold(f64::MIN, f64::max)]
}

/// A math glyph that a rigid motion would set off a curve, such as a stretched brace, bends
/// through its outline, so that it lies as deep across a tight circle as across a loose one,
/// while narrow glyphs stay text.
#[test]
fn wide_math_glyphs_bend_through_their_outlines() {
    let engine = engine();
    let label = |radius: f64| Label {
        source: LabelSource::Markup("$overbrace(a + b + c + d)$"),
        options: options(),
        path: Some(TextPath {
            offset: PathOffset::Fraction(0.5),
            anchor: PathAnchor::Middle,
            baseline: PathBaseline::Bottom,
            ..along(arc(radius, -80.0, 80.0))
        }),
    };
    // A rigid brace would sag across the tight circle by about 5 points more than across the
    // loose one.
    let [near, far] = widest_piece(&engine, &label(40.0));
    let [loose_near, loose_far] = widest_piece(&engine, &label(400.0));
    assert!(near > 40.0, "{near}");
    assert!(
        ((far - near) - (loose_far - loose_near)).abs() < 0.1,
        "{near} {far} {loose_near} {loose_far}"
    );
    let (_, runs) = pdf_runs(&engine, &label(40.0));
    let texts: String = runs.iter().map(|(run, _)| run.text.as_str()).collect();
    assert!(!texts.contains('⏞') && texts.matches('+').count() == 3, "{texts}");
}
