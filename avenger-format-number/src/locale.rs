use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LocaleId(pub String);

impl LocaleId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl AsRef<str> for LocaleId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LocaleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupingSpec {
    pub primary: usize,
    pub secondary: Option<usize>,
    #[serde(default)]
    pub min_grouping_digits: usize,
}

impl Default for GroupingSpec {
    fn default() -> Self {
        Self {
            primary: 3,
            secondary: Some(3),
            min_grouping_digits: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecimalPattern {
    pub positive_prefix: String,
    pub positive_suffix: String,
    pub negative_prefix: String,
    pub negative_suffix: String,
}

impl Default for DecimalPattern {
    fn default() -> Self {
        Self {
            positive_prefix: String::new(),
            positive_suffix: String::new(),
            negative_prefix: "-".to_string(),
            negative_suffix: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyPattern {
    pub positive_prefix: String,
    pub positive_suffix: String,
    pub negative_prefix: String,
    pub negative_suffix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyFormat {
    pub standard: CurrencyPattern,
    pub accounting: CurrencyPattern,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CurrencyDisplay {
    #[default]
    Symbol,
    Code,
    Name,
    NarrowSymbol,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct NumberLocaleSpec {
    pub base: Option<LocaleId>,
    pub decimal: Option<String>,
    pub group: Option<String>,
    pub grouping: Option<GroupingSpec>,
    pub minus: Option<String>,
    pub plus: Option<String>,
    pub percent: Option<String>,
    pub permille: Option<String>,
    pub nan: Option<String>,
    pub infinity: Option<String>,
    pub digits: Option<[String; 10]>,
    pub decimal_pattern: Option<DecimalPattern>,
    pub percent_pattern: Option<DecimalPattern>,
    pub currency: Option<CurrencyFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNumberLocale {
    pub id: LocaleId,
    pub decimal: String,
    pub group: String,
    pub grouping: GroupingSpec,
    pub minus: String,
    pub plus: String,
    pub percent: String,
    pub permille: String,
    pub nan: String,
    pub infinity: String,
    pub digits: Option<[String; 10]>,
    pub decimal_pattern: DecimalPattern,
    pub percent_pattern: DecimalPattern,
    pub currency: CurrencyFormat,
}

impl ResolvedNumberLocale {
    pub fn en_us() -> Self {
        Self {
            id: LocaleId::new("en-US"),
            decimal: ".".to_string(),
            group: ",".to_string(),
            grouping: GroupingSpec::default(),
            minus: "-".to_string(),
            plus: "+".to_string(),
            percent: "%".to_string(),
            permille: "\u{2030}".to_string(),
            nan: "NaN".to_string(),
            infinity: "\u{221e}".to_string(),
            digits: None,
            decimal_pattern: DecimalPattern::default(),
            percent_pattern: DecimalPattern {
                positive_prefix: String::new(),
                positive_suffix: "%".to_string(),
                negative_prefix: "-".to_string(),
                negative_suffix: "%".to_string(),
            },
            currency: CurrencyFormat {
                standard: CurrencyPattern {
                    positive_prefix: "\u{00a4}".to_string(),
                    positive_suffix: String::new(),
                    negative_prefix: "-\u{00a4}".to_string(),
                    negative_suffix: String::new(),
                },
                accounting: CurrencyPattern {
                    positive_prefix: "\u{00a4}".to_string(),
                    positive_suffix: String::new(),
                    negative_prefix: "(\u{00a4}".to_string(),
                    negative_suffix: ")".to_string(),
                },
            },
        }
    }
}
