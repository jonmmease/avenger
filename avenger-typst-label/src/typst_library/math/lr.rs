//! Ported from crates/typst-library/src/math/lr.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: the left/right wrapper functions that delimiter symbols call are static function
//! data, made by `delims!` from upstream's delimiter table, so they need no allocation or
//! closures.

use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::{
    Args, Content, Func, IntoValue, NativeElement, NativeFunc, NativeFuncData,
    SymbolElem, elem, func,
};
use crate::typst_library::layout::{Em, Length, Rel};

/// How much less high scaled delimiters can be than what they wrap.
pub const DELIM_SHORT_FALL: Em = Em::new(0.1);

elem! {
/// Scales delimiters.
///
/// While matched delimiters scale by default, this can be used to scale
/// unmatched delimiters and to control the delimiter scaling more precisely.
#[elem(name = "lr", title = "Left/Right", Mathy)]
pub struct LrElem {
    /// The size of the brackets, relative to the height of the wrapped content.
    #[default(Rel::one())]
    pub size: Rel<Length>,

    /// The delimited content, including the delimiters.
    #[required]
    #[parse(
        let mut arguments = args.all::<Content>()?.into_iter();
        let mut body = arguments.next().unwrap_or_default();
        arguments.for_each(|arg| body += SymbolElem::packed(',') + arg);
        body
    )]
    pub body: Content,
}
}

impl LrElem {
    // upstream: the `#[parse]` attribute of `LrElem::body` @ v0.15.1
    fn parse_body(_: &mut Engine, args: &mut Args) -> SourceResult<Content> {
        let mut arguments = args.all::<Content>()?.into_iter();
        let mut body = arguments.next().unwrap_or_default();
        arguments.for_each(|arg| body += SymbolElem::packed(',') + arg);
        Ok(body)
    }
}

elem! {
/// Scales delimiters vertically to the nearest surrounding `{lr()}` group.
///
/// ```example
/// $ { x mid(|) sum_(i=1)^n w_i abs(f_i (x)) < 1 } $
/// ```
#[elem(name = "mid", Mathy)]
pub struct MidElem {
    /// The content to be scaled.
    #[required]
    pub body: Content,
}
}

func! {
/// Floors an expression.
///
/// ```example
/// $ floor(x/2) $
/// ```
#[func]
pub fn floor(
    /// The size of the brackets, relative to the height of the wrapped content.
    ///
    /// Default: The current value of @math.lr.size[`lr.size`].
    #[named]
    size: Option<Rel<Length>>,
    /// The expression to floor.
    body: Content,
) -> Content {
    delimited(body, '⌊', '⌋', size)
}
}

func! {
/// Ceils an expression.
///
/// ```example
/// $ ceil(x/2) $
/// ```
#[func]
pub fn ceil(
    /// The size of the brackets, relative to the height of the wrapped content.
    ///
    /// Default: The current value of @math.lr.size[`lr.size`].
    #[named]
    size: Option<Rel<Length>>,
    /// The expression to ceil.
    body: Content,
) -> Content {
    delimited(body, '⌈', '⌉', size)
}
}

func! {
/// Rounds an expression.
///
/// ```example
/// $ round(x/2) $
/// ```
#[func]
pub fn round(
    /// The size of the brackets, relative to the height of the wrapped content.
    ///
    /// Default: The current value of @math.lr.size[`lr.size`].
    #[named]
    size: Option<Rel<Length>>,
    /// The expression to round.
    body: Content,
) -> Content {
    delimited(body, '⌊', '⌉', size)
}
}

func! {
/// Takes the absolute value of an expression.
///
/// ```example
/// $ abs(x/2) $
/// ```
#[func]
pub fn abs(
    /// The size of the brackets, relative to the height of the wrapped content.
    ///
    /// Default: The current value of @math.lr.size[`lr.size`].
    #[named]
    size: Option<Rel<Length>>,
    /// The expression to take the absolute value of.
    body: Content,
) -> Content {
    delimited(body, '|', '|', size)
}
}

func! {
/// Takes the norm of an expression.
///
/// ```example
/// $ norm(x/2) $
/// ```
#[func]
pub fn norm(
    /// The size of the brackets, relative to the height of the wrapped content.
    ///
    /// Default: The current value of @math.lr.size[`lr.size`].
    #[named]
    size: Option<Rel<Length>>,
    /// The expression to take the norm of.
    body: Content,
) -> Content {
    delimited(body, '‖', '‖', size)
}
}

/// Gets the Left/Right wrapper function corresponding to a symbol value, if
/// any.
// avenger: the function at the delimiter's position in the table.
pub fn get_lr_wrapper_func(value: &str) -> Option<Func> {
    let left = value.parse::<char>().ok()?;
    match left {
        // Unlike `round`, `abs`, and `norm`, `floor` and `ceil` are of type
        // `symbol` and cast to a function like other L/R symbols. We could thus
        // rely on autogeneration for these as well, but since they are
        // specifically called out in the documentation on the L/R page (via the
        // group mechanism), it's nice for them to have a bit of extra
        // documentation.
        '⌈' => Some(ceil::func()),
        '⌊' => Some(floor::func()),
        l => DELIMS
            .iter()
            .position(|&(left, _)| left == l)
            .map(|i| Func::from(&FUNCS[i])),
    }
}

/// Defines the delimiter table and one left/right wrapper function per pair.
// avenger: in place of upstream's lazily created functions, which capture their delimiters.
macro_rules! delims {
    ($(($left:literal, $right:literal),)*) => {
        /// The delimiter pairings supported for use as callable symbols.
        const DELIMS: &[(char, char)] = &[$(($left, $right),)*];

        /// The left/right wrapper functions, in the order of [`DELIMS`].
        static FUNCS: &[NativeFuncData] = &[$(NativeFuncData {
            function: |_, args| {
                let size = args.named("size")?;
                let body = args.expect("body")?;
                Ok(delimited(body, $left, $right, size).into_value())
            },
            name: "(..) => ..",
        },)*];
    };
}

delims! {
    // The `ceil` and `floor` pairs are omitted here because they are handled
    // manually.
    ('(', ')'),
    ('⟮', '⟯'),
    ('⦇', '⦈'),
    ('⦅', '⦆'),
    ('⦓', '⦔'),
    ('⦕', '⦖'),
    ('{', '}'),
    ('⦃', '⦄'),
    ('[', ']'),
    ('⦍', '⦐'),
    ('⦏', '⦎'),
    ('⟦', '⟧'),
    ('⦋', '⦌'),
    ('❲', '❳'),
    ('⟬', '⟭'),
    ('⦗', '⦘'),
    ('⟅', '⟆'),
    ('⎰', '⎱'),
    ('⎱', '⎰'),
    ('⧘', '⧙'),
    ('⧚', '⧛'),
    ('⟨', '⟩'),
    ('⧼', '⧽'),
    ('⦑', '⦒'),
    ('⦉', '⦊'),
    ('⟪', '⟫'),
    ('⌜', '⌝'),
    ('⌞', '⌟'),
    // Fences.
    ('|', '|'),
    ('‖', '‖'),
    ('⦀', '⦀'),
    ('⦙', '⦙'),
    ('⦚', '⦚'),
}

/// Creates an L/R element with the given delimiters.
fn delimited(
    body: Content,
    left: char,
    right: char,
    size: Option<Rel<Length>>,
) -> Content {
    let span = body.span();
    let mut elem = LrElem::new(Content::sequence([
        SymbolElem::packed(left),
        body,
        SymbolElem::packed(right),
    ]));
    // Push size only if size is provided
    if let Some(size) = size {
        elem.size.set(size);
    }
    elem.pack().spanned(span)
}
