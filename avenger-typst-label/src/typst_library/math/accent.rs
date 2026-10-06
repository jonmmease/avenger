//! Ported from crates/typst-library/src/math/accent.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: the accent functions that accent symbols call are static function data, made by
//! `accents!` from upstream's accent table, so they need no allocation or closures.
//! Upstream's doc examples with bare fences are marked `ignore`, so that rustdoc doesn't run them.

use crate::typst_library::foundations::{
    Content, Func, IntoValue, NativeElement, NativeFuncData, Str, SymbolElem, cast, elem,
};
use crate::typst_library::layout::{Em, Length, Rel};

/// How much the accent can be shorter than the base.
pub const ACCENT_SHORT_FALL: Em = Em::new(0.5);

elem! {
/// Attaches an accent to a base.
///
/// In math mode, common accents are also available as named @symbol[symbols]
/// that can be directly called (like @function[functions]) to attach them to
/// some content.
///
/// = Example <example>
/// ```example
/// $grave(a) = accent(a, `)$ \
/// $arrow(a) = accent(a, arrow)$ \
/// $tilde(a) = accent(a, \u{0303})$
/// ```
#[elem(name = "accent", Mathy)]
pub struct AccentElem {
    /// The base to which the accent is applied. May consist of multiple
    /// letters.
    ///
    /// ```example
    /// $arrow(A B C)$
    /// ```
    #[required]
    pub base: Content,

    /// The accent to apply to the base.
    ///
    /// Supported accents include:
    ///
    /// #docs-table(
    ///   table.header[Accent][Name][Codepoint],
    ///
    ///   [Grave],
    ///   [`grave`],
    ///   [``` ` ```],
    ///
    ///   [Acute],
    ///   [`acute`],
    ///   [`´`],
    ///
    ///   [Circumflex],
    ///   [`hat`],
    ///   [`^`],
    ///
    ///   [Tilde],
    ///   [`tilde`],
    ///   [`~`],
    ///
    ///   [Macron],
    ///   [`macron`],
    ///   [`¯`],
    ///
    ///   [Dash],
    ///   [`dash`],
    ///   [`‾`],
    ///
    ///   [Breve],
    ///   [`breve`],
    ///   [`˘`],
    ///
    ///   [Dot],
    ///   [`dot`],
    ///   [`.`],
    ///
    ///   [Double dot, Diaeresis],
    ///   [`dot.double`, `diaer`],
    ///   [`¨`],
    ///
    ///   [Triple dot],
    ///   [`dot.triple`],
    ///   raw(lang: "typ", "\\u{20db}"),
    ///
    ///   [Quadruple dot],
    ///   [`dot.quad`],
    ///   raw(lang: "typ", "\\u{20dc}"),
    ///
    ///   [Circle],
    ///   [`circle`],
    ///   [`∘`],
    ///
    ///   [Double acute],
    ///   [`acute.double`],
    ///   [`˝`],
    ///
    ///   [Caron],
    ///   [`caron`],
    ///   [`ˇ`],
    ///
    ///   [Right arrow],
    ///   [`arrow`, `->`],
    ///   [`→`],
    ///
    ///   [Left arrow],
    ///   [`arrow.l`, `<-`],
    ///   [`←`],
    ///
    ///   [Left/Right arrow],
    ///   [`arrow.l.r`],
    ///   [`↔`],
    ///
    ///   [Right harpoon],
    ///   [`harpoon`],
    ///   [`⇀`],
    ///
    ///   [Left harpoon],
    ///   [`harpoon.lt`],
    ///   [`↼`],
    /// )
    #[required]
    pub accent: Accent,

    /// The size of the accent, relative to the width of the base.
    ///
    /// #example(
    ///   title: "Basic usage",
    ///   ```ignore
    ///   $dash(A, size: #150%)$
    ///   ```
    /// )
    ///
    /// Note that the resulting accent may not have the exact desired size. For
    /// example, an arrow may be either a pre-defined short glyph, or a long
    /// glyph assembled from building blocks (arrowhead + line) provided by the
    /// font. The sizes of the two possibilities may not cover the entire span.
    /// Consequently, arrows of certain intermediate sizes cannot be
    /// constructed.
    ///
    /// #example(
    ///   title: "Size of arrow growing discontinuously",
    ///   ```ignore
    ///   >>> #set par(spacing: 0.3em)
    ///   #for i in range(6) {
    ///     $ arrow(#box(
    ///       width: 0.4em + 0.3em * i,
    ///       fill: aqua,
    ///       height: 0.4em,
    ///     )) $
    ///   }
    ///   ```
    /// )
    #[default(Rel::one())]
    pub size: Rel<Length>,

    /// Whether to remove the dot on top of lowercase i and j when adding a top
    /// accent.
    ///
    /// This enables the `dtls` OpenType feature.
    ///
    /// ```example
    /// $hat(dotless: #false, i)$
    /// ```
    #[default(true)]
    pub dotless: bool,
}
}

/// An accent character.
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Hash)]
pub struct Accent(pub char);

impl Accent {
    /// Tries to select the appropriate combining accent for a string, falling
    /// back to the string's lone character if there is no corresponding one.
    ///
    /// Returns `None` if there isn't one and the string has more than one
    /// character.
    pub fn normalize(s: &str) -> Option<Self> {
        Self::combining(s).or_else(|| s.parse::<char>().ok().map(Self))
    }

    /// Tries to select a well-known combining accent that matches for the
    /// value.
    pub fn combining(value: &str) -> Option<Self> {
        let c = value.parse::<char>().ok();
        ACCENTS
            .iter()
            .copied()
            .find(|&(accent, names)| Some(accent) == c || names.contains(&value))
            .map(|(accent, _)| Self(accent))
    }

    /// Whether this accent is a bottom accent or not.
    pub fn is_bottom(&self) -> bool {
        if matches!(self.0, '⏟' | '⎵' | '⏝' | '⏡') {
            return true;
        }

        // avenger: a generated table in place of `icu_properties`.
        is_combining_below(self.0)
    }
}

/// Gets the accent function corresponding to a symbol value, if any.
// avenger: the function at the accent's position in the table.
pub fn get_accent_func(value: &str) -> Option<Func> {
    let accent = Accent::combining(value)?;
    let index = ACCENTS.iter().position(|&(c, _)| c == accent.0)?;
    Some(Func::from(&FUNCS[index]))
}

/// Defines the accent table and one accent function per accent.
// avenger: in place of upstream's lazily created functions, which capture their accent.
macro_rules! accents {
    ($($(#[$attr:meta])* ($accent:literal, $names:expr),)*) => {
        /// A list of accents, each with a list of alternative names.
        const ACCENTS: &[(char, &[&str])] = &[$(($accent, $names),)*];

        /// The accent functions, in the order of [`ACCENTS`].
        static FUNCS: &[NativeFuncData] = &[$(NativeFuncData {
            function: |_, args| {
                let base = args.expect("base")?;
                let size = args.named("size")?;
                let dotless = args.named("dotless")?;
                let mut elem = AccentElem::new(base, Accent($accent));
                if let Some(size) = size {
                    elem = elem.with_size(size);
                }
                if let Some(dotless) = dotless {
                    elem = elem.with_dotless(dotless);
                }
                Ok(elem.pack().into_value())
            },
            name: "(..) => ..",
        },)*];
    };
}

// Keep it synced with the documenting table above and the
// `math-accent-sym-call` test.`
accents! {
    // Note: Symbols that can have a text presentation must explicitly have that
    // alternative listed here.
    ('\u{0300}', &["`"]),
    ('\u{0301}', &["´"]),
    ('\u{0302}', &["^", "ˆ"]),
    ('\u{0303}', &["~", "∼", "˜"]),
    ('\u{0304}', &["¯"]),
    ('\u{0305}', &["-", "–", "‾", "−"]),
    ('\u{0306}', &["˘"]),
    ('\u{0307}', &[".", "˙", "⋅"]),
    ('\u{0308}', &["¨"]),
    ('\u{20db}', &[]),
    ('\u{20dc}', &[]),
    ('\u{030a}', &["∘", "○"]),
    ('\u{030b}', &["˝"]),
    ('\u{030c}', &["ˇ"]),
    ('\u{20d6}', &["←"]),
    ('\u{20d7}', &["→", "⟶"]),
    ('\u{20e1}', &["↔", "↔\u{fe0e}", "⟷"]),
    ('\u{20d0}', &["↼"]),
    ('\u{20d1}', &["⇀"]),
}

/// Whether a codepoint's canonical combining class is Below.
// avenger: a table of the class's ranges in place of an `icu_properties` lookup, generated from
// `icu_properties` 2.2.0 (upstream's version), which carries Unicode 16.0.
fn is_combining_below(c: char) -> bool {
    matches!(
        c,
        '\u{0316}'..='\u{0319}'
            | '\u{031C}'..='\u{0320}'
            | '\u{0323}'..='\u{0326}'
            | '\u{0329}'..='\u{0333}'
            | '\u{0339}'..='\u{033C}'
            | '\u{0347}'..='\u{0349}'
            | '\u{034D}'..='\u{034E}'
            | '\u{0353}'..='\u{0356}'
            | '\u{0359}'..='\u{035A}'
            | '\u{0591}'
            | '\u{0596}'
            | '\u{059B}'
            | '\u{05A2}'..='\u{05A7}'
            | '\u{05AA}'
            | '\u{05C5}'
            | '\u{0655}'..='\u{0656}'
            | '\u{065C}'
            | '\u{065F}'
            | '\u{06E3}'
            | '\u{06EA}'
            | '\u{06ED}'
            | '\u{0731}'
            | '\u{0734}'
            | '\u{0737}'..='\u{0739}'
            | '\u{073B}'..='\u{073C}'
            | '\u{073E}'
            | '\u{0742}'
            | '\u{0744}'
            | '\u{0746}'
            | '\u{0748}'
            | '\u{07F2}'
            | '\u{07FD}'
            | '\u{0859}'..='\u{085B}'
            | '\u{0899}'..='\u{089B}'
            | '\u{08CF}'..='\u{08D3}'
            | '\u{08E3}'
            | '\u{08E6}'
            | '\u{08E9}'
            | '\u{08ED}'..='\u{08EF}'
            | '\u{08F6}'
            | '\u{08F9}'..='\u{08FA}'
            | '\u{0952}'
            | '\u{0F18}'..='\u{0F19}'
            | '\u{0F35}'
            | '\u{0F37}'
            | '\u{0FC6}'
            | '\u{108D}'
            | '\u{193B}'
            | '\u{1A18}'
            | '\u{1A7F}'
            | '\u{1AB5}'..='\u{1ABA}'
            | '\u{1ABD}'
            | '\u{1ABF}'..='\u{1AC0}'
            | '\u{1AC3}'..='\u{1AC4}'
            | '\u{1ACA}'
            | '\u{1ADD}'
            | '\u{1AE6}'
            | '\u{1B6C}'
            | '\u{1CD5}'..='\u{1CD9}'
            | '\u{1CDC}'..='\u{1CDF}'
            | '\u{1CED}'
            | '\u{1DC2}'
            | '\u{1DCA}'
            | '\u{1DCF}'
            | '\u{1DF9}'
            | '\u{1DFD}'
            | '\u{1DFF}'
            | '\u{20E8}'
            | '\u{20EC}'..='\u{20EF}'
            | '\u{A92B}'..='\u{A92D}'
            | '\u{AAB4}'
            | '\u{FE27}'..='\u{FE2D}'
            | '\u{101FD}'
            | '\u{102E0}'
            | '\u{10A0D}'
            | '\u{10A3A}'
            | '\u{10AE6}'
            | '\u{10EFA}'..='\u{10EFB}'
            | '\u{10EFD}'..='\u{10EFF}'
            | '\u{10F46}'..='\u{10F47}'
            | '\u{10F4B}'
            | '\u{10F4D}'..='\u{10F50}'
            | '\u{10F83}'
            | '\u{10F85}'
            | '\u{1D17B}'..='\u{1D182}'
            | '\u{1D18A}'..='\u{1D18B}'
            | '\u{1E4EE}'
            | '\u{1E5EF}'
            | '\u{1E8D0}'..='\u{1E8D6}'
    )
}

cast! {
    Accent,
    self => self.0.into_value(),
    // The string cast handles
    // - strings: `accent(a, "↔")`
    // - symbol values: `accent(a, <->)`
    // - shorthands: `accent(a, arrow.l.r)`
    v: Str => Self::normalize(&v).ok_or("expected exactly one character")?,
    // The content cast is for accent uses like `accent(a, ↔)`
    v: Content => v.to_packed::<SymbolElem>()
        .and_then(|elem| Accent::normalize(&elem.text))
        .ok_or("expected a single-codepoint symbol")?,
}
