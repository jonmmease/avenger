use std::{fs, path::Path};

use avenger_lang_core::{SourceFile, SourceId, SourceOrigin, syntax::parse_file};

const CURSOR: &str = "⟦cursor⟧";

fn fixture(relative: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative),
    )
    .unwrap_or_else(|error| panic!("read fixture {relative}: {error}"))
}

#[test]
fn cursor_fixtures_have_one_valid_byte_marker() {
    for relative in ["expression.cursor.avenger", "sql_from_first.cursor.avenger"] {
        let text = fixture(relative);
        assert_eq!(
            text.matches(CURSOR).count(),
            1,
            "{relative} must contain exactly one cursor marker"
        );
        let offset = text.find(CURSOR).expect("cursor marker");
        let source = text.replacen(CURSOR, "", 1);
        assert!(source.is_char_boundary(offset));
    }
}

#[test]
fn frozen_corpus_covers_the_required_editing_shapes() {
    for relative in [
        "valid_chart.avenger",
        "incomplete_chart.avenger",
        "unterminated_string.avenger",
        "unicode.avenger",
        "expression.cursor.avenger",
        "sql_from_first.cursor.avenger",
        "import_chart.avenger",
        "schema.data.avenger",
        "multi_file/chart.avenger",
        "multi_file/definitions.avenger",
    ] {
        assert!(!fixture(relative).is_empty(), "empty fixture: {relative}");
    }
}

#[test]
fn frozen_valid_sources_remain_accepted_by_the_strict_parser() {
    for relative in ["valid_chart.avenger", "unicode.avenger"] {
        let text = fixture(relative);
        let source = SourceFile::new(
            SourceId::new(0),
            SourceOrigin::Memory(relative.to_owned()),
            text,
        );
        parse_file(&source).unwrap_or_else(|error| panic!("strict parse {relative}: {error}"));
    }
}
