use crate::{
    arithmetic::{decimal128, from_float, rational},
    conversion::Conversion,
    currency::Width,
    data::{data_error, Context},
    locale::LocaleData,
    precision::{Digits, Precision},
    prepared::Prepared,
    skeleton::{unsupported, Skeleton, Unit},
};
use avenger_format::{FormattedNumber, NumberFormatError, PreparedNumberFormatter};
use fixed_decimal::Sign as DecimalSign;
use icu_experimental::measure::{measureunit::MeasureUnit, parser::ids::unit_id};
use icu_list::{options::ListLength, ListFormatter};
use icu_locale_core::{
    extensions::unicode::key,
    preferences::extensions::unicode::keywords::{MeasurementSystem, RegionOverride},
};
use num_rational::BigRational;
use num_traits::{One, Signed, Zero};
use std::collections::BTreeMap;
use writeable::Writeable;

struct Preference {
    unit: &'static str,
    /// Lower bound expressed in the largest output component's unit.
    threshold: &'static str,
    precision: &'static str,
}
include!("generated_usage.rs");

const LABELED_CATEGORIES: &str =
    "usage supports length, area, volume, mass, and duration, which have ICU unit labels";

/// Prepared output candidates in CLDR preference order.
#[derive(Debug)]
pub(crate) struct Usage {
    outputs: Vec<Output>,
    epsilon: BigRational,
}
#[derive(Debug)]
struct Output {
    input: Conversion,
    threshold: BigRational,
    components: Vec<Component>,
    steps: Vec<Conversion>,
    ratios: Vec<BigRational>,
    list: ListFormatter,
    precision: Precision,
}
#[derive(Debug)]
struct Component {
    /// Original identifier order, retained when conversion sorts components by size.
    index: usize,
    formatter: Prepared,
}

impl Usage {
    pub fn new(locale_name: &str, skeleton: Skeleton) -> Result<Self, NumberFormatError> {
        let Unit::Measure(input) = &skeleton.unit else {
            return Err(unsupported("usage", "requires a measurement input unit"));
        };
        if skeleton.width == Width::Hidden {
            return Err(unsupported(
                "unit-width-hidden",
                "hidden labels would conceal which unit each value uses",
            ));
        }
        let context = Context::new(locale_name, skeleton.numbering_system.as_deref())?;
        // ICU formats larger mixed components with default settings, including digits.
        let base = Context::new(locale_name, None)?;
        let locale = &context.locale;
        // ICU permits one f64 ULP at 1 when selecting units and carrying rounded components.
        let epsilon = rational(
            &"1.0000000000000002220446049250313"
                .parse()
                .expect("epsilon decimal"),
        );
        let mut outputs = Vec::new();
        if let Some(usage) = &skeleton.usage {
            if input.contains("-and-") {
                return Err(unsupported(
                    "usage",
                    "input must be a single or compound unit",
                ));
            }
            let category = category(input)?;
            let mut expanded = locale.id.clone();
            icu_locale::LocaleExpander::new_extended().maximize(&mut expanded);
            let keywords = &locale.extensions.unicode.keywords;
            // A region override can name a subdivision, such as `ussf`, whose region applies.
            let region = keywords
                .get(&key!("rg"))
                .and_then(|value| RegionOverride::try_from(value.clone()).ok())
                .map(|rg| rg.region.to_string())
                .or_else(|| expanded.region.map(|r| r.to_string()))
                .unwrap_or_else(|| "001".into());
            // Each measurement system, with a region whose preferences use it.
            let system = keywords
                .get(&key!("ms"))
                .and_then(|value| MeasurementSystem::try_from(value).ok())
                .and_then(|system| match system {
                    MeasurementSystem::Metric => Some(("metric", "001")),
                    MeasurementSystem::USSystem => Some(("ussystem", "US")),
                    MeasurementSystem::UKSystem => Some(("uksystem", "GB")),
                    _ => None,
                });
            let mut current = usage.as_str();
            let preferences = loop {
                if let Some(mut rows) = preferences(category, current, &region) {
                    if let Some((system, region)) = system {
                        if !rows.iter().all(|p| matches_system(p.unit, system)) {
                            rows = preferences(category, current, region).unwrap_or(rows);
                        }
                    }
                    break rows;
                }
                current = if let Some((parent, _)) = current.rsplit_once('-') {
                    parent
                } else if current != "default" {
                    "default"
                } else {
                    return Err(unsupported("usage", LABELED_CATEGORIES));
                };
            };
            for p in preferences {
                outputs.push(Output::new(
                    input,
                    p.unit,
                    p.threshold,
                    p.precision,
                    [&context, &base],
                    &skeleton,
                )?);
            }
        } else {
            let components = sorted_units(input)?;
            let largest = components
                .first()
                .ok_or_else(|| unsupported("unit", "empty mixed unit"))?
                .1;
            outputs.push(Output::new(
                largest,
                input,
                "1",
                "",
                [&context, &base],
                &skeleton,
            )?);
        }
        Ok(Self { outputs, epsilon })
    }
}
impl PreparedNumberFormatter for Usage {
    fn format(&self, value: f64) -> FormattedNumber {
        if !value.is_finite() {
            return self.outputs[0].components[0].formatter.format(value);
        }
        let input = from_float(value);
        let quantity = rational(&input);
        let output = self
            .outputs
            .iter()
            .find(|o| {
                o.input
                    .convert(&quantity.abs())
                    .is_none_or(|q| q * &self.epsilon >= o.threshold)
            })
            .unwrap_or_else(|| self.outputs.last().expect("prepared usage has candidates"));
        output.format(&quantity, value.is_sign_negative(), &self.epsilon)
    }
}
impl Output {
    /// Contexts for the smallest component and for larger components.
    fn new(
        input: &str,
        target: &str,
        threshold: &str,
        precision_spec: &str,
        [smallest, larger]: [&Context; 2],
        skeleton: &Skeleton,
    ) -> Result<Self, NumberFormatError> {
        let units = sorted_units(target)?;
        let precision = if skeleton.explicit_precision
            || skeleton.notation.is_compact()
            || skeleton.usage.is_none()
        {
            skeleton.precision.clone()
        } else if precision_spec.is_empty() {
            Precision::usage()
        } else {
            Skeleton::parse(precision_spec)?.precision
        };
        let mut components = Vec::new();
        for (i, (index, name)) in units.iter().enumerate() {
            let mut s = if i + 1 == units.len() {
                skeleton.clone()
            } else {
                Skeleton::default()
            };
            s.unit = Unit::Measure((*name).into());
            s.usage = None;
            s.width = skeleton.width;
            s.precision = if i + 1 == units.len() {
                precision.clone()
            } else {
                Precision::fraction(Digits {
                    min: 0,
                    max: Some(0),
                })
            };
            let context = if i + 1 == units.len() {
                smallest
            } else {
                larger
            };
            let locale = LocaleData::new(context, &s)?;
            components.push(Component {
                index: *index,
                formatter: Prepared {
                    skeleton: s,
                    locale,
                },
            });
        }
        let mut steps = Vec::new();
        let mut ratios = Vec::new();
        for pair in units.windows(2) {
            let conversion = Conversion::new(pair[0].1, pair[1].1)?;
            let ratio = conversion.positive_ratio().ok_or_else(|| {
                unsupported("unit", "mixed components must use proportional conversions")
            })?;
            if ratio <= BigRational::one() {
                return Err(unsupported(
                    "unit",
                    "mixed components must have distinct sizes",
                ));
            }
            ratios.push(ratio);
            steps.push(conversion);
        }
        let mut options = icu_list::options::ListFormatterOptions::default();
        options.length = Some(match skeleton.width {
            Width::Narrow => ListLength::Narrow,
            Width::FullName => ListLength::Wide,
            _ => ListLength::Short,
        });
        let list = ListFormatter::try_new_unit(smallest.locale.clone().into(), options)
            .map_err(data_error)?;
        Ok(Self {
            input: Conversion::new(input, units[0].1)?,
            threshold: rational(
                &threshold
                    .parse()
                    .map_err(|_| unsupported("usage", "invalid preference threshold"))?,
            ),
            components,
            steps,
            ratios,
            list,
            precision,
        })
    }

    fn format(
        &self,
        input: &BigRational,
        negative: bool,
        epsilon: &BigRational,
    ) -> FormattedNumber {
        let mixed = self.components.len() > 1;
        let Some(mut value) = self
            .input
            .convert(&if mixed { input.abs() } else { input.clone() })
        else {
            return self.components[0].formatter.format(if negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            });
        };
        let last = self.components.len() - 1;
        let mut quantities = Vec::with_capacity(self.components.len());
        for step in &self.steps {
            let integer = (&value * epsilon).floor();
            quantities.push(integer.clone());
            value = step
                .convert(&(value - integer).max(BigRational::zero()))
                .expect("validated proportional conversion");
        }
        let mut number = decimal128(&value);
        self.precision.apply(
            &mut number,
            self.components[last].formatter.skeleton.rounding,
        );
        value = rational(&number);
        quantities.push(value);
        for i in (1..quantities.len()).rev() {
            let carry = (&quantities[i] / &self.ratios[i - 1] * epsilon).floor();
            if carry <= BigRational::zero() {
                continue;
            }
            quantities[i] -= &carry * &self.ratios[i - 1];
            quantities[i - 1] += carry;
        }
        let mut labels = Vec::with_capacity(self.components.len());
        for (i, (component, quantity)) in self.components.iter().zip(quantities).enumerate() {
            let mut number = decimal128(&quantity);
            if i == last {
                component.formatter.skeleton.scale.scale(&mut number);
            }
            // Only the first displayed component carries a sign, even after a negative scale.
            if mixed {
                number.sign = if negative && component.index == 0 {
                    DecimalSign::Negative
                } else {
                    DecimalSign::None
                };
            }
            let text = if i == last {
                component.formatter.format_decimal(number).text
            } else {
                component
                    .formatter
                    .format_mixed_integer(number, &self.precision)
                    .text
            };
            labels.push((component.index, text));
        }
        labels.sort_by_key(|(index, _)| *index);
        FormattedNumber::plain(
            self.list
                .format(labels.iter().map(|(_, text)| text))
                .write_to_string()
                .into_owned(),
        )
    }
}

fn preferences(category: &str, usage: &str, region: &str) -> Option<&'static [Preference]> {
    for region in [region, "001"] {
        let key = format!("{category}/{usage}/{region}");
        if let Ok(i) = PREFERENCES.binary_search_by_key(&key.as_str(), |(k, _)| *k) {
            return Some(PREFERENCES[i].1);
        }
    }
    None
}

/// Sort convertible components by size while retaining their display order.
fn sorted_units(input: &str) -> Result<Vec<(usize, &str)>, NumberFormatError> {
    let names: Vec<_> = input.split("-and-").collect();
    if names.iter().any(|s| s.is_empty()) {
        return Err(unsupported(
            "unit",
            "mixed units require nonempty components",
        ));
    }
    if names.len() == 1 {
        return Ok(vec![(0, names[0])]);
    }
    let mut order = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let ratio = Conversion::new(name, names[0])?
            .positive_ratio()
            .ok_or_else(|| unsupported("unit", "mixed units require proportional components"))?;
        if order.iter().any(|(_, _, previous)| *previous == ratio) {
            return Err(unsupported("unit", "duplicate mixed-unit component"));
        }
        order.push((i, *name, ratio));
    }
    order.sort_by(|a, b| b.2.cmp(&a.2));
    Ok(order.into_iter().map(|(i, n, _)| (i, n)).collect())
}

/// Retain numerator and denominator dimensions before trying cancellation, as CLDR distinguishes consumption from area.
fn dimensions(
    unit: &MeasureUnit,
    expand: bool,
) -> Result<BTreeMap<(u16, bool), i16>, NumberFormatError> {
    let mut result = BTreeMap::new();
    for single in unit.single_units() {
        if expand {
            let name = simple_name(single.unit_id)?;
            let i = BASE_UNITS
                .binary_search_by_key(&name, |(k, _, _)| *k)
                .map_err(|_| unsupported("usage", LABELED_CATEGORIES))?;
            let base = MeasureUnit::try_from_str(BASE_UNITS[i].1)
                .map_err(|_| unsupported("usage", "unrecognized base dimensions"))?;
            for b in base.single_units() {
                let power = i16::from(b.power) * i16::from(single.power);
                *result.entry((b.unit_id, power < 0)).or_default() += power.abs();
            }
        } else {
            *result
                .entry((single.unit_id, single.power < 0))
                .or_default() += i16::from(single.power).abs();
        }
    }
    Ok(result)
}
fn category(name: &str) -> Result<&'static str, NumberFormatError> {
    let unit = MeasureUnit::try_from_str(name)
        .map_err(|_| unsupported("usage", "unrecognized input unit"))?;
    let original = dimensions(&unit, true)?;
    let simplified = {
        let mut powers = BTreeMap::<u16, i16>::new();
        for ((id, negative), power) in &original {
            *powers.entry(*id).or_default() += if *negative { -*power } else { *power };
        }
        powers
            .into_iter()
            .filter(|(_, p)| *p != 0)
            .map(|(id, p)| ((id, p < 0), p.abs()))
            .collect::<BTreeMap<_, _>>()
    };
    for base in [&original, &simplified] {
        for reciprocal in [false, true] {
            let wanted: BTreeMap<_, _> = base
                .iter()
                .map(|((id, negative), p)| ((*id, *negative ^ reciprocal), *p))
                .collect();
            for (name, category) in CATEGORIES {
                if let Ok(unit) = MeasureUnit::try_from_str(name) {
                    if dimensions(&unit, false)? == wanted {
                        return Ok(category);
                    }
                }
            }
        }
    }
    Err(unsupported(
        "usage",
        "unit has no CLDR measurement category",
    ))
}
fn matches_system(unit: &str, system: &str) -> bool {
    unit.split("-and-").all(|name| {
        MeasureUnit::try_from_str(name).is_ok_and(|u| {
            u.single_units().iter().all(|s| {
                simple_name(s.unit_id)
                    .ok()
                    .and_then(|name| BASE_UNITS.binary_search_by_key(&name, |(k, _, _)| *k).ok())
                    .is_some_and(|i| {
                        BASE_UNITS[i]
                            .2
                            .iter()
                            .any(|s| *s == system || *s == "metric_adjacent")
                    })
            })
        })
    })
}

fn simple_name(id: u16) -> Result<&'static str, NumberFormatError> {
    BASE_UNITS
        .iter()
        .find(|(name, _, _)| unit_id(name) == Some(id))
        .map(|(name, _, _)| *name)
        .ok_or_else(|| unsupported("usage", "unit has no base dimensions"))
}
