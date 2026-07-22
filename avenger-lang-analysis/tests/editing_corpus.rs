use std::{fs, path::Path, time::Instant};

use avenger_lang_analysis::{DocumentSnapshot, SourceRevision, SyntaxContextKind, analyze_syntax};
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

#[test]
fn incomplete_source_still_produces_symbols_and_context() {
    let text = fixture("incomplete_chart.avenger");
    let snapshot = DocumentSnapshot::new(
        SourceOrigin::Memory("incomplete_chart.avenger".into()),
        SourceRevision::new("1"),
        text.clone(),
    );
    let analysis = analyze_syntax(&snapshot);
    assert!(!analysis.diagnostics.is_empty());
    assert!(analysis.symbols.iter().any(|symbol| symbol.name == "chart"));
    let offset = text.find("default:").expect("default property") + "default:".len();
    assert!(matches!(
        analysis.context_at(offset).kind,
        SyntaxContextKind::Property | SyntaxContextKind::Expression
    ));
}

#[test]
fn sql_cursor_is_classified_as_query_context() {
    let marked = fixture("sql_from_first.cursor.avenger");
    let offset = marked.find(CURSOR).expect("cursor");
    let text = marked.replacen(CURSOR, "", 1);
    let snapshot = DocumentSnapshot::new(
        SourceOrigin::Memory("sql_from_first.cursor.avenger".into()),
        SourceRevision::new("1"),
        text,
    );
    let analysis = analyze_syntax(&snapshot);
    assert_eq!(analysis.context_at(offset).kind, SyntaxContextKind::Query);
}

#[test]
#[ignore = "manual timing baseline; not a CI threshold"]
fn record_syntax_timing_baseline() {
    let snapshot = DocumentSnapshot::new(
        SourceOrigin::Memory("valid_chart.avenger".into()),
        SourceRevision::new("timing"),
        fixture("valid_chart.avenger"),
    );
    let start = Instant::now();
    std::hint::black_box(analyze_syntax(&snapshot));
    let cold = start.elapsed();
    let mut samples = Vec::with_capacity(500);
    for _ in 0..500 {
        let start = Instant::now();
        std::hint::black_box(analyze_syntax(&snapshot));
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    eprintln!(
        "syntax cold={cold:?} warm_p50={:?} warm_p95={:?}",
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100]
    );
}
