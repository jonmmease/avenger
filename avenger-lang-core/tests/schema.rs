use avenger_chart_schema::NativeSchemaSnapshot;
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, PhysicalType,
    ProjectLoadLimits, ProjectLoadRequest, ProjectLoader, ProjectRoot, SourceFile, SourceId,
    SourceOrigin,
    ast::{Root, Value},
    resolve_project, semantic_json_schema,
    syntax::parse_file,
};
use std::{fs, path::PathBuf};

fn bootstrap_schema() -> NativeSchemaSnapshot {
    serde_json::from_str(include_str!(
        "../../avenger-chart-lang-registry/snapshots/bootstrap-schema.json"
    ))
    .unwrap()
}

async fn semantic_project(text: &str) -> avenger_lang_core::ParsedProject {
    let loader = InMemorySourceLoader::default().with_source(LoadedSource::new(
        SourceOrigin::Memory("chart.avenger".to_owned()),
        text,
        ContentVersion::new("fixture-v1"),
    ));
    ProjectLoader::new(&loader)
        .load(ProjectLoadRequest {
            project_root: "/project".into(),
            roots: vec![ProjectRoot::chart(SourceOrigin::Memory(
                "chart.avenger".to_owned(),
            ))],
            capabilities: ImportCapabilities::in_memory("/project"),
            schema_version: "semantic-v1".to_owned(),
            registry_version: "bootstrap".to_owned(),
            limits: ProjectLoadLimits::default(),
        })
        .await
        .result
        .unwrap()
}

fn type_value(spelling: &str) -> Value {
    let text = format!(
        "avenger 1; chart cartesian as chart {{ param as value {{ type: {spelling}; default: NULL; }} }}"
    );
    let source = SourceFile::new(
        SourceId::new(1),
        SourceOrigin::Memory("chart.avenger".into()),
        text,
    );
    let parsed = parse_file(&source).unwrap_or_else(|error| {
        panic!("failed to parse physical type spelling {spelling}: {error:?}")
    });
    let Root::Chart(chart) = parsed.ast.root else {
        panic!("chart root")
    };
    chart.children[0].props.get("type").unwrap().clone()
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
        "struct(field('position',struct(field('x',float64),field('y',float64))),field('labels',list(utf8)))",
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
        "struct(field('',int64))",
        "struct(field('x',int64),field('x',utf8))",
        "struct(int64)",
        "map(utf8)",
    ] {
        assert!(
            PhysicalType::parse(&type_value(invalid)).is_err(),
            "unexpectedly accepted {invalid}"
        );
    }
}

#[test]
fn schema_destination_typed_literals_are_exact() {
    let int8 = PhysicalType::parse(&type_value("int8")).unwrap();
    let decimal = PhysicalType::parse(&type_value("decimal128(5,2)")).unwrap();
    let null = type_value("int64");
    assert!(int8.accepts_literal(&number("127")).is_ok());
    assert!(int8.accepts_literal(&number("128")).is_err());
    assert!(decimal.accepts_literal(&number("123.45")).is_ok());
    assert!(decimal.accepts_literal(&number("1234.56")).is_err());
    assert!(PhysicalType::Int64.accepts_literal(&Value::Null).is_ok());
    assert!(PhysicalType::parse(&null).is_ok());
}

#[test]
fn schema_recursive_list_and_struct_literals_use_destination_types() {
    let list = PhysicalType::parse(&type_value("fixed_size_list(int8,2)")).unwrap();
    assert!(
        list.accepts_literal(&Value::Array(vec![number("1"), number("127")]))
            .is_ok()
    );
    assert!(
        list.accepts_literal(&Value::Array(vec![number("1")]))
            .is_err()
    );

    let data_type = PhysicalType::parse(&type_value(
        "struct(field('x',float64),field('labels',list(utf8)))",
    ))
    .unwrap();
    let value = object(&[("x", number("1.25")), ("labels", strings(&["a", "b"]))]);
    assert!(data_type.accepts_literal(&value).is_ok());
    let invalid = object(&[("x", Value::Str("not a number".to_owned()))]);
    assert!(data_type.accepts_literal(&invalid).is_err());
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
                mark symbol as dots { x: "x"; y: "y"; }
            }"#,
        ),
        (
            true,
            r#"avenger 1; chart cartesian as chart {
                mark symbol as dots { x: "x"; }
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
                mark symbol as dots { x: "x"; y: "y"; bogus: 1; }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                param as limit { type: int64; }
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
                    mark symbol { x: "x"; y: "y"; }
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
                param as limit { type: int64; default: 1; mark symbol { x: "x"; y: "y"; } }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                selection as picked { mark symbol { x: "x"; y: "y"; } }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                store as rows { primary_key: []; field id: utf8; }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                group { view cartesian as viewport { export child; } }
            }"#,
        ),
        (
            false,
            r#"avenger 1; chart cartesian as chart {
                on click { mark symbol { x: "x"; y: "y"; } }
            }"#,
        ),
    ];
    for (expected, source) in corpus {
        let project = semantic_project(source).await;
        let semantic_valid = resolve_project(&project, &registry).result.is_ok();
        let file = project.files.values().next().expect("one source file");
        let instance = serde_json::to_value(&file.parsed.ast).unwrap();
        let schema_valid = validator.is_valid(&instance);
        assert_eq!(semantic_valid, expected, "semantic result for {source}");
        assert_eq!(schema_valid, expected, "JSON Schema result for {source}");
    }

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

fn number(spelling: &str) -> Value {
    let text = format!(
        "avenger 1; chart cartesian as chart {{ param as value {{ type: int64; default: {spelling}; }} }}"
    );
    let source = SourceFile::new(
        SourceId::new(2),
        SourceOrigin::Memory("literal.avenger".into()),
        text,
    );
    let parsed = parse_file(&source).unwrap();
    let Root::Chart(chart) = parsed.ast.root else {
        panic!("chart root")
    };
    chart.children[0].props.get("default").unwrap().clone()
}

fn strings(values: &[&str]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| Value::Str((*value).to_owned()))
            .collect(),
    )
}

fn object(fields: &[(&str, Value)]) -> Value {
    let mut body = avenger_lang_core::ast::Body::default();
    for (name, value) in fields {
        body.props
            .insert(
                avenger_lang_core::ast::Name::new(*name).unwrap(),
                value.clone(),
            )
            .unwrap();
    }
    Value::Block { head: None, body }
}
