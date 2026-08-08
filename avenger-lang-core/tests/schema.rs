use avenger_chart_schema::NativeSchemaSnapshot;
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, ModuleGraphLoadLimits,
    ModuleGraphLoadRequest, ModuleGraphLoader, ModuleRoot, PhysicalType, SourceFile, SourceId,
    SourceOrigin, ast::Value, resolve_module_graph, semantic_json_schema, syntax::parse_file,
};
use std::{fs, path::PathBuf};

fn bootstrap_schema() -> NativeSchemaSnapshot {
    serde_json::from_str(include_str!(
        "../../avenger-chart-lang-registry/snapshots/bootstrap-schema.json"
    ))
    .unwrap()
}

async fn semantic_project(text: &str) -> avenger_lang_core::ParsedModuleGraph {
    let loader = InMemorySourceLoader::default().with_source(LoadedSource::new(
        SourceOrigin::Memory("chart.avenger".to_owned()),
        text,
        ContentVersion::new("fixture-v1"),
    ));
    ModuleGraphLoader::new(&loader)
        .load(ModuleGraphLoadRequest {
            project_root: "/project".into(),
            roots: vec![ModuleRoot::requested(SourceOrigin::Memory(
                "chart.avenger".to_owned(),
            ))],
            native_modules: Default::default(),
            capabilities: ImportCapabilities::in_memory("/project"),
            schema_version: "semantic-v1".to_owned(),
            registry_version: "bootstrap".to_owned(),
            limits: ModuleGraphLoadLimits::default(),
        })
        .await
        .result
        .unwrap()
}

fn type_value(spelling: &str) -> Value {
    let text = format!(
        "avenger 1; chart cartesian as chart {{ store as rows {{ field {spelling} value; }} }}"
    );
    let source = SourceFile::new(
        SourceId::new(1),
        SourceOrigin::Memory("chart.avenger".into()),
        text,
    );
    let parsed = parse_file(&source).unwrap_or_else(|error| {
        panic!("failed to parse physical type spelling {spelling}: {error:?}")
    });
    let chart = parsed.ast.items.into_iter().next().unwrap().declaration;
    chart.children[0].children[0]
        .props
        .get("type")
        .unwrap()
        .clone()
}

fn type_source(spelling: &str) -> SourceFile {
    SourceFile::new(
        SourceId::new(3),
        SourceOrigin::Memory("invalid-type.avenger".into()),
        format!(
            "avenger 1; chart cartesian as chart {{ store as rows {{ field {spelling} value; }} }}"
        ),
    )
}

#[test]
fn schema_physical_arrow_type_corpus_is_canonical_and_recursive() {
    let corpus = [
        "boolean",
        "int8",
        "int16",
        "int32",
        "int64",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "float16",
        "float32",
        "float64",
        "utf8",
        "large_utf8",
        "binary",
        "large_binary",
        "date32",
        "date64",
        "time32(second)",
        "time32(millisecond)",
        "time64(microsecond)",
        "time64(nanosecond)",
        "timestamp(microsecond,'America/New_York')",
        "duration(nanosecond)",
        "interval(month_day_nano)",
        "fixed_size_binary(16)",
        "decimal128(38,-2)",
        "decimal256(76,12)",
        "list(float64)",
        "large_list(utf8)",
        "fixed_size_list(float32,4)",
        "struct()",
        "struct(field(struct(field(float64,'x'),field(float64,'y')),'position'),field(list(utf8),'labels'))",
        "map(utf8,list(int64))",
    ];
    for spelling in corpus {
        let value = type_value(spelling);
        let parsed = PhysicalType::parse(&value).unwrap_or_else(|error| {
            panic!("failed to parse {spelling}: {error}");
        });
        assert_eq!(parsed.to_string(), spelling);
    }
}

#[test]
fn schema_physical_arrow_types_reject_aliases_shapes_and_duplicate_fields() {
    for invalid in [
        "double",
        "varchar",
        "array(int64)",
        "time32(nanosecond)",
        "time64(second)",
        "fixed_size_binary(0)",
        "decimal128(39,0)",
        "decimal256(77,0)",
        "list()",
        "fixed_size_list(int64,-1)",
        "struct(field(int64, ''))",
        "struct(field(int64, 'x'),field(utf8, 'x'))",
        "struct(int64)",
        "map(utf8)",
    ] {
        assert!(
            parse_file(&type_source(invalid)).is_err(),
            "unexpectedly accepted {invalid}"
        );
    }
}

#[tokio::test]
async fn schema_generated_bootstrap_corpus_agrees_with_semantic_validation() {
    let registry = bootstrap_schema();
    let schema = semantic_json_schema(&registry, "bootstrap-test-profile");
    let validator = jsonschema::validator_for(&schema).expect("generated schema compiles");
    let corpus = [
        (
            true,
            r#"avenger 1; chart cartesian as chart {
                mark symbol as dots { x: encoded "x"; y: encoded "y"; }
            }"#,
        ),
        (
            true,
            r#"avenger 1; chart cartesian as chart {
                mark symbol as dots { x: encoded "x"; }
            }"#,
        ),
        (
            true,
            r#"avenger 1; chart cartesian as chart {
                mark symbol as dots {
                    fill: direct '#94a3b8' {
                        when { predicate: "selected"; encoded: "kind"; }
                        otherwise: { direct: '#2563eb'; }
                    }
                }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                mark symbol as dots {
                    fill: direct '#94a3b8' {
                        when { direct: '#2563eb'; }
                    }
                }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                mark symbol as dots {
                    fill: direct '#94a3b8' {
                        when { predicate: true; encoded: "kind"; direct: '#2563eb'; }
                    }
                }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                mark symbol as dots {
                    fill: direct '#94a3b8' {
                        otherwise: { predicate: true; direct: '#2563eb'; }
                    }
                }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                mark symbol as dots {
                    fill: direct '#94a3b8' {
                        mark symbol { fill: direct '#2563eb'; }
                    }
                }
            }"#,
        ),
        (
            true,
            r#"avenger 1; chart cartesian as chart {
                selection as picked { empty: none; combine: union; }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                selection as picked { empty: maybe; combine: union; }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                mark symbol as dots { x: encoded "x"; y: encoded "y"; bogus: 1; }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                on invented_event as handler {}
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                widget radio_button_list as choice {
                    position: top;
                }
            }"#,
        ),
        (
            true,
            r#"avenger 1; chart cartesian as chart {
                widget radio_button_list as choice {
                    data: { values: [{ value: 1; label: 'one'; }]; }
                    position: top;
                }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                widget radio_button_list as choice {
                    data: { values: [{ value: 1; label: 'one'; }]; }
                    position: top;
                    mark symbol { x: encoded "x"; y: encoded "y"; }
                }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                widget radio_button_list as choice {
                    data: { values: [{ value: 1; label: 'one'; }]; }
                    position: center;
                }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                param 1 as limit { mark symbol { x: encoded "x"; y: encoded "y"; } }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                selection as picked { mark symbol { x: encoded "x"; y: encoded "y"; } }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                store as rows { primary_key: []; field utf8 id; }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                mark group { view cartesian as viewport { export child; } }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                on click { mark symbol { x: encoded "x"; y: encoded "y"; } }
            }"#,
        ),
    ];
    for (expected, source) in corpus {
        let project = semantic_project(source).await;
        let semantic_valid = resolve_module_graph(&project, &registry).result.is_ok();
        let file = project
            .source_modules
            .values()
            .next()
            .expect("one source file");
        let instance = serde_json::to_value(&file.parsed.ast).unwrap();
        let schema_valid = validator.is_valid(&instance);
        assert_eq!(semantic_valid, expected, "semantic result for {source}");
        assert_eq!(schema_valid, expected, "JSON Schema result for {source}");
    }

    let missing_value = semantic_project(
        r#"avenger 1; chart cartesian as chart {
            param 1 as limit;
        }"#,
    )
    .await;
    let file = missing_value
        .source_modules
        .values()
        .next()
        .expect("one source file");
    let mut instance = serde_json::to_value(&file.parsed.ast).unwrap();
    instance["items"][0]["declaration"]["children"][0]["props"]
        .as_object_mut()
        .unwrap()
        .remove("value");
    assert!(
        !validator.is_valid(&instance),
        "the interchange schema must require a scalar param initializer"
    );

    let second = semantic_json_schema(&registry, "bootstrap-test-profile");
    assert_eq!(
        schema, second,
        "semantic schema generation is deterministic"
    );
}

#[test]
fn schema_generated_bootstrap_snapshot_is_reviewed() {
    let schema = semantic_json_schema(&bootstrap_schema(), "bootstrap-test-profile");
    let actual = format!("{}\n", serde_json::to_string_pretty(&schema).unwrap());
    let baseline = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/baselines/schema/bootstrap-semantic-schema.json");
    if std::env::var_os("AVENGER_LANG_UPDATE_BASELINES").is_some() {
        fs::create_dir_all(baseline.parent().unwrap()).unwrap();
        fs::write(&baseline, actual).unwrap();
    } else {
        let expected = fs::read_to_string(&baseline).unwrap_or_else(|error| {
            panic!(
                "failed to read reviewed baseline {}: {error}",
                baseline.display()
            )
        });
        assert_eq!(actual, expected, "generated semantic schema changed");
    }
}

#[test]
fn interchange_name_schema_matches_unicode_identifier_contract() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schemas/ast-core-1.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).expect("AST schema compiles");
    let source = SourceFile::new(
        SourceId::new(4),
        SourceOrigin::Memory("unicode.avenger".into()),
        "avenger 1; chart cartesian as café_2 {}",
    );
    let parsed = parse_file(&source).unwrap();
    let mut instance = serde_json::to_value(&parsed.ast).unwrap();
    assert!(validator.is_valid(&instance));
    let mut legacy_group_ref = instance.clone();
    legacy_group_ref["items"][0]["declaration"]["props"] = serde_json::json!({
        "target": {
            "ref": {
                "kind": "group",
                "path": ["layers"]
            }
        }
    });
    assert!(
        !validator.is_valid(&legacy_group_ref),
        "mark groups use the ordinary mark reference category"
    );
    instance["items"][0]["declaration"]["name"] = serde_json::Value::String("bad$name".to_owned());
    assert!(!validator.is_valid(&instance));

    let invalid_top_level = serde_json::json!({
        "version": 1,
        "items": [{
            "exported": false,
            "declaration": {
                "decl": "param",
                "name": "value",
                "props": {
                    "type": { "atom": "float64" },
                    "value": { "num": "1" }
                }
            }
        }]
    });
    assert!(!validator.is_valid(&invalid_top_level));
}
