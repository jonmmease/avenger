use serde::{Deserialize, Serialize};

/// Placement of padding around a formatted value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Align {
    Left,
    Right,
    Center,
    /// Place padding after the sign and any currency or radix prefix (`=`).
    AfterSign,
}

impl Align {
    /// Decode `<`, `>`, `^`, or `=`.
    pub fn from_char(value: char) -> Option<Self> {
        match value {
            '<' => Some(Self::Left),
            '>' => Some(Self::Right),
            '^' => Some(Self::Center),
            '=' => Some(Self::AfterSign),
            _ => None,
        }
    }
}

/// Signs for positive and negative values.
/// Values that round to zero lose their negative sign unless `Plus` is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignPolicy {
    /// Show the locale minus only for negative values (`-`).
    Minus,
    /// Show a sign for every value, retaining the minus when a negative value rounds to zero (`+`).
    Plus,
    /// Use a space for nonnegative values and the locale minus for negative values (` `).
    Space,
    /// Enclose negative values in parentheses instead of using a minus sign (`(`).
    Parentheses,
}

impl SignPolicy {
    /// Decode `-`, `+`, a space, or `(`.
    pub fn from_char(value: char) -> Option<Self> {
        match value {
            '-' => Some(Self::Minus),
            '+' => Some(Self::Plus),
            ' ' => Some(Self::Space),
            '(' => Some(Self::Parentheses),
            _ => None,
        }
    }
}

/// Locale currency affixes (`$`) or a radix prefix (`#`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Symbol {
    /// Use the locale currency prefix and suffix without changing precision (`$`).
    CurrencyCompat,
    /// Add `0b`, `0o`, or `0x` for radix formats (`#`).
    Alternate,
}

impl Symbol {
    /// Decode `$` or `#`.
    pub fn from_char(value: char) -> Option<Self> {
        match value {
            '$' => Some(Self::CurrencyCompat),
            '#' => Some(Self::Alternate),
            _ => None,
        }
    }
}

/// Numeric notation selected by a D3 type character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormatType {
    /// Scientific notation with a fixed number of fraction digits (`e`).
    Exponent,
    /// Fixed fraction digits, switching to scientific notation at magnitudes of `1e21` (`f`).
    Fixed,
    /// Significant digits with automatic selection of fixed or scientific notation (`g`).
    General,
    /// Significant digits rendered without an exponent (`r`).
    Rounded,
    /// Significant digits with an SI prefix selected for each value (`s`).
    Si,
    /// Multiply by 100 and format with fixed fraction digits (`%`).
    Percent,
    /// Multiply by 100 and round to significant digits without an exponent (`p`).
    PercentRounded,
    /// Round to an integer and render in base 2 (`b`).
    Binary,
    /// Round to an integer and render in base 8 (`o`).
    Octal,
    /// Round to an integer and render in base 10 without an exponent (`d`).
    DecimalInteger,
    /// Round to an integer and render in base 16 with lowercase letters (`x`).
    HexLower,
    /// Round to an integer and render in base 16 with uppercase letters (`X`).
    HexUpper,
    /// Render numeric input using JavaScript number-to-string conversion (`c`).
    /// Sign and precision options are ignored.
    Character,
    /// General notation with locale grouping enabled by default (`n`, equivalent to `,g`).
    LocaleDefault,
}

impl FormatType {
    /// Decode a supported D3 number format type.
    pub fn from_char(value: char) -> Option<Self> {
        match value {
            'e' => Some(Self::Exponent),
            'f' => Some(Self::Fixed),
            'g' => Some(Self::General),
            'r' => Some(Self::Rounded),
            's' => Some(Self::Si),
            '%' => Some(Self::Percent),
            'p' => Some(Self::PercentRounded),
            'b' => Some(Self::Binary),
            'o' => Some(Self::Octal),
            'd' => Some(Self::DecimalInteger),
            'x' => Some(Self::HexLower),
            'X' => Some(Self::HexUpper),
            'c' => Some(Self::Character),
            'n' => Some(Self::LocaleDefault),
            _ => None,
        }
    }

    /// Return the D3 type character.
    pub fn as_char(self) -> char {
        match self {
            Self::Exponent => 'e',
            Self::Fixed => 'f',
            Self::General => 'g',
            Self::Rounded => 'r',
            Self::Si => 's',
            Self::Percent => '%',
            Self::PercentRounded => 'p',
            Self::Binary => 'b',
            Self::Octal => 'o',
            Self::DecimalInteger => 'd',
            Self::HexLower => 'x',
            Self::HexUpper => 'X',
            Self::Character => 'c',
            Self::LocaleDefault => 'n',
        }
    }
}

/// Caller-supplied precision or the formatter default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DigitSpec {
    /// Use the formatter's default or automatic precision selection.
    #[default]
    Auto,
    /// Fraction digits for `f`, `e`, and `%`, significant digits for `g`, `r`, `s`, and `p`.
    /// Clamped to `0..=20` fraction digits or `1..=21` significant digits when rendered.
    /// Integer formats and `c` ignore precision.
    Precision(u8),
}

/// Parsed D3 fields before defaults and overrides are applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NumberFormatSpec {
    /// Padding character, defaulting to a space unless `zero` is enabled.
    pub fill: Option<char>,
    /// Padding placement, defaulting to right alignment unless `zero` is enabled.
    pub align: Option<Align>,
    /// Sign treatment, defaulting to [`SignPolicy::Minus`].
    pub sign: Option<SignPolicy>,
    pub symbol: Option<Symbol>,
    /// Set fill to `0` and alignment to [`Align::AfterSign`] during resolution.
    pub zero: bool,
    /// Minimum field width in UTF-16 code units, measured before numeral substitution.
    pub width: Option<usize>,
    /// Enable locale digit grouping, which `n` enables by default.
    pub group: Option<bool>,
    /// Parsed `.precision`, saturated at 255 before type-specific clamping.
    pub precision: Option<u8>,
    /// Remove trailing fractional zeros, enabled by default when the type is omitted.
    pub trim: Option<bool>,
    /// `None` selects general notation, defaulting to 12 significant digits and trimming.
    pub format_type: Option<FormatType>,
}
