use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use comemo::Track;
use ttf_parser::{GlyphId, OutlineBuilder};

use crate::api::TypstEngineConfig;
use crate::delimiter::{parse_segments, ParsedSegment};
use crate::error::{MathTypesetError, TypstInitError};
use crate::paths::{
    MathPathArtifact, MathPathCommand, MathPathData, MathPathItem, MathPathKind, MathStroke,
    MathTransform,
};
use crate::pdf::{
    MathFontResource, MathFontResourceId, MathPdfGlyph, MathPdfGlyphRun, MathPdfTextLayer,
};
#[cfg(feature = "raster")]
use crate::raster::rasterize_path_artifact;
use crate::raster::RasterRequest;
use crate::style::{
    Color, FontStyle as AvengerFontStyle, FontWeight as AvengerFontWeight, MathDisplayStyle,
    MathStyle, PlainTextStyle,
};
use crate::types::{
    MathFragmentOptions, MathRunArtifact, PositionedTextLineRun, PositionedTextLineRunKind,
    TextLineArtifact, TextLineOptions, TypesetMetrics,
};

use typst_library::diag::{FileError, FileResult, SourceResult};
use typst_library::engine::{Engine, Route, Sink, Traced};
use typst_library::foundations::{
    Args, Closure, Content, Context, Func, Module, NativeElement, NativeRuleMap, Packed, Scope,
    SequenceElem, ShowSet, StyleChain, Styles, SymbolElem, Value,
};
use typst_library::introspection::{
    EmptyIntrospector, Introspector, Location, Locator, Tag, TagElem, TagFlags,
};
use typst_library::layout::{
    Abs, Frame, FrameItem, InlineElem, InlineItem, Point, Size, Transform,
};
use typst_library::math::{
    AlignPointElem, AttachElem, EquationElem, FracElem, LrElem, MatElem, MathSize, PrimesElem,
    RootElem,
};
use typst_library::routines::{Arenas, Pair, RealizationKind, Routines, SpanMode};
use typst_library::text::{
    is_default_ignorable, Font, FontBook, FontFamily, FontFlags, FontInstance, FontList, FontStyle,
    FontWeight, SpaceElem, TextElem, TextItem, TextSize,
};
use typst_library::visualize::{
    Color as TypstColor, Curve, CurveItem, FixedStroke, Geometry, Paint, ProcessColorSpace, Shape,
};
use typst_library::{Library, LibraryBuilder, World};
use typst_syntax::ast::{self, Arg, MathTextKind};
use typst_syntax::{FileId, Source, SyntaxMode};
use typst_utils::{hash128, LazyHash, Protected};

#[derive(Clone)]
pub(crate) struct TypstMathEngine {
    world: Arc<TypstMathWorld>,
    math_font_family: String,
    text_font_family: String,
}

impl std::fmt::Debug for TypstMathEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypstMathEngine")
            .field("math_font_family", &self.math_font_family)
            .field("text_font_family", &self.text_font_family)
            .finish_non_exhaustive()
    }
}

struct TextLineLayout {
    frame: Frame,
    segments: Vec<TextLineSegment>,
}

#[derive(Clone)]
struct TextLineSegment {
    index: usize,
    kind: PositionedTextLineRunKind,
    text: String,
    byte_range: std::ops::Range<usize>,
    marker: TextLineSegmentMarker,
}

#[derive(Clone, Copy)]
struct TextLineSegmentMarker {
    location: Location,
    key: u128,
}

impl TypstMathEngine {
    pub(crate) fn new(config: &TypstEngineConfig) -> Result<Self, TypstInitError> {
        let fonts = load_typst_fonts(config)?;
        let book = FontBook::from_fonts(&fonts);
        let math_font_family = select_math_font_family(&book, config)?;
        let text_font_family = select_default_text_font_family(&book)?;
        let world = Arc::new(TypstMathWorld::new(fonts));

        Ok(Self {
            world,
            math_font_family,
            text_font_family,
        })
    }

    pub(crate) fn typeset_fragment(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, MathTypesetError> {
        let items = self.layout_inline_items(source, options)?;
        let metrics = metrics_from_inline_items(&items, source.len())?;
        let path_artifact = if options.outputs.paths || options.outputs.raster.is_some() {
            Some(paths_from_inline_items(&items, metrics)?)
        } else {
            None
        };
        let raster = raster_from_path_artifact(path_artifact.as_ref(), options)?;
        let paths = if options.outputs.paths {
            path_artifact
        } else {
            None
        };
        let pdf_artifact = if options.outputs.pdf_text_layer {
            Some(pdf_text_from_inline_items(&items, metrics, source)?)
        } else {
            None
        };
        let (pdf_text, font_resources) = match pdf_artifact {
            Some(artifact) => (Some(artifact.text_layer), artifact.font_resources),
            None => (None, Vec::new()),
        };

        Ok(MathRunArtifact {
            metrics,
            paths,
            raster,
            pdf_text,
            font_resources,
            warnings: Vec::new(),
        })
    }

    pub(crate) fn typeset_text_line(
        &self,
        source: &str,
        options: &TextLineOptions,
    ) -> Result<TextLineArtifact, MathTypesetError> {
        let layout = self.layout_text_line_frame(source, options)?;
        let metrics = metrics_from_frame(&layout.frame);
        let path_artifact = if options.outputs.paths || options.outputs.raster.is_some() {
            Some(paths_from_frame(&layout.frame, metrics)?)
        } else {
            None
        };
        let raster =
            raster_from_path_artifact_request(path_artifact.as_ref(), options.outputs.raster)?;
        let paths = if options.outputs.paths {
            path_artifact
        } else {
            None
        };
        let pdf_artifact = if options.outputs.pdf_text_layer {
            Some(pdf_text_from_frame(&layout.frame, metrics, source)?)
        } else {
            None
        };
        let (pdf_text, font_resources) = match pdf_artifact {
            Some(artifact) => (Some(artifact.text_layer), artifact.font_resources),
            None => (None, Vec::new()),
        };
        let positioned_runs = if options.outputs.positioned_runs {
            positioned_runs_from_layout(&layout, metrics, options.outputs.pdf_text_layer)?
        } else {
            Vec::new()
        };

        Ok(TextLineArtifact {
            source: source.to_string(),
            metrics,
            paths,
            raster,
            pdf_text,
            positioned_runs,
            font_resources,
            warnings: Vec::new(),
        })
    }

    fn layout_inline_items(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<Vec<InlineItem>, MathTypesetError> {
        let body = lower_math_source(source)?;
        let equation = Packed::new(EquationElem::new(body).with_block(false));

        let library = self.world.library();
        let root_styles = StyleChain::new(&library.styles);
        let equation_styles = equation.show_set(root_styles);
        let override_styles = self.override_styles(options);
        let equation_chain = root_styles.chain(&equation_styles);
        let styles = equation_chain.chain(&override_styles);

        let world: &dyn World = self.world.as_ref();
        let empty_introspector = EmptyIntrospector;
        let traced = Traced::default();
        let mut sink = Sink::new();
        let mut engine = Engine {
            world: world.track(),
            library,
            introspector: Protected::new(empty_introspector.track()),
            traced: traced.track(),
            sink: sink.track_mut(),
            route: Route::default(),
        };

        let region = Size::new(Abs::inf(), Abs::inf());
        let items = typst_layout::layout_equation_inline(
            &equation,
            &mut engine,
            Locator::root(),
            styles,
            region,
        )
        .map_err(|errors| MathTypesetError::Engine {
            start: 0,
            end: source.len(),
            message: errors
                .into_iter()
                .next()
                .map(|error| error.message.to_string())
                .unwrap_or_else(|| "Typst math layout failed".to_string()),
        })?;

        Ok(items)
    }

    fn layout_text_line_frame(
        &self,
        source: &str,
        options: &TextLineOptions,
    ) -> Result<TextLineLayout, MathTypesetError> {
        let segments = parse_segments(source, &options.delimiters)?;
        let library = self.world.library();
        let root_styles = StyleChain::new(&library.styles);

        let mut contents = Vec::new();
        let mut segment_styles = Vec::new();
        let mut line_segments = Vec::new();

        for segment in segments {
            match segment {
                ParsedSegment::Plain { text, range } => {
                    if text.is_empty() {
                        continue;
                    }
                    let line_segment = tagged_text_line_segment(
                        source,
                        line_segments.len(),
                        PositionedTextLineRunKind::Plain,
                        text.clone(),
                        range,
                    );
                    contents.push(segment_tag_content(line_segment.marker, true));
                    segment_styles.push(Styles::new());
                    contents.push(TextElem::packed(text));
                    segment_styles.push(self.plain_text_styles(&options.text_style));
                    contents.push(segment_tag_content(line_segment.marker, false));
                    segment_styles.push(Styles::new());
                    line_segments.push(line_segment);
                }
                ParsedSegment::Math {
                    source: math_source,
                    source_range,
                    ..
                } => {
                    let body = lower_math_source(&math_source)?;
                    let equation = Packed::new(EquationElem::new(body).with_block(false));
                    let line_segment = tagged_text_line_segment(
                        source,
                        line_segments.len(),
                        PositionedTextLineRunKind::Math,
                        math_source,
                        source_range,
                    );
                    contents.push(segment_tag_content(line_segment.marker, true));
                    segment_styles.push(Styles::new());
                    contents.push(
                        InlineElem::layouter(equation, typst_layout::layout_equation_inline).pack(),
                    );
                    segment_styles.push(self.math_text_styles(&options.math_style));
                    contents.push(segment_tag_content(line_segment.marker, false));
                    segment_styles.push(Styles::new());
                    line_segments.push(line_segment);
                }
            }
        }

        if contents.is_empty() {
            let mut frame = Frame::soft(Size::zero());
            frame.set_baseline(Abs::zero());
            return Ok(TextLineLayout {
                frame,
                segments: line_segments,
            });
        }

        let chains = segment_styles
            .iter()
            .map(|styles| root_styles.chain(styles))
            .collect::<Vec<_>>();
        let pairs = contents
            .iter()
            .zip(chains.iter())
            .map(|(content, styles)| (content, *styles))
            .collect::<Vec<_>>();

        let world: &dyn World = self.world.as_ref();
        let empty_introspector = EmptyIntrospector;
        let traced = Traced::default();
        let mut sink = Sink::new();
        let mut engine = Engine {
            world: world.track(),
            library,
            introspector: Protected::new(empty_introspector.track()),
            traced: traced.track(),
            sink: sink.track_mut(),
            route: Route::default(),
        };

        let region = Size::new(Abs::inf(), Abs::inf());
        let mut locator = Locator::root().split();
        let fragment = typst_layout::layout_inline(
            &mut engine,
            &pairs,
            &mut locator,
            root_styles,
            region,
            false,
        )
        .map_err(|errors| MathTypesetError::Engine {
            start: 0,
            end: source.len(),
            message: errors
                .into_iter()
                .next()
                .map(|error| error.message.to_string())
                .unwrap_or_else(|| "Typst text line layout failed".to_string()),
        })?;

        let mut frames = fragment.into_frames();
        let frame = if frames.len() == 1 {
            frames.remove(0)
        } else {
            return Err(MathTypesetError::UnsupportedOutput(
                "Typst text line layout produced multiple frames",
            ));
        };

        Ok(TextLineLayout {
            frame,
            segments: line_segments,
        })
    }

    fn override_styles(&self, options: &MathFragmentOptions) -> Styles {
        let mut styles = Styles::new();
        styles.set(
            TextElem::font,
            FontList(vec![FontFamily::new(&self.math_font_family)]),
        );
        styles.set(
            TextElem::size,
            TextSize(Abs::pt(options.style.font_size.max(1.0) as f64).into()),
        );
        styles.set(
            TextElem::fill,
            Paint::Solid(typst_color(options.style.fill)),
        );
        if matches!(options.style.display_style, MathDisplayStyle::Display) {
            styles.set(EquationElem::size, MathSize::Display);
        }
        styles
    }

    fn plain_text_styles(&self, style: &PlainTextStyle) -> Styles {
        let mut styles = Styles::new();
        styles.set(
            TextElem::font,
            FontList(vec![FontFamily::new(
                &self.resolve_text_font_family(&style.font_family),
            )]),
        );
        styles.set(
            TextElem::size,
            TextSize(Abs::pt(style.font_size.max(1.0) as f64).into()),
        );
        styles.set(TextElem::fill, Paint::Solid(typst_color(style.fill)));
        styles.set(TextElem::weight, typst_font_weight(&style.font_weight));
        styles.set(TextElem::style, typst_font_style(style.font_style));
        styles
    }

    fn math_text_styles(&self, style: &MathStyle) -> Styles {
        let mut styles = Styles::new();
        styles.set(
            TextElem::font,
            FontList(vec![FontFamily::new(&self.math_font_family)]),
        );
        styles.set(
            TextElem::size,
            TextSize(Abs::pt(style.font_size.max(1.0) as f64).into()),
        );
        styles.set(TextElem::fill, Paint::Solid(typst_color(style.fill)));
        styles.set(TextElem::weight, FontWeight::from_number(450));
        styles.set(
            EquationElem::size,
            if matches!(style.display_style, MathDisplayStyle::Display) {
                MathSize::Display
            } else {
                MathSize::Text
            },
        );
        styles
    }

    fn resolve_text_font_family(&self, requested: &str) -> String {
        let book = self.world.book();
        for family in requested.split(',').map(normalize_font_family) {
            if family.eq_ignore_ascii_case("sans-serif") {
                return self.text_font_family.clone();
            }
            if family.eq_ignore_ascii_case("serif") || family.eq_ignore_ascii_case("monospace") {
                continue;
            }
            if let Some(family) = find_font_family(book, &family) {
                return family;
            }
        }
        self.text_font_family.clone()
    }
}

#[cfg(feature = "raster")]
fn raster_from_path_artifact(
    path_artifact: Option<&MathPathArtifact>,
    options: &MathFragmentOptions,
) -> Result<Option<crate::raster::MathRasterArtifact>, MathTypesetError> {
    raster_from_path_artifact_request(path_artifact, options.outputs.raster)
}

#[cfg(not(feature = "raster"))]
fn raster_from_path_artifact(
    _path_artifact: Option<&MathPathArtifact>,
    options: &MathFragmentOptions,
) -> Result<Option<crate::raster::MathRasterArtifact>, MathTypesetError> {
    if options.outputs.raster.is_some() {
        Err(MathTypesetError::UnsupportedOutput(
            "avenger-typst raster output requires the raster feature",
        ))
    } else {
        Ok(None)
    }
}

#[cfg(feature = "raster")]
fn raster_from_path_artifact_request(
    path_artifact: Option<&MathPathArtifact>,
    request: Option<RasterRequest>,
) -> Result<Option<crate::raster::MathRasterArtifact>, MathTypesetError> {
    request
        .map(|request| {
            let path_artifact = path_artifact.ok_or(MathTypesetError::UnsupportedOutput(
                "path output is required for raster output",
            ))?;
            rasterize_path_artifact(path_artifact, request)
        })
        .transpose()
}

#[cfg(not(feature = "raster"))]
fn raster_from_path_artifact_request(
    _path_artifact: Option<&MathPathArtifact>,
    request: Option<RasterRequest>,
) -> Result<Option<crate::raster::MathRasterArtifact>, MathTypesetError> {
    if request.is_some() {
        Err(MathTypesetError::UnsupportedOutput(
            "avenger-typst raster output requires the raster feature",
        ))
    } else {
        Ok(None)
    }
}

struct TypstMathWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    source: Source,
}

impl TypstMathWorld {
    fn new(fonts: Vec<Font>) -> Self {
        let book = FontBook::from_fonts(&fonts);
        let library = LibraryBuilder::from_routines(&ROUTINES).build();
        Self {
            library: LazyHash::new(library),
            book: LazyHash::new(book),
            fonts,
            source: Source::detached(""),
        }
    }
}

impl World for TypstMathWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.source.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.source.id() {
            Ok(self.source.clone())
        } else {
            Err(FileError::Other(Some("unknown source file".into())))
        }
    }

    fn file(&self, _id: FileId) -> FileResult<typst_library::foundations::Bytes> {
        Err(FileError::Other(Some(
            "file loading is disabled in avenger-typst math fragments".into(),
        )))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(
        &self,
        _offset: Option<typst_library::foundations::Duration>,
    ) -> Option<typst_library::foundations::Datetime> {
        None
    }
}

static ROUTINES: LazyLock<Routines> = LazyLock::new(|| Routines {
    rules: || {
        let mut rules = NativeRuleMap::new();
        typst_layout::register(&mut rules);
        rules
    },
    eval_string: unsupported_eval_string,
    eval_closure: unsupported_eval_closure,
    realize: realize_math_subset,
    layout_frame: typst_layout::layout_frame,
    html_module: empty_html_module,
    html_mathml_body: no_html_mathml_body,
    html_span_filled: pass_through_html_span,
});

fn unsupported_eval_string(
    _world: comemo::Tracked<dyn World + '_>,
    _library: &LazyHash<Library>,
    _sink: comemo::TrackedMut<Sink>,
    _introspector: comemo::Tracked<dyn Introspector + '_>,
    _context: comemo::Tracked<Context>,
    _string: &str,
    _spans: SpanMode,
    _mode: SyntaxMode,
    _scope: Scope,
) -> SourceResult<Value> {
    panic!("string evaluation is disabled in avenger-typst math fragments")
}

fn unsupported_eval_closure(
    _func: &Func,
    _closure: &LazyHash<Closure>,
    _world: comemo::Tracked<dyn World + '_>,
    _library: &LazyHash<Library>,
    _introspector: comemo::Tracked<dyn Introspector + '_>,
    _traced: comemo::Tracked<Traced>,
    _sink: comemo::TrackedMut<Sink>,
    _route: comemo::Tracked<Route>,
    _context: comemo::Tracked<Context>,
    _args: Args,
) -> SourceResult<Value> {
    panic!("closure evaluation is disabled in avenger-typst math fragments")
}

fn realize_math_subset<'a>(
    _kind: RealizationKind,
    _engine: &mut Engine,
    _locator: &mut typst_library::introspection::SplitLocator,
    _arenas: &'a Arenas,
    content: &'a Content,
    styles: StyleChain<'a>,
) -> SourceResult<Vec<Pair<'a>>> {
    let mut pairs = Vec::new();
    collect_realized_pairs(content, styles, &mut pairs);
    Ok(pairs)
}

fn collect_realized_pairs<'a>(
    content: &'a Content,
    styles: StyleChain<'a>,
    pairs: &mut Vec<Pair<'a>>,
) {
    if let Some(sequence) = content.to_packed::<SequenceElem>() {
        for child in &sequence.children {
            collect_realized_pairs(child, styles, pairs);
        }
    } else {
        pairs.push((content, styles));
    }
}

fn empty_html_module() -> Module {
    Module::new("html", Scope::new())
}

fn no_html_mathml_body<'a>(
    _content: &'a Content,
    _styles: StyleChain<'a>,
) -> Option<Option<&'a Content>> {
    None
}

fn pass_through_html_span(content: Content, _color: TypstColor) -> Content {
    content
}

fn load_typst_fonts(config: &TypstEngineConfig) -> Result<Vec<Font>, TypstInitError> {
    let mut fonts = Vec::new();
    for face in crate::fonts::ATKINSON_FACES {
        fonts.extend(Font::iter(typst_library::foundations::Bytes::new(
            face.data.to_vec(),
        )));
    }

    for path in candidate_font_paths(config) {
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        fonts.extend(
            Font::iter(typst_library::foundations::Bytes::new(data))
                .filter(|font| font.info().flags.contains(FontFlags::MATH)),
        );
    }

    if !fonts
        .iter()
        .any(|font| font.info().flags.contains(FontFlags::MATH))
    {
        return Err(TypstInitError::BackendUnavailable(
            "no supported math font found",
        ));
    }

    Ok(fonts)
}

fn candidate_font_paths(config: &TypstEngineConfig) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut push = |path: PathBuf| {
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    };

    for path in hardcoded_math_font_paths() {
        push(path.into());
    }

    for dir in system_font_dirs() {
        collect_font_paths(
            dir,
            &mut push,
            !config.font_config.extra_font_families.is_empty(),
        );
    }

    paths
}

fn hardcoded_math_font_paths() -> &'static [&'static str] {
    &[
        "/System/Library/Fonts/Supplemental/STIXTwoMath.otf",
        "/Library/Fonts/STIXTwoMath.otf",
        "/usr/share/fonts/opentype/stix/STIXTwoMath-Regular.otf",
        "/usr/share/fonts/opentype/stix/STIXTwoMath.otf",
        "/usr/share/fonts/truetype/noto/NotoSansMath-Regular.ttf",
        "C:\\Windows\\Fonts\\cambria.ttc",
    ]
}

fn system_font_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
        PathBuf::from("C:\\Windows\\Fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Library/Fonts"));
    }
    dirs
}

fn collect_font_paths(dir: PathBuf, push: &mut impl FnMut(PathBuf), include_all_fonts: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_font_paths(path, push, include_all_fonts);
            continue;
        }

        if !is_font_file(&path) {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if include_all_fonts || file_name.contains("math") || file_name.contains("stix") {
            push(path);
        }
    }
}

fn is_font_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("otf" | "ttf" | "ttc")
    )
}

fn select_math_font_family(
    book: &FontBook,
    config: &TypstEngineConfig,
) -> Result<String, TypstInitError> {
    let preferred = config
        .font_config
        .extra_font_families
        .iter()
        .map(String::as_str)
        .chain([
            "New Computer Modern Math",
            "STIX Two Math",
            "Cambria Math",
            "Noto Sans Math",
            "STIXGeneral",
        ]);

    for family in preferred {
        if let Some(family) = find_math_font_family(book, family) {
            return Ok(family);
        }
    }

    Err(TypstInitError::BackendUnavailable(
        "no supported math font found",
    ))
}

fn select_default_text_font_family(book: &FontBook) -> Result<String, TypstInitError> {
    for family in ["Atkinson Hyperlegible Next", "Arial", "Helvetica"] {
        if let Some(family) = find_font_family(book, family) {
            return Ok(family);
        }
    }

    book.families()
        .next()
        .map(|(family, _)| family.to_string())
        .ok_or(TypstInitError::BackendUnavailable(
            "no supported text font found",
        ))
}

fn find_font_family(book: &FontBook, family: &str) -> Option<String> {
    book.families().find_map(|(candidate, _)| {
        candidate
            .eq_ignore_ascii_case(family)
            .then(|| candidate.to_string())
    })
}

fn find_math_font_family(book: &FontBook, family: &str) -> Option<String> {
    book.families().find_map(|(candidate, ids)| {
        if !candidate.eq_ignore_ascii_case(family) {
            return None;
        }

        ids.into_iter()
            .any(|id| {
                book.info(id)
                    .is_some_and(|info| info.flags.contains(FontFlags::MATH))
            })
            .then(|| candidate.to_string())
    })
}

fn tagged_text_line_segment(
    source: &str,
    index: usize,
    kind: PositionedTextLineRunKind,
    text: String,
    byte_range: std::ops::Range<usize>,
) -> TextLineSegment {
    let key = hash128(&(
        "avenger-typst-text-line-segment",
        source,
        index,
        byte_range.start,
        byte_range.end,
        kind,
    ));
    TextLineSegment {
        index,
        kind,
        text,
        byte_range,
        marker: TextLineSegmentMarker {
            location: Location::new(key),
            key,
        },
    }
}

fn segment_tag_content(marker: TextLineSegmentMarker, start: bool) -> Content {
    let flags = TagFlags {
        introspectable: false,
        tagged: false,
    };
    if start {
        let mut marker_content = TextElem::packed("");
        marker_content.set_location(marker.location);
        TagElem::packed(Tag::Start(marker_content, flags))
    } else {
        TagElem::packed(Tag::End(marker.location, marker.key, flags))
    }
}

fn normalize_font_family(family: &str) -> String {
    family
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn typst_font_weight(weight: &AvengerFontWeight) -> FontWeight {
    match weight {
        AvengerFontWeight::Normal => FontWeight::REGULAR,
        AvengerFontWeight::Bold => FontWeight::BOLD,
        AvengerFontWeight::Number(value) => FontWeight::from_number(*value),
    }
}

fn typst_font_style(style: AvengerFontStyle) -> FontStyle {
    match style {
        AvengerFontStyle::Normal => FontStyle::Normal,
        AvengerFontStyle::Italic => FontStyle::Italic,
        AvengerFontStyle::Oblique => FontStyle::Oblique,
    }
}

fn metrics_from_inline_items(
    items: &[InlineItem],
    source_len: usize,
) -> Result<TypesetMetrics, MathTypesetError> {
    let mut width = Abs::zero();
    let mut ascent = Abs::zero();
    let mut descent = Abs::zero();
    let mut has_frame = false;

    for item in items {
        match item {
            InlineItem::Space(amount, _) => {
                width += *amount;
            }
            InlineItem::Frame(frame) => {
                width += frame.width();
                ascent = ascent.max(frame.ascent());
                descent = descent.max(frame.descent());
                has_frame = true;
            }
        }
    }

    if !has_frame {
        return Err(MathTypesetError::Engine {
            start: 0,
            end: source_len,
            message: "Typst math layout did not produce a frame".to_string(),
        });
    }

    Ok(TypesetMetrics {
        width: abs_to_f32(width),
        height: abs_to_f32(ascent + descent),
        baseline: abs_to_f32(ascent),
        ascent: abs_to_f32(ascent),
        descent: abs_to_f32(descent),
    })
}

fn metrics_from_frame(frame: &Frame) -> TypesetMetrics {
    TypesetMetrics {
        width: abs_to_f32(frame.width()),
        height: abs_to_f32(frame.height()),
        baseline: abs_to_f32(frame.baseline()),
        ascent: abs_to_f32(frame.ascent()),
        descent: abs_to_f32(frame.descent()),
    }
}

fn paths_from_inline_items(
    items: &[InlineItem],
    metrics: TypesetMetrics,
) -> Result<MathPathArtifact, MathTypesetError> {
    let mut lowerer = PathLowerer::default();
    let mut x = Abs::zero();
    let baseline = Abs::pt(metrics.baseline as f64);

    for item in items {
        match item {
            InlineItem::Space(amount, _) => {
                x += *amount;
            }
            InlineItem::Frame(frame) => {
                let y = baseline - frame.ascent();
                lowerer.lower_frame(frame, Transform::translate(x, y))?;
                x += frame.width();
            }
        }
    }

    Ok(MathPathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items: lowerer.items,
    })
}

fn paths_from_frame(
    frame: &Frame,
    metrics: TypesetMetrics,
) -> Result<MathPathArtifact, MathTypesetError> {
    let mut lowerer = PathLowerer::default();
    lowerer.lower_frame(frame, Transform::identity())?;

    Ok(MathPathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items: lowerer.items,
    })
}

struct ActivePositionedSegment {
    segment_index: usize,
    start: Point,
    items: Vec<(Point, FrameItem)>,
}

fn positioned_runs_from_layout(
    layout: &TextLineLayout,
    line_metrics: TypesetMetrics,
    include_pdf_text_layer: bool,
) -> Result<Vec<PositionedTextLineRun>, MathTypesetError> {
    let segment_by_location = layout
        .segments
        .iter()
        .map(|segment| (segment.marker.location, segment.index))
        .collect::<HashMap<_, _>>();
    let mut active: Option<ActivePositionedSegment> = None;
    let mut runs = Vec::new();

    for (pos, item) in layout.frame.items() {
        if let FrameItem::Tag(tag) = item {
            match tag {
                Tag::Start(content, _) => {
                    if let Some(location) = content.location() {
                        if let Some(&segment_index) = segment_by_location.get(&location) {
                            if active.is_some() {
                                return Err(MathTypesetError::UnsupportedOutput(
                                    "nested Avenger Typst text segment tags are not supported",
                                ));
                            }
                            active = Some(ActivePositionedSegment {
                                segment_index,
                                start: *pos,
                                items: Vec::new(),
                            });
                            continue;
                        }
                    }
                }
                Tag::End(location, _, _) => {
                    if let Some(&segment_index) = segment_by_location.get(location) {
                        let Some(segment) = active.take() else {
                            return Err(MathTypesetError::UnsupportedOutput(
                                "Typst text segment end tag did not have a matching start tag",
                            ));
                        };
                        if segment.segment_index != segment_index {
                            return Err(MathTypesetError::UnsupportedOutput(
                                "Typst text segment tags closed out of order",
                            ));
                        }
                        runs.push(positioned_run_from_segment(
                            &layout.segments[segment_index],
                            segment,
                            *pos,
                            line_metrics,
                            include_pdf_text_layer,
                        )?);
                        continue;
                    }
                }
            }
        }

        if let Some(segment) = &mut active {
            segment.items.push((*pos, item.clone()));
        }
    }

    if active.is_some() {
        return Err(MathTypesetError::UnsupportedOutput(
            "Typst text segment start tag did not have a matching end tag",
        ));
    }
    if runs.len() != layout.segments.len() {
        return Err(MathTypesetError::UnsupportedOutput(
            "Typst text segment positioning was incomplete",
        ));
    }

    Ok(runs)
}

fn positioned_run_from_segment(
    segment: &TextLineSegment,
    positioned: ActivePositionedSegment,
    end: Point,
    line_metrics: TypesetMetrics,
    include_pdf_text_layer: bool,
) -> Result<PositionedTextLineRun, MathTypesetError> {
    let x = abs_to_f32(positioned.start.x);
    let y = first_text_baseline_y(&positioned.items).unwrap_or(line_metrics.baseline);
    let width = (abs_to_f32(end.x) - x).max(0.0);
    let metrics = TypesetMetrics {
        width,
        height: line_metrics.height,
        baseline: line_metrics.baseline,
        ascent: line_metrics.ascent,
        descent: line_metrics.descent,
    };
    let (paths, pdf_text, font_resources) =
        if matches!(segment.kind, PositionedTextLineRunKind::Math) {
            let mut lowerer = PathLowerer::default();
            for (pos, item) in &positioned.items {
                lowerer.lower_positioned_item(*pos, item, Transform::identity())?;
            }
            let paths = Some(MathPathArtifact {
                logical_width: width,
                logical_height: line_metrics.height,
                items: lowerer.items,
            });
            let pdf_artifact = if include_pdf_text_layer {
                Some(pdf_text_from_positioned_items(
                    &positioned.items,
                    line_metrics,
                    &segment.text,
                )?)
            } else {
                None
            };
            match pdf_artifact {
                Some(artifact) => (paths, Some(artifact.text_layer), artifact.font_resources),
                None => (paths, None, Vec::new()),
            }
        } else {
            (None, None, Vec::new())
        };

    Ok(PositionedTextLineRun {
        kind: segment.kind,
        text: segment.text.clone(),
        byte_range: segment.byte_range.clone(),
        x,
        y,
        metrics,
        paths,
        pdf_text,
        font_resources,
    })
}

fn first_text_baseline_y(items: &[(Point, FrameItem)]) -> Option<f32> {
    items
        .iter()
        .find_map(|(pos, item)| first_text_baseline_y_in_item(*pos, item, Transform::identity()))
}

fn first_text_baseline_y_in_item(
    pos: Point,
    item: &FrameItem,
    transform: Transform,
) -> Option<f32> {
    let item_transform = transform.pre_concat(Transform::translate(pos.x, pos.y));
    match item {
        FrameItem::Text(_) => Some(abs_to_f32(item_transform.ty)),
        FrameItem::Group(group) => {
            let group_transform = item_transform.pre_concat(group.transform);
            group
                .frame
                .items()
                .find_map(|(pos, item)| first_text_baseline_y_in_item(*pos, item, group_transform))
        }
        _ => None,
    }
}

#[derive(Default)]
struct PathLowerer {
    items: Vec<MathPathItem>,
    next_glyph_run: usize,
}

impl PathLowerer {
    fn lower_frame(&mut self, frame: &Frame, transform: Transform) -> Result<(), MathTypesetError> {
        for (pos, item) in frame.items() {
            self.lower_positioned_item(*pos, item, transform)?;
        }

        Ok(())
    }

    fn lower_positioned_item(
        &mut self,
        pos: Point,
        item: &FrameItem,
        transform: Transform,
    ) -> Result<(), MathTypesetError> {
        let item_transform = transform.pre_concat(Transform::translate(pos.x, pos.y));
        match item {
            FrameItem::Group(group) => {
                if group.clip.is_some() {
                    return Err(MathTypesetError::UnsupportedOutput(
                        "clipped Typst math groups are not supported in path output yet",
                    ));
                }
                let group_transform = item_transform.pre_concat(group.transform);
                self.lower_frame(&group.frame, group_transform)
            }
            FrameItem::Text(text) => self.lower_text(text, item_transform),
            FrameItem::Shape(shape, _) => self.lower_shape(shape, item_transform),
            FrameItem::Image(..) => Err(MathTypesetError::UnsupportedOutput(
                "Typst image frame items are not supported in math path output",
            )),
            FrameItem::Link(..) | FrameItem::Tag(_) => Ok(()),
        }
    }

    fn lower_text(
        &mut self,
        text: &TextItem,
        transform: Transform,
    ) -> Result<(), MathTypesetError> {
        let glyph_run = self.next_glyph_run;
        self.next_glyph_run += 1;

        let fill = Some(color_from_paint(&text.fill)?);
        let stroke = text
            .stroke
            .as_ref()
            .map(math_stroke_from_typst)
            .transpose()?;
        let mut cursor_x = Abs::zero();
        let mut cursor_y = Abs::zero();

        for (glyph_index, glyph) in text.glyphs.iter().enumerate() {
            let x_offset = abs_to_f32(cursor_x + glyph.x_offset.at(text.size));
            let y_offset = -abs_to_f32(cursor_y + glyph.y_offset.at(text.size));
            let mut builder = GlyphPathBuilder::new(&text.font, text.size, x_offset, y_offset);
            text.font
                .ttf()
                .outline_glyph(GlyphId(glyph.id), &mut builder);
            let path = builder.finish();

            if !path.commands.is_empty() {
                self.items.push(MathPathItem {
                    path,
                    kind: MathPathKind::GlyphOutline {
                        glyph_run,
                        glyph_index,
                    },
                    fill,
                    stroke: stroke.clone(),
                    transform: math_transform_from_typst(transform),
                    clip: None,
                });
            }

            cursor_x += glyph.x_advance.at(text.size);
            cursor_y += glyph.y_advance.at(text.size);
        }

        Ok(())
    }

    fn lower_shape(&mut self, shape: &Shape, transform: Transform) -> Result<(), MathTypesetError> {
        let path = path_from_geometry(&shape.geometry);
        if path.commands.is_empty() {
            return Ok(());
        }

        self.items.push(MathPathItem {
            path,
            kind: MathPathKind::MathShape,
            fill: shape.fill.as_ref().map(color_from_paint).transpose()?,
            stroke: shape
                .stroke
                .as_ref()
                .map(math_stroke_from_typst)
                .transpose()?,
            transform: math_transform_from_typst(transform),
            clip: None,
        });

        Ok(())
    }
}

struct PdfArtifact {
    text_layer: MathPdfTextLayer,
    font_resources: Vec<MathFontResource>,
}

fn pdf_text_from_inline_items(
    items: &[InlineItem],
    metrics: TypesetMetrics,
    source: &str,
) -> Result<PdfArtifact, MathTypesetError> {
    let mut lowerer = PdfLowerer::default();
    let mut x = Abs::zero();
    let baseline = Abs::pt(metrics.baseline as f64);

    for item in items {
        match item {
            InlineItem::Space(amount, _) => {
                x += *amount;
            }
            InlineItem::Frame(frame) => {
                let y = baseline - frame.ascent();
                lowerer.lower_frame(frame, Transform::translate(x, y))?;
                x += frame.width();
            }
        }
    }

    Ok(PdfArtifact {
        text_layer: MathPdfTextLayer {
            logical_width: metrics.width,
            logical_height: metrics.height,
            semantic_text: source.to_string(),
            glyph_runs: lowerer.glyph_runs,
        },
        font_resources: lowerer.font_resources,
    })
}

fn pdf_text_from_frame(
    frame: &Frame,
    metrics: TypesetMetrics,
    source: &str,
) -> Result<PdfArtifact, MathTypesetError> {
    let mut lowerer = PdfLowerer::default();
    lowerer.lower_frame(frame, Transform::identity())?;

    Ok(PdfArtifact {
        text_layer: MathPdfTextLayer {
            logical_width: metrics.width,
            logical_height: metrics.height,
            semantic_text: source.to_string(),
            glyph_runs: lowerer.glyph_runs,
        },
        font_resources: lowerer.font_resources,
    })
}

fn pdf_text_from_positioned_items(
    items: &[(Point, FrameItem)],
    metrics: TypesetMetrics,
    source: &str,
) -> Result<PdfArtifact, MathTypesetError> {
    let mut lowerer = PdfLowerer::default();
    for (pos, item) in items {
        lowerer.lower_positioned_item(*pos, item, Transform::identity())?;
    }

    Ok(PdfArtifact {
        text_layer: MathPdfTextLayer {
            logical_width: metrics.width,
            logical_height: metrics.height,
            semantic_text: source.to_string(),
            glyph_runs: lowerer.glyph_runs,
        },
        font_resources: lowerer.font_resources,
    })
}

#[derive(Default)]
struct PdfLowerer {
    glyph_runs: Vec<MathPdfGlyphRun>,
    font_resources: Vec<MathFontResource>,
}

impl PdfLowerer {
    fn lower_frame(&mut self, frame: &Frame, transform: Transform) -> Result<(), MathTypesetError> {
        for (pos, item) in frame.items() {
            self.lower_positioned_item(*pos, item, transform)?;
        }

        Ok(())
    }

    fn lower_positioned_item(
        &mut self,
        pos: Point,
        item: &FrameItem,
        transform: Transform,
    ) -> Result<(), MathTypesetError> {
        let item_transform = transform.pre_concat(Transform::translate(pos.x, pos.y));
        match item {
            FrameItem::Group(group) => {
                if group.clip.is_some() {
                    return Err(MathTypesetError::UnsupportedOutput(
                        "clipped Typst math groups are not supported in PDF glyph output yet",
                    ));
                }
                let group_transform = item_transform.pre_concat(group.transform);
                self.lower_frame(&group.frame, group_transform)
            }
            FrameItem::Text(text) => self.lower_text(text, item_transform),
            FrameItem::Shape(..) | FrameItem::Link(..) | FrameItem::Tag(_) => Ok(()),
            FrameItem::Image(..) => Err(MathTypesetError::UnsupportedOutput(
                "Typst image frame items are not supported in PDF glyph output",
            )),
        }
    }

    fn lower_text(
        &mut self,
        text: &TextItem,
        transform: Transform,
    ) -> Result<(), MathTypesetError> {
        if text.glyphs.is_empty() {
            return Ok(());
        }

        let font = self.font_resource_id(&text.font);
        let fill = color_from_paint(&text.fill)?;
        let stroke = text
            .stroke
            .as_ref()
            .map(math_stroke_from_typst)
            .transpose()?;
        let transform = math_transform_from_typst(transform);
        let mut cursor_x = Abs::zero();
        let mut cursor_y = Abs::zero();

        let glyphs = text
            .glyphs
            .iter()
            .map(|glyph| {
                let x = cursor_x + glyph.x_offset.at(text.size);
                let y = -(cursor_y + glyph.y_offset.at(text.size));
                let x_advance = glyph.x_advance.at(text.size);
                let y_advance = -glyph.y_advance.at(text.size);
                cursor_x += glyph.x_advance.at(text.size);
                cursor_y += glyph.y_advance.at(text.size);

                MathPdfGlyph {
                    glyph_id: glyph.id,
                    unicode: glyph_unicode(text, glyph),
                    x: abs_to_f32(x),
                    y: abs_to_f32(y),
                    x_advance: abs_to_f32(x_advance),
                    y_advance: abs_to_f32(y_advance),
                    transform,
                }
            })
            .collect();

        self.glyph_runs.push(MathPdfGlyphRun {
            font,
            font_size: abs_to_f32(text.size),
            fill,
            stroke,
            glyphs,
        });

        Ok(())
    }

    fn font_resource_id(&mut self, instance: &FontInstance) -> MathFontResourceId {
        let font = instance.font();
        let family = font.info().family.clone();
        let postscript_name = font.post_script_name();
        let face_index = font.index();
        let units_per_em = instance.units_per_em() as f32;
        let data = font.data().as_slice();

        if let Some(resource) = self.font_resources.iter().find(|resource| {
            resource.family == family
                && resource.postscript_name == postscript_name
                && resource.face_index == face_index
                && (resource.units_per_em - units_per_em).abs() < f32::EPSILON
                && resource.data.as_ref() == data
        }) {
            return resource.id;
        }

        let id = MathFontResourceId(self.font_resources.len() as u32);
        self.font_resources.push(MathFontResource {
            id,
            family,
            postscript_name,
            face_index,
            units_per_em,
            data: Arc::<[u8]>::from(data),
        });
        id
    }
}

fn glyph_unicode(text: &TextItem, glyph: &typst_library::text::Glyph) -> String {
    text.text[glyph.range()]
        .trim_matches(is_default_ignorable)
        .to_string()
}

struct GlyphPathBuilder<'a> {
    font: &'a typst_library::text::FontInstance,
    font_size: Abs,
    x_offset: f32,
    y_offset: f32,
    path: MathPathData,
}

impl<'a> GlyphPathBuilder<'a> {
    fn new(
        font: &'a typst_library::text::FontInstance,
        font_size: Abs,
        x_offset: f32,
        y_offset: f32,
    ) -> Self {
        Self {
            font,
            font_size,
            x_offset,
            y_offset,
            path: MathPathData::default(),
        }
    }

    fn finish(self) -> MathPathData {
        self.path
    }

    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.x_offset + self.scale_font_units(x),
            self.y_offset - self.scale_font_units(y),
        )
    }

    fn scale_font_units(&self, value: f32) -> f32 {
        abs_to_f32(self.font.to_em(value).at(self.font_size))
    }
}

impl OutlineBuilder for GlyphPathBuilder<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.path.commands.push(MathPathCommand::MoveTo { x, y });
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.path.commands.push(MathPathCommand::LineTo { x, y });
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x, y) = self.point(x, y);
        self.path
            .commands
            .push(MathPathCommand::QuadTo { x1, y1, x, y });
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x2, y2) = self.point(x2, y2);
        let (x, y) = self.point(x, y);
        self.path.commands.push(MathPathCommand::CubicTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        });
    }

    fn close(&mut self) {
        self.path.commands.push(MathPathCommand::Close);
    }
}

fn path_from_geometry(geometry: &Geometry) -> MathPathData {
    match geometry {
        Geometry::Line(to) => MathPathData {
            commands: vec![
                MathPathCommand::MoveTo { x: 0.0, y: 0.0 },
                MathPathCommand::LineTo {
                    x: abs_to_f32(to.x),
                    y: abs_to_f32(to.y),
                },
            ],
        },
        Geometry::Rect(size) => MathPathData::rect(abs_to_f32(size.x), abs_to_f32(size.y)),
        Geometry::Curve(curve) => path_from_curve(curve),
    }
}

fn path_from_curve(curve: &Curve) -> MathPathData {
    let commands = curve
        .0
        .iter()
        .map(|item| match item {
            CurveItem::Move(point) => MathPathCommand::MoveTo {
                x: abs_to_f32(point.x),
                y: abs_to_f32(point.y),
            },
            CurveItem::Line(point) => MathPathCommand::LineTo {
                x: abs_to_f32(point.x),
                y: abs_to_f32(point.y),
            },
            CurveItem::Cubic(a, b, c) => MathPathCommand::CubicTo {
                x1: abs_to_f32(a.x),
                y1: abs_to_f32(a.y),
                x2: abs_to_f32(b.x),
                y2: abs_to_f32(b.y),
                x: abs_to_f32(c.x),
                y: abs_to_f32(c.y),
            },
            CurveItem::Close => MathPathCommand::Close,
        })
        .collect();

    MathPathData { commands }
}

fn math_stroke_from_typst(stroke: &FixedStroke) -> Result<MathStroke, MathTypesetError> {
    if stroke.dash.is_some() {
        return Err(MathTypesetError::UnsupportedOutput(
            "dashed Typst math strokes are not supported in path output",
        ));
    }

    Ok(MathStroke {
        color: color_from_paint(&stroke.paint)?,
        width: abs_to_f32(stroke.thickness),
    })
}

fn color_from_paint(paint: &Paint) -> Result<Color, MathTypesetError> {
    match paint {
        Paint::Solid(color) => Ok(color_from_typst(color)),
        Paint::Gradient(_) | Paint::Tiling(_) => Err(MathTypesetError::UnsupportedOutput(
            "non-solid Typst paints are not supported in math path output",
        )),
    }
}

fn color_from_typst(color: &TypstColor) -> Color {
    let [r, g, b, a] = color.to_process_space(ProcessColorSpace::Srgb).to_vec4();
    Color::rgba(r, g, b, a)
}

fn math_transform_from_typst(transform: Transform) -> MathTransform {
    MathTransform {
        xx: transform.sx.get() as f32,
        yx: transform.ky.get() as f32,
        xy: transform.kx.get() as f32,
        yy: transform.sy.get() as f32,
        dx: abs_to_f32(transform.tx),
        dy: abs_to_f32(transform.ty),
    }
}

fn abs_to_f32(abs: Abs) -> f32 {
    abs.to_pt() as f32
}

fn typst_color(color: Color) -> TypstColor {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    TypstColor::from_u8(
        channel(color.r),
        channel(color.g),
        channel(color.b),
        channel(color.a),
    )
}

fn lower_math_source(source: &str) -> Result<Content, MathTypesetError> {
    let root = typst_syntax::parse_math(source);
    let math = root
        .cast::<ast::Math>()
        .ok_or_else(|| unsupported(0, "expected Typst math root"))?;
    lower_math(math)
}

fn lower_math(math: ast::Math<'_>) -> Result<Content, MathTypesetError> {
    math.exprs()
        .map(lower_expr)
        .collect::<Result<Vec<_>, _>>()
        .map(Content::sequence)
}

fn lower_expr(expr: ast::Expr<'_>) -> Result<Content, MathTypesetError> {
    match expr {
        ast::Expr::Space(_) => Ok(SpaceElem::shared().clone()),
        ast::Expr::Math(math) => lower_math(math),
        ast::Expr::MathText(text) => lower_math_text(text),
        ast::Expr::MathIdent(ident) => Ok(symbol_or_identifier(ident.as_str())),
        ast::Expr::MathShorthand(shorthand) => Ok(SymbolElem::packed(shorthand.get().to_string())),
        ast::Expr::MathAlignPoint(_) => Ok(AlignPointElem::shared().clone()),
        ast::Expr::MathDelimited(delimited) => lower_math_delimited(delimited),
        ast::Expr::MathAttach(attach) => lower_math_attach(attach),
        ast::Expr::MathPrimes(primes) => Ok(PrimesElem::new(primes.count()).pack()),
        ast::Expr::MathFrac(frac) => lower_math_frac(frac),
        ast::Expr::MathRoot(root) => lower_math_root(root),
        ast::Expr::MathCall(call) => lower_math_call(call),
        _ => Err(unsupported(
            0,
            "unsupported Typst math expression in strict Avenger subset",
        )),
    }
}

fn lower_math_text(text: ast::MathText<'_>) -> Result<Content, MathTypesetError> {
    match text.get() {
        MathTextKind::Grapheme(text) => Ok(SymbolElem::packed(text.clone())),
        MathTextKind::Number(text) => Ok(TextElem::packed(text.clone())),
    }
}

fn lower_math_delimited(delimited: ast::MathDelimited<'_>) -> Result<Content, MathTypesetError> {
    let open = lower_expr(delimited.open())?;
    let body = lower_math(delimited.body())?;
    let close = lower_expr(delimited.close())?;
    Ok(LrElem::new(open + body + close).pack())
}

fn lower_math_attach(attach: ast::MathAttach<'_>) -> Result<Content, MathTypesetError> {
    let mut elem = AttachElem::new(lower_expr(attach.base())?);

    if let Some(top) = attach.top() {
        elem.t.set(Some(lower_expr(top)?));
    }
    if let Some(primes) = attach.primes() {
        elem.tr.set(Some(PrimesElem::new(primes.count()).pack()));
    }
    if let Some(bottom) = attach.bottom() {
        elem.b.set(Some(lower_expr(bottom)?));
    }

    Ok(elem.pack())
}

fn lower_math_frac(frac: ast::MathFrac<'_>) -> Result<Content, MathTypesetError> {
    let num_expr = frac.num();
    let denom_expr = frac.denom();
    let num_deparenthesized =
        matches!(num_expr, ast::Expr::Math(math) if math.was_deparenthesized());
    let denom_deparenthesized =
        matches!(denom_expr, ast::Expr::Math(math) if math.was_deparenthesized());

    Ok(
        FracElem::new(lower_expr(num_expr)?, lower_expr(denom_expr)?)
            .with_num_deparenthesized(num_deparenthesized)
            .with_denom_deparenthesized(denom_deparenthesized)
            .pack(),
    )
}

fn lower_math_root(root: ast::MathRoot<'_>) -> Result<Content, MathTypesetError> {
    let index = root
        .index()
        .map(|index| TextElem::packed(index.to_string()));
    Ok(RootElem::new(lower_expr(root.radicand())?)
        .with_index(index)
        .pack())
}

fn lower_math_call(call: ast::MathCall<'_>) -> Result<Content, MathTypesetError> {
    let ast::MathAccess::MathIdent(callee) = call.callee() else {
        return Err(unsupported(
            0,
            "field-access math calls are not supported in strict Avenger subset",
        ));
    };

    match callee.as_str() {
        "frac" => lower_two_arg_call(call, |num, denom| FracElem::new(num, denom).pack()),
        "sqrt" => lower_one_arg_call(call, |radicand| RootElem::new(radicand).pack()),
        "root" => lower_two_arg_call(call, |index, radicand| {
            RootElem::new(radicand).with_index(Some(index)).pack()
        }),
        "mat" => lower_matrix_call(call),
        _ => Err(unsupported(
            0,
            "unsupported math function in strict Avenger subset",
        )),
    }
}

fn lower_one_arg_call(
    call: ast::MathCall<'_>,
    build: impl FnOnce(Content) -> Content,
) -> Result<Content, MathTypesetError> {
    let args = positional_args(call)?;
    let [arg]: [Content; 1] = args
        .try_into()
        .map_err(|_| unsupported(0, "math function expects exactly one positional argument"))?;
    Ok(build(arg))
}

fn lower_two_arg_call(
    call: ast::MathCall<'_>,
    build: impl FnOnce(Content, Content) -> Content,
) -> Result<Content, MathTypesetError> {
    let args = positional_args(call)?;
    let [first, second]: [Content; 2] = args
        .try_into()
        .map_err(|_| unsupported(0, "math function expects exactly two positional arguments"))?;
    Ok(build(first, second))
}

fn positional_args(call: ast::MathCall<'_>) -> Result<Vec<Content>, MathTypesetError> {
    call.args()
        .arg_items()
        .map(|item| lower_positional_arg(item.arg))
        .collect()
}

fn lower_positional_arg(arg: Arg<'_>) -> Result<Content, MathTypesetError> {
    match arg {
        Arg::Pos(expr) => lower_expr(expr),
        Arg::Named(_) => Err(unsupported(
            0,
            "named math arguments are not supported in strict Avenger subset",
        )),
        Arg::Spread(_) => Err(unsupported(
            0,
            "spread math arguments are not supported in strict Avenger subset",
        )),
    }
}

fn lower_matrix_call(call: ast::MathCall<'_>) -> Result<Content, MathTypesetError> {
    let mut rows = vec![Vec::new()];

    for item in call.args().arg_items() {
        rows.last_mut()
            .expect("matrix row is initialized")
            .push(lower_positional_arg(item.arg)?);
        if item.ends_in_semicolon {
            rows.push(Vec::new());
        }
    }

    if matches!(rows.last(), Some(row) if row.is_empty()) && rows.len() > 1 {
        rows.pop();
    }

    Ok(MatElem::new(rows).pack())
}

fn symbol_or_identifier(name: &str) -> Content {
    SymbolElem::packed(match named_math_symbol(name) {
        Some(symbol) => symbol,
        None => name,
    })
}

fn named_math_symbol(name: &str) -> Option<&'static str> {
    match name {
        "alpha" => Some("α"),
        "beta" => Some("β"),
        "gamma" => Some("γ"),
        "delta" => Some("δ"),
        "epsilon" => Some("ε"),
        "zeta" => Some("ζ"),
        "eta" => Some("η"),
        "theta" => Some("θ"),
        "iota" => Some("ι"),
        "kappa" => Some("κ"),
        "lambda" => Some("λ"),
        "mu" => Some("μ"),
        "nu" => Some("ν"),
        "xi" => Some("ξ"),
        "pi" => Some("π"),
        "rho" => Some("ρ"),
        "sigma" => Some("σ"),
        "tau" => Some("τ"),
        "upsilon" => Some("υ"),
        "phi" => Some("φ"),
        "chi" => Some("χ"),
        "psi" => Some("ψ"),
        "omega" => Some("ω"),
        "Gamma" => Some("Γ"),
        "Delta" => Some("Δ"),
        "Theta" => Some("Θ"),
        "Lambda" => Some("Λ"),
        "Xi" => Some("Ξ"),
        "Pi" => Some("Π"),
        "Sigma" => Some("Σ"),
        "Upsilon" => Some("Υ"),
        "Phi" => Some("Φ"),
        "Psi" => Some("Ψ"),
        "Omega" => Some("Ω"),
        "sum" => Some("∑"),
        "prod" => Some("∏"),
        "integral" => Some("∫"),
        "oo" | "infinity" => Some("∞"),
        "partial" => Some("∂"),
        "nabla" => Some("∇"),
        _ => None,
    }
}

fn unsupported(position: usize, message: &'static str) -> MathTypesetError {
    MathTypesetError::UnsupportedSyntax { position, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use typst_library::math::{AttachElem, FracElem, MatElem, RootElem};

    #[test]
    fn typst_font_loader_embeds_atkinson_weight_style_faces() {
        let fonts = load_typst_fonts(&TypstEngineConfig::default()).unwrap();
        let book = FontBook::from_fonts(&fonts);
        let Some((_, ids)) = book
            .families()
            .find(|(family, _)| family == &"Atkinson Hyperlegible Next")
        else {
            panic!("Atkinson Hyperlegible Next should be embedded");
        };

        let faces = ids
            .into_iter()
            .filter_map(|id| book.info(id))
            .map(|info| (info.variant.weight.to_number(), info.variant.style))
            .collect::<HashSet<_>>();

        for weight in [250, 300, 400, 500, 600, 700, 800] {
            assert!(faces.contains(&(weight, FontStyle::Normal)));
            assert!(faces.contains(&(weight, FontStyle::Italic)));
        }
    }

    #[test]
    fn lowers_basic_symbol_text() {
        let content = lower_math_source("x^2 + y^2").unwrap();

        assert!(content.plain_text().contains("x2 + y2"));
    }

    #[test]
    fn lowers_named_symbols() {
        let content = lower_math_source("alpha + pi + sum").unwrap();

        assert_eq!(content.plain_text().as_str(), "α + π + ∑");
    }

    #[test]
    fn lowers_fraction_syntax() {
        let content = lower_math_source("a / b").unwrap();

        assert!(content.is::<FracElem>());
    }

    #[test]
    fn lowers_root_call() {
        let content = lower_math_source("sqrt(x^2 + y^2)").unwrap();

        assert!(content.is::<RootElem>());
    }

    #[test]
    fn lowers_attachment_syntax() {
        let content = lower_math_source("sum_(i=1)^n x_i").unwrap();

        assert!(content.plain_text().contains("∑"));
        assert!(contains_sequence_child::<AttachElem>(&content));
    }

    #[test]
    fn lowers_matrix_call() {
        let content = lower_math_source("mat(1, 2; 3, 4)").unwrap();

        assert!(content.is::<MatElem>());
    }

    #[test]
    fn rejects_unsupported_call() {
        let err = lower_math_source("foo(x)").unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "unsupported math function in strict Avenger subset"
            }
        );
    }

    fn contains_sequence_child<T: NativeElement>(content: &Content) -> bool {
        let mut found = false;
        content.sequence_recursive_for_each(&mut |child| {
            found |= child.is::<T>();
        });
        found
    }
}
