//! The label engine: compiles labels through the pipeline, in the engine's world.

use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;

use avenger_format::{DateTimeFormatProvider, NumberFormatProvider};

use super::error::{LabelError, LabelWarning, source_error, source_warning};
use super::format::FormattingCache;
use super::frame::LabelFrame;
use super::lower::lower;
use super::options::{
    EngineOptions, LabelFormatting, LabelLimits, LabelOptions, MissingFontPolicy,
    TextStyle,
};
use super::params;
use super::styles::{Defaults, root_styles};
use super::world::LabelWorld;
use crate::typst_eval::{eval_label, math_nesting_depth, parse_label};
use crate::typst_layout::inline::layout_label_line;
use crate::typst_library::World;
use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{Content, StyleChain};
use crate::typst_library::layout::{Abs, Frame, InlineElem};
use crate::typst_library::routines::{Arenas, RealizationKind};
use crate::typst_library::text::{
    Font, FontBook, FontInstance, FontStretch, FontVariant, FontVariations, SpaceElem,
    TextElem,
};
use crate::typst_realize::realize;
use crate::typst_syntax::{FileId, Span, SyntaxKind, SyntaxNode, is_newline};

/// Compiles labels: single lines of Typst markup with inline math.
///
/// An engine holds its fonts and caches, and is cheap to clone. Fonts load on first use.
#[derive(Clone)]
pub struct LabelEngine {
    /// The fonts.
    world: Arc<LabelWorld>,
    /// The families labels fall back to.
    defaults: Defaults,
    /// What happens when families of a label's font lists are not available.
    missing_font: MissingFontPolicy,
    /// The provider of `#numfmt`, unless a label brings its own.
    number_format: Option<Arc<dyn NumberFormatProvider>>,
    /// The provider of `#datefmt`, unless a label brings its own.
    datetime_format: Option<Arc<dyn DateTimeFormatProvider>>,
    /// The prepared formats of `#numfmt` and `#datefmt`.
    formatting_cache: Arc<FormattingCache>,
}

impl LabelEngine {
    /// An engine with the given fonts.
    pub fn new(options: EngineOptions) -> Self {
        let fonts = options.fonts;
        let registered = fonts
            .registered_fonts
            .into_iter()
            .map(|font| (font.data, font.face_index));
        let mut world =
            LabelWorld::new(registered, &fonts.extra_font_dirs, fonts.load_system_fonts);
        if let Some(family) = &fonts.default_sans_serif_family {
            world.set_sans_serif_family(family);
        }
        if let Some(family) = &fonts.default_monospace_family {
            world.set_monospace_family(family);
        }
        let defaults = Defaults::new(
            &world,
            fonts.default_monospace_family,
            fonts.default_math_family,
        );
        Self {
            world: Arc::new(world),
            defaults,
            missing_font: fonts.missing_font,
            number_format: None,
            datetime_format: None,
            formatting_cache: Arc::default(),
        }
    }

    /// Sets the provider of `#numfmt`. Its settings, such as the locale, apply to every
    /// pattern it prepares.
    pub fn with_number_formatting(
        mut self,
        provider: Arc<dyn NumberFormatProvider>,
    ) -> Self {
        self.number_format = Some(provider);
        self
    }

    /// The provider of `#numfmt` for labels that bring none.
    pub fn number_format(&self) -> Option<&Arc<dyn NumberFormatProvider>> {
        self.number_format.as_ref()
    }

    /// Sets the provider of `#datefmt`. Its settings, such as the locale and the display
    /// timezone, apply to every pattern it prepares.
    pub fn with_datetime_formatting(
        mut self,
        provider: Arc<dyn DateTimeFormatProvider>,
    ) -> Self {
        self.datetime_format = Some(provider);
        self
    }

    /// The provider of `#datefmt` for labels that bring none.
    pub fn datetime_format(&self) -> Option<&Arc<dyn DateTimeFormatProvider>> {
        self.datetime_format.as_ref()
    }

    /// Compiles a label's markup.
    pub fn compile(
        &self,
        source: &str,
        options: &LabelOptions,
    ) -> Result<CompiledLabel, LabelError> {
        self.compile_with_formatting(source, options, LabelFormatting::default())
    }

    /// Compiles a label's markup with its own formatting providers, which fall back to the
    /// engine's.
    pub fn compile_with_formatting(
        &self,
        source: &str,
        options: &LabelOptions,
        formatting: LabelFormatting<'_>,
    ) -> Result<CompiledLabel, LabelError> {
        let typeset = self.typeset_markup(source, options, formatting)?;
        Ok(typeset.compiled(source))
    }

    /// Compiles literal text: the label `escape_text(text)` is, without parsing it.
    pub fn compile_text(
        &self,
        text: &str,
        options: &LabelOptions,
    ) -> Result<CompiledLabel, LabelError> {
        Ok(self.typeset_text(text, options)?.compiled(text))
    }

    /// The metrics a label's markup compiles to.
    pub fn measure(
        &self,
        source: &str,
        options: &LabelOptions,
    ) -> Result<LabelMetrics, LabelError> {
        let typeset = self.typeset_markup(source, options, LabelFormatting::default())?;
        Ok(LabelMetrics::of(&typeset.frame))
    }

    /// The metrics literal text compiles to.
    pub fn measure_text(
        &self,
        text: &str,
        options: &LabelOptions,
    ) -> Result<LabelMetrics, LabelError> {
        Ok(LabelMetrics::of(&self.typeset_text(text, options)?.frame))
    }

    /// The vertical metrics of the face that a text style's text uses first: the first
    /// available family of its font list, else the default sans-serif family.
    pub fn font_metrics(&self, style: &TextStyle) -> Result<FontMetrics, LabelError> {
        self.check_fonts(&[&style.font_family])?;
        let variant =
            FontVariant::new(style.font_style, style.font_weight, FontStretch::NORMAL);
        let book = self.world.book();
        let size = Abs::pt(f64::from(style.font_size));
        let font = self
            .world
            .families(&style.font_family)
            .into_iter()
            .chain([self.defaults.sans().to_string()])
            .find_map(|family| book.select(&family.to_lowercase(), variant))
            .and_then(|index| self.world.font(index))
            .map(|font| font.instantiate(variant, size, &FontVariations::default()))
            .ok_or_else(|| LabelError::MissingFont {
                family: style.font_family.clone(),
            })?;
        Ok(FontMetrics::of(&font, style.font_size))
    }

    /// The parameters a label's source refers to.
    pub fn referenced_params(&self, source: &str) -> Result<Vec<String>, LabelError> {
        params::referenced_params(source)
    }

    /// Typesets a label's markup.
    fn typeset_markup(
        &self,
        source: &str,
        options: &LabelOptions,
        formatting: LabelFormatting<'_>,
    ) -> Result<Typeset, LabelError> {
        check_size(source, options.limits)?;
        let root = parse_label(source);
        check_math(&root, options.limits)?;
        let scope = params::scope(&options.params);
        self.typeset(source, options, formatting, |engine| {
            eval_label(engine, &root, scope)
        })
    }

    /// Typesets literal text.
    fn typeset_text(
        &self,
        text: &str,
        options: &LabelOptions,
    ) -> Result<Typeset, LabelError> {
        check_size(text, options.limits)?;
        self.typeset(text, options, LabelFormatting::default(), |_| Ok(literal(text)))
    }

    /// Realizes and lays out a label's content, which `content` makes in the label's world.
    fn typeset(
        &self,
        source: &str,
        options: &LabelOptions,
        formatting: LabelFormatting<'_>,
        content: impl FnOnce(&mut Engine) -> SourceResult<Content>,
    ) -> Result<Typeset, LabelError> {
        let mut warnings =
            self.check_fonts(&[&options.text.font_family, &options.math.font_family])?;
        let world = CompileWorld {
            fonts: &self.world,
            source,
            number_format: formatting.number.or(self.number_format.as_ref()),
            datetime_format: formatting.datetime.or(self.datetime_format.as_ref()),
            formatting_cache: &self.formatting_cache,
        };
        let mut sink = Sink::new();
        let styles = root_styles(&self.world, &self.defaults, options);
        let root = StyleChain::new(&styles);
        let laid_out: SourceResult<_> = (|| {
            let mut engine = Engine { world: &world, sink: &mut sink };
            let content = content(&mut engine)?;
            let arenas = Arenas::default();
            let children =
                realize(RealizationKind::Par, &mut engine, &arenas, &content, root)?;
            let has_math = children.iter().any(|(child, _)| child.is::<InlineElem>());
            Ok((layout_label_line(&mut engine, &children, root)?, has_math))
        })();
        let (line, has_math) =
            laid_out.map_err(|errors| source_error(source, &errors[0]))?;
        warnings.extend(
            sink.warnings().iter().map(|warning| source_warning(source, warning)),
        );
        Ok(Typeset {
            frame: line.frame,
            text: line.text,
            flags: LabelFlags { has_math },
            warnings,
        })
    }

    /// Checks the families of font lists, as the engine's policy says.
    fn check_fonts(&self, lists: &[&str]) -> Result<Vec<LabelWarning>, LabelError> {
        let mut warnings = vec![];
        if self.missing_font == MissingFontPolicy::Fallback {
            return Ok(warnings);
        }
        let book = self.world.book();
        for list in lists.iter().filter(|list| !list.trim().is_empty()) {
            let families = self.world.families(list);
            let missing: Vec<_> =
                families.iter().filter(|family| !has_family(book, family)).collect();
            match self.missing_font {
                MissingFontPolicy::Error if missing.len() == families.len() => {
                    return Err(LabelError::MissingFont { family: list.to_string() });
                }
                MissingFontPolicy::Warn => {
                    for family in missing {
                        let warning =
                            LabelWarning::MissingFont { family: family.clone() };
                        if !warnings.contains(&warning) {
                            warnings.push(warning);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(warnings)
    }
}

impl Debug for LabelEngine {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.debug_struct("LabelEngine")
            .field("defaults", &self.defaults)
            .field("missing_font", &self.missing_font)
            .finish_non_exhaustive()
    }
}

/// Whether the book has a family.
fn has_family(book: &FontBook, family: &str) -> bool {
    book.select_family(&family.to_lowercase()).next().is_some()
}

/// Checks the size of a label's source.
fn check_size(source: &str, limits: LabelLimits) -> Result<(), LabelError> {
    if source.len() > limits.max_source_bytes {
        return Err(LabelError::SourceTooLarge {
            actual: source.len(),
            limit: limits.max_source_bytes,
        });
    }
    Ok(())
}

/// Checks a label's number of equations and how deep its math nests.
fn check_math(root: &SyntaxNode, limits: LabelLimits) -> Result<(), LabelError> {
    fn equations(node: &SyntaxNode) -> usize {
        usize::from(node.kind() == SyntaxKind::Equation)
            + node.children().map(equations).sum::<usize>()
    }
    let count = equations(root);
    if count > limits.max_math_spans {
        return Err(LabelError::TooManyMathSpans {
            actual: count,
            limit: limits.max_math_spans,
        });
    }
    let depth = math_nesting_depth(root);
    if depth > limits.max_math_depth {
        return Err(LabelError::MathDepthExceeded {
            actual: depth,
            limit: limits.max_math_depth,
        });
    }
    Ok(())
}

/// Escapes text so that, as a label, it is literal text: each character that can start markup
/// gets a backslash (`h`, for one, can start a link), and each run of line breaks becomes a
/// space.
pub fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if is_newline(c) {
            while chars.next_if(|&c| is_newline(c)).is_some() {}
            escaped.push(' ');
            continue;
        }
        if matches!(
            c,
            '\\' | '/'
                | '['
                | ']'
                | '~'
                | '-'
                | '+'
                | '='
                | '.'
                | '\''
                | '"'
                | '*'
                | '_'
                | ':'
                | '`'
                | '$'
                | '<'
                | '>'
                | '@'
                | '#'
                | 'h'
        ) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// The content that the markup of `escape_text(text)` evaluates to, with the spans of the
/// text: runs of whitespace are spaces, and everything else is text.
fn literal(text: &str) -> Content {
    let is_space = |c: char| c == ' ' || c == '\t' || is_newline(c);
    let mut children = vec![];
    let mut start = 0;
    while start < text.len() {
        let space = text[start..].starts_with(is_space);
        let end = text[start..]
            .find(|c: char| is_space(c) != space)
            .map_or(text.len(), |offset| start + offset);
        let span = Span::from_range(FileId::LABEL, start..end);
        children.push(if space {
            SpaceElem::shared().clone().spanned(span)
        } else {
            TextElem::packed(&text[start..end]).spanned(span)
        });
        start = end;
    }
    Content::sequence(children)
}

/// A typeset label, before lowering.
struct Typeset {
    frame: Frame,
    text: String,
    flags: LabelFlags,
    warnings: Vec<LabelWarning>,
}

impl Typeset {
    fn compiled(self, source: &str) -> CompiledLabel {
        CompiledLabel {
            source: source.into(),
            metrics: LabelMetrics::of(&self.frame),
            frame: lower(&self.frame),
            semantic_text: self.text,
            flags: self.flags,
            warnings: self.warnings,
        }
    }
}

/// The world a label compiles in: the engine's fonts, the label's source and formatting.
struct CompileWorld<'a> {
    fonts: &'a LabelWorld,
    source: &'a str,
    number_format: Option<&'a Arc<dyn NumberFormatProvider>>,
    datetime_format: Option<&'a Arc<dyn DateTimeFormatProvider>>,
    formatting_cache: &'a FormattingCache,
}

impl World for CompileWorld<'_> {
    fn book(&self) -> &FontBook {
        self.fonts.book()
    }

    fn source(&self, id: FileId) -> Option<&str> {
        (id == FileId::LABEL).then_some(self.source)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.font(index)
    }

    fn number_format(&self) -> Option<&Arc<dyn NumberFormatProvider>> {
        self.number_format
    }

    fn datetime_format(&self) -> Option<&Arc<dyn DateTimeFormatProvider>> {
        self.datetime_format
    }

    fn formatting_cache(&self) -> Option<&FormattingCache> {
        Some(self.formatting_cache)
    }
}

/// A compiled label.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledLabel {
    /// The label's source.
    pub source: String,
    /// The laid-out line.
    pub frame: LabelFrame,
    /// The line's metrics.
    pub metrics: LabelMetrics,
    /// The label's text in reading order, for text extraction: its text in logical order,
    /// with each equation's text in drawing order.
    pub semantic_text: String,
    /// What the label contains.
    pub flags: LabelFlags,
    /// The problems that didn't keep the label from compiling.
    pub warnings: Vec<LabelWarning>,
}

/// The metrics of a label's line, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelMetrics {
    /// The line's width.
    pub width: f32,
    /// The line's height.
    pub height: f32,
    /// The baseline's distance from the top.
    pub baseline: f32,
    /// The line's extent above the baseline.
    pub ascent: f32,
    /// The line's extent below the baseline.
    pub descent: f32,
}

impl LabelMetrics {
    /// The metrics of a laid-out line.
    fn of(frame: &Frame) -> Self {
        let pt = |abs: Abs| abs.to_pt() as f32;
        Self {
            width: pt(frame.width()),
            height: pt(frame.height()),
            baseline: pt(frame.baseline()),
            ascent: pt(frame.ascent()),
            descent: pt(frame.descent()),
        }
    }
}

/// What a label contains.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LabelFlags {
    /// Whether the label has an equation.
    pub has_math: bool,
}

/// A face's vertical metrics at a font size, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    /// The typographic ascender.
    pub ascent: f32,
    /// The typographic descender, as a positive distance below the baseline.
    pub descent: f32,
    /// The face's line gap.
    pub line_gap: f32,
}

impl FontMetrics {
    fn of(font: &FontInstance, size: f32) -> Self {
        let metrics = font.metrics();
        let ttf = font.ttf();
        let units = f64::from(ttf.units_per_em());
        let scale = |em: f64| (em * f64::from(size)) as f32;
        Self {
            ascent: scale(metrics.ascender.get()),
            descent: scale(-metrics.descender.get()),
            line_gap: scale(f64::from(ttf.line_gap()) / units),
        }
    }
}
