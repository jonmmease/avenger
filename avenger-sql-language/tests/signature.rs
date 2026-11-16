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
