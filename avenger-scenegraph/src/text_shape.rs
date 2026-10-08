//! The shapes a label draws, as lyon paths.

use avenger_color::AbsoluteColor;
use avenger_typst_label::{Curve, CurveItem, DashPattern, FillRule, PathItem, Stroke, Transform};
use lyon_path::{geom::point, Path};

/// A filled or stroked shape: a glyph's outline, or a shape such as a fraction line or a text
/// decoration. Its path and stroke are in the label's coordinates.
#[derive(Debug, Clone)]
pub struct TextShape {
    pub path: Path,
    pub fill: Option<AbsoluteColor>,
    pub fill_rule: FillRule,
    pub stroke: Option<Stroke>,
}

impl TextShape {
    /// A path item's shape, in the label's coordinates.
    pub fn new(item: &PathItem) -> Self {
        let transform = item.transform;
        // The stroke is in the path's coordinates; its lengths scale with the transform.
        let scale = (transform.sx * transform.sy - transform.kx * transform.ky)
            .abs()
            .sqrt();
        Self {
            path: lyon_path(&item.path, transform),
            fill: item.fill,
            fill_rule: item.fill_rule,
            stroke: item.stroke.clone().map(|stroke| Stroke {
                thickness: stroke.thickness * scale,
                dash: stroke.dash.map(|dash| DashPattern {
                    array: dash.array.iter().map(|length| length * scale).collect(),
                    phase: dash.phase * scale,
                }),
                ..stroke
            }),
        }
    }
}

fn lyon_path(curve: &Curve, transform: Transform) -> Path {
    let to_point = |p: avenger_typst_label::Point| {
        let p = transform.apply(p);
        point(p.x, p.y)
    };
    let mut builder = Path::builder().with_svg();
    for item in &curve.0 {
        match *item {
            CurveItem::Move(to) => {
                builder.move_to(to_point(to));
            }
            CurveItem::Line(to) => {
                builder.line_to(to_point(to));
            }
            CurveItem::Cubic(first, second, to) => {
                builder.cubic_bezier_to(to_point(first), to_point(second), to_point(to));
            }
            CurveItem::Close => builder.close(),
        }
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_typst_label::{
        bundled_font_options, EngineOptions, FontOptions, Label, LabelEngine, LabelOptions,
        LabelSource, SvgItem, TextStyle,
    };

    /// The drawing items of a markup label, set in the bundled fonts only.
    fn items(source: &str) -> Vec<SvgItem> {
        let engine = LabelEngine::new(EngineOptions {
            fonts: FontOptions {
                load_system_fonts: false,
                ..bundled_font_options()
            },
        });
        let label = Label {
            source: LabelSource::Markup(source),
            options: LabelOptions {
                text: TextStyle {
                    font_size: 10.0,
                    fill: AbsoluteColor::from_rgba([0.1, 0.2, 0.3, 1.0]),
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        engine.svg(&label).unwrap().1.items
    }

    #[test]
    fn math_draws_as_shapes_after_the_text_run() {
        let items = items("speed $v^2$");
        assert!(matches!(&items[0], SvgItem::Text(run) if run.text == "speed "));
        assert!(items.len() > 1);
        assert!(items[1..]
            .iter()
            .all(|item| matches!(item, SvgItem::Path(_))));
    }

    #[test]
    fn decorations_draw_as_open_stroked_shapes() {
        let items = items("#underline[important]");
        let [SvgItem::Text(run), SvgItem::Path(line)] = items.as_slice() else {
            panic!("{items:?}");
        };
        assert_eq!(run.text, "important");
        let line = TextShape::new(line);
        assert!(line.stroke.is_some());
        assert!(matches!(
            line.path.iter().last(),
            Some(lyon_path::Event::End { close: false, .. })
        ));
    }

    #[test]
    fn decoration_strokes_keep_their_dash_phase_and_miter_limit() {
        let items = items("#underline(stroke: (thickness: 1pt, dash: (array: (2pt, 1pt), phase: 0.5pt), miter-limit: 2))[important]");
        let Some(TextShape {
            stroke: Some(stroke),
            ..
        }) = items.iter().find_map(|item| match item {
            SvgItem::Path(path) => Some(TextShape::new(path)),
            _ => None,
        })
        else {
            panic!("{items:?}");
        };
        let dash = stroke.dash.as_ref().expect("dash should resolve");
        assert_eq!(dash.array.as_slice(), [2.0, 1.0].as_slice());
        assert_eq!(dash.phase, 0.5);
        assert_eq!(stroke.miter_limit, 2.0);
    }
}
