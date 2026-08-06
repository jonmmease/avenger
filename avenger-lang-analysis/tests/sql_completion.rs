use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use avenger_lang_analysis::{
    AnalysisCancellation, AnalysisGeneration, AnalysisService, CodeActionRequest,
    CompletionInvocation, CompletionKind, CompletionOptions, CompletionSemanticKind,
    CompletionTextFormat, DocumentRequest, DocumentSnapshot, IndexedValueKind, PositionRequest,
    SemanticTokenKind, SourceRevision, WorkspaceAnalysis, WorkspaceSnapshot, analyze_syntax,
};
use avenger_lang_compiler::Compiler;
use avenger_lang_core::{
    ByteSpan, InMemorySourceLoader, ModuleRoot, SourceFile, SourceId, SourceOrigin, SourceSpan,
    syntax::parse_file,
};

const CURSOR: &str = "⟦cursor⟧";

struct CountingAllocator;

static ALLOCATION_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static TEST_ALLOCATOR: CountingAllocator = CountingAllocator;

// SAFETY: every operation delegates to `System` with the original layout and
// pointer. The relaxed counters are observational test telemetry only.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: delegated with the original pointer and layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: delegated with the original pointer/layout and requested size.
        let pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !pointer.is_null() {
            ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        pointer
    }
}

fn allocation_snapshot() -> (u64, u64) {
    (
        ALLOCATION_COUNT.load(Ordering::Relaxed),
        ALLOCATED_BYTES.load(Ordering::Relaxed),
    )
}

struct Fixture {
    analysis: WorkspaceAnalysis,
    data: SourceOrigin,
    chart: SourceOrigin,
}

fn data_source(query: &str) -> String {
    format!(
        r#"avenger 1;

export schema tables as vega {{
  table inline as movies {{
    values: [
      {{ id: 1; title: 'A'; category: 'drama'; rating: 8.5; }},
      {{ id: 2; title: 'B'; category: 'comedy'; rating: 7.0; }}
    ];
  }}
  table inline as ratings {{
    values: [
      {{ id: 1; movie_id: 1; value: 4.0; }},
      {{ id: 2; movie_id: 2; value: 3.0; }}
    ];
  }}
  table sql as popular {{
    sql: {query};
  }}
}}
"#
    )
}

fn chart_source(expression: &str) -> String {
    format!(
        r#"avenger 1;

import {{ vega }} from 'data.avenger';

chart cartesian as chart {{
  data: {{ table: vega.movies; }}
  param 5.0 as minimum;
  param named_struct('label', 'base', 'weight', 2) as config;
  param store as selected {{
    field int64 id;
    field utf8 label;
  }}
  mark symbol as points {{
    x: encoded {expression};
    y: encoded "rating";
  }}
  on cursor_moved as inspect {{
    target: mark points;
    filter: true;
  }}
}}
"#
    )
}

fn chart_query_source(query: &str) -> String {
    chart_source("\"rating\"").replace(
        "  mark symbol as points {",
        &format!(
            "  transform sql as edited {{\n    query: {query};\n  }}\n  mark symbol as points {{"
        ),
    )
}

fn chart_handler_source(expression: &str) -> String {
    chart_source("\"rating\"").replace("    filter: true;", &format!("    filter: {expression};"))
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    // Keep the directory alive for the duration of the test process. The
    // compiler reads through the immutable in-memory snapshot, not disk.
    let _ = Box::leak(Box::new(directory));
    let data = SourceOrigin::File(project_root.join("data.avenger"));
    let chart = SourceOrigin::File(project_root.join("chart.avenger"));
    let data_text =
        data_source("SELECT \"id\", \"title\", \"category\", \"rating\" FROM vega.movies");
    let chart_text = chart_source("\"rating\"");
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .source_loader(Arc::new(InMemorySourceLoader::default()))
        .build()
        .unwrap();
    let profile = compiler
        .language_host()
        .registry()
        .profile_id()
        .as_str()
        .to_owned();
    let snapshot = WorkspaceSnapshot {
        generation: AnalysisGeneration::new(1),
        project_root,
        roots: vec![ModuleRoot::requested(chart.clone())],
        open_documents: BTreeMap::from([
            (
                data.clone(),
                DocumentSnapshot::new(
                    data.clone(),
                    SourceRevision::from_text(&data_text),
                    data_text,
                ),
            ),
            (
                chart.clone(),
                DocumentSnapshot::new(
                    chart.clone(),
                    SourceRevision::from_text(&chart_text),
                    chart_text,
                ),
            ),
        ]),
        known_disk_sources: vec![data.clone(), chart.clone()],
        native_registry_profile: profile,
    };
    let analysis = AnalysisService::new(compiler)
        .analyze_workspace(snapshot, &AnalysisCancellation::default())
        .await
        .unwrap();
    assert!(
        analysis.semantic_roots[&chart.canonical_uri()]
            .result
            .is_ok(),
        "fixture failed: {:?}",
        analysis.semantic_roots[&chart.canonical_uri()].result
    );
    Fixture {
        analysis,
        data,
        chart,
    }
}

async fn chart_only_fixture(text: &str) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    let _ = Box::leak(Box::new(directory));
    let chart = SourceOrigin::File(project_root.join("chart.avenger"));
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .source_loader(Arc::new(InMemorySourceLoader::default()))
        .build()
        .unwrap();
    let profile = compiler
        .language_host()
        .registry()
        .profile_id()
        .as_str()
        .to_owned();
    let analysis = AnalysisService::new(compiler)
        .analyze_workspace(
            WorkspaceSnapshot {
                generation: AnalysisGeneration::new(1),
                project_root,
                roots: vec![ModuleRoot::requested(chart.clone())],
                open_documents: BTreeMap::from([(
                    chart.clone(),
                    DocumentSnapshot::new(
                        chart.clone(),
                        SourceRevision::from_text(text),
                        text.to_owned(),
                    ),
                )]),
                known_disk_sources: vec![chart.clone()],
                native_registry_profile: profile,
            },
            &AnalysisCancellation::default(),
        )
        .await
        .unwrap();
    assert!(
        analysis.semantic_roots[&chart.canonical_uri()]
            .result
            .is_ok(),
        "{:?}",
        analysis.semantic_roots[&chart.canonical_uri()].result
    );
    Fixture {
        analysis,
        data: chart.clone(),
        chart,
    }
}

async fn transform_pipeline_fixture() -> (Fixture, String) {
    let project_root = std::fs::canonicalize(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-lang-compiler/tests/fixtures/projects/02_sql_pipeline"),
    )
    .unwrap();
    let chart = SourceOrigin::File(project_root.join("chart.avenger"));
    let text = std::fs::read_to_string(match &chart {
        SourceOrigin::File(path) => path,
        _ => unreachable!(),
    })
    .unwrap();
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .build()
        .unwrap();
    let profile = compiler
        .language_host()
        .registry()
        .profile_id()
        .as_str()
        .to_owned();
    let analysis = AnalysisService::new(compiler)
        .analyze_workspace(
            WorkspaceSnapshot {
                generation: AnalysisGeneration::new(1),
                project_root,
                roots: vec![ModuleRoot::requested(chart.clone())],
                open_documents: BTreeMap::from([(
                    chart.clone(),
                    DocumentSnapshot::new(
                        chart.clone(),
                        SourceRevision::from_text(&text),
                        text.clone(),
                    ),
                )]),
                known_disk_sources: vec![chart.clone()],
                native_registry_profile: profile,
            },
            &AnalysisCancellation::default(),
        )
        .await
        .unwrap();
    assert!(
        analysis.semantic_roots[&chart.canonical_uri()]
            .result
            .is_ok()
    );
    (
        Fixture {
            analysis,
            data: chart.clone(),
            chart,
        },
        text,
    )
}

async fn disk_chart_fixture(relative_project: &str) -> (Fixture, String) {
    disk_module_fixture(relative_project, "chart.avenger").await
}

async fn disk_module_fixture(relative_project: &str, file_name: &str) -> (Fixture, String) {
    let project_root = std::fs::canonicalize(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-lang-compiler/tests/fixtures/projects")
            .join(relative_project),
    )
    .unwrap();
    let chart = SourceOrigin::File(project_root.join(file_name));
    let text = std::fs::read_to_string(match &chart {
        SourceOrigin::File(path) => path,
        _ => unreachable!(),
    })
    .unwrap();
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .build()
        .unwrap();
    let profile = compiler
        .language_host()
        .registry()
        .profile_id()
        .as_str()
        .to_owned();
    let analysis = AnalysisService::new(compiler)
        .analyze_workspace(
            WorkspaceSnapshot {
                generation: AnalysisGeneration::new(1),
                project_root,
                roots: vec![ModuleRoot::requested(chart.clone())],
                open_documents: BTreeMap::from([(
                    chart.clone(),
                    DocumentSnapshot::new(
                        chart.clone(),
                        SourceRevision::from_text(&text),
                        text.clone(),
                    ),
                )]),
                known_disk_sources: vec![chart.clone()],
                native_registry_profile: profile,
            },
            &AnalysisCancellation::default(),
        )
        .await
        .unwrap();
    assert!(
        analysis.semantic_roots[&chart.canonical_uri()]
            .result
            .is_ok(),
        "{:?}",
        analysis.semantic_roots[&chart.canonical_uri()].result
    );
    (
        Fixture {
            analysis,
            data: chart.clone(),
            chart,
        },
        text,
    )
}

fn complete_marked(
    fixture: &Fixture,
    origin: &SourceOrigin,
    marked: String,
) -> avenger_lang_analysis::CompletionResult {
    assert_eq!(marked.matches(CURSOR).count(), 1);
    let cursor = marked.find(CURSOR).unwrap();
    let text = marked.replacen(CURSOR, "", 1);
    let revision = SourceRevision::from_text(&text);
    let mut syntax = fixture.analysis.syntax.clone();
    syntax.insert(
        origin.clone(),
        analyze_syntax(&DocumentSnapshot::new(
            origin.clone(),
            revision.clone(),
            text,
        )),
    );
    fixture
        .analysis
        .with_syntax(AnalysisGeneration::new(2), syntax)
        .complete(
            &PositionRequest {
                source: origin.clone(),
                byte_offset: cursor,
                source_revision: revision,
            },
            CompletionOptions::default(),
            &AnalysisCancellation::default(),
        )
        .unwrap()
}

fn complete_marked_with_options(
    fixture: &Fixture,
    origin: &SourceOrigin,
    marked: String,
    options: CompletionOptions,
) -> avenger_lang_analysis::CompletionResult {
    assert_eq!(marked.matches(CURSOR).count(), 1);
    let cursor = marked.find(CURSOR).unwrap();
    let text = marked.replacen(CURSOR, "", 1);
    let revision = SourceRevision::from_text(&text);
    let mut syntax = fixture.analysis.syntax.clone();
    syntax.insert(
        origin.clone(),
        analyze_syntax(&DocumentSnapshot::new(
            origin.clone(),
            revision.clone(),
            text,
        )),
    );
    fixture
        .analysis
        .with_syntax(AnalysisGeneration::new(2), syntax)
        .complete(
            &PositionRequest {
                source: origin.clone(),
                byte_offset: cursor,
                source_revision: revision,
            },
            options,
            &AnalysisCancellation::default(),
        )
        .unwrap()
}

fn hover_marked(
    fixture: &Fixture,
    origin: &SourceOrigin,
    marked: String,
) -> Option<avenger_lang_analysis::HoverResult> {
    assert_eq!(marked.matches(CURSOR).count(), 1);
    let cursor = marked.find(CURSOR).unwrap();
    let text = marked.replacen(CURSOR, "", 1);
    let revision = SourceRevision::from_text(&text);
    let mut syntax = fixture.analysis.syntax.clone();
    syntax.insert(
        origin.clone(),
        analyze_syntax(&DocumentSnapshot::new(
            origin.clone(),
            revision.clone(),
            text,
        )),
    );
    fixture
        .analysis
        .with_syntax(AnalysisGeneration::new(2), syntax)
        .hover(
            &PositionRequest {
                source: origin.clone(),
                byte_offset: cursor,
                source_revision: revision,
            },
            &AnalysisCancellation::default(),
        )
        .unwrap()
}

fn debug_marked(
    fixture: &Fixture,
    origin: &SourceOrigin,
    marked: String,
) -> avenger_lang_analysis::SqlCompletionDebug {
    assert_eq!(marked.matches(CURSOR).count(), 1);
    let cursor = marked.find(CURSOR).unwrap();
    let text = marked.replacen(CURSOR, "", 1);
    let revision = SourceRevision::from_text(&text);
    let mut syntax = fixture.analysis.syntax.clone();
    syntax.insert(
        origin.clone(),
        analyze_syntax(&DocumentSnapshot::new(
            origin.clone(),
            revision.clone(),
            text,
        )),
    );
    fixture
        .analysis
        .with_syntax(AnalysisGeneration::new(2), syntax)
        .debug_sql_completion(
            &PositionRequest {
                source: origin.clone(),
                byte_offset: cursor,
                source_revision: revision,
            },
            CompletionOptions::default(),
            &AnalysisCancellation::default(),
        )
        .unwrap()
        .expect("SQL completion debug")
}

fn labels(result: &avenger_lang_analysis::CompletionResult) -> Vec<&str> {
    result
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect()
}

fn apply_completion(marked: &str, item: &avenger_lang_analysis::CompletionItem) -> String {
    let mut text = marked.replacen(CURSOR, "", 1);
    text.replace_range(item.replacement.range.as_range(), &item.insert_text);
    text
}

#[tokio::test]
async fn select_first_and_from_first_share_qualified_columns() {
    let fixture = fixture().await;
    let mut reference_order = None;
    for query in [
        "SELECT m.\"⟦cursor⟧ FROM vega.movies AS m",
        "FROM vega.movies AS m SELECT m.\"⟦cursor⟧",
    ] {
        let result = complete_marked(&fixture, &fixture.data, data_source(query));
        let labels = labels(&result);
        assert_eq!(labels.first().copied(), Some("id"));
        for expected in ["id", "title", "category", "rating"] {
            assert!(labels.contains(&expected), "missing {expected}: {labels:?}");
        }
        let owned_labels = labels
            .iter()
            .map(|label| (*label).to_owned())
            .collect::<Vec<_>>();
        if let Some(reference_order) = &reference_order {
            assert_eq!(&owned_labels, reference_order);
        } else {
            reference_order = Some(owned_labels);
        }
        let rating = result
            .items
            .iter()
            .find(|item| item.label == "rating")
            .unwrap();
        assert!(
            rating
                .detail
                .as_deref()
                .unwrap()
                .contains("Decimal128(2, 1)")
        );
        assert_eq!(rating.insert_text, "\"rating\"");
        assert_eq!(rating.filter_text.as_deref(), Some("\"rating\""));
    }
}

#[tokio::test]
async fn quoted_column_and_relation_edits_produce_strict_parseable_source() {
    let fixture = fixture().await;
    for (marked, label) in [
        (
            data_source("FROM vega.movies AS m SELECT m.\"ra⟦cursor⟧"),
            "rating",
        ),
        (data_source("SELECT * FROM vega.\"mo⟦cursor⟧"), "movies"),
    ] {
        let result = complete_marked(&fixture, &fixture.data, marked.clone());
        let item = result
            .items
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("missing {label}: {:#?}", result.items));
        let applied = apply_completion(&marked, item);
        let source = SourceFile::new(
            SourceId::new(90),
            SourceOrigin::Memory(format!("applied-{label}.avenger")),
            applied,
        );
        parse_file(&source).unwrap_or_else(|failure| {
            panic!("applied {label} completion did not parse: {failure:#?}")
        });
    }
}

#[tokio::test]
async fn using_completion_is_the_intersection_of_prior_join_sources() {
    let fixture = fixture().await;
    let quoted = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * FROM vega.movies AS m JOIN vega.ratings AS r USING (\"⟦cursor⟧)"),
    );
    assert_eq!(labels(&quoted), ["id"], "{:#?}", quoted.items);

    let bare = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * FROM vega.movies AS m JOIN vega.ratings AS r USING (⟦cursor⟧)"),
    );
    assert!(bare.items.is_empty(), "{:#?}", bare.items);

    let newest_join = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT * FROM vega.movies AS first JOIN vega.movies AS second ON true JOIN vega.ratings AS newest USING (\"⟦cursor⟧)",
        ),
    );
    assert_eq!(labels(&newest_join), ["id"], "{:#?}", newest_join.items);

    for join in [
        "JOIN vega.ratings AS r USING (\"id\")",
        "NATURAL JOIN vega.ratings AS r",
    ] {
        let query = format!("SELECT \"i⟦cursor⟧ FROM vega.movies AS m {join}");
        let merged = complete_marked(&fixture, &fixture.data, data_source(&query));
        let ids = merged
            .items
            .iter()
            .filter(|item| item.label == "id")
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 1, "{join}: {:#?}", merged.items);
        assert_eq!(ids[0].insert_text, "\"id\"");
        assert_eq!(
            ids[0].qualification,
            avenger_lang_analysis::CompletionQualification::Unqualified
        );
    }

    let mixed = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT \"i⟦cursor⟧ FROM vega.movies AS m NATURAL JOIN vega.ratings AS r JOIN vega.movies AS third ON true",
        ),
    );
    let mixed_ids = mixed
        .items
        .iter()
        .filter(|item| item.label == "id")
        .collect::<Vec<_>>();
    assert_eq!(mixed_ids.len(), 3, "{:#?}", mixed.items);
    assert!(mixed_ids.iter().all(|item| {
        item.qualification == avenger_lang_analysis::CompletionQualification::Ambiguous
            && item.insert_text.contains(".\"id\"")
    }));
}

#[tokio::test]
async fn query_intent_matrix_is_clause_exact() {
    let fixture = fixture().await;

    let start = complete_marked(&fixture, &fixture.data, data_source("⟦cursor⟧"));
    assert_eq!(labels(&start), ["FROM", "SELECT", "VALUES", "WITH"]);

    let select_operand = complete_marked(&fixture, &fixture.data, data_source("SELECT ⟦cursor⟧"));
    for expected in ["ALL", "DISTINCT", "CASE", "CAST", "NULL"] {
        assert!(
            labels(&select_operand).contains(&expected),
            "missing {expected}: {:#?}",
            select_operand.items
        );
    }
    assert!(
        select_operand
            .items
            .iter()
            .all(|item| item.semantic_kind != CompletionSemanticKind::Relation)
    );

    let select_transition =
        complete_marked(&fixture, &fixture.data, data_source("SELECT 1 ⟦cursor⟧"));
    for expected in ["AS", "FILTER", "FROM", "OVER"] {
        assert!(labels(&select_transition).contains(&expected));
    }

    let cte_name = complete_marked(&fixture, &fixture.data, data_source("WITH ⟦cursor⟧"));
    assert!(cte_name.items.is_empty(), "{:#?}", cte_name.items);
    let cte_as = complete_marked(
        &fixture,
        &fixture.data,
        data_source("WITH filtered ⟦cursor⟧"),
    );
    assert_eq!(labels(&cte_as), ["AS"]);
    let cte_body = complete_marked(
        &fixture,
        &fixture.data,
        data_source("WITH filtered AS ⟦cursor⟧"),
    );
    assert_eq!(labels(&cte_body), ["("]);

    let relation = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * FROM ⟦cursor⟧"),
    );
    assert!(
        labels(&relation).contains(&"vega.movies"),
        "{:#?}",
        relation.items
    );
    assert!(labels(&relation).contains(&"LATERAL"));
    assert!(
        labels(&relation)
            .iter()
            .all(|label| !label.starts_with("chart:")),
        "compiler-internal relations leaked: {:#?}",
        relation.items
    );
    assert!(relation.items.iter().all(|item| !matches!(
        item.semantic_kind,
        CompletionSemanticKind::ScalarFunction
            | CompletionSemanticKind::AggregateFunction
            | CompletionSemanticKind::WindowFunction
    )));

    let alias = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * FROM vega.movies AS ⟦cursor⟧"),
    );
    assert!(alias.items.is_empty(), "{:#?}", alias.items);

    let predicate = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * FROM vega.movies WHERE ⟦cursor⟧"),
    );
    assert!(labels(&predicate).contains(&"CASE"));
    assert!(labels(&predicate).contains(&"round"));
    assert!(
        predicate
            .items
            .iter()
            .all(|item| { item.semantic_kind != CompletionSemanticKind::DataColumn })
    );

    let predicate_operator = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * FROM vega.movies WHERE \"id\" = 1 ⟦cursor⟧"),
    );
    for expected in ["AND", "OR", "IS NULL", "ORDER BY"] {
        assert!(
            labels(&predicate_operator).contains(&expected),
            "missing {expected}: {:#?}",
            predicate_operator.items
        );
    }

    for (query, expected) in [
        ("SELECT CASE ⟦cursor⟧ FROM vega.movies", &["WHEN"][..]),
        (
            "SELECT CASE WHEN \"id\" > 0 ⟦cursor⟧ FROM vega.movies",
            &["THEN"][..],
        ),
        (
            "SELECT CASE WHEN \"id\" > 0 THEN 1 ⟦cursor⟧ FROM vega.movies",
            &["WHEN", "ELSE", "END"][..],
        ),
        (
            "SELECT CASE WHEN \"id\" > 0 THEN 1 ELSE 0 ⟦cursor⟧ FROM vega.movies",
            &["END"][..],
        ),
    ] {
        let result = complete_marked(&fixture, &fixture.data, data_source(query));
        assert_eq!(labels(&result), expected, "{query}: {:#?}", result.items);
    }

    let wildcard = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * EX⟦cursor⟧ FROM vega.movies"),
    );
    assert_eq!(labels(&wildcard), ["EXCEPT", "EXCLUDE"]);

    let named_window = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT sum(\"rating\") OVER ra⟦cursor⟧ FROM vega.movies WINDOW ratings AS (PARTITION BY \"category\")",
        ),
    );
    assert_eq!(labels(&named_window), ["ratings"]);
    assert_eq!(
        named_window.items[0].semantic_kind,
        CompletionSemanticKind::WindowName
    );

    let named_window_body = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT sum(\"rating\") OVER ratings FROM vega.movies WINDOW ratings AS (⟦cursor⟧)",
        ),
    );
    for expected in ["PARTITION BY", "ORDER BY", "ROWS", "RANGE", "GROUPS"] {
        assert!(
            labels(&named_window_body).contains(&expected),
            "missing {expected}: {:#?}",
            named_window_body.items
        );
    }

    let limit_column = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * FROM vega.movies LIMIT \"⟦cursor⟧"),
    );
    assert!(limit_column.items.is_empty(), "{:#?}", limit_column.items);

    let no_window = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT 1 FROM vega.movies ⟦cursor⟧"),
    );
    assert!(!labels(&no_window).contains(&"QUALIFY"));
    let with_window = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT row_number() OVER () AS n FROM vega.movies ⟦cursor⟧"),
    );
    assert!(labels(&with_window).contains(&"QUALIFY"));
}

#[tokio::test]
async fn table_functions_are_inventory_driven_and_relation_only() {
    let fixture = fixture().await;
    let table_functions = fixture
        .analysis
        .semantic_roots
        .values()
        .filter_map(|root| root.result.as_ref().ok())
        .flat_map(|analysis| &analysis.functions.functions)
        .filter(|function| function.category == avenger_lang_compiler::FunctionCategory::Table)
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();
    assert!(table_functions.contains(&"generate_series"));
    assert!(table_functions.contains(&"range"));

    let relation = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * FROM gen⟦cursor⟧"),
    );
    let generate_series = relation
        .items
        .iter()
        .find(|item| item.label == "generate_series")
        .expect("table function in relation intent");
    assert_eq!(
        generate_series.semantic_kind,
        CompletionSemanticKind::TableFunction
    );

    let scalar = complete_marked(&fixture, &fixture.chart, chart_source("gen⟦cursor⟧"));
    assert!(
        scalar
            .items
            .iter()
            .all(|item| item.semantic_kind != CompletionSemanticKind::TableFunction),
        "{:#?}",
        scalar.items
    );
}

#[tokio::test]
async fn lexical_namespace_gates_are_hard_in_every_invocation_mode() {
    let fixture = fixture().await;
    for invocation in [
        CompletionInvocation::Invoked,
        CompletionInvocation::TriggerCharacter("\"".to_owned()),
        CompletionInvocation::TriggerForIncompleteCompletions,
    ] {
        let options = CompletionOptions {
            snippets: true,
            invocation,
        };
        let bare = complete_marked_with_options(
            &fixture,
            &fixture.chart,
            chart_source("rat⟦cursor⟧"),
            options.clone(),
        );
        assert!(
            bare.items
                .iter()
                .all(|item| item.semantic_kind != CompletionSemanticKind::DataColumn),
            "{:?}",
            bare.items
        );

        let quoted = complete_marked_with_options(
            &fixture,
            &fixture.chart,
            chart_source("\"rat⟦cursor⟧"),
            options.clone(),
        );
        assert!(!quoted.items.is_empty());
        assert!(quoted.items.iter().all(|item| {
            matches!(
                item.semantic_kind,
                CompletionSemanticKind::DataColumn
                    | CompletionSemanticKind::Relation
                    | CompletionSemanticKind::Catalog
                    | CompletionSemanticKind::Schema
            )
        }));

        let bare_param = complete_marked_with_options(
            &fixture,
            &fixture.chart,
            chart_source("min⟦cursor⟧"),
            options.clone(),
        );
        assert!(bare_param.items.iter().all(|item| {
            !matches!(
                item.semantic_kind,
                CompletionSemanticKind::ScalarParam | CompletionSemanticKind::StoreParam
            )
        }));
        let scalar = complete_marked_with_options(
            &fixture,
            &fixture.chart,
            chart_source("$min⟦cursor⟧"),
            options,
        );
        let minimum = scalar
            .items
            .iter()
            .find(|item| item.label == "$minimum")
            .expect("scalar binding completion");
        assert_eq!(minimum.filter_text.as_deref(), Some("$minimum"));
        assert!(
            scalar
                .items
                .iter()
                .all(|item| { !matches!(item.semantic_kind, CompletionSemanticKind::StoreParam) })
        );
    }

    for expression in ["'rat⟦cursor⟧'", "$$rat⟦cursor⟧", "1 /* rat⟦cursor⟧"] {
        let result = complete_marked(&fixture, &fixture.chart, chart_source(expression));
        assert!(result.items.is_empty(), "{expression}: {:?}", result.items);
    }

    let bare_member = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT m.⟦cursor⟧ FROM vega.movies AS m"),
    );
    assert_eq!(labels(&bare_member), ["*"]);
}

#[tokio::test]
async fn param_initializers_follow_predeclaration_visibility() {
    let source = r#"avenger 1;
chart cartesian as chart {
  param $later + 1 as earlier;
  param 2 as later;
  mark symbol { x: encoded 1; y: encoded 1; }
}
"#;
    let fixture = chart_only_fixture(source).await;
    let forward = complete_marked(
        &fixture,
        &fixture.chart,
        source.replace("$later + 1", "$lat⟦cursor⟧ + 1"),
    );
    assert!(labels(&forward).contains(&"$later"));
    assert!(!labels(&forward).contains(&"$earlier"));
}

#[tokio::test]
async fn explicit_completion_allows_fuzzy_matching_but_triggers_are_prefix_only() {
    let fixture = fixture().await;
    let invoked = complete_marked_with_options(
        &fixture,
        &fixture.chart,
        chart_source("rn⟦cursor⟧"),
        CompletionOptions {
            snippets: false,
            invocation: CompletionInvocation::Invoked,
        },
    );
    assert!(
        labels(&invoked)
            .iter()
            .any(|label| label.eq_ignore_ascii_case("round"))
    );
    let automatic = complete_marked_with_options(
        &fixture,
        &fixture.chart,
        chart_source("rn⟦cursor⟧"),
        CompletionOptions {
            snippets: false,
            invocation: CompletionInvocation::TriggerForIncompleteCompletions,
        },
    );
    assert!(
        labels(&automatic)
            .iter()
            .all(|label| !label.eq_ignore_ascii_case("round"))
    );
}

#[tokio::test]
async fn registered_functions_offer_snippets_and_plain_text_fallbacks() {
    let fixture = fixture().await;
    let function = fixture
        .analysis
        .semantic_roots
        .values()
        .filter_map(|root| root.result.as_ref().ok())
        .flat_map(|analysis| &analysis.functions.functions)
        .find(|function| {
            function.category == avenger_lang_compiler::FunctionCategory::Scalar
                && !function.parameter_names.is_empty()
        })
        .cloned()
        .expect("a registered scalar function with named parameters");
    let marked = chart_source(&format!("{}{}", function.name, CURSOR));

    let snippet_result = complete_marked_with_options(
        &fixture,
        &fixture.chart,
        marked.clone(),
        CompletionOptions {
            snippets: true,
            invocation: CompletionInvocation::Invoked,
        },
    );
    let snippet = snippet_result
        .items
        .iter()
        .find(|item| {
            item.label == function.name
                && item.semantic_kind == CompletionSemanticKind::ScalarFunction
        })
        .expect("registered function snippet");
    let arguments = function
        .parameter_names
        .iter()
        .enumerate()
        .map(|(index, parameter)| format!("${{{}:{parameter}}}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        snippet.insert_text,
        format!("{}({arguments})$0", function.name)
    );
    assert_eq!(snippet.insert_text_format, CompletionTextFormat::Snippet);
    assert!(
        snippet
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("DataFusion scalar function"))
    );

    let plain_result = complete_marked_with_options(
        &fixture,
        &fixture.chart,
        marked,
        CompletionOptions {
            snippets: false,
            invocation: CompletionInvocation::Invoked,
        },
    );
    let plain = plain_result
        .items
        .iter()
        .find(|item| {
            item.label == function.name
                && item.semantic_kind == CompletionSemanticKind::ScalarFunction
        })
        .expect("registered function plain-text completion");
    assert_eq!(plain.insert_text, format!("{}()", function.name));
    assert_eq!(plain.insert_text_format, CompletionTextFormat::PlainText);
}

#[tokio::test]
async fn catalog_cte_subquery_and_join_scopes_are_semantic() {
    let fixture = fixture().await;
    let catalog = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * FROM vega.\"⟦cursor⟧"),
    );
    assert!(
        labels(&catalog).contains(&"movies"),
        "catalog items: {:#?}\ndatasets: {:#?}",
        catalog.items,
        fixture.analysis.semantic_roots[&fixture.chart.canonical_uri()]
            .result
            .as_ref()
            .unwrap()
            .datasets
            .iter()
            .map(|(_, dataset)| (&dataset.qualified_name, &dataset.qualified_path))
            .collect::<Vec<_>>()
    );

    let cte = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "WITH filtered AS (SELECT \"title\", \"rating\" FROM vega.movies) SELECT f.\"⟦cursor⟧ FROM filtered AS f",
        ),
    );
    assert!(labels(&cte).contains(&"title"));
    assert!(labels(&cte).contains(&"rating"));

    let subquery = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT s.\"⟦cursor⟧ FROM (SELECT \"title\" AS movie_title FROM vega.movies) AS s",
        ),
    );
    assert!(
        labels(&subquery).contains(&"movie_title"),
        "{:#?}",
        subquery.items
    );

    let join = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "FROM vega.movies AS m JOIN vega.ratings AS r ON m.\"id\" = r.\"movie_id\" SELECT \"i⟦cursor⟧",
        ),
    );
    let id_insertions = join
        .items
        .iter()
        .filter(|item| item.label == "id")
        .map(|item| item.insert_text.as_str())
        .collect::<Vec<_>>();
    assert!(id_insertions.contains(&"m.\"id\""), "{id_insertions:?}");
    assert!(id_insertions.contains(&"r.\"id\""), "{id_insertions:?}");

    let correlated = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT * FROM vega.movies AS m WHERE EXISTS (SELECT 1 FROM vega.ratings AS r WHERE r.\"movie_id\" = m.\"⟦cursor⟧)",
        ),
    );
    assert!(labels(&correlated).contains(&"id"));
    assert!(labels(&correlated).contains(&"title"));

    let set_output = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT u.\"⟦cursor⟧ FROM (SELECT \"title\" FROM vega.movies UNION ALL SELECT \"title\" FROM vega.movies) AS u",
        ),
    );
    assert!(labels(&set_output).contains(&"title"));

    let plans_before = avenger_lang_analysis::SqlCompletionMetrics::snapshot().logical_query_plans;
    let projection_alias = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT \"rating\" AS score FROM vega.movies ORDER BY \"sco⟦cursor⟧"),
    );
    assert!(labels(&projection_alias).contains(&"score"));
    let score = projection_alias
        .items
        .iter()
        .find(|item| item.label == "score")
        .unwrap();
    assert!(
        score
            .detail
            .as_deref()
            .unwrap()
            .contains("Decimal128(2, 1)"),
        "{:?}",
        score.detail
    );
    assert!(
        avenger_lang_analysis::SqlCompletionMetrics::snapshot().logical_query_plans > plans_before
    );
}

#[tokio::test]
async fn nested_scope_shadowing_cte_names_join_order_and_set_arms_are_exact() {
    let fixture = fixture().await;

    let named_cte = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "WITH filtered(movie_title, score) AS (SELECT \"title\", \"rating\" FROM vega.movies) SELECT f.\"⟦cursor⟧ FROM filtered AS f",
        ),
    );
    assert!(labels(&named_cte).contains(&"movie_title"));
    assert!(labels(&named_cte).contains(&"score"));
    assert!(!labels(&named_cte).contains(&"title"));

    let non_recursive_self = complete_marked(
        &fixture,
        &fixture.data,
        data_source("WITH current AS (SELECT * FROM cur⟦cursor⟧) SELECT * FROM current"),
    );
    assert!(
        !labels(&non_recursive_self).contains(&"current"),
        "non-recursive CTE leaked into its own body: {:#?}",
        non_recursive_self.items
    );

    let recursive_self = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "WITH RECURSIVE current AS (SELECT 1 AS id UNION ALL SELECT c.\"⟦cursor⟧ AS id FROM current AS c) SELECT * FROM current",
        ),
    );
    assert_eq!(
        labels(&recursive_self),
        ["id"],
        "{:#?}",
        recursive_self.items
    );

    let shadowed = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT * FROM vega.movies AS m WHERE EXISTS (SELECT m.\"⟦cursor⟧ FROM vega.ratings AS m)",
        ),
    );
    for expected in ["id", "movie_id", "value"] {
        assert!(
            labels(&shadowed).contains(&expected),
            "{:?}",
            labels(&shadowed)
        );
    }
    assert!(!labels(&shadowed).contains(&"title"));

    let future_join = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT * FROM vega.movies AS m JOIN vega.ratings AS r ON future.\"⟦cursor⟧ = r.\"id\" JOIN vega.movies AS future ON true",
        ),
    );
    assert!(future_join.items.is_empty(), "{:?}", future_join.items);

    let set_arm = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT first.\"title\" FROM vega.movies AS first UNION ALL SELECT second.\"⟦cursor⟧ FROM vega.ratings AS second",
        ),
    );
    assert!(labels(&set_arm).contains(&"movie_id"));
    assert!(!labels(&set_arm).contains(&"title"));

    let wildcard_source =
        data_source("SELECT d.\"⟦cursor⟧ FROM (SELECT * EXCLUDE (\"id\") FROM vega.movies) AS d");
    let wildcard_debug = debug_marked(&fixture, &fixture.data, wildcard_source.clone());
    let wildcard_exclude = complete_marked(&fixture, &fixture.data, wildcard_source);
    assert!(
        labels(&wildcard_exclude).contains(&"title"),
        "items: {:#?}\ndebug: {:#?}",
        wildcard_exclude.items,
        wildcard_debug
    );
    assert!(!labels(&wildcard_exclude).contains(&"id"));

    let where_alias = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT \"rating\" AS score FROM vega.movies WHERE \"sco⟦cursor⟧ > 0"),
    );
    assert!(!labels(&where_alias).contains(&"score"));

    let isolated_derived = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * FROM vega.movies AS m CROSS JOIN (SELECT m.\"⟦cursor⟧) AS derived"),
    );
    assert!(
        isolated_derived.items.is_empty(),
        "non-lateral derived relation captured its parent: {:#?}",
        isolated_derived.items
    );

    let lateral_derived = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT * FROM vega.movies AS m CROSS JOIN LATERAL (SELECT m.\"⟦cursor⟧) AS derived",
        ),
    );
    for expected in ["id", "title", "category", "rating"] {
        assert!(
            labels(&lateral_derived).contains(&expected),
            "missing lateral parent field {expected}: {:#?}",
            lateral_derived.items
        );
    }
}

#[tokio::test]
async fn exact_pipeline_schema_bindings_functions_and_types_complete() {
    let fixture = fixture().await;
    let expression = complete_marked(&fixture, &fixture.chart, chart_source("\"rat⟦cursor⟧"));
    assert!(labels(&expression).contains(&"rating"));
    let scalar = complete_marked(&fixture, &fixture.chart, chart_source("$min⟦cursor⟧"));
    assert!(labels(&scalar).contains(&"$minimum"));

    let struct_param = complete_marked(&fixture, &fixture.chart, chart_source("$config.⟦cursor⟧"));
    assert!(labels(&struct_param).contains(&"label"));
    assert!(labels(&struct_param).contains(&"weight"));

    let store = complete_marked(
        &fixture,
        &fixture.chart,
        chart_query_source("SELECT s.\"⟦cursor⟧ FROM $selected AS s"),
    );
    assert!(labels(&store).contains(&"id"), "{:#?}", store.items);
    assert!(labels(&store).contains(&"label"));

    let temporal = complete_marked(
        &fixture,
        &fixture.chart,
        chart_handler_source("$minimum@⟦cursor⟧"),
    );
    assert!(labels(&temporal).contains(&"$minimum@start"));
    assert!(labels(&temporal).contains(&"$minimum@previous"));

    let function = complete_marked(
        &fixture,
        &fixture.chart,
        chart_source("ro⟦cursor⟧(\"rating\")"),
    );
    assert!(
        labels(&function)
            .iter()
            .any(|label| label.eq_ignore_ascii_case("round"))
    );
    let operation = complete_marked(
        &fixture,
        &fixture.chart,
        chart_handler_source("spa⟦cursor⟧"),
    );
    for expected in ["span", "span_ordered"] {
        assert!(
            labels(&operation).contains(&expected),
            "missing intrinsic operation {expected}: {:?}",
            labels(&operation)
        );
    }

    let data_type = complete_marked(
        &fixture,
        &fixture.chart,
        chart_source("CAST(\"rating\" AS DO⟦cursor⟧)"),
    );
    assert!(labels(&data_type).contains(&"DOUBLE"));
}

#[tokio::test]
async fn table_source_completes_first_class_relation_paths() {
    let fixture = fixture().await;
    let source = chart_source("\"rating\"");
    let relation_offset = source.find("table: vega.movies").unwrap() + "table: vega.".len();
    let reference = fixture
        .analysis
        .semantic_index
        .reference_at(&fixture.chart, relation_offset)
        .expect("table relation reference");
    assert_eq!(reference.name, "vega.movies");
    assert_eq!(reference.value_kind, IndexedValueKind::Table);
    assert!(reference.target_identity.is_some());

    let member = complete_marked(
        &fixture,
        &fixture.chart,
        source.replace("table: vega.movies;", "table: vega.mo⟦cursor⟧;"),
    );
    let movies = member
        .items
        .iter()
        .find(|item| item.label == "vega.movies")
        .unwrap_or_else(|| panic!("missing relation completion: {:#?}", member.items));
    assert_eq!(movies.insert_text, "vega.movies");
    assert_eq!(movies.kind, CompletionKind::Table);

    let root = complete_marked(
        &fixture,
        &fixture.chart,
        chart_source("\"rating\"").replace("table: vega.movies;", "table: ve⟦cursor⟧;"),
    );
    let movies = root
        .items
        .iter()
        .find(|item| item.label == "vega.movies")
        .unwrap_or_else(|| panic!("missing root relation completion: {:#?}", root.items));
    assert_eq!(movies.insert_text, "vega.movies");
}

#[tokio::test]
async fn encoded_and_direct_channel_expressions_share_sql_completion() {
    let fixture = fixture().await;
    let encoded_source = chart_source("\"rat⟦cursor⟧");
    let direct_source = encoded_source.replacen("x: encoded", "x: direct", 1);
    let encoded = complete_marked(&fixture, &fixture.chart, encoded_source);
    let direct = complete_marked(&fixture, &fixture.chart, direct_source);
    assert_eq!(labels(&encoded), labels(&direct));
    assert!(labels(&direct).contains(&"rating"));
}

#[tokio::test]
async fn expanded_definitions_reconnect_dataset_stages_to_authored_islands() {
    let (transform_fixture, transform_source) =
        disk_module_fixture("08_multi_chart_project", "cartesian.avenger").await;
    let transformed = complete_marked(
        &transform_fixture,
        &transform_fixture.chart,
        transform_source.replace("x: encoded \"x\"", "x: encoded \"⟦cursor⟧\""),
    );
    for expected in ["radius", "theta", "x", "y"] {
        assert!(
            labels(&transformed).contains(&expected),
            "missing {expected} after an expanded transform: {:#?}",
            transformed.items
        );
    }

    let (mark_fixture, mark_source) =
        disk_module_fixture("04_custom_error_bar", "chart.avenger").await;
    let supplied_block = complete_marked(
        &mark_fixture,
        &mark_fixture.chart,
        mark_source.replacen(
            "text: encoded \"category\"",
            "text: encoded \"⟦cursor⟧\"",
            1,
        ),
    );
    for expected in ["category", "center", "high", "low"] {
        assert!(
            labels(&supplied_block).contains(&expected),
            "missing {expected} inside a supplied defined-mark block: {:#?}",
            supplied_block.items
        );
    }
}

#[tokio::test]
async fn array_elements_and_definition_outputs_use_their_exact_semantic_environment() {
    let aggregate = r#"avenger 1;

schema tables as vega {
  table inline as movies {
    values: [
      { category: 'drama'; rating: 8.5; },
      { category: 'comedy'; rating: 7.0; }
    ];
  }
}

chart cartesian as chart {
  data: { table: vega.movies; }
  transform aggregate {
    group_by: ["category"];
    expressions: count(*) AS count;
  }
  mark symbol as points {
    x: encoded "category";
    y: encoded "count";
  }
}
"#;
    let aggregate_fixture = chart_only_fixture(aggregate).await;
    let aggregate_marked =
        aggregate.replace("group_by: [\"category\"]", "group_by: [\"cat⟦cursor⟧\"]");
    let array = complete_marked(
        &aggregate_fixture,
        &aggregate_fixture.chart,
        aggregate_marked,
    );
    let category = array
        .items
        .iter()
        .find(|item| item.label == "category")
        .unwrap_or_else(|| panic!("array element lost its input stage: {:#?}", array.items));
    assert_eq!(category.insert_text, "\"category\"");
    assert_eq!(
        debug_marked(
            &aggregate_fixture,
            &aggregate_fixture.chart,
            aggregate.replace("group_by: [\"category\"]", "group_by: [\"cat⟦cursor⟧\"]",),
        )
        .site,
        avenger_lang_core::syntax::SqlIslandSite::ArrayElement
    );

    let definition = r#"avenger 1;

define transform binned {
  slot expr field;
  output bins.start as start;
  transform bin as bins {
    field: field;
    maxbins: 10;
  }
}
"#;
    let definition_fixture = chart_only_fixture(definition).await;
    let alias = complete_marked(
        &definition_fixture,
        &definition_fixture.chart,
        definition.replace(
            "output bins.start as start",
            "output bins.st⟦cursor⟧ as start",
        ),
    );
    assert_eq!(labels(&alias), ["start"], "{:#?}", alias.items);
    assert_eq!(alias.items[0].insert_text, "start");

    let slot = complete_marked(
        &definition_fixture,
        &definition_fixture.chart,
        definition.replace("output bins.start as start", "output fi⟦cursor⟧ as start"),
    );
    assert!(labels(&slot).contains(&"field"), "{:#?}", slot.items);
    let field = slot
        .items
        .iter()
        .find(|item| item.label == "field")
        .unwrap();
    assert_eq!(field.insert_text, "field");

    let sequential = complete_marked(
        &definition_fixture,
        &definition_fixture.chart,
        definition.replace(
            "slot expr field;",
            "slot expr field { default: bins.st⟦cursor⟧; }",
        ),
    );
    assert!(
        !labels(&sequential).contains(&"start"),
        "the output-interface forward-reference exception leaked into an ordinary expression: {:#?}",
        sequential.items
    );
}

#[tokio::test]
async fn event_datum_completion_is_target_aware_and_always_quotes_fields() {
    let fixture = fixture().await;
    let completion = complete_marked(
        &fixture,
        &fixture.chart,
        chart_handler_source("datum.\"⟦cursor⟧"),
    );
    for expected in ["id", "title", "category", "rating"] {
        let item = completion
            .items
            .iter()
            .find(|item| item.label == expected)
            .unwrap_or_else(|| panic!("missing {expected}: {:?}", labels(&completion)));
        assert_eq!(item.insert_text, format!("\"{expected}\""));
        assert!(
            item.detail
                .as_deref()
                .is_some_and(|detail| detail.contains("logical hit row")),
            "{:?}",
            item.detail
        );
    }

    let partial = complete_marked(
        &fixture,
        &fixture.chart,
        chart_handler_source("datum.\"ra⟦cursor⟧"),
    );
    let rating = partial
        .items
        .iter()
        .find(|item| item.label == "rating")
        .expect("partial quoted datum field completion");
    assert_eq!(rating.insert_text, "\"rating\"");
}

#[tokio::test]
async fn contextual_access_completion_is_staged_and_schema_aware() {
    let fixture = fixture().await;

    let event = complete_marked(
        &fixture,
        &fixture.chart,
        chart_handler_source("event.⟦cursor⟧"),
    );
    for expected in ["coord", "domain", "facet"] {
        assert!(
            labels(&event).contains(&expected),
            "missing event member {expected}: {:?}",
            labels(&event)
        );
    }
    assert!(!labels(&event).contains(&"start"));
    assert!(!labels(&event).contains(&"path"));

    let event_channel = complete_marked(
        &fixture,
        &fixture.chart,
        chart_handler_source("event.coord.⟦cursor⟧"),
    );
    for expected in ["x", "y"] {
        assert!(
            labels(&event_channel).contains(&expected),
            "missing event channel {expected}: {:?}",
            labels(&event_channel)
        );
    }

    let domain = complete_marked(
        &fixture,
        &fixture.chart,
        chart_handler_source("event.domain.x.⟦cursor⟧"),
    );
    assert!(labels(&domain).contains(&"start"));
    assert!(labels(&domain).contains(&"end"));

    let mark_channel = complete_marked(&fixture, &fixture.chart, chart_source("channel.⟦cursor⟧"));
    assert!(labels(&mark_channel).contains(&"x"));
    assert!(labels(&mark_channel).contains(&"y"));
    let x = mark_channel
        .items
        .iter()
        .find(|item| item.label == "x")
        .expect("mark x channel");
    assert!(
        x.detail
            .as_deref()
            .is_some_and(|detail| detail.contains("Decimal128(2, 1)")),
        "{:?}",
        x.detail
    );
}

#[tokio::test]
async fn item_and_inline_view_completion_use_runtime_schemas() {
    let effects = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../avenger-lang-compiler/tests/fixtures/projects/10_native_surface_contracts/mark_effects.avenger",
        ),
    )
    .unwrap();
    let fixture = chart_only_fixture(&effects).await;

    let item = complete_marked(
        &fixture,
        &fixture.chart,
        effects.replace("text: item.data.\"name\";", "text: item.⟦cursor⟧;"),
    );
    for expected in ["channel", "data", "bbox"] {
        assert!(labels(&item).contains(&expected), "{:?}", labels(&item));
    }

    let channels = complete_marked(
        &fixture,
        &fixture.chart,
        effects.replace("text: item.data.\"name\";", "text: item.channel.⟦cursor⟧;"),
    );
    for expected in ["x", "y", "size", "fill", "stroke"] {
        assert!(
            labels(&channels).contains(&expected),
            "missing item channel {expected}: {:?}",
            labels(&channels)
        );
    }
    let x = channels
        .items
        .iter()
        .find(|item| item.label == "x")
        .expect("item x channel");
    assert!(
        x.detail
            .as_deref()
            .is_some_and(|detail| detail.contains("float32")),
        "{:?}",
        x.detail
    );

    let data = complete_marked(
        &fixture,
        &fixture.chart,
        effects.replace("text: item.data.\"name\";", "text: item.data.\"⟦cursor⟧;"),
    );
    for expected in ["name", "x", "y"] {
        let item = data
            .items
            .iter()
            .find(|item| item.label == expected)
            .unwrap_or_else(|| panic!("missing item data {expected}: {:?}", labels(&data)));
        assert_eq!(item.insert_text, format!("\"{expected}\""));
    }

    let bbox = complete_marked(
        &fixture,
        &fixture.chart,
        effects.replace("text: item.data.\"name\";", "text: item.bbox.⟦cursor⟧;"),
    );
    for expected in ["top", "right", "bottom", "left"] {
        assert!(labels(&bbox).contains(&expected), "{:?}", labels(&bbox));
    }

    let (view_fixture, view_source) = disk_chart_fixture("07_inline_view_raster").await;
    let view = complete_marked(
        &view_fixture,
        &view_fixture.chart,
        view_source.replacen("bins: 32;", "bins: viewport.x.⟦cursor⟧;", 1),
    );
    assert!(labels(&view).contains(&"domain"));
    assert!(labels(&view).contains(&"pixels"));
    let domain = complete_marked(
        &view_fixture,
        &view_fixture.chart,
        view_source.replacen("bins: 32;", "bins: viewport.x.domain.⟦cursor⟧;", 1),
    );
    assert!(labels(&domain).contains(&"start"));
    assert!(labels(&domain).contains(&"end"));
}

#[tokio::test]
async fn event_datum_hover_tokens_and_migration_actions_are_contextual() {
    let fixture = fixture().await;
    let marked = chart_handler_source(r#"da⟦cursor⟧tum."rating""#);
    let cursor = marked.find(CURSOR).unwrap();
    let text = marked.replacen(CURSOR, "", 1);
    let revision = SourceRevision::from_text(&text);
    let mut syntax = fixture.analysis.syntax.clone();
    syntax.insert(
        fixture.chart.clone(),
        analyze_syntax(&DocumentSnapshot::new(
            fixture.chart.clone(),
            revision.clone(),
            text,
        )),
    );
    let analysis = fixture
        .analysis
        .with_syntax(AnalysisGeneration::new(2), syntax);
    let request = PositionRequest {
        source: fixture.chart.clone(),
        byte_offset: cursor,
        source_revision: revision.clone(),
    };
    let hover = analysis
        .hover(&request, &AnalysisCancellation::default())
        .unwrap()
        .expect("datum hover");
    assert!(
        hover.markdown.contains("Decimal128(2, 1)"),
        "{}",
        hover.markdown
    );
    assert!(
        hover
            .markdown
            .to_ascii_lowercase()
            .contains("logical pre-scale field"),
        "{}",
        hover.markdown
    );
    let tokens = analysis
        .semantic_tokens(
            &DocumentRequest {
                source: fixture.chart.clone(),
                source_revision: revision,
            },
            &AnalysisCancellation::default(),
        )
        .unwrap()
        .tokens;
    assert!(tokens.iter().any(|token| {
        token.kind == SemanticTokenKind::Namespace && token.modifiers.default_library
    }));
    assert!(
        tokens
            .iter()
            .any(|token| { token.kind == SemanticTokenKind::Field && token.modifiers.readonly })
    );

    for (authored, expected) in [
        ("datum('rating')", r#"datum."rating""#),
        ("datum.rating", r#"datum."rating""#),
    ] {
        let source = chart_handler_source(authored);
        let revision = SourceRevision::from_text(&source);
        let mut syntax = fixture.analysis.syntax.clone();
        syntax.insert(
            fixture.chart.clone(),
            analyze_syntax(&DocumentSnapshot::new(
                fixture.chart.clone(),
                revision.clone(),
                source.clone(),
            )),
        );
        let analysis = fixture
            .analysis
            .with_syntax(AnalysisGeneration::new(3), syntax);
        let start = source.find(authored).unwrap();
        let actions = analysis
            .code_actions(
                &CodeActionRequest {
                    source: fixture.chart.clone(),
                    range: SourceSpan {
                        source: analysis.syntax[&fixture.chart].parsed.nodes[0].span.source,
                        range: ByteSpan {
                            start,
                            end: start + authored.len(),
                        },
                    },
                    source_revision: revision,
                    diagnostic_codes: Vec::new(),
                },
                &AnalysisCancellation::default(),
            )
            .unwrap();
        assert!(
            actions.iter().any(|action| {
                action
                    .edit
                    .sources
                    .get(&fixture.chart)
                    .is_some_and(|edits| edits.edits.iter().any(|edit| edit.new_text == expected))
            }),
            "missing migration action for {authored}: {actions:?}"
        );
    }

    for authored in ["'datum(''rating'')'", "'datum.rating'"] {
        let source = chart_handler_source(authored);
        let revision = SourceRevision::from_text(&source);
        let mut syntax = fixture.analysis.syntax.clone();
        syntax.insert(
            fixture.chart.clone(),
            analyze_syntax(&DocumentSnapshot::new(
                fixture.chart.clone(),
                revision.clone(),
                source.clone(),
            )),
        );
        let analysis = fixture
            .analysis
            .with_syntax(AnalysisGeneration::new(4), syntax);
        let start = source.find(authored).unwrap();
        let actions = analysis
            .code_actions(
                &CodeActionRequest {
                    source: fixture.chart.clone(),
                    range: SourceSpan {
                        source: analysis.syntax[&fixture.chart].parsed.nodes[0].span.source,
                        range: ByteSpan {
                            start,
                            end: start + authored.len(),
                        },
                    },
                    source_revision: revision,
                    diagnostic_codes: Vec::new(),
                },
                &AnalysisCancellation::default(),
            )
            .unwrap();
        assert!(
            actions
                .iter()
                .all(|action| !action.title.starts_with("Use `datum.")),
            "datum text inside a SQL string received a migration action: {actions:?}"
        );
    }
}

#[tokio::test]
async fn quoted_column_hover_uses_effective_sql_scope_and_arrow_schema() {
    let fixture = fixture().await;

    let expression = hover_marked(
        &fixture,
        &fixture.chart,
        chart_source(r#""ra⟦cursor⟧ting""#),
    )
    .expect("expression column hover");
    assert_eq!(expression.markdown.lines().next(), Some("```sql"));
    assert!(
        expression
            .markdown
            .contains("Arrow type: `Decimal128(2, 1)`"),
        "{}",
        expression.markdown
    );
    assert!(expression.markdown.contains("Nullable: `true`"));

    let qualified = hover_marked(
        &fixture,
        &fixture.data,
        data_source(r#"SELECT m."ra⟦cursor⟧ting" FROM vega.movies AS m"#),
    )
    .expect("qualified query column hover");
    assert!(
        qualified
            .markdown
            .contains("Arrow type: `Decimal128(2, 1)`"),
        "{}",
        qualified.markdown
    );

    let ambiguous = hover_marked(
        &fixture,
        &fixture.data,
        data_source(r#"SELECT "i⟦cursor⟧d" FROM vega.movies AS m JOIN vega.ratings AS r ON true"#),
    )
    .expect("ambiguous query column hover");
    assert!(ambiguous.markdown.contains("Ambiguous SQL column"));
    assert!(ambiguous.markdown.contains("m.\"id\""));
    assert!(ambiguous.markdown.contains("r.\"id\""));

    assert!(
        hover_marked(
            &fixture,
            &fixture.data,
            data_source(r#"SELECT "rating" AS "sc⟦cursor⟧ore" FROM vega.movies"#),
        )
        .is_none(),
        "a quoted output alias is a binder, not a column reference"
    );
    assert!(
        hover_marked(
            &fixture,
            &fixture.data,
            data_source(r#"SELECT * FROM vega."mov⟦cursor⟧ies""#),
        )
        .is_none(),
        "a quoted relation path is not a column reference"
    );
}

#[tokio::test]
async fn quoted_column_hover_works_in_projection_lists() {
    let (fixture, source) = transform_pipeline_fixture().await;
    let marked = source.replacen(
        r#"expressions: sum("amount") AS total"#,
        r#"expressions: sum("am⟦cursor⟧ount") AS total"#,
        1,
    );
    let hover = hover_marked(&fixture, &fixture.chart, marked).expect("projection column hover");
    assert!(
        hover.markdown.contains("Arrow type: `Float64`"),
        "{}",
        hover.markdown
    );
}

#[tokio::test]
async fn event_datum_completion_reports_union_coverage_and_type_conflicts() {
    let source = r#"avenger 1;
chart cartesian as chart {
  mark symbol as numeric {
    data: { values: [{ id: 1; only_numeric: 2.0; }]; }
    x: encoded 1;
    y: encoded 1;
  }
  mark symbol as textual {
    data: { values: [{ id: 'one'; only_textual: true; }]; }
    x: encoded 2;
    y: encoded 2;
  }
  on click as inspect {
    filter: datum."id" IS NOT NULL;
  }
}
"#;
    let fixture = chart_only_fixture(source).await;
    let marked = source.replace(r#"datum."id" IS NOT NULL"#, "datum.\"⟦cursor⟧");
    let completion = complete_marked(&fixture, &fixture.chart, marked);
    let id = completion
        .items
        .iter()
        .find(|item| item.label == "id")
        .expect("shared conflicting datum field");
    assert!(
        id.detail
            .as_deref()
            .is_some_and(|detail| detail.contains("target-dependent")),
        "{:?}",
        id.detail
    );
    let only_numeric = completion
        .items
        .iter()
        .find(|item| item.label == "only_numeric")
        .expect("partial-coverage datum field");
    assert!(
        only_numeric
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("1/2 targets")),
        "{:?}",
        only_numeric.detail
    );
    assert_eq!(only_numeric.insert_text, "\"only_numeric\"");
}

#[tokio::test]
async fn chained_transforms_complete_the_exact_input_and_output_stage() {
    let (fixture, source) = transform_pipeline_fixture().await;

    let projection_input = complete_marked(
        &fixture,
        &fixture.chart,
        source.replace(
            "expressions: sum(\"amount\") AS total",
            "expressions: sum(\"am⟦cursor⟧) AS total",
        ),
    );
    assert!(labels(&projection_input).contains(&"amount"));
    assert!(!labels(&projection_input).contains(&"total"));

    let projection_as = complete_marked(
        &fixture,
        &fixture.chart,
        source.replace(
            "expressions: sum(\"amount\") AS total",
            "expressions: sum(\"amount\") ⟦cursor⟧",
        ),
    );
    assert!(labels(&projection_as).contains(&"AS"));

    let projection_alias = complete_marked(
        &fixture,
        &fixture.chart,
        source.replace(
            "expressions: sum(\"amount\") AS total",
            "expressions: sum(\"amount\") AS ⟦cursor⟧",
        ),
    );
    assert!(
        projection_alias.items.is_empty(),
        "{:?}",
        labels(&projection_alias)
    );

    let sql_input_source = source.replace(
        "SELECT \"category\", \"total\" * 2.0 AS doubled FROM input ORDER BY \"category\"",
        "SELECT i.\"⟦cursor⟧\n        FROM input AS i",
    );
    let sql_input = complete_marked(&fixture, &fixture.chart, sql_input_source);
    assert!(
        labels(&sql_input).contains(&"category"),
        "{:#?}",
        sql_input.items
    );
    assert!(labels(&sql_input).contains(&"total"));
    assert!(!labels(&sql_input).contains(&"amount"));
    assert!(!labels(&sql_input).contains(&"doubled"));

    let final_output = complete_marked(
        &fixture,
        &fixture.chart,
        source.replace("y: encoded \"doubled\"", "y: encoded \"dou⟦cursor⟧"),
    );
    assert!(labels(&final_output).contains(&"doubled"));
    let final_category = complete_marked(
        &fixture,
        &fixture.chart,
        source.replace("y: encoded \"doubled\"", "y: encoded \"cat⟦cursor⟧"),
    );
    assert!(labels(&final_category).contains(&"category"));
    assert!(!labels(&final_output).contains(&"amount"));
    assert!(!labels(&final_output).contains(&"total"));
}

#[tokio::test]
async fn every_native_projection_transform_has_policy_aware_completion() {
    let (fixture, source) = disk_module_fixture("13_projection_transforms", "charts.avenger").await;
    let column_cases = [
        (
            "aggregate",
            "sum(\"amount\") AS total,",
            "sum(\"am⟦cursor⟧\") AS total,",
        ),
        (
            "join_aggregate",
            "expressions: sum(\"amount\") AS total;",
            "expressions: sum(\"am⟦cursor⟧\") AS total;",
        ),
        (
            "scalar_aggregate",
            "expressions: avg(\"amount\") AS average;",
            "expressions: avg(\"am⟦cursor⟧\") AS average;",
        ),
        (
            "calculate",
            "expressions: \"amount\" * 2.0 AS doubled;",
            "expressions: \"am⟦cursor⟧\" * 2.0 AS doubled;",
        ),
        (
            "window",
            "expressions: row_number() OVER () AS ordinal;",
            "expressions: row_number() OVER (ORDER BY \"am⟦cursor⟧\") AS ordinal;",
        ),
        (
            "select",
            "expressions:\n      \"category\",\n      \"amount\" * 2.0 AS doubled;",
            "expressions:\n      \"category\",\n      \"am⟦cursor⟧\" * 2.0 AS doubled;",
        ),
    ];
    for (kind, authored, marked) in column_cases {
        let marked_source = source.replacen(authored, marked, 1);
        assert!(marked_source.contains(CURSOR), "missing {kind} replacement");
        let completion = complete_marked(&fixture, &fixture.chart, marked_source.clone());
        let amount = completion
            .items
            .iter()
            .find(|item| item.label == "amount")
            .unwrap_or_else(|| panic!("missing {kind} input column: {:#?}", completion.items));
        assert_eq!(amount.insert_text, "\"amount\"");
        let applied = apply_completion(&marked_source, amount);
        assert!(
            parse_file(&SourceFile::new(
                SourceId::new(700),
                SourceOrigin::Memory(format!("{kind}-projection.avenger")),
                applied,
            ))
            .is_ok(),
            "{kind} completion did not produce strict syntax"
        );
    }

    for (kind, authored, marked, expected) in [
        (
            "aggregate",
            "sum(\"amount\") AS total,",
            "su⟦cursor⟧(\"amount\") AS total,",
            true,
        ),
        (
            "join_aggregate",
            "expressions: sum(\"amount\") AS total;",
            "expressions: su⟦cursor⟧(\"amount\") AS total;",
            true,
        ),
        (
            "scalar_aggregate",
            "expressions: avg(\"amount\") AS average;",
            "expressions: su⟦cursor⟧(\"amount\") AS average;",
            true,
        ),
        (
            "calculate",
            "expressions: \"amount\" * 2.0 AS doubled;",
            "expressions: su⟦cursor⟧(\"amount\") AS doubled;",
            false,
        ),
        (
            "select",
            "expressions:\n      \"category\",\n      \"amount\" * 2.0 AS doubled;",
            "expressions:\n      \"category\",\n      su⟦cursor⟧(\"amount\") AS doubled;",
            false,
        ),
    ] {
        let marked_source = source.replacen(authored, marked, 1);
        let completion = complete_marked(&fixture, &fixture.chart, marked_source);
        assert_eq!(
            completion.items.iter().any(|item| {
                item.label.eq_ignore_ascii_case("sum")
                    && item.semantic_kind == CompletionSemanticKind::AggregateFunction
            }),
            expected,
            "aggregate function legality for {kind}: {:#?}",
            completion.items
        );
    }

    let window = complete_marked(
        &fixture,
        &fixture.chart,
        source.replacen(
            "row_number() OVER () AS ordinal",
            "row_n⟦cursor⟧() OVER () AS ordinal",
            1,
        ),
    );
    assert!(window.items.iter().any(|item| {
        item.label.eq_ignore_ascii_case("row_number")
            && item.semantic_kind == CompletionSemanticKind::WindowFunction
    }));

    for authored in [
        "sum(\"amount\") AS total,",
        "expressions: sum(\"amount\") AS total;",
        "expressions: avg(\"amount\") AS average;",
        "expressions: \"amount\" * 2.0 AS doubled;",
        "expressions: row_number() OVER () AS ordinal;",
        "expressions:\n      \"category\",\n      \"amount\" * 2.0 AS doubled;",
    ] {
        let marked = authored.replacen(" AS ", " AS ⟦cursor⟧", 1);
        let completion = complete_marked(
            &fixture,
            &fixture.chart,
            source.replacen(authored, &marked, 1),
        );
        assert!(
            completion.items.is_empty(),
            "projection alias binder leaked candidates for `{authored}`: {:#?}",
            completion.items
        );
    }
}

#[tokio::test]
async fn exact_fingerprint_cache_reuses_authored_analysis_only() {
    let fixture = fixture().await;
    let marked = data_source("FROM vega.movies AS m SELECT m.\"⟦cursor⟧");
    let before = avenger_lang_analysis::SqlCompletionMetrics::snapshot();
    let first = complete_marked(&fixture, &fixture.data, marked.clone());
    let second = complete_marked(&fixture, &fixture.data, marked);
    assert_eq!(labels(&first), labels(&second));
    let after = avenger_lang_analysis::SqlCompletionMetrics::snapshot();
    assert!(after.cache_hits > before.cache_hits);
}

#[tokio::test]
async fn ambiguous_column_quick_fixes_use_scoped_qualified_insertions() {
    let fixture = fixture().await;
    let text =
        data_source("SELECT id FROM vega.movies AS m JOIN vega.ratings AS r ON m.id = r.movie_id");
    let revision = SourceRevision::from_text(&text);
    let mut syntax = fixture.analysis.syntax.clone();
    syntax.insert(
        fixture.data.clone(),
        analyze_syntax(&DocumentSnapshot::new(
            fixture.data.clone(),
            revision.clone(),
            text.clone(),
        )),
    );
    let analysis = fixture
        .analysis
        .with_syntax(AnalysisGeneration::new(3), syntax);
    let start = text.find("SELECT id").unwrap() + "SELECT ".len();
    let actions = analysis
        .code_actions(
            &CodeActionRequest {
                source: fixture.data.clone(),
                range: SourceSpan {
                    source: analysis.syntax[&fixture.data].parsed.tokens.source(),
                    range: ByteSpan {
                        start,
                        end: start + 2,
                    },
                },
                source_revision: revision,
                diagnostic_codes: vec!["DataFusion".to_owned()],
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    let titles = actions
        .iter()
        .map(|action| action.title.as_str())
        .collect::<Vec<_>>();
    assert!(titles.contains(&"Qualify `id` as `m.\"id\"`"), "{titles:?}");
    assert!(titles.contains(&"Qualify `id` as `r.\"id\"`"), "{titles:?}");
}

#[tokio::test]
async fn syntax_only_completion_is_immediate_and_reports_incomplete_metadata() {
    let fixture = fixture().await;
    let source = data_source("FROM vega.movies AS m SELECT m.\"⟦cursor⟧");
    let cursor = source.find(CURSOR).unwrap();
    let text = source.replacen(CURSOR, "", 1);
    let revision = SourceRevision::from_text(&text);
    let syntax = BTreeMap::from([(
        fixture.data.clone(),
        analyze_syntax(&DocumentSnapshot::new(
            fixture.data.clone(),
            revision.clone(),
            text,
        )),
    )]);
    let analysis = WorkspaceAnalysis::syntax_only(
        AnalysisGeneration::new(3),
        fixture.analysis.project_root.clone(),
        vec![fixture.data.clone()],
        syntax,
        fixture.analysis.registry.clone(),
    );
    let result = analysis
        .complete(
            &PositionRequest {
                source: fixture.data.clone(),
                byte_offset: cursor,
                source_revision: revision,
            },
            CompletionOptions::default(),
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert!(result.is_incomplete);
}

#[tokio::test]
#[ignore = "manual SQL completion timing baseline; not a CI threshold"]
async fn record_sql_completion_timing_baseline() {
    let fixture = fixture().await;
    let marked = data_source("FROM vega.movies AS m SELECT m.\"⟦cursor⟧");
    let metrics_before = avenger_lang_analysis::SqlCompletionMetrics::snapshot();
    let cold_allocations_before = allocation_snapshot();
    let start = Instant::now();
    let cold_result =
        std::hint::black_box(complete_marked(&fixture, &fixture.data, marked.clone()));
    let cold = start.elapsed();
    let cold_allocations_after = allocation_snapshot();
    let cold_allocation_count = cold_allocations_after
        .0
        .saturating_sub(cold_allocations_before.0);
    let cold_allocated_bytes = cold_allocations_after
        .1
        .saturating_sub(cold_allocations_before.1);
    let mut samples = Vec::with_capacity(500);
    let mut allocation_samples = Vec::with_capacity(500);
    let mut allocated_byte_samples = Vec::with_capacity(500);
    for _ in 0..500 {
        let allocations_before = allocation_snapshot();
        let start = Instant::now();
        std::hint::black_box(complete_marked(&fixture, &fixture.data, marked.clone()));
        samples.push(start.elapsed());
        let allocations_after = allocation_snapshot();
        allocation_samples.push(allocations_after.0.saturating_sub(allocations_before.0));
        allocated_byte_samples.push(allocations_after.1.saturating_sub(allocations_before.1));
    }
    samples.sort_unstable();
    allocation_samples.sort_unstable();
    allocated_byte_samples.sort_unstable();
    let metrics_after = avenger_lang_analysis::SqlCompletionMetrics::snapshot();
    eprintln!(
        "sql completion cold={cold:?} cold_allocations={cold_allocation_count} cold_allocated_bytes={cold_allocated_bytes} warm_p50={:?} warm_p95={:?} warm_p99={:?} warm_allocations_p50={} warm_allocations_p95={} warm_allocated_bytes_p50={} warm_allocated_bytes_p95={} items={} cache_hits={} cache_misses={}",
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100],
        samples[samples.len() * 99 / 100],
        allocation_samples[allocation_samples.len() / 2],
        allocation_samples[allocation_samples.len() * 95 / 100],
        allocated_byte_samples[allocated_byte_samples.len() / 2],
        allocated_byte_samples[allocated_byte_samples.len() * 95 / 100],
        cold_result.items.len(),
        metrics_after
            .cache_hits
            .saturating_sub(metrics_before.cache_hits),
        metrics_after
            .cache_misses
            .saturating_sub(metrics_before.cache_misses),
    );
}

#[test]
fn completion_instrumentation_has_no_execution_path() {
    let metrics = avenger_lang_analysis::SqlCompletionMetrics::snapshot();
    assert_eq!(metrics.physical_plans, 0);
    assert_eq!(metrics.scans, 0);
    assert_eq!(metrics.collects, 0);
    assert_eq!(metrics.executions, 0);
}
