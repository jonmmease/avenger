//! The evaluation of `@media` and `@container` conditions against the host inputs.

use crate::{
    condition::{
        Comparison, Condition, Feature, MediaQuery, MediaType, Preference, Query, SizeFeature,
        SizeValue,
    },
    host::{ColorScheme, Contrast, HostInputs},
    values::UnitSizes,
};

/// Whether `condition` holds at `host`. A test of a dimension that the host box lacks is
/// unknown, and an unknown condition is false, also under `not`.
pub(crate) fn holds(condition: &Condition, host: &HostInputs) -> bool {
    match condition {
        Condition::Media(queries) => queries.iter().any(|query| media(query, host)),
        Condition::Container(query) => evaluate(query, host) == Some(true),
    }
}

fn media(query: &MediaQuery, host: &HostInputs) -> bool {
    let typed = match query.media_type {
        None | Some(MediaType::All) => true,
        Some(MediaType::Print) => host.preferences.print,
        Some(MediaType::Screen) => !host.preferences.print,
    };
    let holds = match &query.condition {
        None => Some(typed),
        Some(condition) => evaluate(condition, host).map(|holds| holds && typed),
    };
    match (holds, query.negated) {
        (Some(holds), false) => holds,
        (Some(holds), true) => !holds,
        (None, _) => false,
    }
}

/// The value of a query in Kleene logic: `None` is unknown.
fn evaluate(query: &Query, host: &HostInputs) -> Option<bool> {
    match query {
        Query::Not(query) => evaluate(query, host).map(|holds| !holds),
        Query::And(queries) => {
            let mut result = Some(true);
            for query in queries {
                match evaluate(query, host) {
                    Some(false) => return Some(false),
                    None => result = None,
                    Some(true) => {}
                }
            }
            result
        }
        Query::Or(queries) => {
            let mut result = Some(false);
            for query in queries {
                match evaluate(query, host) {
                    Some(true) => return Some(true),
                    None => result = None,
                    Some(false) => {}
                }
            }
            result
        }
        Query::Feature(feature) => match feature {
            Feature::Preference { name, value } => Some(preference(*name, *value, host)),
            Feature::Size { feature, tests } => size(*feature, tests, host),
        },
    }
}

fn preference(name: Preference, value: Option<&str>, host: &HostInputs) -> bool {
    let preferences = &host.preferences;
    let current = match name {
        Preference::ColorScheme => match preferences.color_scheme {
            ColorScheme::Light => "light",
            ColorScheme::Dark => "dark",
        },
        Preference::Contrast => match preferences.contrast {
            Contrast::NoPreference => "no-preference",
            Contrast::More => "more",
            Contrast::Less => "less",
            Contrast::Custom => "custom",
        },
        Preference::ReducedMotion => match preferences.reduced_motion {
            true => "reduce",
            false => "no-preference",
        },
        Preference::ForcedColors => match preferences.forced_colors {
            true => "active",
            false => "none",
        },
    };
    match value {
        Some(value) => current == value,
        // A test without a value holds for every value except the default.
        None => match name {
            Preference::ColorScheme => true,
            _ => current != name.values()[0],
        },
    }
}

fn size(
    feature: SizeFeature,
    tests: &[(Comparison, SizeValue)],
    host: &HostInputs,
) -> Option<bool> {
    let width = host.width;
    let height = host.height;
    let sizes = UnitSizes {
        rem: 10.0 * host.preferences.text_scale,
        em: 10.0 * host.preferences.text_scale,
    };
    let actual = match feature {
        SizeFeature::Width => width?,
        SizeFeature::Height => height?,
        SizeFeature::AspectRatio => width? / height?,
        SizeFeature::Orientation => {
            let portrait = height? >= width?;
            return Some(tests.iter().all(|(_, value)| match value {
                SizeValue::Orientation("portrait") => portrait,
                SizeValue::Orientation(_) => !portrait,
                _ => false,
            }));
        }
    };
    let mut holds = true;
    for (comparison, value) in tests {
        let value = match value {
            SizeValue::Length(calc) => calc.resolve(&sizes),
            SizeValue::Ratio(ratio) => *ratio,
            SizeValue::Orientation(_) => return Some(false),
        };
        holds &= match comparison {
            Comparison::Less => actual < value,
            Comparison::LessOrEqual => actual <= value,
            Comparison::Greater => actual > value,
            Comparison::GreaterOrEqual => actual >= value,
            Comparison::Equal => (actual - value).abs() < 1e-6,
        };
    }
    Some(holds)
}
