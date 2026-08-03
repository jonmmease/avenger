use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use arrow::datatypes::{DataType, Field, Schema};
use avenger_lang_compiler::{CompileFailure, Compiler, DatasetStageKind};
use avenger_lang_core::{
    ContentVersion, InMemorySourceLoader, LoadedSource, SourceLoader, SourceOrigin, ast::SqlQuery,
};
use datafusion::{datasource::MemTable, prelude::SessionContext};

fn project_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

async fn data_failure(data: &str) -> CompileFailure {
    let declarations = data
        .trim()
        .strip_prefix("avenger 1;")
        .expect("diagnostic fixture starts with the language version");
    let data = format!("avenger 1; schema tables as test {{ {declarations} }}");
    let loader = InMemorySourceLoader::default().with_source(LoadedSource::new(
        SourceOrigin::File("/project/data.avenger".into()),
        data,
        ContentVersion::new("data-v1"),
    ));
    Compiler::builder()
        .project_root("/project")
        .source_loader(Arc::new(loader) as Arc<dyn SourceLoader>)
        .build()
        .unwrap()
        .analyze_module("data.avenger")
        .await
        .unwrap_err()
}

#[tokio::test]
async fn data_project_propagates_exact_schemas_through_a_multi_query_dag_without_execution() {
    let root = project_fixture("phase9-schema-chain");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_module("chart.avenger").await.unwrap();
    let tables = analysis
        .datasets
        .iter()
        .filter(|(_, dataset)| {
            matches!(
                dataset.provenance.stage_kind,
                DatasetStageKind::CatalogTable | DatasetStageKind::SqlView
            )
        })
        .filter_map(|(_, dataset)| {
            dataset
                .qualified_name
                .as_ref()
                .map(|name| (name.clone(), dataset))
        })
        .collect::<BTreeMap<_, _>>();

    assert_eq!(tables.len(), 6);
    let joined = tables["analytics.joined"];
    assert_eq!(
        joined
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["row_id", "category", "amount", "meta", "label"]
    );
    assert!(matches!(joined.columns[3].data_type, DataType::Struct(_)));
    assert!(
        joined.columns[4].nullable,
        "LEFT JOIN must nullable-extend label"
    );

    let adjusted = tables["analytics.adjusted"];
    assert_eq!(
        adjusted
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        [
            "row_id",
            "group_name",
            "amount",
            "adjusted_amount",
            "meta",
            "label",
        ]
    );
    assert!(matches!(adjusted.columns[4].data_type, DataType::Struct(_)));
    assert_eq!(adjusted.columns[3].data_type, DataType::Int64);
    assert!(adjusted.columns[5].nullable);

    let summarized = tables["analytics.summarized"];
    assert_eq!(
        summarized
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["group_name", "row_count", "total"]
    );
    assert_eq!(summarized.columns[1].data_type, DataType::Int64);
    assert_eq!(summarized.columns[2].data_type, DataType::Int64);
    assert!(summarized.columns[2].nullable);

    let final_table = tables["analytics.final"];
    assert_eq!(
        final_table
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["category", "total", "doubled"]
    );
    assert_eq!(final_table.columns[2].data_type, DataType::Int64);

    for (name, upstream_count) in [
        ("analytics.joined", 2),
        ("analytics.adjusted", 1),
        ("analytics.summarized", 1),
        ("analytics.final", 1),
    ] {
        let stage = &tables[name].stage;
        assert_eq!(
            analysis.lineage.get(stage).unwrap().upstream_stages.len(),
            upstream_count,
            "direct lineage for {name}"
        );
    }

    let artifact = compiler.compile_chart("chart.avenger", None).await.unwrap();
    assert_eq!(
        artifact.dependency_fingerprint.as_str(),
        analysis.dependency_fingerprints.charts[&artifact.id].as_str(),
        "analysis and full compilation must consume the same data snapshot"
    );
    assert_eq!(artifact.compiled_plot().marks().len(), 1);
}

#[tokio::test]
async fn data_project_reports_catalog_dag_argument_capability_and_option_failures_precisely() {
    let cases = [
        (
            "unknown table",
            r#"avenger 1;
               table sql as derived { sql: SELECT * FROM missing; }"#,
            "AVENGER-DATA-106",
        ),
        (
            "dependency cycle",
            r#"avenger 1;
               table sql as a { sql: SELECT * FROM b; }
               table sql as b { sql: SELECT * FROM a; }"#,
            "AVENGER-RESOLVE-101",
        ),
        (
            "FROM-first query without SELECT",
            r#"avenger 1;
               table inline as rows { values: [{ value: 1; }]; }
               table sql as invalid { sql: FROM rows; }"#,
            "AVENGER-SQL-009",
        ),
        (
            "duplicate table path",
            r#"avenger 1;
               table inline as rows { values: [{ value: 1; }]; }
               table inline as rows { values: [{ value: 2; }]; }"#,
            "AVENGER-RESOLVE-102",
        ),
        (
            "positional table argument",
            r#"avenger 1;
               table inline as rows { values: [{ value: 1; }]; }
               table sql as filtered {
                 param 0 as minimum;
                 sql: SELECT * FROM rows WHERE value >= $minimum;
               }
               table sql as use_filtered { sql: SELECT * FROM filtered(1); }"#,
            "AVENGER-DATA-050",
        ),
        (
            "unknown named table argument",
            r#"avenger 1;
               table inline as rows { values: [{ value: 1; }]; }
               table sql as filtered {
                 param 0 as minimum;
                 sql: SELECT * FROM rows WHERE value >= $minimum;
               }
               table sql as use_filtered {
                 sql: SELECT * FROM filtered(other => 1);
               }"#,
            "AVENGER-DATA-050",
        ),
        (
            "denied HTTP access",
            r#"avenger 1;
               table csv as rows { path: 'https://example.invalid/rows.csv'; }"#,
            "AVENGER-DATA-041",
        ),
        (
            "denied object-store access",
            r#"avenger 1;
               table parquet as rows { path: 's3://bucket/rows.parquet'; }"#,
            "AVENGER-DATA-041",
        ),
        (
            "unavailable table provider",
            r#"avenger 1;
               table delta as rows { uri: 's3://bucket/table'; }"#,
            "AVENGER-DATA-033",
        ),
    ];

    for (name, data, expected) in cases {
        let failure = data_failure(data).await;
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_str() == expected),
            "{name} should report {expected}, got {:?}",
            failure
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn data_project_validates_file_options_before_provider_planning() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "avenger-lang-data-options-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("rows.csv"), "value\n1\n").unwrap();
    fs::write(
        root.join("chart.avenger"),
        "avenger 1; chart cartesian as chart {}",
    )
    .unwrap();

    for (options, expected) in [
        ("has_header: 'yes';", "AVENGER-DATA-045"),
        ("invented: true;", "AVENGER-DATA-046"),
    ] {
        fs::write(
            root.join("data.avenger"),
            format!(
                "avenger 1; schema tables as test {{ \
                 table csv as rows {{ path: 'rows.csv'; options: {{ {options} }} }} }}"
            ),
        )
        .unwrap();
        let failure = Compiler::builder()
            .project_root(&root)
            .build()
            .unwrap()
            .analyze_module(root.join("data.avenger"))
            .await
            .unwrap_err();
        assert_eq!(failure.diagnostics[0].code.as_str(), expected);
    }
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn data_project_sql_frontend_corpus_reaches_the_datafusion_planning_boundary() {
    let context = SessionContext::new();
    let movies = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("title", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, true),
        Field::new("rating", DataType::Float64, true),
    ]));
    let ratings = Arc::new(Schema::new(vec![
        Field::new("movie_id", DataType::Int64, false),
        Field::new("value", DataType::Float64, true),
    ]));
    context
        .register_table(
            "movies",
            Arc::new(MemTable::try_new(movies, vec![vec![]]).unwrap()),
        )
        .unwrap();
    context
        .register_table(
            "ratings",
            Arc::new(MemTable::try_new(ratings, vec![vec![]]).unwrap()),
        )
        .unwrap();

    let corpus = [
        "SELECT m.\"title\", m.\"rating\" FROM movies AS m WHERE m.\"rating\" > 0",
        "FROM movies AS m SELECT m.\"title\", m.\"rating\" WHERE m.\"rating\" > 0",
        "WITH filtered AS (SELECT * FROM movies WHERE \"rating\" > 0) \
         SELECT \"title\" FROM filtered",
        "SELECT \"category\", sum(\"rating\") AS total FROM movies GROUP BY \"category\"",
        "SELECT m.\"title\", avg(r.\"value\") AS mean_rating FROM movies AS m \
         LEFT JOIN ratings AS r ON m.\"id\" = r.\"movie_id\" GROUP BY m.\"title\"",
        "SELECT \"title\" FROM movies AS m WHERE EXISTS \
         (SELECT 1 FROM ratings AS r WHERE r.\"movie_id\" = m.\"id\")",
        "SELECT \"category\" FROM movies UNION ALL SELECT \"category\" FROM movies",
        "VALUES (1, 'one'), (2, 'two')",
        "SELECT \"title\", row_number() OVER \
         (PARTITION BY \"category\" ORDER BY \"rating\") AS ordinal FROM movies",
        "SELECT nested.\"title\" FROM (SELECT \"title\" FROM movies) AS nested",
    ];

    for source in corpus {
        let canonical = SqlQuery::parse(source).unwrap().canonical_sql();
        let planned = context.sql(&canonical).await.unwrap_or_else(|error| {
            panic!("frontend-accepted SQL did not plan: {canonical}\n{error}")
        });
        assert!(!planned.schema().fields().is_empty(), "{canonical}");
    }
}

#[tokio::test]
async fn data_project_advanced_sql_forms_match_datafusion_54() {
    let context = SessionContext::new();
    let movies = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("title", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, true),
        Field::new("rating", DataType::Float64, true),
    ]));
    context
        .register_table(
            "movies",
            Arc::new(MemTable::try_new(movies, vec![vec![]]).unwrap()),
        )
        .unwrap();

    let accepted = [
        // Recursive CTEs are visible to their recursive term only when the
        // query explicitly opts into recursive semantics.
        "WITH RECURSIVE seq AS (SELECT 1 AS n UNION ALL \
         SELECT \"n\" + 1 AS n FROM seq WHERE \"n\" < 3) SELECT \"n\" FROM seq",
        // A lateral derived relation may correlate to relations on its left.
        "SELECT m.\"title\", d.\"score\" FROM movies AS m CROSS JOIN LATERAL \
         (SELECT m.\"rating\" * 2 AS score) AS d",
        "SELECT row_number() OVER ratings AS ordinal FROM movies \
         WINDOW ratings AS (PARTITION BY \"category\" ORDER BY \"rating\")",
        "SELECT * EXCLUDE (\"category\") FROM movies",
        "SELECT * EXCEPT (\"category\") FROM movies",
        "SELECT * REPLACE (\"rating\" * 2 AS rating) FROM movies",
        "SELECT \"title\", row_number() OVER (ORDER BY \"rating\") AS ordinal \
         FROM movies QUALIFY row_number() OVER (ORDER BY \"rating\") = 1",
    ];

    for source in accepted {
        let canonical = SqlQuery::parse(source).unwrap().canonical_sql();
        let planned = context.sql(&canonical).await.unwrap_or_else(|error| {
            panic!("frontend-accepted advanced SQL did not plan: {canonical}\n{error}")
        });
        assert!(!planned.schema().fields().is_empty(), "{canonical}");
    }

    for (source, expected) in [
        (
            "SELECT \"title\" FROM movies QUALIFY \"rating\" > 0",
            "QUALIFY clause requires window functions",
        ),
        (
            "SELECT \"category\", count(*) AS count FROM movies GROUP BY \"category\" \
             QUALIFY row_number() OVER () > 0 AND \"rating\" > 0",
            "must appear in the GROUP BY clause or must be part of an aggregate function",
        ),
    ] {
        let canonical = SqlQuery::parse(source).unwrap().canonical_sql();
        let error = context
            .sql(&canonical)
            .await
            .expect_err("query must reach the documented DataFusion QUALIFY restriction");
        assert!(
            error.to_string().contains(expected),
            "unexpected error for {canonical}: {error}"
        );
    }
}
