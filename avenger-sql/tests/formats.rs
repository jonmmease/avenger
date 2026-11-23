//! The format patterns of charts: the check of a pattern at its argument, by the type of the
//! value that it formats where the analysis knows it, and otherwise as a pattern of numbers or of
//! dates, and the formats of GoogleSQL's functions, which take no check.

mod pass;

/// The messages of the errors of the script `text`, each with the text of its range.
fn errors(text: &str) -> Vec<(String, String)> {
    let analysis = pass::analysis(text);
    (analysis.diagnostics.iter())
        .map(|diagnostic| {
            (
                text[diagnostic.range].to_string(),
                diagnostic.message.clone(),
            )
        })
        .collect()
}

/// A chart of the penguins, with the date `day`, whose items are `items`.
fn chart(items: &str) -> String {
    format!("FROM penguins |> EXTEND DATE '2024-01-01' AS day |> AGGREGATE {items}")
}

#[test]
fn a_pattern_of_a_value_of_a_known_type_takes_the_check_of_its_type() {
    // The text of a text mark, a number.
    let text = chart(
        "mark_text(x => species, y => body_mass_g, text => body_mass_g, format => '%Y') AS a",
    );
    let errors = errors(&text);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].0, "'%Y'");
    assert!(
        errors[0]
            .1
            .starts_with("`%Y` is not a conversion of a number"),
        "{errors:?}"
    );
    // A tooltip's field of a date column, and an element of a part that a date lacks.
    let field = |format: &str| {
        chart(&format!(
            "mark_bar(x => species, y => body_mass_g, tools => [tooltip(fields => [field(column => 'day', format => '{format}')])]) AS b"
        ))
    };
    let errors = self::errors(&field("%q"));
    assert_eq!(
        errors,
        [(
            "'%q'".to_string(),
            "`%q` is not an element of GoogleSQL's formats".to_string()
        )]
    );
    let errors = self::errors(&field("%H"));
    assert!(
        errors[0]
            .1
            .contains("needs a part that the value does not have"),
        "{errors:?}"
    );
    // Valid patterns of each type, and a pattern of a text of strings, which shows as text.
    for valid in [
        "mark_text(x => species, y => body_mass_g, text => body_mass_g, format => '%.1f') AS a",
        "mark_bar(x => species, y => body_mass_g, tools => [tooltip(fields => [field(column => 'day', format => '%Y-%m')])]) AS b",
        "mark_text(x => species, y => body_mass_g, text => species, format => '%Y') AS c",
    ] {
        assert_eq!(self::errors(&chart(valid)), Vec::new(), "{valid}");
    }
}

#[test]
fn a_pattern_of_a_value_of_an_unknown_type_is_one_of_numbers_or_of_dates() {
    let labels = |format: &str| {
        chart(&format!(
            "mark_bar(x => x(species, axis => axis(labels => labels(format => '{format}'))), y => body_mass_g) AS c"
        ))
    };
    assert_eq!(
        errors(&labels("%q")),
        [(
            "'%q'".to_string(),
            "`%q` is a format of neither numbers nor dates".to_string()
        )]
    );
    assert_eq!(errors(&labels("%d")), Vec::new());
    assert_eq!(errors(&labels("%Y")), Vec::new());
}

#[test]
fn the_formats_of_googlesql_s_functions_take_no_check() {
    let text = "FROM penguins |> SELECT FORMAT_DATE('%q', DATE '2024-01-01') AS d";
    assert_eq!(errors(text), Vec::new());
}
