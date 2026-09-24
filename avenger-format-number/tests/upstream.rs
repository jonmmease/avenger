use avenger_format_number::{
    prepare_number_float_format, prepare_number_prefix_format, prepare_number_step_format,
    NumberLocaleSpec, PreparedNumberFormat, ResolvedNumberLocale,
};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Fixtures {
    locales: BTreeMap<String, NumberLocaleSpec>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    locale: String,
    mode: String,
    spec: Option<String>,
    args: Vec<f64>,
    bits: String,
    expected: String,
}
#[test]
fn matches_d3_and_vega_number_formats() {
    let fixtures: Fixtures = serde_json::from_str(include_str!("fixtures/upstream.json")).unwrap();
    let locales: BTreeMap<_, _> = fixtures
        .locales
        .into_iter()
        .map(|(id, spec)| (id.clone(), ResolvedNumberLocale::new(id, spec).unwrap()))
        .collect();
    let mut failures = Vec::new();
    for case in fixtures.cases {
        let value = f64::from_bits(u64::from_str_radix(&case.bits, 16).unwrap());
        let locale = &locales[&case.locale];
        let format = match case.mode.as_str() {
            "format" => PreparedNumberFormat::new(case.spec.as_deref(), Default::default(), locale),
            "float" => prepare_number_float_format(case.spec.as_deref(), locale),
            "prefix" => {
                prepare_number_prefix_format(case.spec.as_deref().unwrap(), case.args[0], locale)
            }
            "step" => prepare_number_step_format(
                case.args[0],
                case.args[1],
                case.spec.as_deref(),
                Default::default(),
                locale,
            ),
            _ => unreachable!(),
        }
        .unwrap();
        let actual = format.format(value).text;
        if actual != case.expected {
            failures.push(format!(
                "{} {} {:?} {value}: {actual:?} != {:?}",
                case.locale, case.mode, case.spec, case.expected
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
