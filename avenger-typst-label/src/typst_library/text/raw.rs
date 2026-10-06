//! Ported from crates/typst-library/src/text/raw.rs @ v0.15.1, modified for Avenger.

//! avenger: no syntax highlighting, so no syntect and no `syntaxes` or `theme` fields. Lines
//! are synthesized as upstream synthesizes them without a theme, which for plain text matches
//! its highlighted output.

use ecow::{EcoString, EcoVec};
use unicode_segmentation::UnicodeSegmentation;

use super::Lang;
use crate::typst_library::diag::{SourceResult, bail};
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::{
    Content, Packed, ShowSet, Smart, StyleChain, Styles, Synthesize, cast, elem,
};
use crate::typst_library::layout::{Em, HAlignment};
use crate::typst_library::model::ParElem;
use crate::typst_library::text::{FontFamily, FontList, TextElem, TextSize};
use typst_syntax::{Span, split_newlines};

elem! {
/// Raw text with optional syntax highlighting.
///
/// Displays the text verbatim and in a monospace font. This is typically used
/// to embed computer code into a document.
///
/// Text given to this element will ignore markup syntax, such as `[*strong*]`
/// or `[_emphasis_]`, and will be displayed verbatim. If you would like to
/// display content with a monospace font while still allowing markup syntax,
/// instead of using @raw, you can explicitly set the text font to a monospace
/// font with the @text.font parameter.
///
/// Raw elements are mainly produced with their @raw:syntax[dedicated syntax] by
/// enclosing text with either one or three-plus backtick characters (``` ` ```)
/// on both sides. When using three or more backticks, text immediately after
/// the initial backticks will be treated as a @raw.lang[language tag] used for
/// syntax highlighting, and the raw text begins after the first whitespace.
///
/// = Example <example>
/// ````example
/// Adding `rbx` to `rcx` gives
/// the desired result.
///
/// What is ```rust fn main()``` in Rust
/// would be ```c int main()``` in C.
///
/// ```rust
/// fn main() {
///     println!("Hello World!");
/// }
/// ```
///
/// This has ``` `backticks` ``` in it
/// (but the spaces are trimmed). And
/// ``` here``` the leading space is
/// also trimmed.
/// ````
///
/// You can also construct a @raw element programmatically from a string (and
/// provide the language tag via the optional @raw.lang[`lang`] parameter).
///
/// ```example
/// #raw("fn " + "main() {}", lang: "rust")
/// ```
///
/// If no syntax highlighting is available by default for your specified
/// language tag (or if you want to override the built-in definition), you may
/// provide a custom syntax specification file to the @raw.syntaxes[`syntaxes`]
/// parameter.
///
/// = Styling <styling>
/// By default, the `raw` element uses the `DejaVu Sans Mono` font (included
/// with Typst), with a smaller font size of `{0.8em}` (that is, 80% of the
/// global font size). This is because monospace fonts tend to be visually
/// larger than non-monospace fonts.
///
/// You can customize these properties with show-set rules:
///
/// ````example
/// // Switch to Cascadia Code for both
/// // inline and block raw.
/// #show raw: set text(font: "Cascadia Code")
///
/// // Reset raw blocks to the same size as normal text,
/// // but keep inline raw at the reduced size.
/// #show raw.where(block: true): set text(1em / 0.8)
///
/// Now using the `Cascadia Code` font for raw text.
/// Here's some Python code. It looks larger now:
///
/// ```py
/// def python():
///   return 5 + 5
/// ```
/// ````
///
/// In addition, you can customize the syntax highlighting colors by setting a
/// custom theme through the @raw.theme[`theme`] parameter.
///
/// For complete customization of the appearance of a raw block, a show rule on
/// @raw.line could be helpful, such as to add line numbers.
///
/// Note that in raw text, typesetting features like
/// @text.hyphenate[hyphenation], @text.overhang[overhang],
/// @text.cjk-latin-spacing[CJK-Latin spacing], and (for raw blocks)
/// @par.justify[justification] will be disabled by default.
///
/// = Syntax <syntax>
/// This function has dedicated syntax that produces a raw element in both
/// markup and code mode. You can enclose text in one or three-plus backtick
/// characters (``` ` ```) on both sides to make it raw. The number of backticks
/// must be the same on both sides, and the enclosed text cannot contain a group
/// of that many backticks in a row. Writing just two backticks (``` `` ```)
/// produces empty raw text.
///
/// Notable differences from Markdown include that single backticks can enclose
/// text spanning multiple lines without removing indentation, and that the
/// three-plus backtick syntax still interprets language tags when used inline.
///
/// Raw text enclosed in _single_ backticks has no way to specify a language tag
/// and is always treated as inline for use within a paragraph, i.e. the
/// @raw.block[`block`] parameter is `{false}`.
///
/// Raw syntax using _three or more_ backticks has the following properties:
///
/// - *After the initial backticks, the raw block is only terminated by a
///   sequence of the same number of backticks*
///
///   To include text containing a sequence of backticks, the initial and final
///   backticks must have at least one more backtick than the sequence.
///
/// - *If the raw text contains a linebreak, it will be block-level, otherwise
///   it will be inline*
///
///   This sets the @raw.block[`block`] parameter to `{true}` or `{false}`
///   accordingly.
///
/// - *Text immediately after the initial backticks, up to the first whitespace,
///   is treated as a _language tag_ used for syntax highlighting*
///
///   The specific rules for which text can be treated as the language tag are
///   planned to change, and are @raw:language-tag-changes[explained in detail
///   below.]
///
/// - *The initial and final lines have special trimming behavior*
///
///   For the initial line, if all characters following the initial backticks or
///   language tag are whitespace, the entire line will be trimmed. However, if
///   there are non-whitespace characters on that line, only a single space
///   immediately following the initial backticks or language tag will be
///   trimmed if present.
///
///   If the final line is entirely whitespace up to the closing backticks, it
///   will be trimmed. Otherwise, if the last non-whitespace character of the
///   final line is a backtick, then one space character will be trimmed from
///   the end of the line if present.
///
/// - *Common indentation at the beginning of lines is trimmed*
///
///   Typst will remove initial whitespace at the beginning of lines in the raw
///   text that is shared between all lines, i.e. common indentation. Although
///   this excludes text on the line with the initial backticks.
///
///   Typst first finds the line with the fewest initial whitespace characters
///   that contains some non-whitespace characters, including the line with the
///   closing backticks. Then Typst trims characters from every line equal to
///   the number of initial whitespace characters in that line. Lines which are
///   only whitespace will remove the same number of characters until they are
///   empty, but will keep any extra trailing whitespace.
///
///   #let code-point = "https://www.unicode.org/glossary/#code_point"
///
///   Note that this check treats tabs and spaces as equivalent characters for
///   simplicity, and that it operates on numbers of #link(code-point)[Unicode
///   code points], i.e. characters, not on byte lengths.
///
/// These properties of the three-plus backtick syntax allow for some use cases
/// that may not be obvious:
///
/// - To write text containing a sequence of backticks, enclose it with one or
///   more backticks than the sequence:
///   ````` ```` enclosed```backticks```` `````
///
/// - To write text that starts or ends with a backtick, add a space inside the
///   opening and closing backticks: ```` ``` `backticks` ``` ````
///
/// - To write inline text highlighted with a language tag, add a space between
///   the language tag and the text ````rust ```rust fn main() {}``` ````
///
/// - To write inline text without any language tag, add a space after the
///   initial backticks: ```` ``` text``` ```` or use the single backtick
///   syntax: ``` `text` ```
///
/// == Embedding strings with raw syntax <embedding-strings>
/// A common use-case for raw syntax is to embed data as strings with formatting
/// by accessing the `.text` field on raw content to get the underlying string.
/// This may also be paired with the @bytes constructor to convert the string to
/// bytes.
///
/// ````example
/// An inline YAML dictionary via `.text`
///
/// #yaml(bytes(
///   ```yaml
///   Magic:
///     limited-by: Mana
///   Pokémon:
///     limited-by: Energy
///   Yu-Gi-Oh:
///     limited-by: false
///   ```.text
///   //  ^^^^ used as a string
/// ))
/// ````
///
/// == Language tag changes <language-tag-changes>
///
/// When using raw syntax with three or more backticks, text immediately after
/// the initial backticks (up to the first whitespace) is treated as a
/// @raw.lang[language tag]. However in the current version of Typst, only text
/// that would be a valid Typst identifier is treated as the language tag. The
/// first character not valid for an identifier will be interpreted as starting
/// the raw text.
///
/// For example, in the current verion of Typst, if a raw block starts with
/// `C++`, the identifier `C` will be the language tag, and the raw text will
/// start with `++`. If a raw block starts with `++C`, it will have no language
/// tag and the raw text will start with `++C`.
///
/// To use language tags that are not valid as identifiers in the current
/// version of Typst, you must use the @raw.lang[`lang`] parameter, either by
/// calling the constructor with a string: ```typ #raw("text", lang: "...")```,
/// or by writing a set rule: ```typ #set raw(lang: "...")```.
///
/// In the next version of Typst, _all text_ up to the first whitespace or
/// backtick will be treated as the language tag, allowing a wider character set
/// for language tags. Tags including spaces or backticks will still need to be
/// set manually via the @raw.lang[`lang`] parameter.
///
/// Typst will alert you if your raw blocks will be interpreted differently in
/// the next Typst version by emitting a warning.
#[elem(
    name = "raw",
    scope,
    title = "Raw Text / Code",
    Synthesize,
    Locatable,
    Tagged,
    ShowSet,
    LocalName,
    Figurable,
    PlainText
)]
pub struct RawElem {
    // avenger: the example's fence is marked `ignore`, so that rustdoc doesn't run it.
    /// The raw text.
    ///
    /// You can also use raw blocks creatively to create custom syntaxes for
    /// your automations.
    ///
    /// #example(
    ///   title: "Implementing a DSL using raw and show rules",
    ///   ````ignore
    ///   // Parse numbers in raw blocks with the
    ///   // `mydsl` tag and sum them up.
    ///   #show raw.where(lang: "mydsl"): it => {
    ///     let sum = 0
    ///     for part in it.text.split("+") {
    ///       sum += int(part.trim())
    ///     }
    ///     sum
    ///   }
    ///
    ///   ```mydsl
    ///   1 + 2 + 3 + 4 + 5
    ///   ```
    ///   ````
    /// )
    #[required]
    pub text: RawContent,

    /// Whether the raw text is displayed as a separate block.
    ///
    /// In markup mode, using one-backtick notation makes this `{false}`. Using
    /// three-backtick notation makes it `{true}` if the enclosed content
    /// contains at least one line break.
    ///
    /// ````example
    /// // Display inline code in a small box
    /// // that retains the correct baseline.
    /// #show raw.where(block: false): box.with(
    ///   fill: luma(240),
    ///   inset: (x: 3pt, y: 0pt),
    ///   outset: (y: 3pt),
    ///   radius: 2pt,
    /// )
    ///
    /// // Display block code in a larger block
    /// // with more padding.
    /// #show raw.where(block: true): block.with(
    ///   fill: luma(240),
    ///   inset: 10pt,
    ///   radius: 4pt,
    /// )
    ///
    /// With `rg`, you can search through your files quickly.
    /// This example searches the current directory recursively
    /// for the text `Hello World`:
    ///
    /// ```bash
    /// rg "Hello World"
    /// ```
    /// ````
    #[default(false)]
    pub block: bool,

    /// The language to interpret the raw text as for syntax highlighting.
    ///
    /// In @html[HTML export], this sets the `data-lang` attribute of the
    /// generated @html.code element.
    ///
    /// Apart from typical language tags known from Markdown, this supports the
    /// `{"typ"}`, `{"typc"}`, and `{"typm"}` tags for
    /// @reference:syntax:markup[Typst markup],
    /// @reference:syntax:code[Typst code], and
    /// @reference:syntax:math[Typst math], respectively.
    ///
    /// ````example
    /// ```typ
    /// This is *Typst!*
    /// ```
    ///
    /// This is ```typ also *Typst*```, but inline!
    /// ````
    pub lang: Option<EcoString>,

    /// The horizontal alignment that each line in a raw block should have. This
    /// option is ignored if this is not a raw block (if specified
    /// `block: false` or single backticks were used in markup mode).
    ///
    /// By default, this is set to `{start}`, meaning that raw text is aligned
    /// towards the start of the text direction inside the block by default,
    /// regardless of the current context's alignment (allowing you to center
    /// the raw block itself without centering the text inside it, for example).
    ///
    /// ````example
    /// #set raw(align: center)
    ///
    /// ```typc
    /// let f(x) = x
    /// code = "centered"
    /// ```
    /// ````
    #[default(HAlignment::Start)]
    pub align: HAlignment,

    // avenger: no `syntaxes` or `theme`, since labels highlight nothing.

    /// The size for a tab stop in spaces. A tab is replaced with enough spaces
    /// to align with the next multiple of the size.
    ///
    /// ````example
    /// #set raw(tab-size: 8)
    /// ```tsv
    /// Year	Month	Day
    /// 2000	2	3
    /// 2001	2	1
    /// 2002	3	10
    /// ```
    /// ````
    #[default(2)]
    pub tab_size: usize,

    /// The stylized lines of raw text.
    ///
    /// Made accessible for the @raw.line[`raw.line` element]. Allows more
    /// styling control in `show` rules.
    #[synthesized]
    pub lines: Vec<Packed<RawLine>>,

    /// The font families of the label's raw text.
    // avenger: the engine's monospace family, which the show-set rule applies in place of
    // upstream's bundled DejaVu Sans Mono.
    #[internal]
    #[ghost]
    pub label_font: Option<FontList>,
}
}

impl Synthesize for Packed<RawElem> {
    // avenger: raw text from a call follows the rules evaluation checks for raw markup: a
    // label highlights nothing, and is one line.
    fn synthesize(&mut self, _: &mut Engine, styles: StyleChain) -> SourceResult<()> {
        if self.lang.get_ref(styles).is_some() {
            bail!(
                self.span(), "syntax highlighting is not supported in labels";
                hint: "remove the `lang` argument";
            );
        }
        let seq = self.highlight(styles);
        if seq.len() > 1 {
            bail!(self.span(), "raw text in a label must be a single line");
        }
        self.lines = Some(seq);
        Ok(())
    }
}

impl Packed<RawElem> {
    // upstream: crates/typst-library/src/text/raw.rs::Packed<RawElem>::highlight @ v0.15.1
    // avenger: the result without a theme, upstream's `non_highlighted_result`.
    fn highlight(&self, styles: StyleChain) -> Vec<Packed<RawLine>> {
        let elem = self.as_ref();
        let lines = preprocess(&elem.text, styles, self.span());

        let count = lines.len() as i64;

        lines
            .into_iter()
            .enumerate()
            .map(|(i, (line, line_span))| {
                Packed::new(RawLine::new(
                    i as i64 + 1,
                    count,
                    line.clone(),
                    TextElem::packed(line).spanned(line_span),
                ))
                .spanned(line_span)
            })
            .collect()
    }
}

impl ShowSet for Packed<RawElem> {
    fn show_set(&self, styles: StyleChain) -> Styles {
        let mut out = Styles::new();
        out.set(TextElem::overhang, false);
        out.set(TextElem::lang, Lang::ENGLISH);
        // avenger: no `hyphenate`, since a label never breaks lines.
        out.set(TextElem::size, TextSize(Em::new(0.8).into()));
        out.set(TextElem::font, FontList(vec![FontFamily::new("DejaVu Sans Mono")]));
        // avenger: the label's monospace family.
        if let Some(font) = styles.get_cloned(RawElem::label_font) {
            out.set(TextElem::font, font);
        }
        out.set(TextElem::cjk_latin_spacing, Smart::Custom(None));
        if self.block.get(styles) {
            out.set(ParElem::justify, false);
        }
        out
    }
}

cast! {
    RawElem,
    v: Content => v.unpack::<Self>().map_err(|_| "expected raw text")?
}

/// The content of the raw text.
// avenger: no `Hash`, since nothing hashes content.
#[derive(Debug, Clone)]
pub enum RawContent {
    /// From a string.
    Text(EcoString),
    /// From lines of text.
    Lines(EcoVec<(EcoString, Span)>),
}

impl RawContent {
    /// Returns or synthesizes the text content of the raw text.
    fn get(&self) -> EcoString {
        match self.clone() {
            RawContent::Text(text) => text,
            RawContent::Lines(lines) => {
                let mut lines = lines.into_iter().map(|(s, _)| s);
                if lines.len() <= 1 {
                    lines.next().unwrap_or_default()
                } else {
                    lines.collect::<Vec<_>>().join("\n").into()
                }
            }
        }
    }
}

impl PartialEq for RawContent {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (RawContent::Text(a), RawContent::Text(b)) => a == b,
            (lines @ RawContent::Lines(_), RawContent::Text(text))
            | (RawContent::Text(text), lines @ RawContent::Lines(_)) => {
                *text == lines.get()
            }
            (RawContent::Lines(a), RawContent::Lines(b)) => Iterator::eq(
                a.iter().map(|(line, _)| line),
                b.iter().map(|(line, _)| line),
            ),
        }
    }
}

cast! {
    RawContent,
    self => self.get().into_value(),
    v: EcoString => Self::Text(v),
}

elem! {
/// A highlighted line of raw text.
///
/// This is a helper element that is synthesized by @raw elements.
///
/// It allows you to access various properties of the line, such as the line
/// number, the raw non-highlighted text, the highlighted text, and whether it
/// is the first or last line of the raw block.
#[elem(name = "line", title = "Raw Text / Code Line", Tagged, PlainText)]
pub struct RawLine {
    /// The line number of the raw line inside of the raw block, starts at 1.
    #[required]
    pub number: i64,

    /// The total number of lines in the raw block.
    #[required]
    pub count: i64,

    /// The line of raw text.
    #[required]
    pub text: EcoString,

    /// The highlighted raw text.
    #[required]
    pub body: Content,
}
}

fn preprocess(
    text: &RawContent,
    styles: StyleChain,
    span: Span,
) -> EcoVec<(EcoString, Span)> {
    if let RawContent::Lines(lines) = text
        && lines.iter().all(|(s, _)| !s.contains('\t'))
    {
        return lines.clone();
    }

    let mut text = text.get();
    if text.contains('\t') {
        let tab_size = styles.get(RawElem::tab_size);
        text = align_tabs(&text, tab_size);
    }
    split_newlines(&text)
        .into_iter()
        .map(|line| (line.into(), span))
        .collect()
}

/// Replace tabs with spaces to align with multiples of `tab_size`.
fn align_tabs(text: &str, tab_size: usize) -> EcoString {
    let replacement = " ".repeat(tab_size);
    let divisor = tab_size.max(1);
    let amount = text.chars().filter(|&c| c == '\t').count();

    let mut res = EcoString::with_capacity(text.len() - amount + amount * tab_size);
    let mut column = 0;

    for grapheme in text.graphemes(true) {
        let c = grapheme.parse::<char>();
        if c == Ok('\t') {
            let required = tab_size - column % divisor;
            res.push_str(&replacement[..required]);
            column += required;
        } else if c.is_ok_and(typst_syntax::is_newline) || grapheme == "\r\n" {
            res.push_str(grapheme);
            column = 0;
        } else {
            res.push_str(grapheme);
            column += 1;
        }
    }

    res
}
