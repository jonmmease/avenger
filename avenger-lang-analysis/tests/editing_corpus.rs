use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::Arc,
    time::Instant,
};

use avenger_lang_analysis::{
    AnalysisCancellation, AnalysisGeneration, CompletionOptions, DocumentRequest, DocumentSnapshot,
    DocumentSymbol, PositionRequest, SnapshotSourceLoader, SourceRevision, SyntaxContextKind,
    WorkspaceAnalysis, analyze_syntax,
};
use avenger_lang_compiler::Compiler;
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, SourceFile, SourceId,
    SourceLoader, SourceOrigin,
    syntax::{SqlIslandSite, TolerantSyntaxNodeKind, parse_file},
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

fn completion_labels(marked: &str) -> Vec<String> {
    let offset = marked.find(CURSOR).expect("cursor marker");
    let text = marked.replacen(CURSOR, "", 1);
    let origin = SourceOrigin::Memory("channel-completion.avenger".into());
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
    analysis
        .complete(
            &PositionRequest {
                source: origin,
                byte_offset: offset,
                source_revision: revision,
            },
            CompletionOptions::default(),
            &AnalysisCancellation::default(),
        )
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.label)
        .collect()
}

#[test]
fn adjustment_completion_uses_the_registered_inventory_and_schema() {
    let kinds = completion_labels("avenger 1; chart cartesian { mark symbol { adjust ⟦cursor⟧ } }");
    for expected in ["expr", "nudge", "jitter", "dodge"] {
        assert!(kinds.contains(&expected.to_string()), "{kinds:?}");
    }

    let properties = completion_labels(
        "avenger 1; chart cartesian { mark symbol { adjust jitter as jittered { ⟦cursor⟧ } } }",
    );
    for expected in ["apply", "axis", "width_px", "seed"] {
        assert!(properties.contains(&expected.to_string()), "{properties:?}");
    }
    assert!(!properties.contains(&"dx".to_string()), "{properties:?}");
}

#[test]
fn structural_binding_completion_is_shape_exact_and_has_no_global_fallback() {
    let source = |body: &str| {
        format!(
            "avenger 1; chart cartesian as chart {{\n  param 1 as scalar;\n  store as rows {{ field int64 id; }}\n  selection as picked {{ combine: union; empty: none; }}\n  {body}\n}}"
        )
    };

    let root = completion_labels(&source("$⟦cursor⟧"));
    assert!(
        root.is_empty(),
        "unknown structural position leaked bindings: {root:?}"
    );

    let table = completion_labels(&source(
        "mark symbol as points { data: $⟦cursor⟧; x: encoded 1; y: encoded 1; }",
    ));
    assert_eq!(table, ["$rows"]);

    let selection = completion_labels(&source(
        "tool box_selection as brush { selection: ⟦cursor⟧; }",
    ));
    assert_eq!(selection, ["picked"]);

    let scalar = completion_labels(&source(
        "widget checkbox as toggle { checked_param: $⟦cursor⟧; }",
    ));
    assert_eq!(scalar, ["$scalar"]);
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
        "schema.avenger",
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
        64,
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
    assert_eq!(analysis.semantic_index.documents.len(), 64);
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
fn every_compiler_fixture_sql_island_cursor_completes_with_valid_spans() {
    let (project_root, files) = compiler_fixture_files();
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
    let requests = analysis
        .syntax
        .iter()
        .flat_map(|(origin, syntax)| {
            let text = syntax.parsed.tokens.text();
            syntax.parsed.nodes.iter().filter_map(move |node| {
                let TolerantSyntaxNodeKind::SqlIsland { site, .. } = &node.kind else {
                    return None;
                };
                let offsets = text[node.span.range.as_range()]
                    .char_indices()
                    .map(|(offset, _)| node.span.range.start + offset)
                    .chain(std::iter::once(node.span.range.end))
                    .collect::<Vec<_>>();
                Some((origin.clone(), syntax.revision.clone(), *site, offsets))
            })
        })
        .collect::<Vec<_>>();
    let cancellation = AnalysisCancellation::default();
    let mut sites = BTreeSet::new();
    let mut cursors = 0usize;
    for (origin, revision, site, offsets) in requests {
        sites.insert(site);
        let text = analysis.syntax[&origin].parsed.tokens.text();
        for byte_offset in offsets {
            let result = analysis
                .complete(
                    &PositionRequest {
                        source: origin.clone(),
                        byte_offset,
                        source_revision: revision.clone(),
                    },
                    CompletionOptions::default(),
                    &cancellation,
                )
                .unwrap_or_else(|error| {
                    panic!("completion failed for {origin} {site:?} at {byte_offset}: {error}")
                });
            for item in result.items {
                assert!(
                    item.replacement.range.start <= item.replacement.range.end
                        && item.replacement.range.end <= text.len()
                        && text.is_char_boundary(item.replacement.range.start)
                        && text.is_char_boundary(item.replacement.range.end),
                    "invalid completion span for {origin} {site:?} at {byte_offset}: {:?}",
                    item.replacement
                );
            }
            cursors += 1;
        }
    }
    assert_eq!(sites, SqlIslandSite::ALL.into_iter().collect());
    assert!(
        cursors >= 5_000,
        "fixture sweep unexpectedly shrank: {cursors}"
    );
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
    let expected = case["expected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(labels, expected, "extra candidates are baseline failures");
}

#[test]
fn every_structural_cursor_state_matches_an_exact_ordered_allow_list() {
    let baseline: serde_json::Value =
        serde_json::from_str(&fixture("completion-baselines.json")).unwrap();
    let compiler = Compiler::builder()
        .project_root(env!("CARGO_MANIFEST_DIR"))
        .build()
        .unwrap();

    for case in baseline["structural_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let marked = case["source"].as_str().unwrap();
        assert_eq!(marked.matches('|').count(), 1, "cursor marker for {id}");
        let offset = marked.find('|').unwrap();
        let text = marked.replacen('|', "", 1);
        let origin = SourceOrigin::Memory(format!("structural-{id}.avenger"));
        let revision = SourceRevision::from_text(&text);
        let syntax = analyze_syntax(&DocumentSnapshot::new(
            origin.clone(),
            revision.clone(),
            text.clone(),
        ));
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
        let actual = result
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        let expected = case["expected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "exact structural menu for {id}");

        let identities = result
            .items
            .iter()
            .map(|item| {
                assert!(!item.semantic_identity.is_empty(), "identity for {id}");
                assert!(
                    item.replacement.range.end <= text.len()
                        && text.is_char_boundary(item.replacement.range.start)
                        && text.is_char_boundary(item.replacement.range.end),
                    "replacement boundary for {id}: {:?}",
                    item.replacement
                );
                format!(
                    "{}:{:?}:{}:{:?}",
                    item.semantic_identity,
                    item.replacement,
                    item.insert_text,
                    item.insert_text_format
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            identities.len(),
            result.items.len(),
            "duplicate semantic/edit identity for {id}"
        );
    }
}

#[test]
fn channel_domain_contribution_property_and_values_complete() {
    let properties = completion_labels(
        r#"avenger 1;
chart cartesian as chart {
  mark symbol {
    x: encoded "x" {
      ⟦cursor⟧
    }
    y: encoded "y";
  }
}"#,
    );
    assert!(
        properties
            .iter()
            .any(|label| label == "domain_contribution")
    );

    let values = completion_labels(
        r#"avenger 1;
chart cartesian as chart {
  mark symbol {
    x: encoded "x" {
      domain_contribution: ⟦cursor⟧
    }
    y: encoded "y";
  }
}"#,
    );
    assert!(values.iter().any(|label| label == "infer"));
    assert!(values.iter().any(|label| label == "exclude"));
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
    let offset = text.find("sharing:").expect("sharing property") + "sharing:".len();
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
