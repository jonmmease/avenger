use std::{collections::BTreeMap, fs, path::Path, sync::Arc, time::Instant};

use avenger_lang_analysis::{
    DocumentSnapshot, SnapshotSourceLoader, SourceRevision, SyntaxContextKind, analyze_syntax,
};
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, SourceFile, SourceId,
    SourceLoader, SourceOrigin, syntax::parse_file,
};

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

#[tokio::test]
async fn snapshot_loader_overlays_every_origin_kind_and_falls_back() {
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
    let file = SourceOrigin::File(project_root.join("open.avenger"));
    let memory = SourceOrigin::Memory("untitled".into());
    let std = SourceOrigin::Std("marks.avenger".into());
    let http = SourceOrigin::Http("https://example.com/chart.avenger".into());
    let fallback_origin = SourceOrigin::Memory("fallback".into());
    let fallback = InMemorySourceLoader::default().with_source(LoadedSource::new(
        fallback_origin.clone(),
        "fallback text",
        ContentVersion::new("disk-1"),
    ));
    let overlay = [file.clone(), memory.clone(), std.clone(), http.clone()]
        .into_iter()
        .map(|origin| {
            (
                origin.clone(),
                DocumentSnapshot::new(origin, SourceRevision::new("open-1"), "open text"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let loader = SnapshotSourceLoader::new(overlay, Arc::new(fallback.clone()));
    let capabilities = ImportCapabilities {
        project_root,
        allow_memory: true,
        allow_std: true,
        allow_filesystem: true,
        allow_http: true,
    };
    for origin in [file.clone(), memory, std, http] {
        let loaded = loader.load(&origin, &capabilities).await.unwrap();
        assert_eq!(&*loaded.text, "open text");
        assert_eq!(loaded.version.as_str(), "open-1");
    }
    let loaded = loader.load(&fallback_origin, &capabilities).await.unwrap();
    assert_eq!(&*loaded.text, "fallback text");
    fallback.insert(LoadedSource::new(
        fallback_origin.clone(),
        "updated disk text",
        ContentVersion::new("disk-2"),
    ));
    fallback.insert(LoadedSource::new(
        file.clone(),
        "watcher must not replace open text",
        ContentVersion::new("disk-file-2"),
    ));
    let closed = loader.load(&fallback_origin, &capabilities).await.unwrap();
    assert_eq!(&*closed.text, "updated disk text");
    let still_open = loader.load(&file, &capabilities).await.unwrap();
    assert_eq!(&*still_open.text, "open text");
}
