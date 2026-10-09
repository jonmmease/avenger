# Typst support in labels

Labels are paragraphs of [Typst](https://typst.app) markup, on one line or several. This page
goes through the [Typst reference](https://typst.app/docs/reference/) section by section and
marks what labels support:

- **Yes**: works as in Typst.
- **Partial**: works within the limits in the notes.
- **No**: an error. A name the label library doesn't define gives `unknown variable: …`, and
  syntax a label can't use gives `… are not supported in labels`.

Supported features behave as in Typst, except for the divergences that
[UPSTREAM.md](../UPSTREAM.md#deliberate-divergences) lists. The ones you'll meet most:

- Values without a text form, such as booleans, lengths and arrays, can't be displayed; pass
  them as arguments instead (D3).
- Numbers count as content (D4).
- Line breaks in data become spaces (D5).
- Named colors are CSS colors, so `red` is `#ff0000` (D22).

## Language

### [Syntax](https://typst.app/docs/reference/syntax/)

#### Markup

| Element | Example | Labels | Notes |
|---|---|---|---|
| Paragraph break | blank line | No | A label is one paragraph. |
| Strong emphasis | `*strong*` | Yes | |
| Emphasis | `_emphasis_` | Yes | |
| Raw text | `` `print(1)` `` | Partial | Inline and on one line. A language tag is an error, since labels don't highlight. |
| Link | `https://typst.app/` | No | Escape it to show it as text, as `escape_text` does. |
| Label | `<intro>` | No | |
| Reference | `@intro` | No | |
| Heading | `= Heading` | No | |
| Bullet list | `- item` | No | |
| Numbered list | `+ item` | No | |
| Term list | `/ Term: description` | No | |
| Math | `$x^2$` | Yes | Inline equations; see [Math](#math). |
| Line break | `\` | Yes | Line breaks in data stay spaces (D5). |
| Smart quote | `'single' or "double"` | Yes | The quotes follow the text's language. |
| Symbol shorthand | `~`, `---` | Yes | |
| Code expression | `#rect(width: 1cm)` | Partial | Calls of the label library; see [Scripting](#scripting). |
| Character escape | `Tweet at us \#ad` | Yes | Unicode escapes too. |
| Comment | `/* block */`, `// line` | Yes | |

#### Math mode

| Element | Example | Labels | Notes |
|---|---|---|---|
| Inline math | `$x^2$` | Yes | |
| Block-level math | `$ x^2 $` | No | Block equations are errors. |
| Bottom attachment | `$x_1$` | Yes | |
| Top attachment | `$x^2$` | Yes | |
| Fraction | `$1 + (a+b)/5$` | Yes | |
| Line break | `$x \ y$` | No | |
| Alignment point | `$x &= 2 \ &= 3$` | No | |
| Variable access | `$#x$`, `$pi$` | Yes | The library's names. |
| Field access | `$arrow.r.long$` | Yes | |
| Implied multiplication | `$x y$` | Yes | |
| Symbol shorthand | `$->$`, `$!=$` | Yes | |
| Text/string in math | `$a "is natural"$` | Yes | |
| Math function call | `$floor(x)$` | Yes | |
| Code expression | `$#rect(width: 1cm)$` | Partial | As in markup. |
| Character escape | `$x\^2$` | Yes | |
| Comment | `$/* comment */$` | Yes | |

#### Code mode

| Element | Example | Labels | Notes |
|---|---|---|---|
| None | `none` | Yes | Displays as nothing. |
| Auto | `auto` | Partial | As an argument; it can't be displayed. |
| Boolean | `false`, `true` | Partial | As an argument; it can't be displayed. |
| Integer | `10`, `0xff` | Yes | |
| Floating-point number | `3.14`, `1e5` | Yes | |
| Length | `2pt`, `3mm`, `1em` | Partial | As an argument; it can't be displayed. |
| Angle | `90deg`, `1rad` | Partial | As an argument; it can't be displayed. |
| Fraction | `2fr` | No | No label function takes one. |
| Ratio | `50%` | Partial | As an argument; it can't be displayed. |
| String | `"hello"` | Yes | |
| Label | `<intro>` | No | |
| Math | `$x^2$` | Yes | |
| Raw text | `` `print(1)` `` | Partial | As in markup. |
| Variable access | `x` | Yes | The library's names. |
| Code block | `{ let x = 1; x + 2 }` | Partial | Expressions only; their results join, as in Typst. |
| Content block | `[*Hello*]` | Yes | |
| Parenthesized expression | `(1 + 2)` | Yes | |
| Array | `(1, 2, 3)` | Partial | As an argument, such as `features: ("smcp",)`. |
| Dictionary | `(a: "hi", b: 2)` | Partial | As an argument, such as a stroke, and with field access. |
| Unary operator | `-x` | Yes | |
| Binary operator | `x + y` | Yes | |
| Assignment | `x = 1` | No | |
| Field access | `x.y` | Yes | |
| Method call | `x.flatten()` | No | Labels' values have no methods. |
| Function call | `min(x, y)` | Yes | Functions of the label library. |
| Argument spreading | `min(..nums)` | Yes | |
| Unnamed function | `(x, y) => x + y` | No | |
| Let binding | `let x = 1` | No | |
| Named function | `let f(x) = 2 * x` | No | |
| Set rule | `set text(14pt)` | No | |
| Set-if rule | `set text(..) if ..` | No | |
| Show-set rule | `show heading: set block(..)` | No | |
| Show rule with function | `show raw: it => {..}` | No | |
| Show-everything rule | `show: template` | No | |
| Context expression | `context text.lang` | No | |
| Conditional | `if x == 1 {..} else {..}` | No | |
| For loop | `for x in (1, 2, 3) {..}` | No | |
| While loop | `while x < 10 {..}` | No | |
| Loop control flow | `break`, `continue` | No | |
| Return from function | `return x` | No | |
| Include module | `include "bar.typ"` | No | |
| Import module | `import "bar.typ"` | No | |
| Import items from module | `import "bar.typ": a, b, c` | No | |
| Comment | `/* block */`, `// line` | Yes | |

### [Styling](https://typst.app/docs/reference/styling/)

| Section | Labels | Notes |
|---|---|---|
| [Set rules](https://typst.app/docs/reference/styling/#set-rules) | No | A label's base style comes from `LabelOptions`; pass element arguments directly, as in `#underline(stroke: red)[a]`. |
| [Show rules](https://typst.app/docs/reference/styling/#show-rules) | No | |

### [Scripting](https://typst.app/docs/reference/scripting/)

| Section | Labels | Notes |
|---|---|---|
| [Expressions](https://typst.app/docs/reference/scripting/#expressions) | Yes | `#` starts an expression in markup and math. |
| [Blocks](https://typst.app/docs/reference/scripting/#blocks) | Partial | Content blocks; code blocks of expressions only. |
| [Bindings and destructuring](https://typst.app/docs/reference/scripting/#bindings) | No | `bind` writes values into a label's markup instead. |
| [Conditionals](https://typst.app/docs/reference/scripting/#conditionals) | No | |
| [Loops](https://typst.app/docs/reference/scripting/#loops) | No | |
| [Fields](https://typst.app/docs/reference/scripting/#fields) | Yes | Of modules, symbols, dictionaries and lengths, as in `sym.arrow.r` and `(1pt + 2em).em`. |
| [Methods](https://typst.app/docs/reference/scripting/#methods) | No | |
| [Modules](https://typst.app/docs/reference/scripting/#modules) | Partial | The library's `sym`, `emoji` and `math` modules; no `include` or `import`. |
| [Packages](https://typst.app/docs/reference/scripting/#packages) | No | |
| [Operators](https://typst.app/docs/reference/scripting/#operators) | Partial | Arithmetic, comparison, `and`, `or`, `not` and `in`; no assignment operators. |

### [Context](https://typst.app/docs/reference/context/)

| Section | Labels | Notes |
|---|---|---|
| [Style context](https://typst.app/docs/reference/context/#style-context) | No | |
| [Location context](https://typst.app/docs/reference/context/#location-context) | No | |

## Library

### [Foundations](https://typst.app/docs/reference/foundations/)

| Item | Labels | Notes |
|---|---|---|
| [`arguments`](https://typst.app/docs/reference/foundations/arguments/) | No | Spreading an array or dictionary into a call works. |
| [`array`](https://typst.app/docs/reference/foundations/array/) | Partial | As an argument; no methods, and it can't be displayed. |
| [`assert`](https://typst.app/docs/reference/foundations/assert/) | No | |
| [`auto`](https://typst.app/docs/reference/foundations/auto/) | Partial | As an argument. |
| [`bool`](https://typst.app/docs/reference/foundations/bool/) | Partial | As an argument, with `and`, `or` and `not`. |
| [`bytes`](https://typst.app/docs/reference/foundations/bytes/) | No | |
| [`calc`](https://typst.app/docs/reference/foundations/calc/) | Partial | Upstream's functions and constants; no decimal arguments. |
| [`content`](https://typst.app/docs/reference/foundations/content/) | Yes | |
| [`datetime`](https://typst.app/docs/reference/foundations/datetime/) | Partial | Built with `datetime(…)` and formatted with `#datetimefmt`. No times without dates, `today` or methods. |
| [`decimal`](https://typst.app/docs/reference/foundations/decimal/) | No | |
| [`dictionary`](https://typst.app/docs/reference/foundations/dictionary/) | Partial | As an argument, with field access; no methods. |
| [`duration`](https://typst.app/docs/reference/foundations/duration/) | No | |
| [`eval`](https://typst.app/docs/reference/foundations/eval/) | No | |
| [`float`](https://typst.app/docs/reference/foundations/float/) | Partial | Numbers, arithmetic, and the constants `float.inf` and `float.nan`; no `float(..)` constructor or methods. |
| [`function`](https://typst.app/docs/reference/foundations/function/) | Partial | Calls of library functions; no `with` or `where`, and no functions of your own. |
| [`int`](https://typst.app/docs/reference/foundations/int/) | Partial | Numbers and arithmetic; no `int(..)` constructor or methods. |
| [`label`](https://typst.app/docs/reference/foundations/label/) | No | |
| [`module`](https://typst.app/docs/reference/foundations/module/) | Partial | `sym`, `emoji`, `math` and `calc`. |
| [`none`](https://typst.app/docs/reference/foundations/none/) | Yes | |
| [`panic`](https://typst.app/docs/reference/foundations/panic/) | No | |
| [`path`](https://typst.app/docs/reference/foundations/path/) | No | |
| [`plugin`](https://typst.app/docs/reference/foundations/plugin/) | No | |
| [`regex`](https://typst.app/docs/reference/foundations/regex/) | No | |
| [`repr`](https://typst.app/docs/reference/foundations/repr/) | No | |
| [`selector`](https://typst.app/docs/reference/foundations/selector/) | No | |
| [`std`](https://typst.app/docs/reference/foundations/std/) | No | |
| [`str`](https://typst.app/docs/reference/foundations/str/) | Partial | Strings, with `+`, `*` and `in`; no `str(..)` constructor or methods. |
| [`symbol`](https://typst.app/docs/reference/foundations/symbol/) | Partial | Symbols from `sym`, `emoji` and shorthands, with modifiers; no `symbol(..)` constructor. |
| [`sys`](https://typst.app/docs/reference/foundations/sys/) | No | |
| [`target`](https://typst.app/docs/reference/foundations/target/) | No | |
| [`type`](https://typst.app/docs/reference/foundations/type/) | Partial | `float` names its type, for its constants; no `type(..)` function and no other type names. |
| [`version`](https://typst.app/docs/reference/foundations/version/) | No | |

### [Model](https://typst.app/docs/reference/model/)

| Item | Labels | Notes |
|---|---|---|
| [`asset`](https://typst.app/docs/reference/model/asset/) | No | |
| [`bibliography`](https://typst.app/docs/reference/model/bibliography/) | No | |
| [`cite`](https://typst.app/docs/reference/model/cite/) | No | |
| [`divider`](https://typst.app/docs/reference/model/divider/) | No | |
| [`document`](https://typst.app/docs/reference/model/document/) | No | |
| [`emph`](https://typst.app/docs/reference/model/emph/) | Yes | |
| [`enum`](https://typst.app/docs/reference/model/enum/) | No | |
| [`figure`](https://typst.app/docs/reference/model/figure/) | No | |
| [`footnote`](https://typst.app/docs/reference/model/footnote/) | No | |
| [`heading`](https://typst.app/docs/reference/model/heading/) | No | |
| [`link`](https://typst.app/docs/reference/model/link/) | No | |
| [`list`](https://typst.app/docs/reference/model/list/) | No | |
| [`numbering`](https://typst.app/docs/reference/model/numbering/) | No | |
| [`outline`](https://typst.app/docs/reference/model/outline/) | No | |
| [`par`](https://typst.app/docs/reference/model/par/) | No | A label is one paragraph, whose width and alignment its options set. |
| [`parbreak`](https://typst.app/docs/reference/model/parbreak/) | No | |
| [`quote`](https://typst.app/docs/reference/model/quote/) | No | |
| [`ref`](https://typst.app/docs/reference/model/ref/) | No | |
| [`strong`](https://typst.app/docs/reference/model/strong/) | Yes | |
| [`table`](https://typst.app/docs/reference/model/table/) | No | |
| [`terms`](https://typst.app/docs/reference/model/terms/) | No | |
| [`title`](https://typst.app/docs/reference/model/title/) | No | |

### [Text](https://typst.app/docs/reference/text/)

| Item | Labels | Notes |
|---|---|---|
| [`highlight`](https://typst.app/docs/reference/text/highlight/) | Yes | |
| [`linebreak`](https://typst.app/docs/reference/text/linebreak/) | Yes | `justify` stretches the line before the break to the label's width. |
| [`lorem`](https://typst.app/docs/reference/text/lorem/) | No | |
| [`lower`](https://typst.app/docs/reference/text/lower/) | Yes | |
| [`overline`](https://typst.app/docs/reference/text/overline/) | Yes | |
| [`raw`](https://typst.app/docs/reference/text/raw/) | Partial | One line, without `lang`; a block lays out inline. The engine's monospace family. |
| [`smallcaps`](https://typst.app/docs/reference/text/smallcaps/) | Yes | |
| [`smartquote`](https://typst.app/docs/reference/text/smartquote/) | Yes | |
| [`strike`](https://typst.app/docs/reference/text/strike/) | Yes | |
| [`sub`](https://typst.app/docs/reference/text/sub/) | Yes | |
| [`super`](https://typst.app/docs/reference/text/super/) | Yes | |
| [`text`](https://typst.app/docs/reference/text/text/) | Partial | `fill`, `size`, `weight`, `style`, `font`, `lang`, `region`, `dir`, `baseline`, `tracking` and `features`; other arguments are errors (D14). |
| [`underline`](https://typst.app/docs/reference/text/underline/) | Yes | |
| [`upper`](https://typst.app/docs/reference/text/upper/) | Yes | |

### [Math](https://typst.app/docs/reference/math/)

Inline equations support all of math layout, including text operators such as `sin` and
`lim`, the spacings `thin`, `med`, `thick`, `quad` and `wide`, and every math symbol.

| Item | Labels | Notes |
|---|---|---|
| [`accent`](https://typst.app/docs/reference/math/accent/) | Yes | |
| [Attach](https://typst.app/docs/reference/math/attach/): `attach`, `scripts`, `limits` | Yes | |
| [`binom`](https://typst.app/docs/reference/math/binom/) | Yes | |
| [`cancel`](https://typst.app/docs/reference/math/cancel/) | Yes | |
| [`cases`](https://typst.app/docs/reference/math/cases/) | No | It spans several lines. |
| [`class`](https://typst.app/docs/reference/math/class/) | Yes | |
| [`equation`](https://typst.app/docs/reference/math/equation/) | Partial | Inline only; block equations are errors. |
| [`frac`](https://typst.app/docs/reference/math/frac/) | Yes | |
| [Left/Right](https://typst.app/docs/reference/math/lr/): `lr`, `mid`, `abs`, `norm`, `floor`, `ceil`, `round` | Yes | |
| [`mat`](https://typst.app/docs/reference/math/mat/) | No | It spans several lines. |
| [`op`](https://typst.app/docs/reference/math/op/) | Yes | |
| [`primes`](https://typst.app/docs/reference/math/primes/) | Yes | |
| [Roots](https://typst.app/docs/reference/math/roots/): `root`, `sqrt` | Yes | |
| [Sizes](https://typst.app/docs/reference/math/sizes/): `display`, `inline`, `script`, `sscript` | Yes | |
| [`stretch`](https://typst.app/docs/reference/math/stretch/) | Yes | |
| [Styles](https://typst.app/docs/reference/math/styles/): `upright`, `italic`, `bold` | Yes | |
| [Under/Over](https://typst.app/docs/reference/math/underover/): `underline`, `overline`, `underbrace`, `overbrace`, `underbracket`, `overbracket`, `underparen`, `overparen`, `undershell`, `overshell` | Yes | |
| [Variants](https://typst.app/docs/reference/math/variants/): `serif`, `sans`, `frak`, `mono`, `bb`, `cal`, `scr` | Yes | |
| [`vec`](https://typst.app/docs/reference/math/vec/) | No | It spans several lines. |

### [Symbols](https://typst.app/docs/reference/symbols/)

| Item | Labels | Notes |
|---|---|---|
| [`sym`](https://typst.app/docs/reference/symbols/sym/) | Yes | In markup as `#sym.arrow.r`, and by name in math. |
| [`emoji`](https://typst.app/docs/reference/symbols/emoji/) | Yes | Color emoji need an emoji font, such as the system's. |

### [Layout](https://typst.app/docs/reference/layout/)

| Item | Labels | Notes |
|---|---|---|
| [`align`](https://typst.app/docs/reference/layout/align/) | No | A label's `align` option aligns its lines. |
| [`alignment`](https://typst.app/docs/reference/layout/alignment/) | No | The names exist, but no label function lays anything out by them. |
| [`angle`](https://typst.app/docs/reference/layout/angle/) | Partial | As an argument, such as `cancel(angle: ..)`. |
| [`block`](https://typst.app/docs/reference/layout/block/) | No | |
| [`box`](https://typst.app/docs/reference/layout/box/) | No | A label's `width` option lays it out as a box of that width. |
| [`colbreak`](https://typst.app/docs/reference/layout/colbreak/) | No | |
| [`columns`](https://typst.app/docs/reference/layout/columns/) | No | |
| [`direction`](https://typst.app/docs/reference/layout/direction/) | Partial | As `text(dir: ..)`. |
| [`fraction`](https://typst.app/docs/reference/layout/fraction/) | No | |
| [`grid`](https://typst.app/docs/reference/layout/grid/) | No | |
| [`h`](https://typst.app/docs/reference/layout/h/) | No | In math, the named spacings work. |
| [`hide`](https://typst.app/docs/reference/layout/hide/) | No | |
| [`layout`](https://typst.app/docs/reference/layout/layout/) | No | |
| [`length`](https://typst.app/docs/reference/layout/length/) | Partial | As an argument, with arithmetic and fields. |
| [`measure`](https://typst.app/docs/reference/layout/measure/) | No | |
| [`move`](https://typst.app/docs/reference/layout/move/) | No | |
| [`pad`](https://typst.app/docs/reference/layout/pad/) | No | |
| [`page`](https://typst.app/docs/reference/layout/page/) | No | |
| [`pagebreak`](https://typst.app/docs/reference/layout/pagebreak/) | No | |
| [`place`](https://typst.app/docs/reference/layout/place/) | No | |
| [`ratio`](https://typst.app/docs/reference/layout/ratio/) | Partial | As an argument, such as `stretch(size: 200%)`. |
| [`relative`](https://typst.app/docs/reference/layout/relative/) | Partial | As an argument, such as `stretch(size: 150% + 1pt)`. |
| [`repeat`](https://typst.app/docs/reference/layout/repeat/) | No | |
| [`rotate`](https://typst.app/docs/reference/layout/rotate/) | No | |
| [`scale`](https://typst.app/docs/reference/layout/scale/) | No | |
| [`skew`](https://typst.app/docs/reference/layout/skew/) | No | |
| [`stack`](https://typst.app/docs/reference/layout/stack/) | No | |
| [`v`](https://typst.app/docs/reference/layout/v/) | No | |

### [Visualize](https://typst.app/docs/reference/visualize/)

| Item | Labels | Notes |
|---|---|---|
| [`circle`](https://typst.app/docs/reference/visualize/circle/) | No | |
| [`color`](https://typst.app/docs/reference/visualize/color/) | Partial | `rgb`, `luma` and CSS color names; `rgb("…")` takes any CSS color string (D22). No other color spaces and no methods, such as `lighten` or `mix`. |
| [`curve`](https://typst.app/docs/reference/visualize/curve/) | No | |
| [`ellipse`](https://typst.app/docs/reference/visualize/ellipse/) | No | |
| [`gradient`](https://typst.app/docs/reference/visualize/gradient/) | No | |
| [`image`](https://typst.app/docs/reference/visualize/image/) | No | |
| [`line`](https://typst.app/docs/reference/visualize/line/) | No | |
| [`polygon`](https://typst.app/docs/reference/visualize/polygon/) | No | |
| [`rect`](https://typst.app/docs/reference/visualize/rect/) | No | |
| [`square`](https://typst.app/docs/reference/visualize/square/) | No | |
| [`stroke`](https://typst.app/docs/reference/visualize/stroke/) | Partial | As a length, color, `length + color` or dictionary; no `stroke(..)` constructor. |
| [`tiling`](https://typst.app/docs/reference/visualize/tiling/) | No | |

### [Introspection](https://typst.app/docs/reference/introspection/)

None of it: [`counter`](https://typst.app/docs/reference/introspection/counter/),
[`here`](https://typst.app/docs/reference/introspection/here/),
[`locate`](https://typst.app/docs/reference/introspection/locate/),
[`location`](https://typst.app/docs/reference/introspection/location/),
[`metadata`](https://typst.app/docs/reference/introspection/metadata/),
[`query`](https://typst.app/docs/reference/introspection/query/) and
[`state`](https://typst.app/docs/reference/introspection/state/) are not available.

### [Data loading](https://typst.app/docs/reference/data-loading/)

None of it: [`cbor`](https://typst.app/docs/reference/data-loading/cbor/),
[`csv`](https://typst.app/docs/reference/data-loading/csv/),
[`json`](https://typst.app/docs/reference/data-loading/json/),
[`read`](https://typst.app/docs/reference/data-loading/read/),
[`toml`](https://typst.app/docs/reference/data-loading/toml/),
[`xml`](https://typst.app/docs/reference/data-loading/xml/) and
[`yaml`](https://typst.app/docs/reference/data-loading/yaml/) are not available. `bind` writes
data into a label's markup.

## [Export](https://typst.app/docs/reference/export/)

Labels don't export documents. A compiled label lowers to drawing items instead:
`svg_items` for vector output, `pdf_items` for PDF text and paths, and, with the `raster`
feature, `rasterize` for an RGBA image.

## Avenger additions

| Feature | Example | Notes |
|---|---|---|
| `numfmt` | `#numfmt(value, ",.2f")` | Formats a number with the engine's number formatting provider. Exponent notation lays out as math. |
| `datetimefmt` | `#datetimefmt(date, "%b %-d, %Y")` | Formats a date with the engine's datetime formatting provider. |
| `datetime` arguments | `datetime(…, nanosecond: 5, utc: true)` | `nanosecond` sets the time within a second, and `utc` makes the datetime an instant, which formatters show in their timezone. |
