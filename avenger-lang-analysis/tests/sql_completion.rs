use std::{collections::BTreeMap, sync::Arc, time::Instant};

use avenger_lang_analysis::{
    AnalysisCancellation, AnalysisGeneration, AnalysisService, CodeActionRequest,
    CompletionOptions, DocumentRequest, DocumentSnapshot, PositionRequest, SemanticTokenKind,
    SourceRevision, WorkspaceAnalysis, WorkspaceSnapshot, analyze_syntax,
};
use avenger_lang_compiler::Compiler;
use avenger_lang_core::{ByteSpan, InMemorySourceLoader, ModuleRoot, SourceOrigin, SourceSpan};

const CURSOR: &str = "⟦cursor⟧";

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
  data: {{ table: 'vega.movies'; }}
  param float64 as minimum {{ value: 5.0; }}
  param struct(field(utf8, 'label'), field(int64, 'weight')) as config {{
    value: {{ label: 'base'; weight: 2; }}
  }}
  param store as selected {{
    field int64 id;
    field utf8 label;
  }}
  mark symbol as points {{
    x: {expression};
    y: rating;
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
    chart_source("rating").replace(
        "  mark symbol as points {",
        &format!(
            "  transform sql as edited {{\n    query: {query};\n  }}\n  mark symbol as points {{"
        ),
    )
}

fn chart_handler_source(expression: &str) -> String {
    chart_source("rating").replace("    filter: true;", &format!("    filter: {expression};"))
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    // Keep the directory alive for the duration of the test process. The
    // compiler reads through the immutable in-memory snapshot, not disk.
    let _ = Box::leak(Box::new(directory));
    let data = SourceOrigin::File(project_root.join("data.avenger"));
    let chart = SourceOrigin::File(project_root.join("chart.avenger"));
    let data_text = data_source("SELECT id, title, category, rating FROM vega.movies");
    let chart_text = chart_source("rating");
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
    let project_root = std::fs::canonicalize(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-lang-compiler/tests/fixtures/projects")
            .join(relative_project),
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

fn labels(result: &avenger_lang_analysis::CompletionResult) -> Vec<&str> {
    result
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect()
}

#[tokio::test]
async fn select_first_and_from_first_share_qualified_columns() {
    let fixture = fixture().await;
    for query in [
        "SELECT m.⟦cursor⟧ FROM vega.movies AS m",
        "FROM vega.movies AS m SELECT m.⟦cursor⟧",
    ] {
        let result = complete_marked(&fixture, &fixture.data, data_source(query));
        let labels = labels(&result);
        assert_eq!(labels.first().copied(), Some("category"));
        for expected in ["id", "title", "category", "rating"] {
            assert!(labels.contains(&expected), "missing {expected}: {labels:?}");
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
        assert_eq!(rating.replacement.range.start, rating.replacement.range.end);
    }
}

#[tokio::test]
async fn catalog_cte_subquery_and_join_scopes_are_semantic() {
    let fixture = fixture().await;
    let catalog = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT * FROM vega.⟦cursor⟧"),
    );
    assert!(labels(&catalog).contains(&"movies"));

    let cte = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "WITH filtered AS (SELECT title, rating FROM vega.movies) SELECT f.⟦cursor⟧ FROM filtered AS f",
        ),
    );
    assert!(labels(&cte).contains(&"title"));
    assert!(labels(&cte).contains(&"rating"));

    let subquery = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT s.⟦cursor⟧ FROM (SELECT title AS movie_title FROM vega.movies) AS s"),
    );
    assert!(labels(&subquery).contains(&"movie_title"));

    let join = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "FROM vega.movies AS m JOIN vega.ratings AS r ON m.id = r.movie_id SELECT i⟦cursor⟧",
        ),
    );
    let id_insertions = join
        .items
        .iter()
        .filter(|item| item.label == "id")
        .map(|item| item.insert_text.as_str())
        .collect::<Vec<_>>();
    assert!(id_insertions.contains(&"m.id"), "{id_insertions:?}");
    assert!(id_insertions.contains(&"r.id"), "{id_insertions:?}");

    let correlated = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT * FROM vega.movies AS m WHERE EXISTS (SELECT 1 FROM vega.ratings AS r WHERE r.movie_id = m.⟦cursor⟧)",
        ),
    );
    assert!(labels(&correlated).contains(&"id"));
    assert!(labels(&correlated).contains(&"title"));

    let set_output = complete_marked(
        &fixture,
        &fixture.data,
        data_source(
            "SELECT u.⟦cursor⟧ FROM (SELECT title FROM vega.movies UNION ALL SELECT title FROM vega.movies) AS u",
        ),
    );
    assert!(labels(&set_output).contains(&"title"));

    let plans_before = avenger_lang_analysis::SqlCompletionMetrics::snapshot().logical_query_plans;
    let projection_alias = complete_marked(
        &fixture,
        &fixture.data,
        data_source("SELECT rating AS score FROM vega.movies ORDER BY score⟦cursor⟧"),
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
            .contains("Decimal128(2, 1)")
    );
    assert!(
        avenger_lang_analysis::SqlCompletionMetrics::snapshot().logical_query_plans > plans_before
    );
}

#[tokio::test]
async fn exact_pipeline_schema_bindings_functions_and_types_complete() {
    let fixture = fixture().await;
    let expression = complete_marked(&fixture, &fixture.chart, chart_source("rat⟦cursor⟧"));
    assert!(labels(&expression).contains(&"rating"));
    let scalar = complete_marked(&fixture, &fixture.chart, chart_source("$min⟦cursor⟧"));
    assert!(labels(&scalar).contains(&"$minimum"));

    let struct_param = complete_marked(&fixture, &fixture.chart, chart_source("$config.⟦cursor⟧"));
    assert!(labels(&struct_param).contains(&"label"));
    assert!(labels(&struct_param).contains(&"weight"));

    let store = complete_marked(
        &fixture,
        &fixture.chart,
        chart_query_source("SELECT s.⟦cursor⟧ FROM $selected AS s"),
    );
    assert!(labels(&store).contains(&"id"));
    assert!(labels(&store).contains(&"label"));

    let temporal = complete_marked(
        &fixture,
        &fixture.chart,
        chart_handler_source("$minimum@⟦cursor⟧"),
    );
    assert!(labels(&temporal).contains(&"$minimum@start"));
    assert!(labels(&temporal).contains(&"$minimum@previous"));

    let function = complete_marked(&fixture, &fixture.chart, chart_source("ro⟦cursor⟧(rating)"));
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
        chart_source("CAST(rating AS DO⟦cursor⟧)"),
    );
    assert!(labels(&data_type).contains(&"DOUBLE"));
}

#[tokio::test]
async fn event_datum_completion_is_target_aware_and_always_quotes_fields() {
    let fixture = fixture().await;
    let completion = complete_marked(
        &fixture,
        &fixture.chart,
        chart_handler_source("datum.⟦cursor⟧"),
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
            .is_some_and(|detail| detail.contains("Utf8")),
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
        effects.replace("text: item.data.\"name\";", "text: item.data.⟦cursor⟧;"),
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
async fn event_datum_completion_reports_union_coverage_and_type_conflicts() {
    let source = r#"avenger 1;
chart cartesian as chart {
  mark symbol as numeric {
    data: { values: [{ id: 1; only_numeric: 2.0; }]; }
    x: 1;
    y: 1;
  }
  mark symbol as textual {
    data: { values: [{ id: 'one'; only_textual: true; }]; }
    x: 2;
    y: 2;
  }
  on click as inspect {
    filter: datum."id" IS NOT NULL;
  }
}
"#;
    let fixture = chart_only_fixture(source).await;
    let marked = source.replace(r#"datum."id" IS NOT NULL"#, "datum.⟦cursor⟧");
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
            "expressions: sum(am⟦cursor⟧) AS total",
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
    assert!(labels(&projection_alias).contains(&"sum_amount"));
    assert!(!labels(&projection_alias).contains(&"amount"));

    let sql_input = complete_marked(
        &fixture,
        &fixture.chart,
        source.replace(
            "SELECT category, total * 2.0 AS doubled\n        FROM input\n        ORDER BY category",
            "SELECT i.⟦cursor⟧\n        FROM input AS i",
        ),
    );
    assert!(labels(&sql_input).contains(&"category"));
    assert!(labels(&sql_input).contains(&"total"));
    assert!(!labels(&sql_input).contains(&"amount"));
    assert!(!labels(&sql_input).contains(&"doubled"));

    let final_output = complete_marked(
        &fixture,
        &fixture.chart,
        source.replace("y: \"doubled\"", "y: dou⟦cursor⟧"),
    );
    assert!(labels(&final_output).contains(&"doubled"));
    let final_category = complete_marked(
        &fixture,
        &fixture.chart,
        source.replace("y: \"doubled\"", "y: cat⟦cursor⟧"),
    );
    assert!(labels(&final_category).contains(&"category"));
    assert!(!labels(&final_output).contains(&"amount"));
    assert!(!labels(&final_output).contains(&"total"));
}

#[tokio::test]
async fn exact_fingerprint_cache_reuses_authored_analysis_only() {
    let fixture = fixture().await;
    let marked = data_source("FROM vega.movies AS m SELECT m.⟦cursor⟧");
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
    assert!(titles.contains(&"Qualify `id` as `m.id`"), "{titles:?}");
    assert!(titles.contains(&"Qualify `id` as `r.id`"), "{titles:?}");
}

#[tokio::test]
async fn syntax_only_completion_is_immediate_and_reports_incomplete_metadata() {
    let fixture = fixture().await;
    let source = data_source("FROM vega.movies AS m SELECT m.⟦cursor⟧");
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
    let marked = data_source("FROM vega.movies AS m SELECT m.⟦cursor⟧");
    let start = Instant::now();
    std::hint::black_box(complete_marked(&fixture, &fixture.data, marked.clone()));
    let cold = start.elapsed();
    let mut samples = Vec::with_capacity(500);
    for _ in 0..500 {
        let start = Instant::now();
        std::hint::black_box(complete_marked(&fixture, &fixture.data, marked.clone()));
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    eprintln!(
        "sql completion cold={cold:?} warm_p50={:?} warm_p95={:?}",
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100]
    );
}

#[test]
fn completion_instrumentation_has_no_execution_path() {
    let metrics = avenger_lang_analysis::SqlCompletionMetrics::snapshot();
    assert_eq!(metrics.physical_plans, 0);
    assert_eq!(metrics.executions, 0);
}
