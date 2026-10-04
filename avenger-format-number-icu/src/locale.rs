use crate::{
    data::{data_error, Context, Symbols},
    notation::Compact,
    skeleton::{Grouping, Skeleton},
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
}

/// Override grouping and separators without changing ICU's digit or symbol lookup.
struct DecimalData {
    grouping: Grouping,
}
impl DataProvider<DecimalSymbolsV1> for DecimalData {
    fn load(&self, req: DataRequest) -> Result<DataResponse<DecimalSymbolsV1>, DataError> {
        let response = DataProvider::<DecimalSymbolsV1>::load(&Baked, req)?;
        let old = response.payload.get();
        let mut grouping_sizes = old.grouping_sizes;
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
        let strings = DecimalSymbolStrsBuilder::from(&*old.strings).build();
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
        let data = DecimalData {
            grouping: skeleton.grouping,
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
        Ok(Self {
            decimal,
            exponent_decimal,
            symbols,
            separator: s.decimal_separator().into(),
            signs,
            latin_digits: context.numbering_system == "latn",
            compact,
        })
    }
}
