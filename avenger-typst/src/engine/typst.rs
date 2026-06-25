use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use comemo::Track;

use crate::api::TypstEngineConfig;
use crate::error::{MathTypesetError, TypstInitError};
use crate::style::{Color, MathDisplayStyle};
use crate::types::{MathFragmentOptions, MathRunArtifact, TypesetMetrics};

use typst_library::diag::{FileError, FileResult, SourceResult};
use typst_library::engine::{Engine, Route, Sink, Traced};
use typst_library::foundations::{
    Args, Closure, Content, Context, Func, Module, NativeElement, NativeRuleMap, Packed, Scope,
    SequenceElem, ShowSet, StyleChain, Styles, SymbolElem, Value,
};
use typst_library::introspection::{EmptyIntrospector, Introspector, Locator};
use typst_library::layout::{Abs, InlineItem, Size};
use typst_library::math::{
    AlignPointElem, AttachElem, EquationElem, FracElem, LrElem, MatElem, MathSize, PrimesElem,
    RootElem,
};
use typst_library::routines::{Arenas, Pair, RealizationKind, Routines, SpanMode};
use typst_library::text::{Font, FontBook, FontFamily, FontList, SpaceElem, TextElem, TextSize};
use typst_library::visualize::{Color as TypstColor, Paint};
use typst_library::{Library, LibraryBuilder, World};
use typst_syntax::ast::{self, Arg, MathTextKind};
use typst_syntax::{FileId, Source, SyntaxMode};
use typst_utils::{LazyHash, Protected};

#[derive(Clone)]
pub(crate) struct TypstMathEngine {
    world: Arc<TypstMathWorld>,
    math_font_family: String,
}

impl std::fmt::Debug for TypstMathEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypstMathEngine")
            .field("math_font_family", &self.math_font_family)
            .finish_non_exhaustive()
    }
}

impl TypstMathEngine {
    pub(crate) fn new(config: &TypstEngineConfig) -> Result<Self, TypstInitError> {
        let fonts = load_math_fonts(config)?;
        let book = FontBook::from_fonts(&fonts);
        let math_font_family = select_math_font_family(&book, config)?;
        let world = Arc::new(TypstMathWorld::new(fonts));

        Ok(Self {
            world,
            math_font_family,
        })
    }

    pub(crate) fn typeset_fragment(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, MathTypesetError> {
        if options.outputs.paths {
            return Err(MathTypesetError::UnsupportedOutput(
                "vendor-typst path output is not wired yet",
            ));
        }
        if options.outputs.pdf_text_layer {
            return Err(MathTypesetError::UnsupportedOutput(
                "vendor-typst PDF glyph output is not wired yet",
            ));
        }
        if options.outputs.raster.is_some() {
            return Err(MathTypesetError::UnsupportedOutput(
                "vendor-typst raster output is not wired yet",
            ));
        }

        let metrics = self.layout_metrics(source, options)?;
        Ok(MathRunArtifact {
            metrics,
            paths: None,
            raster: None,
            pdf_text: None,
            font_resources: Vec::new(),
            warnings: Vec::new(),
        })
    }

    fn layout_metrics(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<TypesetMetrics, MathTypesetError> {
        let items = self.layout_inline_items(source, options)?;
        metrics_from_inline_items(items, source.len())
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

fn load_math_fonts(config: &TypstEngineConfig) -> Result<Vec<Font>, TypstInitError> {
    let mut fonts = Vec::new();
    for path in candidate_font_paths(config) {
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        fonts.extend(Font::iter(typst_library::foundations::Bytes::new(data)));
    }

    if fonts.is_empty() {
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
        if book.contains_family(family) {
            return Ok(family.to_string());
        }
    }

    book.families()
        .next()
        .map(|(family, _)| family.to_string())
        .ok_or(TypstInitError::BackendUnavailable(
            "no supported math font found",
        ))
}

fn metrics_from_inline_items(
    items: Vec<InlineItem>,
    source_len: usize,
) -> Result<TypesetMetrics, MathTypesetError> {
    let mut width = Abs::zero();
    let mut ascent = Abs::zero();
    let mut descent = Abs::zero();
    let mut has_frame = false;

    for item in items {
        match item {
            InlineItem::Space(amount, _) => {
                width += amount;
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
    use typst_library::math::{AttachElem, FracElem, MatElem, RootElem};

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
