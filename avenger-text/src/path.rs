use avenger_color::AbsoluteColor;
use avenger_typst_label::{Curve, CurveItem, PathItem, SvgItem, Transform};
pub use avenger_typst_label::{
    DashPattern, FillRule, ImageItem, LineCap, LineJoin, Stroke, TextRun,
};
use lyon_path::{geom::point, Path};

use crate::{
    error::AvengerTextError,
    measurement::TextBounds,
    types::TextConfig,
    typeset::{bounds_from_metrics, first_baseline, typeset, LabelSettings},
};

/// A filled or stroked shape: a glyph's outline, or a shape such as a fraction line or a text
/// decoration. Its path and stroke are in the label's coordinates.
#[derive(Debug, Clone)]
pub struct TextShape {
    pub path: Path,
    pub fill: Option<AbsoluteColor>,
    pub fill_rule: FillRule,
    pub stroke: Option<Stroke>,
}

/// What a label draws.
#[derive(Debug, Clone)]
pub enum TextPathItem {
    Shape(TextShape),
    /// A bitmap glyph, such as a color emoji.
    Image(ImageItem),
    /// A run of glyphs that viewers draw as the label does. Its bitmap glyphs also come as images
    /// of the same text item.
    Run(TextRun),
}

/// A label's drawing items, in drawing order, in its box's coordinates.
#[derive(Debug, Clone)]
pub struct TextPathBuffer {
    pub bounds: TextBounds,
    pub items: Vec<TextPathItem>,
}

/// A label's drawing items: native text runs where viewers draw text as the label does, outlines
/// for the rest, and bitmap glyphs as images.
pub(crate) fn extract_paths(
    typst: &avenger_typst_label::LabelEngine,
    settings: &LabelSettings,
    config: &TextConfig,
) -> Result<TextPathBuffer, AvengerTextError> {
    let label = typeset(typst, settings, config)?;
    let bounds = bounds_from_metrics(&label.metrics, config.font_size);
    // The label's frame starts below the padding at the top of its box.
    let padding = bounds.ascent - first_baseline(&label.metrics);
    let offset = Transform::translate(0.0, padding);
    let svg = avenger_typst_label::svg_items(
        &label,
        &avenger_typst_label::SvgOptions { native_text: true },
    );
    let items = svg
        .items
        .into_iter()
        .map(|item| match item {
            SvgItem::Path(path) => TextPathItem::Shape(text_shape(&path, offset)),
            SvgItem::Image(image) => TextPathItem::Image(ImageItem {
                transform: offset.pre_concat(image.transform),
                ..image
            }),
            SvgItem::Text(run) => TextPathItem::Run(TextRun {
                baseline: run.baseline + padding,
                ..run
            }),
        })
        .collect();
    Ok(TextPathBuffer { bounds, items })
}

/// A drawing item's shape in the label's box, which `offset` moves its frame into.
pub(crate) fn text_shape(item: &PathItem, offset: Transform) -> TextShape {
    let transform = offset.pre_concat(item.transform);
    // The stroke is in the path's coordinates; its lengths scale with the transform.
    let scale = (transform.sx * transform.sy - transform.kx * transform.ky)
        .abs()
        .sqrt();
    TextShape {
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

    fn config(text: &str) -> TextConfig<'_> {
        TextConfig {
            text,
            syntax_mode: crate::types::TextSyntaxMode::TypstMarkup,
            font_size: 10.0,
            color: [0.1, 0.2, 0.3, 1.0],
            ..Default::default()
        }
    }

    /// The drawing items of a label, set in the bundled fonts only.
    fn items(text: &str) -> Vec<TextPathItem> {
        crate::TextEngine::new(&crate::FontOptions {
            load_system_fonts: false,
            ..crate::default_font_options()
        })
        .extract_paths(&config(text))
        .unwrap()
        .items
    }

    #[test]
    fn math_draws_as_shapes_after_the_text_run() {
        let items = items("speed $v^2$");
        assert!(matches!(&items[0], TextPathItem::Run(run) if run.text == "speed "));
        assert!(items.len() > 1);
        assert!(items[1..]
            .iter()
            .all(|item| matches!(item, TextPathItem::Shape(_))));
    }

    #[test]
    fn decorations_draw_as_open_stroked_shapes() {
        let items = items("#underline[important]");
        let [TextPathItem::Run(run), TextPathItem::Shape(line)] = items.as_slice() else {
            panic!("{items:?}");
        };
        assert_eq!(run.text, "important");
        assert!(line.stroke.is_some());
        assert!(matches!(
            line.path.iter().last(),
            Some(lyon_path::Event::End { close: false, .. })
        ));
    }

    #[test]
    fn decoration_strokes_keep_their_dash_phase_and_miter_limit() {
        let items = items("#underline(stroke: (thickness: 1pt, dash: (array: (2pt, 1pt), phase: 0.5pt), miter-limit: 2))[important]");
        let Some(TextPathItem::Shape(TextShape {
            stroke: Some(stroke),
            ..
        })) = items
            .iter()
            .find(|item| matches!(item, TextPathItem::Shape(_)))
        else {
            panic!("{items:?}");
        };
        let dash = stroke.dash.as_ref().expect("dash should resolve");
        assert_eq!(dash.array.as_slice(), [2.0, 1.0].as_slice());
        assert_eq!(dash.phase, 0.5);
        assert_eq!(stroke.miter_limit, 2.0);
    }
}
