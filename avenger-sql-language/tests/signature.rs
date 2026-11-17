//! Signature help: the forms of a call's function, and the form and the parameter at a cursor.

mod common;

use avenger_sql_language::{SignatureHelp, SignatureInfo};
use common::at_cursor;

fn help(text: &str) -> Option<SignatureHelp> {
    let (document, offset) = at_cursor(text);
    document.signature_help(offset)
}

/// The text of the active parameter of `signature`.
fn active(signature: &SignatureInfo) -> Option<&str> {
    let parameter = &signature.parameters[signature.active_parameter?];
    Some(&signature.label[parameter.range.clone()])
}

#[test]
fn the_active_parameter_follows_the_cursor() {
    let at = |text: &str| {
        let help = help(text).unwrap();
        let signature = &help.signatures[help.active_signature];
        active(signature).map(str::to_string)
    };
    assert_eq!(
        at("FROM penguins |> SELECT SUBSTR($0species, 1)").as_deref(),
        Some("value: STRING")
    );
    assert_eq!(
        at("FROM penguins |> SELECT SUBSTR(species, $0").as_deref(),
        Some("position: INT64")
    );
    assert_eq!(
        at("FROM penguins |> SELECT SUBSTR(species, 1, 3$0)").as_deref(),
        Some("length: INT64")
    );
    let help = help("FROM penguins |> SELECT SUBSTR(species, 1$0)").unwrap();
    // Each parameter's range in its label holds the parameter.
    for signature in &help.signatures {
        let names: Vec<&str> = (signature.parameters.iter())
            .map(|parameter| {
                signature.label[parameter.range.clone()]
                    .split(':')
                    .next()
                    .unwrap()
            })
            .collect();
        assert_eq!(
            names,
            ["value", "position", "length"],
            "{}",
            signature.label
        );
        assert!(signature.documentation.is_some());
    }
}

#[test]
fn the_active_form_is_the_one_that_the_call_resolved() {
    let active = |text: &str| {
        let help = help(text).unwrap();
        assert_eq!(help.signatures.len(), 2);
        help.active_signature
    };
    // `AVG`'s first form takes an INT64, and its second a FLOAT64.
    assert_eq!(
        active("FROM penguins |> AGGREGATE AVG(body_mass_g$0) AS a"),
        0
    );
    assert_eq!(
        active("FROM penguins |> AGGREGATE AVG(bill_length_mm$0) AS a"),
        1
    );
}

#[test]
fn a_named_argument_of_a_mark_is_the_active_parameter() {
    let help = help("FROM penguins |> AGGREGATE bar(x => species, y => $0) AS chart").unwrap();
    let signature = &help.signatures[help.active_signature];
    assert!(signature.label.starts_with("bar("), "{}", signature.label);
    assert_eq!(active(signature), Some("y: ANY"), "{}", signature.label);
}

#[test]
fn a_temporary_function_and_a_table_function_have_signatures() {
    let text = "CREATE TEMP FUNCTION twice(v INT64) AS (v * 2);\nFROM penguins |> SELECT twice($0)";
    let twice = help(text).unwrap();
    assert_eq!(twice.signatures.len(), 1);
    assert_eq!(twice.signatures[0].label, "twice(v INT64)");
    assert_eq!(active(&twice.signatures[0]), Some("v INT64"));
    let impute = help("FROM penguins |> CALL impute($0)").unwrap();
    assert!(
        impute.signatures[0].label.starts_with("impute("),
        "{:?}",
        impute.signatures
    );
}

#[test]
fn outside_an_argument_list_there_is_no_help() {
    assert_eq!(help("FROM penguins |> WHERE $0species = 'Adelie'"), None);
    assert_eq!(help("FROM penguins |> SELECT SUBSTR(species, 1)$0"), None);
}

#[test]
fn a_trailing_comma_keeps_the_call_and_names_no_parameter_past_the_last() {
    let help = help("FROM penguins |> SELECT SUBSTR(species, 1, 3,$0)").unwrap();
    let signature = &help.signatures[help.active_signature];
    assert!(
        signature.label.starts_with("SUBSTR("),
        "{}",
        signature.label
    );
    assert_eq!(active(signature), None);
}

#[test]
fn the_charts_of_a_layer_repeat_and_its_options_go_by_name() {
    let at = |text: &str| {
        let help = help(text).unwrap();
        let signature = &help.signatures[help.active_signature];
        active(signature).map(str::to_string)
    };
    let charts = "FROM (FROM penguins |> AGGREGATE bar(x => species) AS a, bar(x => island) AS b)";
    for text in [
        format!("{charts} |> SELECT layer($0a, b) AS c"),
        format!("{charts} |> SELECT layer(a, b, $0) AS c"),
    ] {
        assert_eq!(at(&text).as_deref(), Some("chart: ANY"), "{text}");
    }
    assert_eq!(
        at(&format!("{charts} |> SELECT layer(a, b, id => $0) AS c")).as_deref(),
        Some("id => STRING")
    );
}

#[test]
fn a_parameter_that_takes_constructors_documents_its_forms() {
    // The documentation of the active parameter at the cursor.
    let doc = |text: &str| {
        let help = help(text).unwrap();
        let signature = &help.signatures[help.active_signature];
        let parameter = &signature.parameters[signature.active_parameter.unwrap()];
        parameter.documentation.clone().unwrap()
    };
    let pin = doc("FROM penguins |> SELECT scale(pin => $0) AS f");
    assert!(
        pin.ends_with(
            "```googlesql\npin => domain_pin(value: ANY, position: 'low' | 'mid' | 'high')\n```"
        ),
        "{pin}"
    );
    let scale = doc("FROM penguins |> SELECT scaled(1, scale => $0) AS s");
    assert!(scale.contains("\n- Its type: `band_scale(...)`"), "{scale}");
}
