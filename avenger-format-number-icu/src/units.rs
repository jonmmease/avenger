use crate::{
    currency::Width,
    data::{data_error, Context, REGION_UNITS},
    skeleton::{invalid, unsupported},
};
use avenger_format::NumberFormatError;
use fixed_decimal::Decimal;
use icu_experimental::{
    dimension::provider::units::{categorized_display_names::*, display_names::UnitsDisplayNames},
    measure::measureunit::MeasureUnit,
    provider::Baked,
    units::converter_factory::ConverterFactory,
};
use icu_locale::LocaleExpander;
use icu_locale_core::LanguageIdentifier;
use icu_plurals::PluralRules;
use icu_provider::{marker::ErasedMarker, prelude::*};
use writeable::Writeable;

type UnitMarker = ErasedMarker<UnitsDisplayNames<'static>>;

/// Legacy `measure-unit` identifiers prefix a CLDR unit with one of these categories.
const LEGACY_CATEGORIES: [&str; 22] = [
    "acceleration",
    "angle",
    "area",
    "concentr",
    "consumption",
    "digital",
    "duration",
    "electric",
    "energy",
    "force",
    "frequency",
    "graphics",
    "length",
    "light",
    "magnetic",
    "mass",
    "power",
    "pressure",
    "speed",
    "temperature",
    "torque",
    "volume",
];

/// CLDR locales whose unit names ICU4X 2.3 lacks. ICU4X would label them with root names.
const ICU4X_MISSING: [&str; 21] = [
    "bgn", "cad", "ccp", "ce", "cic", "dz", "en-Dsrt", "fur", "gsw", "haw", "jgo", "ksh", "lkt",
    "ms-Arab", "mus", "mzn", "os", "osa", "se", "trv", "wae",
];

/// Base units of the categories that have ICU4X unit labels.
const LABELED_BASE_UNITS: [(&str, &str); 5] = [
    ("area", "square-meter"),
    ("duration", "second"),
    ("length", "meter"),
    ("mass", "kilogram"),
    ("volume", "cubic-meter"),
];

pub(crate) fn legacy(input: &str, position: usize) -> Result<String, NumberFormatError> {
    let unknown = || invalid("unknown legacy measurement unit", position);
    let (category, unit) = input
        .split_once('-')
        .filter(|(category, unit)| {
            LEGACY_CATEGORIES.contains(category) && !unit.is_empty() && !unit.contains("-and-")
        })
        .ok_or_else(unknown)?;
    // As in ICU, the unit must measure its category: `length-square-meter` is invalid.
    if let Some((_, base)) = LABELED_BASE_UNITS
        .iter()
        .find(|(name, _)| *name == category)
    {
        let parse = |name: &str| MeasureUnit::try_from_str(name).ok();
        let measures = parse(unit).zip(parse(base)).is_some_and(|(unit, base)| {
            ConverterFactory::new()
                .converter::<f64>(&unit, &base)
                .is_ok()
        });
        if !measures {
            return Err(unknown());
        }
    }
    Ok(unit.into())
}

/// Upstream plural patterns applied to the number's resolved skeleton presentation.
#[derive(Debug)]
pub(crate) enum Units {
    Hidden,
    Named {
        patterns: DataPayload<UnitMarker>,
        plurals: PluralRules,
    },
}
impl Units {
    pub fn new(name: &str, width: Width, context: &Context) -> Result<Self, NumberFormatError> {
        // CLDR names second and third powers with square- and cubic- prefixes.
        let name = name.replace("pow2-", "square-").replace("pow3-", "cubic-");
        let unrecognized = || {
            unsupported(
                "unit",
                &format!("unrecognized CLDR unit identifier: {name}"),
            )
        };
        if width == Width::Hidden {
            MeasureUnit::try_from_str(&name).map_err(|_| unrecognized())?;
            return Ok(Self::Hidden);
        }
        if let Some(missing) = context
            .chain()
            .iter()
            .find(|l| ICU4X_MISSING.contains(&l.as_str()))
        {
            return Err(NumberFormatError::LocaleUnavailable {
                locale: context.locale.to_string(),
                message: format!("ICU4X has no unit names for `{missing}`"),
            });
        }
        let width = match width {
            Width::Narrow => "narrow",
            Width::Short => "short",
            _ => "long",
        };
        let attributes = format!("{width}-{name}");
        let attributes =
            DataMarkerAttributes::try_from_str(&attributes).map_err(|_| unrecognized())?;
        let region = region(context);
        let mut best = None;
        macro_rules! candidates {
            ($($category:literal: $core:ty, $extended:ty, $outlier:ty;)*) => {$(
                let set = data_set($category, &name, &region);
                candidate::<$core>(context, attributes, set == 0, &mut best)?;
                candidate::<$extended>(context, attributes, set == 1, &mut best)?;
                candidate::<$outlier>(context, attributes, set == 2, &mut best)?;
            )*};
        }
        candidates!(
            "length": UnitsNamesLengthCoreV1, UnitsNamesLengthExtendedV1, UnitsNamesLengthOutlierV1;
            "area": UnitsNamesAreaCoreV1, UnitsNamesAreaExtendedV1, UnitsNamesAreaOutlierV1;
            "duration": UnitsNamesDurationCoreV1, UnitsNamesDurationExtendedV1, UnitsNamesDurationOutlierV1;
            "mass": UnitsNamesMassCoreV1, UnitsNamesMassExtendedV1, UnitsNamesMassOutlierV1;
            "volume": UnitsNamesVolumeCoreV1, UnitsNamesVolumeExtendedV1, UnitsNamesVolumeOutlierV1;
        );
        let patterns = best
            .ok_or_else(|| unsupported("unit", &format!("ICU has no {width} pattern for {name}")))?
            .1;
        let plurals = context.plural_rules().map_err(data_error)?;
        Ok(Self::Named { patterns, plurals })
    }

    pub fn render(&self, body: &str, number: Option<&Decimal>, exponent: i16) -> String {
        let Self::Named { patterns, plurals } = self else {
            return body.into();
        };
        let patterns = patterns.get();
        let pattern = number.map_or_else(
            || patterns.elements.get_default().1,
            |n| patterns.get(crate::notation::plural_operands(n, exponent), plurals),
        );
        pattern.interpolate([body]).write_to_string().into_owned()
    }
}

/// The locale's region as ICU4X's data generation infers it.
fn region(context: &Context) -> String {
    let id = &context.locale.id;
    let mut id = LanguageIdentifier::from((id.language, id.script, id.region));
    LocaleExpander::new_extended().maximize(&mut id);
    id.region.map_or_else(|| "001".into(), |r| r.to_string())
}

/// The data set that holds a unit's names for a region: 0 for core, 1 for extended, and 2 for
/// outlier. As in ICU4X's data generation, a unit is core when the region's preferences (or the
/// world's, for regions without their own) list a unit containing its name, and extended when any
/// region's do.
fn data_set(category: &str, unit: &str, region: &str) -> usize {
    let regions = || REGION_UNITS.iter().filter(move |(c, ..)| *c == category);
    let lists = |units: &[&str]| units.iter().any(|u| u.contains(unit));
    let local = regions()
        .find(|(_, r, _)| *r == region)
        .or_else(|| regions().find(|(_, r, _)| *r == "001"));
    if local.is_some_and(|(_, _, units)| lists(units)) {
        0
    } else if regions().any(|(_, _, units)| lists(units)) {
        1
    } else {
        2
    }
}

/// Prefer the data set that the locale's region assigns. ICU4X stores a locale's names only in
/// that set, which can resolve to root names that are still correct after deduplication, while
/// other sets can resolve to a nearer ancestor's names (ICU4X issue #8125). Locale specificity
/// ranks the other sets when the assigned one has no entry.
fn candidate<M>(
    context: &Context,
    attributes: &DataMarkerAttributes,
    assigned: bool,
    best: &mut Option<(usize, DataPayload<UnitMarker>)>,
) -> Result<(), NumberFormatError>
where
    M: DataMarker<DataStruct = UnitsDisplayNames<'static>>,
    Baked: DataProvider<M>,
{
    if best.as_ref().is_some_and(|(rank, _)| *rank == 0) {
        return Ok(());
    }
    let Some(response) = context
        .load_attributes::<M, _>(&Baked, attributes)
        .map_err(data_error)?
    else {
        return Ok(());
    };
    let rank = if assigned {
        0
    } else {
        context.specificity::<M>(response.metadata.locale.as_ref())
    };
    if best.as_ref().is_none_or(|(previous, _)| rank < *previous) {
        *best = Some((rank, response.payload.cast()));
    }
    Ok(())
}
