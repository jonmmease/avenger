use avenger_lang_core::{
    ContentVersion, ExpansionLimits, ImportCapabilities, InMemorySourceLoader, LoadedSource,
    ModuleGraphLoadLimits, ModuleGraphLoadRequest, ModuleGraphLoader, ModuleRoot, SourceFile,
    SourceId, SourceOrigin, expand_project_with_limits, resolve_project,
    sql::SqlParseLimits,
    syntax::{SyntaxLimits, parse_file_with_limits},
};

fn source(text: &str) -> SourceFile {
    SourceFile::new(
        SourceId::new(1),
        SourceOrigin::Memory("limits.avenger".into()),
        text,
    )
}

#[test]
fn syntax_limits_bound_tokens_nesting_and_declarations() {
    let error = parse_file_with_limits(
        &source("avenger 1; chart cartesian {}"),
        SyntaxLimits {
            max_tokens: 1,
            ..SyntaxLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.diagnostic().code.as_str(), "AVENGER-PARSE-022");

    let error = parse_file_with_limits(
        &source("avenger 1; chart cartesian { mark group { mark symbol {} } }"),
        SyntaxLimits {
            max_nesting_depth: 1,
            ..SyntaxLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.diagnostic().code.as_str(), "AVENGER-PARSE-023");

    let error = parse_file_with_limits(
        &source("avenger 1; chart cartesian {}"),
        SyntaxLimits {
            max_declarations: 0,
            ..SyntaxLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.diagnostic().code.as_str(), "AVENGER-PARSE-024");
}

#[test]
fn syntax_limits_bound_sql_tokens_and_recursion() {
    let error = parse_file_with_limits(
        &source("avenger 1; chart cartesian { param int64 as value { value: 1 + 2 + 3; } }"),
        SyntaxLimits {
            sql: SqlParseLimits {
                max_tokens: 2,
                ..SqlParseLimits::default()
            },
            ..SyntaxLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.diagnostic().code.as_str(), "AVENGER-SQL-010");

    let error = parse_file_with_limits(
        &source("avenger 1; chart cartesian { param int64 as value { value: (((((1))))); } }"),
        SyntaxLimits {
            sql: SqlParseLimits {
                max_recursion_depth: 1,
                ..SqlParseLimits::default()
            },
            ..SyntaxLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.diagnostic().code.as_str(), "AVENGER-SQL-011");
}

#[tokio::test]
async fn expansion_limits_bound_declarations_depth_and_output() {
    let chart = SourceOrigin::Memory("chart.avenger".into());
    let definition = SourceOrigin::Memory("badge.mark.avenger".into());
    let loader = InMemorySourceLoader::default()
        .with_source(LoadedSource::new(
            chart.clone(),
            "avenger 1; import { badge } from './badge.mark.avenger'; chart cartesian as chart { mark badge as badge {} }",
            ContentVersion::new("limits-v1"),
        ))
        .with_source(LoadedSource::new(
            definition,
            "avenger 1; define mark badge { mark symbol {} }",
            ContentVersion::new("limits-v1"),
        ));
    let project = ModuleGraphLoader::new(&loader)
        .load(ModuleGraphLoadRequest {
            project_root: "/project".into(),
            roots: vec![ModuleRoot::requested(chart)],
            native_modules: Default::default(),
            capabilities: ImportCapabilities::in_memory("/project"),
            schema_version: "semantic-v1".into(),
            registry_version: "bootstrap".into(),
            limits: ModuleGraphLoadLimits::default(),
        })
        .await
        .result
        .unwrap();
    let schema = serde_json::from_str(include_str!(
        "../../avenger-chart-lang-registry/snapshots/bootstrap-schema.json"
    ))
    .unwrap();
    let resolved = resolve_project(&project, &schema).result.unwrap();

    for (limits, code) in [
        (
            ExpansionLimits {
                max_declarations: 0,
                ..ExpansionLimits::default()
            },
            "AVENGER-EXPAND-005",
        ),
        (
            ExpansionLimits {
                max_depth: 0,
                ..ExpansionLimits::default()
            },
            "AVENGER-EXPAND-006",
        ),
        (
            ExpansionLimits {
                max_output_bytes_per_chart: 1,
                ..ExpansionLimits::default()
            },
            "AVENGER-EXPAND-007",
        ),
        (
            ExpansionLimits {
                max_total_output_bytes: 1,
                ..ExpansionLimits::default()
            },
            "AVENGER-EXPAND-008",
        ),
    ] {
        let failure = expand_project_with_limits(&project, &resolved, limits).unwrap_err();
        assert_eq!(failure.diagnostics[0].code.as_str(), code);
    }
}
