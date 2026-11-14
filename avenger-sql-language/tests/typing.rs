//! Every request on the text of a script as a user types it, which has errors at most offsets.

mod common;

use avenger_sql_language::{Document, Offset};
use common::document;

const SCRIPTS: &[&str] = &[
    "DECLARE lo INT64 DEFAULT 1;
CREATE TEMP FUNCTION twice(v INT64) AS (v * 2);
CREATE TEMP TABLE big AS FROM penguins |> WHERE body_mass_g > 4000;
-- penguins by species
FROM big AS p
|> WITH heavy AS (FROM penguins |> SELECT species, body_mass_g)
|> EXTEND twice(p.body_mass_g) AS doubled, SUBSTR(species, 1, 3) AS short
|> WHERE doubled > lo AND species IN (FROM heavy |> SELECT species) AND island != 'Bïscoe'
|> EXTEND stack(body_mass_g, offset => 'normalize') OVER (PARTITION BY island ORDER BY species) AS share
|> AGGREGATE bar(x => species, y => share.end, orient => 'vertical') AS chart",
    "FROM islands |> WHERE location.lat > 0 |> SELECT name, `order` |> CALL impute(TABLE x)",
];

/// Every request at `offset` of `document`.
fn every_request(document: &Document, offset: Offset) {
    document.selection_ranges(offset);
    document.definition(offset);
    document.references(offset);
    let _ = document.prepare_rename(offset);
    document.hover(offset);
    document.completions(offset);
    document.signature_help(offset);
}

#[test]
fn every_request_answers_as_a_script_is_typed() {
    for script in SCRIPTS {
        let boundaries: Vec<usize> = (0..=script.len())
            .filter(|&i| script.is_char_boundary(i))
            .collect();
        // At the end of each prefix, where a user types.
        for &end in &boundaries {
            let document = document(&script[..end]);
            document.diagnostics();
            document.highlights();
            document.symbols();
            document.folding_ranges();
            every_request(&document, Offset::from(end as u32));
        }
        // At every offset of the whole script, and past its end.
        let document = document(script);
        for &offset in &boundaries {
            every_request(&document, Offset::from(offset as u32));
            let _ = document.rename(Offset::from(offset as u32), "renamed");
        }
        every_request(&document, Offset::from(script.len() as u32 + 10));
    }
}
