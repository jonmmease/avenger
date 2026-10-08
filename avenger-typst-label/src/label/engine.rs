//! The label engine: compiles labels through the pipeline, in the engine's world.

use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;

use avenger_format::{DateTimeFormatProvider, NumberFormatProvider};

use super::bounds::TextBounds;
use super::error::{LabelError, LabelWarning, source_error, source_warning};
use super::format::FormattingCache;
use super::frame::LabelFrame;
use super::lower::lower;
use super::memo::{LabelKey, Memo};
use super::options::{
    EngineOptions, Label, LabelLimits, LabelLineHeight, LabelOptions, LabelSource,
    LabelWidth, MissingFontPolicy, TextStyle,
};
use super::params::{self, LabelParams};
use super::styles::{Defaults, root_styles};
use super::world::LabelWorld;
use super::{label_file, label_span};
use crate::typst_eval::{eval_label, math_nesting_depth, parse_label};
use crate::typst_layout::inline::{LineExtent, LineOptions, layout_label};
use crate::typst_library::World;
use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{Content, StyleChain};
use crate::typst_library::layout::{Abs, Frame, InlineElem, Size};
use crate::typst_library::model::ParElem;
use crate::typst_library::routines::{Arenas, RealizationKind};
use crate::typst_library::text::{
    Font, FontBook, FontInstance, FontStretch, FontVariant, FontVariations,
    LinebreakElem, SpaceElem, TextElem,
};
use crate::typst_realize::realize;
#[cfg(feature = "raster")]
use crate::typst_render::RasterError;
use typst_syntax::{FileId, SyntaxKind, SyntaxNode, is_newline};

#[cfg(feature = "raster")]
use super::{memo::TextRasterKey, raster::TextRaster};

/// How many label boxes and rasters an engine remembers. Boxes are small, and interactive
/// charts measure the same few hundred labels from frame to frame.
const MEASURED_CAPACITY: usize = 8192;
#[cfg(feature = "raster")]
const RASTERIZED_CAPACITY: usize = 1024;
/// How many sources' references to params an engine remembers.
const REFERENCES_CAPACITY: usize = 8192;

/// Compiles labels: paragraphs of Typst markup with inline math, on one line or several.
///
/// An engine holds its fonts and caches, and is cheap to clone. Fonts load on first use. Clones
/// share the memos of labels' boxes and rasters; setting a formatting provider starts new
/// ones, since labels may then read differently.
#[derive(Clone)]
pub struct LabelEngine {
    /// The fonts.
    world: Arc<LabelWorld>,
    /// The families labels fall back to.
    defaults: Defaults,
    /// What happens when families of a label's font lists are not available.
    missing_font: MissingFontPolicy,
    /// The provider of `#numfmt`.
    number_format: Option<Arc<dyn NumberFormatProvider>>,
    /// The provider of `#datetimefmt`.
    datetime_format: Option<Arc<dyn DateTimeFormatProvider>>,
    /// The prepared formats of `#numfmt` and `#datetimefmt`, which belong to the providers.
    formatting_cache: Arc<FormattingCache>,
    /// The values that labels' sources can refer to by name.
    params: Arc<LabelParams>,
    /// The boxes of labels it measured.
    measured: Memo<LabelKey, TextBounds>,
    /// The rasters of labels it rasterized.
    #[cfg(feature = "raster")]
    rasterized: Memo<TextRasterKey, TextRaster>,
    /// The params that markup sources refer to.
    references: Memo<String, Arc<[String]>>,
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
            params: Arc::default(),
            measured: Memo::new(MEASURED_CAPACITY),
            #[cfg(feature = "raster")]
            rasterized: Memo::new(RASTERIZED_CAPACITY),
            references: Memo::new(REFERENCES_CAPACITY),
        }
    }

    /// Sets the provider of `#numfmt`. Its settings, such as the locale, apply to every
    /// pattern it prepares.
    pub fn with_number_formatting(
        mut self,
        provider: Arc<dyn NumberFormatProvider>,
    ) -> Self {
        self.number_format = Some(provider);
        self.with_new_memos()
    }

    /// The provider of `#numfmt`.
    pub fn number_format(&self) -> Option<&Arc<dyn NumberFormatProvider>> {
        self.number_format.as_ref()
    }

    /// Sets the provider of `#datetimefmt`. Its settings, such as the locale and the display
    /// timezone, apply to every pattern it prepares.
    pub fn with_datetime_formatting(
        mut self,
        provider: Arc<dyn DateTimeFormatProvider>,
    ) -> Self {
        self.datetime_format = Some(provider);
        self.with_new_memos()
    }

    /// The provider of `#datetimefmt`.
    pub fn datetime_format(&self) -> Option<&Arc<dyn DateTimeFormatProvider>> {
        self.datetime_format.as_ref()
    }

    /// An engine whose labels' sources can refer to these values by name. It shares this
    /// engine's fonts and caches, so deriving one for each render is cheap: memoized labels
    /// that don't refer to a changed value stay memoized.
    pub fn with_params(&self, params: LabelParams) -> Self {
        Self { params: Arc::new(params), ..self.clone() }
    }

    /// The engine with new formats and memos, for another provider.
    fn with_new_memos(mut self) -> Self {
        self.formatting_cache = Arc::default();
        self.measured = Memo::new(MEASURED_CAPACITY);
        #[cfg(feature = "raster")]
        {
            self.rasterized = Memo::new(RASTERIZED_CAPACITY);
        }
        self
    }

    /// A label's box, memoized, or its source's as literal text if its markup is invalid,
    /// as the outputs draw it.
    pub fn bounds(&self, label: &Label) -> Result<TextBounds, LabelError> {
        plain_fallback(
            label,
            |error| matches!(error, LabelError::Source { .. }),
            |label| {
                self.measured.get_or_try_insert(self.label_key(label), || {
                    let typeset = match label.source {
                        LabelSource::Text(text) => {
                            self.typeset_text(text, &label.options)
                        }
                        LabelSource::Markup(source) => {
                            self.typeset_markup(source, &label.options)
                        }
                    }?;
                    log_warnings(&typeset.warnings);
                    Ok(TextBounds::new(&typeset.metrics(), label.options.text.font_size))
                })
            },
        )
    }

    /// A label rasterized at a scale, memoized, or its source as literal text if its markup
    /// is invalid.
    #[cfg(feature = "raster")]
    pub fn raster(&self, label: &Label, scale: f32) -> Result<TextRaster, RasterError> {
        plain_fallback(
            label,
            |error| matches!(error, RasterError::Label(LabelError::Source { .. })),
            |label| {
                let key =
                    TextRasterKey::new(self.label_key(label), &label.options, scale);
                self.rasterized.get_or_try_insert(key.clone(), || {
                    let compiled = match label.source {
                        LabelSource::Text(text) => {
                            self.compile_text(text, &label.options)
                        }
                        LabelSource::Markup(source) => {
                            self.compile(source, &label.options)
                        }
                    }?;
                    log_warnings(&compiled.warnings);
                    super::raster::raster(
                        &compiled,
                        label.options.text.font_size,
                        scale,
                        key,
                    )
                })
            },
        )
    }

    /// What tells a label's memos apart: the label, and the values of the params its markup
    /// refers to.
    fn label_key(&self, label: &Label) -> LabelKey {
        match label.source {
            LabelSource::Text(_) => LabelKey::new(label, &[], &self.params),
            LabelSource::Markup(source) => {
                LabelKey::new(label, &self.referenced(source), &self.params)
            }
        }
    }

    /// The params a markup source refers to, memoized. Markup that doesn't parse refers to
    /// none, since it fails whatever the params.
    fn referenced(&self, source: &str) -> Arc<[String]> {
        if let Some(names) = self.references.get(source) {
            return names;
        }
        let names: Arc<[String]> =
            params::referenced_params(source).unwrap_or_default().into();
        self.references.insert(source.to_owned(), names.clone());
        names
    }

    /// Compiles a label's markup.
    pub fn compile(
        &self,
        source: &str,
        options: &LabelOptions,
    ) -> Result<CompiledLabel, LabelError> {
        Ok(self.typeset_markup(source, options)?.compiled(source))
    }

    /// Compiles literal text: the label `escape_text(text)` is, without parsing it, except that
    /// with [`LabelOptions::newline_breaks`] each newline ends a line.
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
        Ok(self.typeset_markup(source, options)?.metrics())
    }

    /// The metrics literal text compiles to.
    pub fn measure_text(
        &self,
        text: &str,
        options: &LabelOptions,
    ) -> Result<LabelMetrics, LabelError> {
        Ok(self.typeset_text(text, options)?.metrics())
    }

    /// The vertical metrics of the face that a text style's text uses first: the first
    /// available family of its font list, else the default sans-serif family, else the face
    /// that fallback gives Latin text.
    pub fn font_metrics(&self, style: &TextStyle) -> Result<FontMetrics, LabelError> {
        self.check_fonts(&[&style.font_family])?;
        let face = self.first_face(style).ok_or_else(|| LabelError::MissingFont {
            family: style.font_family.clone(),
        })?;
        Ok(FontMetrics::of(&face, style.font_size))
    }

    /// The face that a text style's text uses first, at its size: the first available family
    /// of its font list, else the default sans-serif family, else, as text falls back when
    /// none of its families is available, the face that fallback gives Latin text. None when
    /// no face covers Latin text.
    fn first_face(&self, style: &TextStyle) -> Option<FontInstance> {
        let variant =
            FontVariant::new(style.font_style, style.font_weight, FontStretch::NORMAL);
        let book = self.world.book();
        let size = Abs::pt(f64::from(style.font_size));
        self.world
            .families(&style.font_family)
            .into_iter()
            .chain([self.defaults.sans().to_string()])
            .find_map(|family| book.select(&family.to_lowercase(), variant))
            .or_else(|| book.select_fallback(None, variant, "A"))
            .and_then(|index| self.world.font(index))
            .map(|font| font.instantiate(variant, size, &FontVariations::default()))
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
    ) -> Result<Typeset, LabelError> {
        check_size(source, options.limits)?;
        let root = parse_label(source);
        check_math(&root, options.limits)?;
        let scope = params::scope(&self.params);
        self.typeset(source, options, |engine| eval_label(engine, &root, scope))
    }

    /// Typesets literal text.
    fn typeset_text(
        &self,
        text: &str,
        options: &LabelOptions,
    ) -> Result<Typeset, LabelError> {
        check_size(text, options.limits)?;
        self.typeset(text, options, |_| Ok(literal(text, options.newline_breaks)))
    }

    /// Realizes and lays out a label's content, which `content` makes in the label's world.
    fn typeset(
        &self,
        source: &str,
        options: &LabelOptions,
        content: impl FnOnce(&mut Engine) -> SourceResult<Content>,
    ) -> Result<Typeset, LabelError> {
        let (region, expand) = region(options.width)?;
        check_line_height(options.line_height)?;
        let mut warnings =
            self.check_fonts(&[&options.text.font_family, &options.math.font_family])?;
        let world = CompileWorld {
            fonts: &self.world,
            source,
            number_format: self.number_format.as_ref(),
            datetime_format: self.datetime_format.as_ref(),
            formatting_cache: &self.formatting_cache,
        };
        let mut sink = Sink::new();
        let styles = root_styles(&self.world, &self.defaults, options);
        let root = StyleChain::new(&styles);
        // The distance between the baselines of plain lines: the cap height of the text's
        // first face, from the top edge of one line, and the leading. Without a face for
        // Latin text, the leading remains.
        let size = Abs::pt(f64::from(options.text.font_size));
        let cap_height = self
            .first_face(&options.text)
            .map_or(Abs::zero(), |face| face.metrics().cap_height.at(size));
        let plain = cap_height + root.resolve(ParElem::leading);
        let (pitch, line_pitch) = match options.line_height {
            LabelLineHeight::Auto => (None, plain),
            LabelLineHeight::Fixed(distance) => {
                let distance = Abs::pt(f64::from(distance));
                (Some(distance), distance)
            }
            LabelLineHeight::Relative(multiple) => {
                let distance = plain * f64::from(multiple);
                (Some(distance), distance)
            }
        };
        let laid_out: SourceResult<_> = (|| {
            let mut engine = Engine { world: &world, sink: &mut sink };
            let content = content(&mut engine)?;
            let arenas = Arenas::default();
            let children =
                realize(RealizationKind::Par, &mut engine, &arenas, &content, root)?;
            let has_math = children.iter().any(|(child, _)| child.is::<InlineElem>());
            let lines = LineOptions {
                wrap: options.wrap,
                pitch,
                max_lines: options.max_lines,
                ellipsis: options.ellipsis,
                hanging_signs: options.hanging_signs,
            };
            let layout =
                layout_label(&mut engine, &children, root, region, expand, lines)?;
            Ok((layout, has_math))
        })();
        let (layout, has_math) =
            laid_out.map_err(|errors| source_error(source, &errors[0]))?;
        warnings.extend(
            sink.warnings().iter().map(|warning| source_warning(source, warning)),
        );
        Ok(Typeset {
            frame: layout.frame,
            lines: layout.lines,
            line_pitch,
            text: layout.text,
            flags: LabelFlags { has_math, truncated: layout.truncated },
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

/// A label's output, or, if its markup is invalid, its source's as literal text. Limit and
/// font errors don't fall back.
fn plain_fallback<T, E>(
    label: &Label,
    is_source_error: impl Fn(&E) -> bool,
    output: impl Fn(&Label) -> Result<T, E>,
) -> Result<T, E> {
    output(label).or_else(|error| match label.source {
        LabelSource::Markup(source) if is_source_error(&error) => output(&Label {
            source: LabelSource::Text(source),
            options: LabelOptions { newline_breaks: false, ..label.options.clone() },
        }),
        _ => Err(error),
    })
}

/// Logs a label's warnings, which outputs without the compiled label would drop.
fn log_warnings(warnings: &[LabelWarning]) {
    for warning in warnings {
        tracing::warn!(?warning, "label typesetting warning");
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

/// The region a label's lines fill, and whether the label expands to the region's width, as a
/// box of that width does.
fn region(width: LabelWidth) -> Result<(Size, bool), LabelError> {
    let (width, expand) = match width {
        LabelWidth::Auto => return Ok((Size::splat(Abs::inf()), false)),
        LabelWidth::Max(width) => (width, false),
        LabelWidth::Fixed(width) => (width, true),
    };
    if !width.is_finite() || width < 0.0 {
        return Err(LabelError::InvalidWidth { width });
    }
    Ok((Size::new(Abs::pt(width.into()), Abs::inf()), expand))
}

/// Checks a line height's distance or multiple.
fn check_line_height(line_height: LabelLineHeight) -> Result<(), LabelError> {
    match line_height {
        LabelLineHeight::Fixed(value) | LabelLineHeight::Relative(value)
            if !value.is_finite() || value < 0.0 =>
        {
            Err(LabelError::InvalidLineHeight { line_height: value })
        }
        _ => Ok(()),
    }
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
/// text: runs of whitespace are spaces, and everything else is text. With newline breaks, each
/// newline, a carriage return and line feed counting once, is a line break instead.
fn literal(text: &str, newline_breaks: bool) -> Content {
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Text,
        Space,
        Break,
    }
    let kind = |c: char| {
        if is_newline(c) && newline_breaks {
            Kind::Break
        } else if c == ' ' || c == '\t' || is_newline(c) {
            Kind::Space
        } else {
            Kind::Text
        }
    };
    let mut children = vec![];
    let mut start = 0;
    while let Some(c) = text[start..].chars().next() {
        let first = kind(c);
        let end = match first {
            Kind::Break if text[start..].starts_with("\r\n") => start + 2,
            Kind::Break => start + c.len_utf8(),
            _ => text[start..]
                .find(|c: char| kind(c) != first)
                .map_or(text.len(), |offset| start + offset),
        };
        let span = label_span(start..end);
        children.push(match first {
            Kind::Text => TextElem::packed(&text[start..end]).spanned(span),
            Kind::Space => SpaceElem::shared().clone().spanned(span),
            Kind::Break => LinebreakElem::shared().clone().spanned(span),
        });
        start = end;
    }
    Content::sequence(children)
}

/// A typeset label, before lowering.
struct Typeset {
    frame: Frame,
    lines: Vec<LineExtent>,
    line_pitch: Abs,
    text: String,
    flags: LabelFlags,
    warnings: Vec<LabelWarning>,
}

impl Typeset {
    /// The label's metrics, in points.
    fn metrics(&self) -> LabelMetrics {
        let pt = |abs: Abs| abs.to_pt() as f32;
        LabelMetrics {
            width: pt(self.frame.width()),
            height: pt(self.frame.height()),
            line_pitch: pt(self.line_pitch),
            lines: self
                .lines
                .iter()
                .map(|line| LineMetrics {
                    left: pt(line.left),
                    right: pt(line.right),
                    top: pt(line.top),
                    baseline: pt(line.baseline),
                    bottom: pt(line.bottom),
                })
                .collect(),
        }
    }

    fn compiled(self, source: &str) -> CompiledLabel {
        CompiledLabel {
            source: source.into(),
            metrics: self.metrics(),
            frame: lower(&self.frame),
            semantic_text: self.text,
            flags: self.flags,
            warnings: self.warnings,
        }
    }
}

/// The world a label compiles in: the engine's fonts and formatting, and the label's source.
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
        (id == label_file()).then_some(self.source)
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
    /// The laid-out label: its lines, stacked.
    pub frame: LabelFrame,
    /// The label's metrics.
    pub metrics: LabelMetrics,
    /// The label's text in reading order, for text extraction: its text in logical order,
    /// with each equation's text in drawing order, and a newline where an explicit break ends
    /// a line.
    pub semantic_text: String,
    /// What the label contains.
    pub flags: LabelFlags,
    /// The problems that didn't keep the label from compiling.
    pub warnings: Vec<LabelWarning>,
}

/// The metrics of a label, in points.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelMetrics {
    /// The label's width.
    pub width: f32,
    /// The label's height, from the top of its lines to their bottom.
    pub height: f32,
    /// The distance between the baselines of two lines of plain text in the label's text
    /// style, under its line height: with Typst's spacing, the cap height of the face the text
    /// uses first plus the leading; with a line height, its distance, or its multiple of that.
    pub line_pitch: f32,
    /// The label's lines, first to last; there is always at least one. The first line's
    /// baseline is the label's, which Typst aligns a box of several lines by.
    pub lines: Vec<LineMetrics>,
}

/// Where a line lies in its label, in points from the label's top left.
///
/// Across, a line spans what it draws: its text by its advances, and its equations and
/// decorations. That is where alignment put it, and past the label's width when a word
/// overflows. A hanging sign lies outside its line, and an empty line lies where its alignment
/// would put content.
///
/// Down, a line is as tall as its own content: its top and bottom are its text's edges, by
/// default the cap height and the baseline, pushed out by anything that reaches further, such
/// as a fraction. With Typst's spacing, the leading lies between one line's bottom and the next
/// line's top; with a line height, consecutive baselines lie its distance apart, and lines can
/// overlap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineMetrics {
    /// The left of what the line draws, other than a hanging sign.
    pub left: f32,
    /// The right of what the line draws, other than a hanging sign.
    pub right: f32,
    /// The line's top.
    pub top: f32,
    /// The line's baseline.
    pub baseline: f32,
    /// The line's bottom.
    pub bottom: f32,
}

/// What a label contains.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LabelFlags {
    /// Whether the label has an equation.
    pub has_math: bool,
    /// Whether the label shows less than its source: its line limit dropped lines that show
    /// something, or, with an ellipsis, lines were shortened to fit its width.
    pub truncated: bool,
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
