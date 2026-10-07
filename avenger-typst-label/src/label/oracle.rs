//! The upstream reference fixtures for the crate's own tests: the reader that the integration
//! tests share, and the layout of label sources through the pipeline.

#[path = "../../tests/common/oracle.rs"]
mod common;

pub(crate) use self::common::*;

use std::sync::LazyLock;

use ecow::EcoVec;

use super::fixtures::{self, WithSource};
use crate::typst_eval::{eval_label, parse_label};
use crate::typst_layout::inline::{LineLimit, layout_label};
use crate::typst_library::diag::{SourceDiagnostic, SourceResult};
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{Content, Scope, Smart, StyleChain, Styles};
use crate::typst_library::layout::{Abs, Dir, Frame, Size};
use crate::typst_library::math::{EquationElem, LabelMathStyle};
use crate::typst_library::routines::{Arenas, RealizationKind};
use crate::typst_library::text::{
    FontFamily, FontList, FontWeight, Lang, Region, TextDir, TextElem, TextSize,
};
use crate::typst_library::visualize::{Color, Paint};
use crate::typst_realize::realize;

const SUITE: &str = "upstream_frames";

static MANIFEST: LazyLock<Manifest> = LazyLock::new(|| Manifest::load(SUITE));

/// `#rrggbbaa`, rounded to bytes as upstream's `Color::to_vec4_u8` rounds.
pub(crate) fn paint(paint: &Paint) -> String {
    let Paint::Solid(color) = paint;
    hex(color)
}

fn hex(color: &Color) -> String {
    let [r, g, b, a] = color.to_rgba().map(|c| (c * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}

// ---------------------------------------------------------------------------------------------
// Layout

/// The frame suite's default settings, for sources that have no reference.
pub(crate) fn default_settings() -> Settings {
    let defaults = &MANIFEST.defaults;
    Settings {
        text_font: defaults.text_font.clone(),
        math_font: defaults.math_font.clone(),
        font_size: defaults.font_size,
        font_weight: defaults.font_weight,
        lang: None,
        region: None,
        dir: None,
        width: None,
        align: None,
    }
}

/// The text and equation styles of the reference generator's wrapper for a case, which live
/// for the rest of the test run.
pub(crate) fn root_styles(settings: &Settings) -> StyleChain<'static> {
    let mut styles = Styles::new();
    styles.set(TextElem::font, FontList(vec![FontFamily::new(&settings.text_font)]));
    styles.set(TextElem::size, TextSize(Abs::pt(settings.font_size).into()));
    styles.set(TextElem::weight, FontWeight::from_number(settings.font_weight));
    if let Some(lang) = &settings.lang {
        styles.set(TextElem::lang, lang.parse::<Lang>().unwrap());
    }
    if let Some(region) = &settings.region {
        styles.set(TextElem::region, Some(region.parse::<Region>().unwrap()));
    }
    if let Some(dir) = &settings.dir {
        let dir = match dir.as_str() {
            "ltr" => Dir::LTR,
            "rtl" => Dir::RTL,
            other => panic!("unexpected direction {other}"),
        };
        styles.set(TextElem::dir, TextDir(Smart::Custom(dir)));
    }
    // The wrapper's `#show math.equation: set text(font: .., weight: ..)`.
    styles.set(
        EquationElem::label_style,
        LabelMathStyle {
            font: Some(FontList(vec![FontFamily::new(&settings.math_font)])),
            weight: Some(FontWeight::from_number(settings.font_weight)),
            ..LabelMathStyle::default()
        },
    );
    StyleChain::new(Box::leak(Box::new(styles)))
}

/// Evaluates a source as a label under a case's settings, realizes it and lays it out.
pub(crate) fn layout_source(source: &str, settings: &Settings) -> SourceResult<Frame> {
    layout_source_in(source, root_styles(settings)).0
}

/// Evaluates a source as a label under root styles, realizes it and lays it out, with the
/// warnings.
pub(crate) fn layout_source_in(
    source: &str,
    root: StyleChain,
) -> (SourceResult<Frame>, EcoVec<SourceDiagnostic>) {
    let world = WithSource { world: fixtures::shared(), source };
    let mut sink = Sink::new();
    let mut engine = Engine { world: &world, sink: &mut sink };
    let frame = eval_label(&mut engine, &parse_label(source), Scope::new())
        .and_then(|content| layout(&mut engine, &content, root));
    (frame, sink.warnings())
}

/// Realizes content that evaluation can't produce under the default settings and lays it out
/// as a label.
pub(crate) fn layout_content(content: &Content) -> SourceResult<Frame> {
    let world = WithSource { world: fixtures::shared(), source: "" };
    let mut sink = Sink::new();
    let mut engine = Engine { world: &world, sink: &mut sink };
    layout(&mut engine, content, root_styles(&default_settings()))
}

fn layout(
    engine: &mut Engine,
    content: &Content,
    root: StyleChain,
) -> SourceResult<Frame> {
    let arenas = Arenas::default();
    let children = realize(RealizationKind::Par, engine, &arenas, content, root)?;
    let region = Size::splat(Abs::inf());
    Ok(layout_label(engine, &children, root, region, false, LineLimit::default())?.frame)
}
