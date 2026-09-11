use std::sync::Arc;

use crate::label::{
    EngineOptions, FontResource, FontResourceId, LabelError, PdfGlyph, PdfGlyphRun, PdfTextLayer,
};
use crate::typst_layout::frame::{MathLayoutOptions, MathRunArtifact, TypesetMetrics};
use crate::typst_layout::glyph_path::outline_glyph_path;
use crate::typst_library::math::item as ast;
use crate::typst_library::text::content::DecorationStroke;
use crate::typst_library::{Color, FontWeight, MathFontSpec};
use crate::typst_svg::{
    DashPattern, PathArtifact, PathCommand, PathData, PathItem, PathKind, Stroke, Transform,
};

use crate::typst_eval::math::predefined_operator_text;
use ast::{
    MathAccent, MathAst, MathCancel, MathCancelAngle, MathFractionStyle, MathNode, MathOperator,
    MathShorthand, MathSpacing, MathText, MathTextKind,
};

#[cfg(test)]
pub(crate) fn try_typeset_simple_row_fragment(
    math: &MathAst,
    options: &MathLayoutOptions,
    config: &EngineOptions,
) -> Result<Option<MathRunArtifact>, LabelError> {
    let fontdb = crate::typst_layout::inline::font::build_text_fontdb(config);
    try_typeset_simple_row_fragment_with_fontdb(math, options, config, &fontdb)
}

pub(crate) fn try_typeset_simple_row_fragment_with_fontdb(
    math: &MathAst,
    options: &MathLayoutOptions,
    config: &EngineOptions,
    fontdb: &fontdb::Database,
) -> Result<Option<MathRunArtifact>, LabelError> {
    if !config.fonts.extra_font_families.is_empty() {
        return Ok(None);
    }

    let Some(mut font) = load_default_math_font_with_fontdb(
        config,
        fontdb,
        &options.style.font,
        &options.style.font_weight,
    ) else {
        return Ok(None);
    };
    font.text_fonts = Some(Arc::new(fontdb.clone()));
    font.weight = options.style.font_weight.clone();
    font.optical_size = options.style.font_size;
    let Some(layout) =
        layout_simple_row(&font, math, options.style.font_size.max(f32::MIN_POSITIVE))?
    else {
        return Ok(None);
    };
    let paths = path_artifact_from_simple_row(&font, &layout, options.style.fill);
    let pdf_artifact = pdf_text_from_simple_row(&font, &layout, &math.source, options.style.fill)?;

    Ok(Some(MathRunArtifact {
        metrics: layout.metrics,
        paths,
        pdf_text: pdf_artifact.text_layer,
        font_resources: pdf_artifact.font_resources,
        warnings: Vec::new(),
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SimpleMathAtom {
    styled_text: String,
    class: SimpleMathClass,
    text_operator: bool,
}

#[derive(Debug, Clone)]
struct SimpleRowLayout {
    metrics: TypesetMetrics,
    atoms: Vec<LaidOutMathAtom>,
}

#[derive(Debug, Clone)]
struct LaidOutMathAtom {
    metrics: TypesetMetrics,
    ink_ascent: f32,
    ink_descent: f32,
    left_spacing: Option<(f32, u8)>,
    right_spacing: Option<(f32, u8)>,
    left_class: SimpleMathClass,
    right_class: SimpleMathClass,
    italic_correction: f32,
    script_kernable: bool,
    base_metrics: Option<(f32, f32)>,
    accent_attachment: Option<(f32, f32)>,
    spaced: bool,
    glyphs: Vec<LaidOutGlyph>,
    shapes: Vec<LaidOutShape>,
    draw_order: Vec<LaidOutDrawItem>,
}

#[derive(Debug, Clone)]
struct LaidOutGlyph {
    glyph_id: ttf_parser::GlyphId,
    unicode: String,
    x: f32,
    y: f32,
    x_advance: f32,
    font_size: f32,
    pdf_run_group: Option<usize>,
    font: Option<crate::typst_layout::inline::font::TextFace>,
    text_range: Option<(Arc<str>, std::ops::Range<usize>)>,
}

#[derive(Debug, Clone)]
struct LaidOutShape {
    path: PathData,
    x: f32,
    y: f32,
    stroke: LaidOutStroke,
}

#[derive(Debug, Clone)]
struct LaidOutStroke {
    paint: Option<Color>,
    width: f32,
    line_cap: crate::typst_svg::LineCap,
    line_join: crate::typst_svg::LineJoin,
    dash: Option<DashPattern>,
    miter_limit: f32,
}

impl LaidOutStroke {
    fn new(width: f32) -> Self {
        Self {
            paint: None,
            width,
            line_cap: crate::typst_svg::LineCap::Butt,
            line_join: crate::typst_svg::LineJoin::Miter,
            dash: None,
            miter_limit: 4.0,
        }
    }

    fn from_decoration(stroke: &DecorationStroke, default_width: f32, font_size: f32) -> Self {
        let width = stroke
            .thickness
            .map(|thickness| thickness.resolve(font_size))
            .unwrap_or(default_width);
        Self {
            paint: stroke.paint,
            width,
            line_cap: stroke.line_cap.unwrap_or_default(),
            line_join: stroke.line_join.unwrap_or_default(),
            dash: stroke
                .dash
                .as_ref()
                .and_then(|dash| dash.resolve(width, font_size)),
            miter_limit: stroke.miter_limit.unwrap_or(4.0),
        }
    }
}

#[derive(Debug, Clone)]
enum LaidOutDrawItem {
    Glyph(usize),
    Shape(usize),
}

struct PdfArtifact {
    text_layer: PdfTextLayer,
    font_resources: Vec<FontResource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimpleMathClass {
    Normal,
    Alphabetic,
    Binary,
    Unary,
    Vary,
    Relation,
    Opening,
    Closing,
    Fence,
    Punctuation,
    Large,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MathLayoutSize {
    level: u8,
    display: bool,
    cramped: bool,
}

#[allow(non_upper_case_globals)]
impl MathLayoutSize {
    const Display: Self = Self {
        level: 0,
        display: true,
        cramped: false,
    };
    const Text: Self = Self {
        level: 0,
        display: false,
        cramped: false,
    };
    const Script: Self = Self {
        level: 1,
        display: false,
        cramped: false,
    };
    const ScriptScript: Self = Self {
        level: 2,
        display: false,
        cramped: false,
    };

    fn cramped(self) -> Self {
        Self {
            cramped: true,
            ..self
        }
    }

    fn fraction_child(self) -> Self {
        if self.display {
            Self {
                display: false,
                ..self
            }
        } else {
            self.script_child()
        }
    }

    fn script_child(self) -> Self {
        Self {
            level: (self.level + 1).min(2),
            display: false,
            ..self
        }
    }

    fn is_display(self) -> bool {
        self.display
    }

    fn child_context(
        self,
        child: Self,
        font: &MathFont,
        font_size: f32,
        script_level: u8,
    ) -> Result<(f32, u8, Self), LabelError> {
        // Math size commands are absolute categories. Recover the equation's
        // root size before selecting a category, including when growing again.
        let root_size = font_size / math_size_scale(font, script_level)?;
        Ok((
            root_size * math_size_scale(font, child.level)?,
            child.level,
            child,
        ))
    }
}

fn math_size_scale(font: &MathFont, level: u8) -> Result<f32, LabelError> {
    let face = parse_math_face(font, "math size constants")?;
    let constants = face.tables().math.and_then(|math| math.constants);
    Ok(match level {
        0 => 1.0,
        1 => {
            constants
                .map_or(70, |c| c.script_percent_scale_down())
                .max(1) as f32
                / 100.0
        }
        _ => {
            constants
                .map_or(50, |c| c.script_script_percent_scale_down())
                .max(1) as f32
                / 100.0
        }
    })
}
