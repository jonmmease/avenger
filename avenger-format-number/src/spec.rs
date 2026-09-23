use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Align {
    Left,
    Right,
    Center,
    AfterSign,
}

impl Align {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignPolicy {
    Minus,
    Plus,
    Space,
    Parentheses,
}

impl SignPolicy {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Symbol {
    CurrencyCompat,
    Alternate,
}

impl Symbol {
    pub fn from_char(value: char) -> Option<Self> {
        match value {
            '$' => Some(Self::CurrencyCompat),
            '#' => Some(Self::Alternate),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormatType {
    Exponent,
    Fixed,
    General,
    Rounded,
    Si,
    Percent,
    PercentRounded,
    Binary,
    Octal,
    DecimalInteger,
    HexLower,
    HexUpper,
    Character,
    LocaleDefault,
}

impl FormatType {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DigitSpec {
    #[default]
    Auto,
    Precision(u8),
    Fraction(u8),
    Significant(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NumberFormatSpec {
    pub fill: Option<char>,
    pub align: Option<Align>,
    pub sign: Option<SignPolicy>,
    pub symbol: Option<Symbol>,
    pub zero: bool,
    pub width: Option<usize>,
    pub group: Option<bool>,
    pub precision: Option<u8>,
    pub trim: Option<bool>,
    pub format_type: Option<FormatType>,
}

impl NumberFormatSpec {
    pub fn format_type_or_default(&self) -> Option<FormatType> {
        self.format_type
    }
}
