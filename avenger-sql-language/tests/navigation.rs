//! Go to definition, references, and rename.

mod common;

use avenger_sql_language::{Document, TextEdit};
use common::{at, document, find};

/// The text of the target of the definition of the `n`th `needle`, and its offset.
fn target<'a>(document: &'a Document, needle: &str, n: usize) -> Option<(&'a str, usize)> {
    let link = document.definition(find(document, needle, n))?;
    assert_eq!(at(document, link.origin), needle);
    Some((at(document, link.target), usize::from(link.target.start())))
}

#[test]
fn a_name_goes_to_its_definition() {
    let text = "DECLARE lo INT64 DEFAULT 1;
CREATE TEMP TABLE big AS FROM penguins |> WHERE body_mass_g > 4000;
CREATE TEMP FUNCTION twice(v INT64) AS (v * 2);
FROM big AS p
|> WITH heavy AS (FROM penguins |> WHERE body_mass_g > 5000)
|> EXTEND twice(p.body_mass_g) AS doubled
|> WHERE doubled > lo AND species IN (FROM heavy |> SELECT species)
|> SELECT species, UPPER(island) AS upper_island";
    let document = document(text);
    let offset = |needle: &str, n: usize| usize::from(find(&document, needle, n));
    assert_eq!(
        target(&document, "doubled", 1),
        Some(("doubled", offset("doubled", 0)))
    );
    let p = document.definition(find(&document, "p.body", 0)).unwrap();
    assert_eq!(
        (at(&document, p.origin), at(&document, p.target)),
        ("p", "p")
    );
    assert_eq!(usize::from(p.target.start()), offset("AS p", 0) + 3);
    assert_eq!(target(&document, "lo", 1), Some(("lo", offset("lo", 0))));
    assert_eq!(target(&document, "big", 1), Some(("big", offset("big", 0))));
    assert_eq!(
        target(&document, "heavy", 1),
        Some(("heavy", offset("heavy", 0)))
    );
    assert_eq!(
        target(&document, "twice", 1),
        Some(("twice", offset("twice", 0)))
    );
    // A column of a table goes to the table that holds it, through the temporary table `big`.
    assert_eq!(
        target(&document, "species", 2),
        Some(("penguins", offset("penguins", 0)))
    );
    // A function of the library has no definition in the text.
    assert_eq!(document.definition(find(&document, "UPPER", 0)), None);
}

#[test]
fn references_of_a_column_alias_stop_at_a_new_column_of_its_name() {
    let text = "FROM penguins
|> EXTEND body_mass_g / 1000 AS kg
|> WHERE kg > 3
|> AGGREGATE AVG(kg) AS kg GROUP BY species
|> WHERE kg > 4";
    let document = document(text);
    let references = |n: usize| -> Vec<(usize, bool)> {
        (document.references(find(&document, "kg", n)).iter())
            .map(|reference| (usize::from(reference.range.start()), reference.definition))
            .collect()
    };
    let kg = |n: usize| usize::from(find(&document, "kg", n));
    let first = vec![(kg(0), true), (kg(1), false), (kg(2), false)];
    // From the alias and from a use.
    assert_eq!(references(0), first);
    assert_eq!(references(2), first);
    // The aggregate's `kg` is another column.
    assert_eq!(references(4), vec![(kg(3), true), (kg(4), false)]);
}

#[test]
fn references_of_a_column_of_a_table_leave_out_its_other_columns() {
    let text = "FROM penguins |> WHERE species = 'Adelie' |> SELECT species, island";
    let document = document(text);
    let references: Vec<(&str, bool)> = (document.references(find(&document, "species", 1)).iter())
        .map(|reference| (at(&document, reference.range), reference.definition))
        .collect();
    assert_eq!(references, [("species", false), ("species", false)]);
}

/// `document`'s text after `edits`.
fn edited(document: &Document, edits: &[TextEdit]) -> String {
    let mut text = document.text().to_string();
    for edit in edits.iter().rev() {
        text.replace_range(
            std::ops::Range::<usize>::from(edit.range),
            &edit.replacement,
        );
    }
    text
}

#[test]
fn a_rename_edits_the_definition_and_its_uses() {
    let text = "DECLARE lo INT64 DEFAULT 1;
FROM penguins AS p
|> EXTEND p.body_mass_g / 1000 AS kg
|> WHERE kg > lo
|> SELECT p.species, kg";
    let document = document(text);
    let rename = |needle: &str, n: usize, name: &str| {
        edited(
            &document,
            &document.rename(find(&document, needle, n), name).unwrap(),
        )
    };
    let p = find(&document, "AS p", 0) + avenger_sql_language::Offset::from(3);
    assert_eq!(
        rename("lo", 1, "low"),
        text.replace("lo INT64", "low INT64")
            .replace("> lo", "> low")
    );
    assert_eq!(
        edited(&document, &document.rename(p, "pg").unwrap()),
        text.replace("AS p\n", "AS pg\n").replace("p.", "pg.")
    );
    assert_eq!(
        rename("kg", 0, "kilograms"),
        text.replace("kg", "kilograms")
    );
    // A reserved word goes in backticks.
    assert_eq!(rename("kg", 1, "order"), text.replace("kg", "`order`"));
}

#[test]
fn a_rename_that_changes_what_a_name_means_fails() {
    let text = "FROM penguins |> EXTEND body_mass_g AS mass |> SELECT mass, species";
    let document = document(text);
    let error = document
        .rename(find(&document, "mass", 1), "species")
        .unwrap_err();
    assert!(error.starts_with("The rename to `species`"), "{error}");
}

#[test]
fn a_column_of_a_table_has_no_rename() {
    let text = "FROM penguins |> WHERE species = 'Adelie' |> AGGREGATE COUNT(*) AS n";
    let document = document(text);
    assert_eq!(
        document.prepare_rename(find(&document, "species", 0)),
        Err("A column of a table has no rename".to_string())
    );
    assert_eq!(
        document.prepare_rename(find(&document, "penguins", 0)),
        Err("A table of the catalog has no rename".to_string())
    );
    assert_eq!(
        document.prepare_rename(find(&document, "COUNT", 0)),
        Err("A function of the library or of a module has no rename".to_string())
    );
    let n = find(&document, "AS n", 0) + avenger_sql_language::Offset::from(3);
    assert_eq!(
        document.prepare_rename(n).map(|range| at(&document, range)),
        Ok("n")
    );
}
