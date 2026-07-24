use std::{collections::BTreeMap, fs, path::Path, sync::Arc, time::Instant};

use avenger_lang_analysis::{
    AnalysisCancellation, AnalysisGeneration, CompletionOptions, DocumentRequest, DocumentSnapshot,
    DocumentSymbol, PositionRequest, SnapshotSourceLoader, SourceRevision, SyntaxContextKind,
    WorkspaceAnalysis, analyze_syntax,
};
use avenger_lang_compiler::Compiler;
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

fn compiler_fixture_files() -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
    fn collect(directory: &Path, output: &mut Vec<std::path::PathBuf>) {
        let mut entries = fs::read_dir(directory)
            .unwrap_or_else(|error| {
                panic!("read fixture directory {}: {error}", directory.display())
            })
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                collect(&path, output);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "avenger")
            {
                output.push(path);
            }
        }
    }

    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../avenger-lang-compiler/tests/fixtures");
    let root = fs::canonicalize(root).unwrap();
    let mut files = Vec::new();
    collect(&root, &mut files);
    (root, files)
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
fn compiler_fixture_corpus_builds_tolerant_indexes_without_panics() {
    let (project_root, files) = compiler_fixture_files();
    assert_eq!(
        files.len(),
        62,
        "fixture additions should update this corpus gate"
    );

    let syntax = files
        .iter()
        .map(|path| {
            let origin = SourceOrigin::File(path.clone());
            let text = fs::read_to_string(path).unwrap();
            let snapshot =
                DocumentSnapshot::new(origin.clone(), SourceRevision::from_text(&text), text);
            (origin, analyze_syntax(&snapshot))
        })
        .collect::<BTreeMap<_, _>>();
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .build()
        .unwrap();
    let analysis = WorkspaceAnalysis::syntax_only(
        AnalysisGeneration::new(1),
        project_root,
        syntax.keys().cloned().collect(),
        syntax,
        compiler.language_host().authoring_schema().clone(),
    );
    let cancellation = AnalysisCancellation::default();
    assert_eq!(analysis.semantic_index.documents.len(), 62);
    for (origin, syntax) in &analysis.syntax {
        assert!(analysis.semantic_index.documents.contains_key(origin));
        let request = DocumentRequest {
            source: origin.clone(),
            source_revision: syntax.revision.clone(),
        };
        let tokens = analysis.semantic_tokens(&request, &cancellation).unwrap();
        for token in tokens.tokens {
            let range = token.span.range.as_range();
            assert!(range.start <= range.end);
            assert!(range.end <= syntax.parsed.tokens.text().len());
        }
        for offset in [0, syntax.parsed.tokens.text().len()] {
            std::hint::black_box(syntax.context_at(offset));
        }
    }
}

#[test]
fn bounded_mutation_corpus_never_panics_or_produces_invalid_spans() {
    fn verify(path: &Path, text: String, offset: usize) {
        let analysis = analyze_syntax(&DocumentSnapshot::new(
            SourceOrigin::File(path.to_path_buf()),
            SourceRevision::from_text(&text),
            text.clone(),
        ));
        for diagnostic in &analysis.diagnostics {
            assert!(diagnostic.primary.span.range.start <= diagnostic.primary.span.range.end);
            assert!(diagnostic.primary.span.range.end <= text.len());
        }
        let mut symbols = analysis.symbols.iter().collect::<Vec<_>>();
        while let Some(symbol) = symbols.pop() {
            assert!(symbol.span.range.start <= symbol.span.range.end);
            assert!(symbol.span.range.end <= text.len());
            symbols.extend(&symbol.children);
        }
        std::hint::black_box(analysis.context_at(offset.min(text.len())));
    }

    let (_, files) = compiler_fixture_files();
    let insertions = ["{", "'", "/*", "$", "😀"];
    for path in files {
        let original = fs::read_to_string(&path).unwrap();
        let boundaries = original
            .char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(original.len()))
            .collect::<Vec<_>>();
        for (ordinal, insertion) in insertions.iter().enumerate() {
            let index = ordinal.wrapping_mul(2_654_435_761usize) % boundaries.len();
            let offset = boundaries[index];
            let mut inserted = original.clone();
            inserted.insert_str(offset, insertion);
            verify(&path, inserted, offset);
            verify(&path, original[..offset].to_owned(), offset);
        }
    }
}

#[test]
fn document_symbols_match_the_zed_outline_baseline() {
    fn value(symbol: &DocumentSymbol) -> serde_json::Value {
        serde_json::json!({
            "name": symbol.name,
            "detail": symbol.detail,
            "children": symbol.children.iter().map(value).collect::<Vec<_>>()
        })
    }

    let baseline: serde_json::Value =
        serde_json::from_str(&fixture("document-symbols-baseline.json")).unwrap();
    let source = fixture(baseline["fixture"].as_str().unwrap());
    let snapshot = DocumentSnapshot::new(
        SourceOrigin::Memory("symbol-baseline".into()),
        SourceRevision::from_text(&source),
        source.clone(),
    );
    let analysis = analyze_syntax(&snapshot);
    let actual = analysis.symbols.iter().map(value).collect::<Vec<_>>();
    assert_eq!(serde_json::Value::Array(actual), baseline["symbols"]);
    for symbol in analysis
        .symbols
        .iter()
        .flat_map(|symbol| std::iter::once(symbol).chain(symbol.children.iter()))
    {
        let selected = &source[symbol.selection_span.range.as_range()];
        assert_eq!(selected, symbol.name);
    }
}

#[test]
fn structural_completion_matches_the_frozen_baseline() {
    let baseline: serde_json::Value =
        serde_json::from_str(&fixture("completion-baselines.json")).unwrap();
    let case = baseline["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["fixture"] == "expression.cursor.avenger")
        .unwrap();
    let marked = fixture(case["fixture"].as_str().unwrap());
    let offset = marked.find(CURSOR).unwrap();
    let text = marked.replacen(CURSOR, "", 1);
    let origin = SourceOrigin::Memory("expression.cursor.avenger".into());
    let revision = SourceRevision::from_text(&text);
    let syntax = analyze_syntax(&DocumentSnapshot::new(
        origin.clone(),
        revision.clone(),
        text,
    ));
    let compiler = Compiler::builder()
        .project_root(env!("CARGO_MANIFEST_DIR"))
        .build()
        .unwrap();
    let analysis = WorkspaceAnalysis::syntax_only(
        AnalysisGeneration::new(1),
        Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf(),
        Vec::new(),
        BTreeMap::from([(origin.clone(), syntax)]),
        compiler.language_host().authoring_schema().clone(),
    );
    let result = analysis
        .complete(
            &PositionRequest {
                source: origin,
                byte_offset: offset,
                source_revision: revision,
            },
            CompletionOptions::default(),
            &AnalysisCancellation::default(),
        )
        .unwrap();
    let labels = result
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();
    let ordered_top = case["ordered_top"].as_array().unwrap();
    for (ordinal, expected) in ordered_top.iter().enumerate() {
        assert_eq!(labels.get(ordinal).copied(), expected.as_str());
    }
    for expected in case["must_include"].as_array().unwrap() {
        assert!(labels.contains(&expected.as_str().unwrap()));
    }
    for excluded in case["must_exclude"].as_array().unwrap() {
        assert!(!labels.contains(&excluded.as_str().unwrap()));
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
    let offset = text.find("value:").expect("value property") + "value:".len();
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
