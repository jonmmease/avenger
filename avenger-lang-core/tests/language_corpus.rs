use std::{
    fs,
    path::{Path, PathBuf},
};

use avenger_lang_core::{
    SourceFile, SourceId, SourceOrigin,
    ast::{
        BindingKind, BindingTime, Body, Decl, File, ModuleItem, Name, NumericLiteral, PropertyMap,
        RefKind, SqlExpression, SqlQuery, Value,
    },
    interchange::{canonical_json, decode_json, encode_json},
    print::print_file,
    syntax::{format_source, parse_file},
};
use sqlparser::ast::{SelectFlavor, SetExpr};

const VALID_FIXTURES: &[&str] = &[
    "chart.avenger",
    "define-mark.avenger",
    "define-tool.avenger",
    "define-transform.avenger",
    "catalog.avenger",
    "multi-chart.avenger",
    "dedicated-shapes.avenger",
    "query-entry.avenger",
    "selection-payloads.avenger",
    "state-actions.avenger",
    "headed-blocks.avenger",
];

const INVALID_FIXTURES: &[&str] = &[
    "missing-delimiter.avenger",
    "multiple-statements.avenger",
    "legacy-import.avenger",
    "late-import.avenger",
    "empty-module.avenger",
    "import-only.avenger",
    "malformed-action.avenger",
    "illegal-visibility.avenger",
    "define-widget.avenger",
    "invalid-slot-shape.avenger",
    "late-interface.avenger",
    "wrong-root.avenger",
    "id-declaration.avenger",
];

fn fixture(relative: impl AsRef<Path>) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/parse")
        .join(relative)
}

fn baseline(relative: impl AsRef<Path>) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/baselines/parse")
        .join(relative)
}

fn loaded(path: &Path, id: u32) -> SourceFile {
    SourceFile::new(
        SourceId::new(id),
        SourceOrigin::File(path.to_owned()),
        fs::read_to_string(path).unwrap(),
    )
}

fn only_declaration(parsed: &avenger_lang_core::syntax::ParsedFile) -> &Decl {
    let [item] = parsed.ast.items.as_slice() else {
        panic!("expected exactly one module item")
    };
    &item.declaration
}

fn assert_baseline(path: &Path, actual: &str) {
    if std::env::var_os("AVENGER_LANG_UPDATE_BASELINES").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, actual).unwrap();
        eprintln!("updated {}", path.display());
    } else {
        let expected = fs::read_to_string(path).unwrap_or_else(|error| {
            panic!(
                "failed to read reviewed baseline {}: {error}",
                path.display()
            )
        });
        assert_eq!(actual, expected, "baseline changed: {}", path.display());
    }
}

#[test]
fn parse_valid_corpus_has_stable_ast_and_print_baselines() {
    for (index, name) in VALID_FIXTURES.iter().enumerate() {
        let path = fixture(name);
        let parsed = parse_file(&loaded(&path, index as u32 + 1)).unwrap_or_else(|error| {
            panic!(
                "fixture {name} failed: {error} at {:?}: {}",
                error.diagnostic().primary.span.range,
                error.diagnostic().primary.message
            )
        });
        let printed = print_file(&parsed.ast);
        let json = canonical_json(&parsed.ast).unwrap();
        assert_baseline(
            &baseline(Path::new(name).with_extension("ast.json")),
            &(json + "\n"),
        );
        assert_baseline(
            &baseline(Path::new(name).with_extension("printed.avenger")),
            &printed,
        );

        let reparsed = parse_file(&SourceFile::new(
            SourceId::new(100 + index as u32),
            SourceOrigin::Memory(format!("printed-{name}")),
            printed.clone(),
        ))
        .unwrap();
        assert_eq!(
            parsed.ast, reparsed.ast,
            "semantic print round trip: {name}"
        );
        let formatted = format_source(&loaded(&path, index as u32 + 200)).unwrap();
        assert_eq!(
            formatted, printed,
            "comment-stripped valid corpus should share semantic layout: {name}"
        );
        assert_eq!(
            formatted,
            format_source(&SourceFile::new(
                SourceId::new(300 + index as u32),
                SourceOrigin::Memory(format!("formatted-{name}")),
                formatted.clone(),
            ))
            .unwrap()
        );
    }
}

#[test]
fn parse_invalid_corpus_has_stable_diagnostics() {
    for (index, name) in INVALID_FIXTURES.iter().enumerate() {
        let path = fixture(format!("invalid/{name}"));
        let error = parse_file(&loaded(&path, index as u32 + 1)).unwrap_err();
        let diagnostic = error.diagnostic();
        let snapshot = format!(
            "{}\n{}\n{}..{}\n",
            diagnostic.code.as_str(),
            diagnostic.message,
            diagnostic.primary.span.range.start,
            diagnostic.primary.span.range.end,
        );
        assert_baseline(
            &baseline(format!("invalid/{}.txt", name.trim_end_matches(".avenger"))),
            &snapshot,
        );
    }
}

#[test]
fn ast_json_round_trips_every_value_variant() {
    let values = vec![
        Value::Str("text".into()),
        Value::Num(NumericLiteral::new("9007199254740993").unwrap()),
        Value::Bool(true),
        Value::Null,
        Value::Column("column".into()),
        Value::Atom(name("median")),
        Value::Expr(Box::new(SqlExpression::parse("1 + 2").unwrap())),
        Value::Projection(Box::new(
            avenger_lang_core::ast::SqlProjection::parse(
                "sum(amount) AS total, avg(amount) AS average",
            )
            .unwrap(),
        )),
        Value::Query(Box::new(SqlQuery::parse("SELECT * FROM movies").unwrap())),
        Value::Binding {
            kind: BindingKind::Param,
            path: vec![name("width")],
            time: BindingTime::Previous,
        },
        Value::Ref {
            kind: RefKind::Mark,
            path: vec![name("layers"), name("points")],
        },
        Value::Channel {
            mode: avenger_lang_core::ast::ChannelMode::Direct,
            expression: Box::new(Value::Num(NumericLiteral::new("4").unwrap())),
        },
        Value::Dim(vec![name("pixels"), name("x_dim")]),
        Value::Pattern(Box::new(Value::Block {
            head: None,
            body: Body::default(),
        })),
        Value::Env("TOKEN".into()),
        Value::None,
        Value::Array(vec![Value::Bool(false)]),
        Value::Block {
            head: Some(Box::new(Value::Atom(name("linear")))),
            body: Body::default(),
        },
        Value::Call {
            function: name("list"),
            args: vec![Value::Atom(name("float64"))],
        },
    ];
    for value in values {
        let encoded = serde_json::to_string(&value).unwrap();
        let decoded: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value, decoded, "JSON round trip: {encoded}");
    }
}

#[test]
fn ast_json_rejects_every_closed_tag_boundary_violation() {
    for invalid in [
        "1",
        r#"{"unknown":true}"#,
        r#"{"none":false}"#,
        r#"{"env":""}"#,
        r#"{"dim":"one"}"#,
        r#"{"dim":"one.two.three"}"#,
        r#"{"binding":{"kind":"param","path":[]}}"#,
        r#"{"binding":{"kind":"store","path":"rows","time":"start"}}"#,
        r#"{"ref":{"kind":"mark","path":[]}}"#,
        r#"{"call":{"fn":"f","extra":true}}"#,
        r#"{"num":"1","atom":"one"}"#,
        r#"{"num":"01"}"#,
    ] {
        assert!(
            serde_json::from_str::<Value>(invalid).is_err(),
            "accepted {invalid}"
        );
    }

    for invalid in [
        r#"{"version":1,"version":1,"items":[{"exported":false,"declaration":{"decl":"chart","kind":"cartesian"}}]}"#,
        r#"{"version":1,"items":[{"exported":false,"declaration":{"decl":"chart","kind":"cartesian","provenance":{}}}]}"#,
        r#"{"version":1,"items":[{"exported":false,"declaration":{"decl":"chart","kind":"cartesian","visibility":"default"}}]}"#,
        r#"{"version":1,"items":[{"exported":false,"declaration":{"decl":"chart","kind":"cartesian","props":{"x":{"binding":{"kind":"param","path":"x","time":"current"}}}}}]}"#,
        r#"{"version":1,"imports":[{"source":"x","clause":{"named":[]}}],"items":[{"exported":false,"declaration":{"decl":"chart","kind":"cartesian"}}]}"#,
    ] {
        assert!(decode_json(invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn ast_generated_generic_trees_obey_json_and_print_properties() {
    for seed in 0..32 {
        let mut props = PropertyMap::default();
        props
            .insert(
                name(if seed % 2 == 0 { "z" } else { "a" }),
                generated_value(seed),
            )
            .unwrap();
        props
            .insert(
                name(if seed % 2 == 0 { "a" } else { "z" }),
                generated_value(seed + 1),
            )
            .unwrap();
        let file = File {
            version: 1,
            imports: Vec::new(),
            items: vec![ModuleItem {
                exported: false,
                declaration: Decl {
                    keyword: name("chart"),
                    kind: Some(name("cartesian").into()),
                    name: None,
                    visibility: Default::default(),
                    doc: None,
                    props,
                    children: vec![Decl::new(name("row")), Decl::new(name("when"))],
                },
            }],
        };
        let json = encode_json(&file).unwrap();
        assert_eq!(file, decode_json(&json).unwrap());
        let printed = print_file(&file);
        let parsed = parse_file(&SourceFile::new(
            SourceId::new(seed),
            SourceOrigin::Memory(format!("generated-{seed}")),
            printed.clone(),
        ))
        .unwrap();
        assert_eq!(file, parsed.ast);
        assert_eq!(printed, print_file(&parsed.ast));
    }
}

#[test]
fn numeric_and_sql_normalization_boundaries_are_exact() {
    let source = SourceFile::new(
        SourceId::new(1),
        SourceOrigin::Memory("numeric.avenger".into()),
        "avenger 1; chart cartesian { a: 9007199254740993; b: 12345678901234567890.12345678901234567890; c: 1.20E+003; d: -0; literal: 1; parenthesized: (1); casted: CAST(1 AS DECIMAL(38, 20)); compound: 1 + 0; }",
    );
    let parsed = parse_file(&source).unwrap();
    let chart = only_declaration(&parsed);
    for key in ["a", "b", "c", "d", "literal"] {
        assert!(matches!(chart.props.get(key), Some(Value::Num(_))), "{key}");
    }
    for key in ["parenthesized", "casted", "compound"] {
        assert!(
            matches!(chart.props.get(key), Some(Value::Expr(_))),
            "{key}"
        );
    }
    assert_eq!(
        chart.props.get("a"),
        Some(&Value::Num(
            NumericLiteral::new("9007199254740993").unwrap()
        ))
    );
    let definition_file = parse_file(&loaded(&fixture("define-mark.avenger"), 9)).unwrap();
    let definition = only_declaration(&definition_file);
    let Value::Call { args, .. } = definition.children[4].props.get("value").unwrap() else {
        panic!("definition output should normalize as a call")
    };
    assert!(matches!(args[1], Value::Binding { .. }));

    for query in [
        "SELECT m.x FROM movies AS m",
        "FROM movies AS m SELECT m.x",
        "VALUES (1), (2)",
        "SELECT 1 UNION ALL SELECT 2",
        "WITH m AS (SELECT * FROM movies) SELECT * FROM m",
    ] {
        let query = SqlQuery::parse(query).unwrap();
        assert_eq!(query, SqlQuery::parse(&query.canonical_sql()).unwrap());
    }
    assert!(SqlQuery::parse("FROM movies").is_err());
    assert!(SqlQuery::parse("SELECT 1; SELECT 2").is_err());

    for value in [
        "99999999999999999999999999999999999999",
        "9999999999999999999999999999999999999999999999999999999999999999999999999999",
    ] {
        assert_eq!(NumericLiteral::new(value).unwrap().as_str(), value);
    }
    let expression = SqlExpression::parse(
        "EXISTS (SELECT 1 FROM movies WHERE movies.id IN (SELECT id FROM ratings))",
    )
    .unwrap();
    assert_eq!(
        expression,
        SqlExpression::parse(&expression.canonical_sql()).unwrap()
    );
    let commented = SqlQuery::parse("SELECT * -- retained only by the CST\nFROM movies").unwrap();
    assert_eq!(
        commented,
        SqlQuery::parse(&commented.canonical_sql()).unwrap()
    );
}

#[test]
fn headed_block_disambiguation_is_stable() {
    let path = fixture("headed-blocks.avenger");
    let parsed = parse_file(&loaded(&path, 1)).unwrap();
    let chart = only_declaration(&parsed);

    let head = |property: &str| match chart.props.get(property) {
        Some(Value::Block {
            head: Some(head), ..
        }) => head.as_ref(),
        value => panic!("expected configured block for {property}, got {value:?}"),
    };

    assert!(matches!(head("typed"), Value::Atom(name) if name.as_str() == "linear"));
    assert!(matches!(head("column"), Value::Column(column) if column == "amount"));
    assert!(matches!(head("binding"), Value::Binding { path, .. } if path[0].as_str() == "radius"));
    assert!(matches!(head("qualified"), Value::Expr(_)));
    assert!(matches!(head("literal"), Value::Num(_)));
    assert!(matches!(head("parenthesized"), Value::Expr(_)));
    assert!(matches!(head("call"), Value::Call { function, .. } if function.as_str() == "clamp"));
}

#[test]
fn id_is_a_property_or_block_slot_splice_but_not_a_declaration() {
    let property = SourceFile::new(
        SourceId::new(1),
        SourceOrigin::Memory("id-property.avenger".into()),
        "avenger 1; chart cartesian { id: 'valid'; }",
    );
    assert!(parse_file(&property).is_ok());

    let splice = SourceFile::new(
        SourceId::new(2),
        SourceOrigin::Memory("id-splice.avenger".into()),
        "avenger 1; define mark example { slot block id; id; }",
    );
    assert!(parse_file(&splice).is_ok());

    let declaration = SourceFile::new(
        SourceId::new(3),
        SourceOrigin::Memory("id-declaration.avenger".into()),
        "avenger 1; chart cartesian { id { value: 1; } }",
    );
    let error = parse_file(&declaration).unwrap_err();
    assert_eq!(error.diagnostic().code.as_str(), "AVENGER-PARSE-025");
}

#[test]
fn syntax_and_ast_ids_are_stable_within_repeated_parses() {
    let path = fixture("chart.avenger");
    let first = parse_file(&loaded(&path, 7)).unwrap();
    let second = parse_file(&loaded(&path, 7)).unwrap();
    let first_ast = first
        .source_map
        .iter()
        .map(|(id, span)| (id, span, first.source_map.role(id).cloned()))
        .collect::<Vec<_>>();
    let second_ast = second
        .source_map
        .iter()
        .map(|(id, span)| (id, span, second.source_map.role(id).cloned()))
        .collect::<Vec<_>>();
    assert_eq!(first_ast, second_ast);
    assert_eq!(first.concrete.nodes(), second.concrete.nodes());
    assert!(first_ast.iter().all(|(_, _, role)| role.is_some()));
}

#[test]
fn standard_and_from_first_queries_preserve_flavor_with_equivalent_roles() {
    let standard = SqlQuery::parse(
        "SELECT m.category, SUM(r.value) OVER (PARTITION BY m.category) AS total FROM movies AS m JOIN ratings AS r ON m.id = r.movie_id WHERE r.value > 0 GROUP BY m.category",
    )
    .unwrap();
    let from_first = SqlQuery::parse(
        "FROM movies AS m JOIN ratings AS r ON m.id = r.movie_id SELECT m.category, SUM(r.value) OVER (PARTITION BY m.category) AS total WHERE r.value > 0 GROUP BY m.category",
    )
    .unwrap();
    let SetExpr::Select(standard_select) = standard.ast().body.as_ref() else {
        panic!()
    };
    let SetExpr::Select(from_first_select) = from_first.ast().body.as_ref() else {
        panic!()
    };
    assert_eq!(standard_select.flavor, SelectFlavor::Standard);
    assert_eq!(from_first_select.flavor, SelectFlavor::FromFirst);
    assert_eq!(standard_select.from, from_first_select.from);
    assert_eq!(standard_select.projection, from_first_select.projection);
    assert_eq!(standard_select.selection, from_first_select.selection);
    assert_eq!(standard_select.group_by, from_first_select.group_by);
    assert!(standard.canonical_sql().starts_with("SELECT"));
    assert!(from_first.canonical_sql().starts_with("FROM"));

    let standard_cte = SqlQuery::parse(
        "WITH filtered AS (SELECT * FROM movies WHERE rating > 0) SELECT f.title FROM filtered AS f",
    )
    .unwrap();
    let from_first_cte = SqlQuery::parse(
        "WITH filtered AS (SELECT * FROM movies WHERE rating > 0) FROM filtered AS f SELECT f.title",
    )
    .unwrap();
    assert_eq!(standard_cte.ast().with, from_first_cte.ast().with);
    let SetExpr::Select(standard_select) = standard_cte.ast().body.as_ref() else {
        panic!()
    };
    let SetExpr::Select(from_first_select) = from_first_cte.ast().body.as_ref() else {
        panic!()
    };
    assert_eq!(standard_select.from, from_first_select.from);
    assert_eq!(standard_select.projection, from_first_select.projection);
}

fn generated_value(seed: u32) -> Value {
    match seed % 6 {
        0 => Value::Num(
            NumericLiteral::new(&(seed as u64 + 9_007_199_254_740_993).to_string()).unwrap(),
        ),
        1 => Value::Str(format!("value-{seed}")),
        2 => Value::Bool(seed.is_multiple_of(2)),
        3 => Value::Column(format!("column_{seed}")),
        4 => Value::Array(vec![Value::Atom(name("x")), Value::Null]),
        _ => Value::Call {
            function: name("span"),
            args: vec![
                Value::Num(NumericLiteral::new("0").unwrap()),
                Value::Num(NumericLiteral::new("1").unwrap()),
            ],
        },
    }
}

fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}
