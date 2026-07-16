use avenger_lang_core::{
    PhysicalType, SourceFile, SourceId, SourceOrigin,
    ast::{Root, Value},
    syntax::parse_file,
};

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
