use crate::{
    currency::{Currency, Width},
    data::{data_error, Context, Symbols},
    notation::Compact,
    percent::Percent,
    skeleton::{unsupported, Grouping, Skeleton, Unit},
};
use avenger_format::NumberFormatError;
use fixed_decimal::Sign;
use icu_decimal::{
    options::{DecimalFormatterOptions, GroupingStrategy},
    provider::*,
    DecimalFormatter,
};
use icu_provider::prelude::*;

/// Sign affixes from ICU4X's decimal symbols, used by every presentation.
#[derive(Debug)]
pub(crate) struct Signs {
    minus: (String, String),
    plus: (String, String),
}

impl Signs {
    pub fn apply(&self, text: &str, sign: Sign) -> String {
        let (prefix, suffix) = match sign {
            Sign::Negative => &self.minus,
            Sign::Positive => &self.plus,
            Sign::None => return text.into(),
        };
        format!("{prefix}{text}{suffix}")
    }

    /// The sign as one string, for patterns that position it themselves.
    pub fn symbol(&self, sign: Sign) -> String {
        self.apply("", sign)
    }
}

/// The presentation that surrounds a formatted number.
#[derive(Debug)]
pub(crate) enum Affix {
    Plain,
    Percent(Percent),
    Currency(Currency),
}

#[derive(Debug)]
pub(crate) struct LocaleData {
    pub decimal: DecimalFormatter,
    pub exponent_decimal: DecimalFormatter,
    pub symbols: &'static Symbols,
    pub separator: String,
    pub signs: Signs,
    pub latin_digits: bool,
    pub compact: Option<Compact>,
    pub affix: Affix,
}

/// Override grouping and separators without changing ICU's digit or symbol lookup.
struct DecimalData {
    grouping: Grouping,
    sizes: Option<(u8, u8)>,
    separators: Option<(&'static str, &'static str)>,
}
impl DataProvider<DecimalSymbolsV1> for DecimalData {
    fn load(&self, req: DataRequest) -> Result<DataResponse<DecimalSymbolsV1>, DataError> {
        let response = DataProvider::<DecimalSymbolsV1>::load(&Baked, req)?;
        let old = response.payload.get();
        let mut grouping_sizes = old.grouping_sizes;
        if let Some((primary, secondary)) = self.sizes {
            grouping_sizes.primary = primary;
            grouping_sizes.secondary = secondary;
        }
        match self.grouping {
            Grouping::Aligned => grouping_sizes.min_grouping = 1,
            Grouping::Thousands => {
                grouping_sizes = GroupingSizes {
                    primary: 3,
                    secondary: 3,
                    min_grouping: 1,
                }
            }
            _ => (),
        }
        let mut builder = DecimalSymbolStrsBuilder::from(&*old.strings);
        if let Some((decimal, group)) = self.separators {
            builder.decimal_separator = decimal.into();
            builder.grouping_separator = group.into();
        }
        let strings = builder.build();
        Ok(DataResponse {
            metadata: response.metadata,
            payload: DataPayload::from_owned(DecimalSymbols {
                strings,
                grouping_sizes,
            }),
        })
    }
}
impl DataProvider<DecimalDigitsV1> for DecimalData {
    fn load(&self, req: DataRequest) -> Result<DataResponse<DecimalDigitsV1>, DataError> {
        DataProvider::<DecimalDigitsV1>::load(&Baked, req)
    }
}

impl LocaleData {
    pub fn new(context: &Context, skeleton: &Skeleton) -> Result<Self, NumberFormatError> {
        let symbols = context.symbols;
        let monetary = match skeleton.unit {
            Unit::Currency(_) => symbols.monetary.as_ref(),
            _ => None,
        };
        let data = DecimalData {
            grouping: skeleton.grouping,
            sizes: match skeleton.unit {
                Unit::Percent | Unit::Permille => symbols.percent_grouping,
                // Full currency names format the number with the decimal pattern.
                Unit::Currency(_) if skeleton.width == Width::FullName => None,
                Unit::Currency(_) if skeleton.accounting => symbols
                    .accounting_grouping
                    .or(monetary.map(|m| (m.primary, m.secondary))),
                _ => monetary.map(|m| (m.primary, m.secondary)),
            },
            separators: monetary.map(|m| (m.decimal, m.group)),
        };
        let mut options = DecimalFormatterOptions::default();
        options.grouping_strategy = Some(match skeleton.grouping {
            Grouping::Off => GroupingStrategy::Never,
            Grouping::Min2 => GroupingStrategy::Min2,
            _ => GroupingStrategy::Auto,
        });
        let decimal = DecimalFormatter::try_new_unstable(&data, context.prefs, options)
            .map_err(data_error)?;
        let strings = context
            .load::<DecimalSymbolsV1, _>(&data)
            .map_err(data_error)?;
        let s = strings.get();
        let signs = Signs {
            minus: (s.minus_sign_prefix().into(), s.minus_sign_suffix().into()),
            plus: (s.plus_sign_prefix().into(), s.plus_sign_suffix().into()),
        };
        let mut exponent_options = DecimalFormatterOptions::default();
        exponent_options.grouping_strategy = Some(GroupingStrategy::Never);
        let exponent_decimal =
            DecimalFormatter::try_new(context.prefs, exponent_options).map_err(data_error)?;
        let compact = skeleton
            .notation
            .is_compact()
            .then(|| Compact::new(context, skeleton.notation))
            .transpose()
            .map_err(data_error)?;
        let affix = match &skeleton.unit {
            Unit::None => Affix::Plain,
            Unit::Percent | Unit::Permille => {
                if skeleton.width == Width::FullName || skeleton.notation.is_compact() {
                    return Err(unsupported(
                        "unit",
                        "ICU has no full-name or compact percent/per-mille patterns",
                    ));
                }
                let unit = if skeleton.unit == Unit::Percent {
                    symbols.percent
                } else {
                    symbols.permille
                };
                Affix::Percent(Percent::new(context, unit).map_err(data_error)?)
            }
            Unit::Currency(code) => {
                Affix::Currency(Currency::new(context, skeleton, *code).map_err(data_error)?)
            }
        };
        Ok(Self {
            decimal,
            exponent_decimal,
            symbols,
            separator: s.decimal_separator().into(),
            signs,
            latin_digits: context.numbering_system == "latn",
            compact,
            affix,
        })
    }
}
