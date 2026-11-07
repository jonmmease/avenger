//! The parts of an evaluation that the tests check: the marks of a plot, their positions and sizes,
//! the texts of a scene, and the warnings.

use avenger_chart_eval::{EvalWarning, EvaluatedChart};
use avenger_color::ColorOrGradient;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark, symbol::SceneSymbolMark};

/// The group of the first plot of an evaluation.
pub(crate) fn plot_group(evaluated: &EvaluatedChart) -> &SceneGroup {
    let name = &evaluated.plots[0].name;
    (evaluated.scene.marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Group(group) if &group.name == name => Some(group),
            _ => None,
        })
        .expect("the plot's group")
}

/// The symbol mark of the first plot of an evaluation.
pub(crate) fn plot_symbols(evaluated: &EvaluatedChart) -> &SceneSymbolMark {
    (plot_group(evaluated).marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Symbol(symbol) => Some(symbol),
            _ => None,
        })
        .expect("a symbol mark")
}

/// The x position, the y position, and the size of each symbol of the first plot.
pub(crate) fn symbol_positions(evaluated: &EvaluatedChart) -> Vec<(f32, f32, f32)> {
    let symbol = plot_symbols(evaluated);
    let len = symbol.len as usize;
    let x = symbol.x.as_vec(len, None);
    let y = symbol.y.as_vec(len, None);
    let size = symbol.size.as_vec(len, None);
    x.into_iter()
        .zip(y)
        .zip(size)
        .map(|((x, y), size)| (x, y, size))
        .collect()
}

/// The text of every text mark of a scene, its groups included.
pub(crate) fn scene_texts(marks: &[SceneMark]) -> Vec<String> {
    let mut texts = Vec::new();
    for mark in marks {
        match mark {
            SceneMark::Group(group) => texts.extend(scene_texts(&group.marks)),
            SceneMark::Text(text) => texts.extend(text.text.as_vec(text.len as usize, None)),
            _ => {}
        }
    }
    texts
}

/// The domain of the scale of `dimension`, 0 for `x` and 1 for `y`, of the first plot.
pub(crate) fn domain(evaluated: &EvaluatedChart, dimension: usize) -> (f64, f64) {
    (evaluated.plots[0].coords.scale(dimension))
        .expect("a scale")
        .numeric_interval_domain()
        .unwrap()
}

pub(crate) fn warns(evaluated: &EvaluatedChart, text: &str) -> bool {
    evaluated.warnings.iter().any(|warning| match warning {
        EvalWarning::NotDrawn { reason, .. }
        | EvalWarning::Fit { reason, .. }
        | EvalWarning::Domain { reason, .. } => reason.contains(text),
        EvalWarning::Style(_) => false,
    })
}

/// The left edges of the rects of the first plot.
pub(crate) fn rect_lefts(evaluated: &EvaluatedChart) -> Vec<f32> {
    (plot_group(evaluated).marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Rect(rect) => Some(rect.x.as_vec(rect.len as usize, None)),
            _ => None,
        })
        .expect("a rect mark")
}

/// The left and right edges of the rects of the first plot.
pub(crate) fn rect_spans(evaluated: &EvaluatedChart) -> Vec<(f32, f32)> {
    (plot_group(evaluated).marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Rect(rect) => {
                let len = rect.len as usize;
                let x = rect.x.as_vec(len, None);
                let x2 = rect.x2.as_ref().expect("an end").as_vec(len, None);
                Some(x.into_iter().zip(x2).collect())
            }
            _ => None,
        })
        .expect("a rect mark")
}

/// The widths and the heights of the image mark of the first plot.
pub(crate) fn image_sizes(evaluated: &EvaluatedChart) -> Vec<(f32, f32)> {
    (plot_group(evaluated).marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Image(image) => {
                let len = image.len as usize;
                let widths = image.width.as_vec(len, None);
                let heights = image.height.as_vec(len, None);
                Some(widths.into_iter().zip(heights).collect())
            }
            _ => None,
        })
        .expect("an image mark")
}

/// The symbols outside the plot's group: the swatches of the legends.
fn legend_symbols(evaluated: &EvaluatedChart) -> Vec<&SceneSymbolMark> {
    fn symbols<'a>(marks: &'a [SceneMark], out: &mut Vec<&'a SceneSymbolMark>) {
        for mark in marks {
            match mark {
                SceneMark::Group(group) => symbols(&group.marks, out),
                SceneMark::Symbol(symbol) => out.push(symbol),
                _ => {}
            }
        }
    }
    let plot = &evaluated.plots[0].name;
    let mut out = Vec::new();
    for mark in &evaluated.scene.marks {
        match mark {
            SceneMark::Group(group) if &group.name == plot => {}
            mark => symbols(std::slice::from_ref(mark), &mut out),
        }
    }
    out
}

/// The fills of the swatches of the legends.
pub(crate) fn legend_symbol_fills(evaluated: &EvaluatedChart) -> Vec<ColorOrGradient> {
    (legend_symbols(evaluated).into_iter())
        .flat_map(|symbol| symbol.fill.as_vec(symbol.len as usize, None))
        .collect()
}

/// The strokes of the swatches of the legends.
pub(crate) fn legend_symbol_strokes(evaluated: &EvaluatedChart) -> Vec<ColorOrGradient> {
    (legend_symbols(evaluated).into_iter())
        .flat_map(|symbol| symbol.stroke.as_vec(symbol.len as usize, None))
        .collect()
}
