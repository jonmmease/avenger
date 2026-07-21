use std::{
    fs,
    path::{Path, PathBuf},
};

use avenger_lang_core::{
    SourceFile, SourceId, SourceOrigin,
    sql::{
        BindingVersion, DOMAIN_RANGE_HELPERS, TokenClass, is_reserved_helper_name,
        normalize_bindings, parse_sql_expression, parse_sql_query, tokenize,
    },
};
use serde::Deserialize;
use sqlparser::{
    ast::{Expr, SelectFlavor, SetExpr},
    dialect::GenericDialect,
    tokenizer::{Token, Tokenizer, Whitespace},
};

const TOKEN_FIXTURES: &[&str] = &[
    "token_classes.avenger",
    "comments.avenger",
    "bindings.avenger",
    "sql_constructs.avenger",
    "cursor_styles.avenger",
    "datafusion_expressions.avenger",
    "kernel_islands.avenger",
];

const CURSOR_STYLES: &[&str] = &[
    "default",
    "pointer",
    "text",
    "crosshair",
    "grab",
    "grabbing",
    "resize_horizontal",
    "resize_vertical",
    "resize_nw_se",
    "resize_ne_sw",
];

#[derive(Deserialize)]
struct TreeSitterConformanceManifest {
    schema_version: u32,
    cases: Vec<TreeSitterConformanceCase>,
}

#[derive(Deserialize)]
struct TreeSitterConformanceCase {
    id: String,
    category: String,
    stage: String,
    source: String,
    accepted: bool,
    #[serde(default)]
    significant_classes: Vec<String>,
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tokens")
        .join(name)
}

fn fixture_source(name: &str) -> SourceFile {
    let path = fixture_path(name);
    SourceFile::new(
        SourceId::new(1),
        SourceOrigin::File(path.clone()),
        fs::read_to_string(path).unwrap(),
    )
}

fn memory_source(text: &str) -> SourceFile {
    SourceFile::new(
        SourceId::new(1),
        SourceOrigin::Memory("token-test.avenger".into()),
        text,
    )
}

fn first_significant(tokens: &[avenger_lang_core::sql::LanguageToken], start: usize) -> usize {
    (start..tokens.len())
        .find(|index| !matches!(tokens[*index].token(), Token::Whitespace(_)))
        .expect("EOF is significant")
}

fn tree_sitter_manifest() -> TreeSitterConformanceManifest {
    let path = fixture_path("tree_sitter_conformance.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn significant_class_names(stream: &avenger_lang_core::sql::TokenStream) -> Vec<&'static str> {
    stream
        .tokens()
        .iter()
        .filter_map(|token| match token.class() {
            TokenClass::Word => Some("word"),
            TokenClass::QuotedIdentifier => Some("quoted_identifier"),
            TokenClass::String => Some("string"),
            TokenClass::Number => Some("number"),
            TokenClass::Punctuation => Some("punctuation"),
            TokenClass::Binding => Some("binding"),
            TokenClass::TemporalVersion => Some("temporal"),
            TokenClass::PositionalPlaceholder => Some("positional_placeholder"),
            TokenClass::OtherPlaceholder => Some("other_placeholder"),
            TokenClass::Comment(avenger_lang_core::sql::CommentKind::Line) => Some("comment_line"),
            TokenClass::Comment(avenger_lang_core::sql::CommentKind::Block) => {
                Some("comment_block")
            }
            TokenClass::Whitespace(_) | TokenClass::Eof => None,
        })
        .collect()
}

fn parsed_to_eof(stream: &avenger_lang_core::sql::TokenStream, next_token: usize) -> bool {
    first_significant(stream.tokens(), next_token) == stream.tokens().len() - 1
}

#[test]
fn tree_sitter_conformance_manifest_matches_the_strict_frontend() {
    let manifest = tree_sitter_manifest();
    assert_eq!(manifest.schema_version, 1);
    assert!(manifest.cases.len() >= 60);

    for case in manifest.cases {
        let source = memory_source(&case.source);
        let tokenized = tokenize(&source);
        let accepted = match case.stage.as_str() {
            "tokenize" => tokenized.is_ok(),
            "normalize" => tokenized
                .as_ref()
                .is_ok_and(|stream| normalize_bindings(stream, 0..stream.tokens().len()).is_ok()),
            "expression" => tokenized.as_ref().is_ok_and(|stream| {
                parse_sql_expression(stream, 0)
                    .is_ok_and(|parsed| parsed_to_eof(stream, parsed.next_token))
            }),
            "query" => tokenized.as_ref().is_ok_and(|stream| {
                parse_sql_query(stream, 0)
                    .is_ok_and(|parsed| parsed_to_eof(stream, parsed.next_token))
            }),
            stage => panic!("unknown manifest stage `{stage}` for {}", case.id),
        };
        assert_eq!(
            accepted, case.accepted,
            "manifest case {} ({}/{}) disagrees with the strict frontend: `{}`",
            case.id, case.category, case.stage, case.source
        );

        if !case.significant_classes.is_empty() {
            let stream = tokenized.unwrap_or_else(|error| {
                panic!("manifest case {} failed tokenization: {error}", case.id)
            });
            assert_eq!(
                significant_class_names(&stream),
                case.significant_classes,
                "token classes for manifest case {}",
                case.id
            );
        }
    }
}

#[test]
fn unsupported_literal_and_identifier_forms_have_stable_spans() {
    for spelling in [
        "`column`",
        "N'text'",
        "U&'text'",
        "B'1010'",
        "R'raw'",
        "Q'|raw|'",
        "NQ'|raw|'",
        "'''multiline'''",
        "\"\"\"multiline\"\"\"",
        "0xdeadbeef",
        "$café$raw$café$",
    ] {
        let source = memory_source(spelling);
        let error = tokenize(&source).unwrap_err();
        assert_eq!(error.diagnostic().code.as_str(), "AVENGER-TOKEN-002");
        let span = error.diagnostic().primary.span;
        assert_eq!(span.source, source.id);
        assert_eq!(span.range.start, 0);
        assert!(span.range.end > span.range.start);
        assert!(span.range.end <= spelling.len());
    }
}

#[test]
fn token_golden_corpus_is_stable() {
    for name in TOKEN_FIXTURES {
        let source = fixture_source(name);
        let stream = tokenize(&source).unwrap();
        let snapshot = stream.snapshot();
        let baseline = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/baselines/tokens")
            .join(Path::new(name).with_extension("txt"));
        if std::env::var_os("AVENGER_LANG_UPDATE_BASELINES").is_some() {
            fs::write(&baseline, &snapshot).unwrap();
            eprintln!("updated {}", baseline.display());
        } else {
            let expected = fs::read_to_string(&baseline).unwrap_or_else(|error| {
                panic!(
                    "failed to read reviewed baseline {}: {error}",
                    baseline.display()
                )
            });
            assert_eq!(snapshot, expected, "token baseline changed for {name}");
        }
    }
}

#[test]
fn token_bindings_normalize_without_resolution() {
    let source = fixture_source("bindings.avenger");
    let stream = tokenize(&source).unwrap();
    let normalized = normalize_bindings(&stream, 0..stream.tokens().len()).unwrap();
    let bindings = normalized.bindings();
    assert_eq!(bindings.len(), 7);
    assert_eq!(bindings[0].path, ["name"]);
    assert_eq!(bindings[0].version, BindingVersion::Current);
    assert_eq!(bindings[2].path, ["component", "alias"]);
    assert_eq!(bindings[2].version, BindingVersion::Start);
    assert_eq!(bindings[4].path, ["deeper", "exported", "value"]);
    assert_eq!(bindings[4].version, BindingVersion::Start);
    assert_eq!(bindings[6].path, ["config"]);
    assert_eq!(bindings[6].version, BindingVersion::Previous);
    assert!(normalized.tokens().iter().any(|token| {
        matches!(&token.token, Token::Word(word) if word.quote_style == Some('"') && word.value.starts_with("__avenger_binding_"))
    }));
}

#[test]
fn token_negative_lexical_fixtures_have_precise_spans() {
    for name in [
        "unterminated_string.avenger",
        "unterminated_comment.avenger",
    ] {
        let source = fixture_source(&format!("invalid/{name}"));
        let error = tokenize(&source).unwrap_err();
        assert_eq!(error.diagnostic().code.as_str(), "AVENGER-TOKEN-001");
        assert_eq!(error.diagnostic().primary.span.source, source.id);
        assert!(error.diagnostic().primary.span.range.start <= source.text().len());
        assert!(error.diagnostic().primary.span.range.end <= source.text().len());
    }
}

#[test]
fn token_negative_binding_fixtures_report_the_offending_source() {
    let cases = [
        ("bare_binding.avenger", "AVENGER-SQL-003"),
        ("missing_path_segment.avenger", "AVENGER-SQL-004"),
        ("quoted_path.avenger", "AVENGER-SQL-004"),
        ("numeric_path.avenger", "AVENGER-SQL-004"),
        ("whitespace_temporal.avenger", "AVENGER-SQL-005"),
        ("unknown_temporal.avenger", "AVENGER-SQL-006"),
        ("positional_dollar.avenger", "AVENGER-SQL-002"),
        ("positional_question.avenger", "AVENGER-SQL-002"),
    ];
    for (name, code) in cases {
        let source = fixture_source(&format!("invalid/{name}"));
        let stream = tokenize(&source).unwrap();
        let error = normalize_bindings(&stream, 0..stream.tokens().len()).unwrap_err();
        assert_eq!(error.diagnostic().code.as_str(), code, "fixture {name}");
        let span = error.diagnostic().primary.span;
        assert_eq!(span.source, source.id);
        assert!(span.range.end > span.range.start, "fixture {name}");
    }
}

#[test]
fn token_comment_contract_and_doc_capture_are_pinned() {
    let source = fixture_source("comments.avenger");
    let stream = tokenize(&source).unwrap();
    let docs = stream.doc_comments();
    assert_eq!(docs.len(), 1);
    assert_eq!(
        docs[0].text,
        "First documentation line.\nSecond documentation line."
    );

    let decrement = stream
        .tokens()
        .windows(4)
        .find(|window| stream.raw(&window[0]) == "amount" && stream.raw(&window[1]) == "-")
        .unwrap();
    assert!(matches!(decrement[1].token(), Token::Minus));
    assert!(matches!(decrement[2].token(), Token::Minus));
    assert!(matches!(decrement[3].token(), Token::Number(..)));
    assert!(stream.tokens().iter().any(|token| {
        matches!(token.token(), Token::Whitespace(Whitespace::MultiLineComment(text)) if text.contains("inner"))
    }));
    for spelling in ["//", "#not_a_comment"] {
        let start = source.text().find(spelling).unwrap();
        assert!(!stream.tokens().iter().any(|token| {
            matches!(token.class(), TokenClass::Comment(_))
                && token.span().range.start <= start
                && start < token.span().range.end
        }));
    }
}

#[test]
fn token_avenger_dialect_retains_generic_tokens_outside_comment_override() {
    for name in ["sql_constructs.avenger", "datafusion_expressions.avenger"] {
        let source = fixture_source(name);
        let avenger = tokenize(&source).unwrap();
        let generic = Tokenizer::new(&GenericDialect, source.text())
            .tokenize_with_location()
            .unwrap();
        assert_eq!(avenger.sql_tokens(), generic, "Generic drift for {name}");
    }
}

#[test]
fn token_sql_expression_constructs_parse_from_the_shared_stream() {
    let expressions = [
        "CAST(\"rating\" AS DOUBLE)",
        "\"rating\"::double precision",
        "$config:theme.color",
        "[1, 2, coalesce(3, nested(4, 5))]",
        "{key: 'value', nested: {enabled: true}}",
        "catalog.schema.\"table\".\"column\"",
    ];
    for expression in expressions {
        let source = memory_source(expression);
        let stream = tokenize(&source).unwrap();
        let parsed = parse_sql_expression(&stream, 0)
            .unwrap_or_else(|error| panic!("failed to parse `{expression}`: {error}"));
        assert_eq!(
            first_significant(stream.tokens(), parsed.next_token),
            stream.tokens().len() - 1,
            "expression `{expression}` left tokens"
        );
    }
}

#[test]
fn token_cursor_atoms_parse_as_identifier_expressions_including_default() {
    let source = fixture_source("cursor_styles.avenger");
    let stream = tokenize(&source).unwrap();
    let words: Vec<_> = stream
        .tokens()
        .iter()
        .filter_map(|token| match token.token() {
            Token::Word(word) => Some(word.value.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(words, CURSOR_STYLES);

    for style in CURSOR_STYLES {
        let source = memory_source(style);
        let stream = tokenize(&source).unwrap();
        let parsed = parse_sql_expression(&stream, 0)
            .unwrap_or_else(|error| panic!("cursor atom `{style}` did not parse: {error}"));
        assert!(matches!(parsed.ast, Expr::Identifier(_)));
    }
}

#[test]
fn token_query_corpus_accepts_ctes_quoted_columns_and_both_orderings() {
    let cte = memory_source(
        "WITH filtered AS (SELECT \"x\" FROM source WHERE \"x\" > 0) SELECT * FROM filtered;",
    );
    let cte = tokenize(&cte).unwrap();
    let parsed = parse_sql_query(&cte, 0).unwrap();
    assert!(parsed.ast.with.is_some());
    assert!(matches!(
        cte.tokens()[first_significant(cte.tokens(), parsed.next_token)].token(),
        Token::SemiColon
    ));

    let standard = memory_source("SELECT m.\"title\" FROM vega.movies AS m;");
    let standard = tokenize(&standard).unwrap();
    let standard = parse_sql_query(&standard, 0).unwrap();
    let SetExpr::Select(standard_select) = standard.ast.body.as_ref() else {
        panic!()
    };
    assert_eq!(standard_select.flavor, SelectFlavor::Standard);

    let from_first = memory_source("FROM vega.movies AS m SELECT m.\"title\";");
    let from_first = tokenize(&from_first).unwrap();
    let from_first = parse_sql_query(&from_first, 0).unwrap();
    let SetExpr::Select(from_first_select) = from_first.ast.body.as_ref() else {
        panic!()
    };
    assert_eq!(from_first_select.flavor, SelectFlavor::FromFirst);
    assert_eq!(standard_select.from, from_first_select.from);
    assert_eq!(standard_select.projection, from_first_select.projection);

    let values = memory_source("VALUES (1, 'one'), (2, 'two');");
    let values = tokenize(&values).unwrap();
    let values = parse_sql_query(&values, 0).unwrap();
    assert!(matches!(values.ast.body.as_ref(), SetExpr::Values(_)));
}

#[test]
fn token_query_entry_point_rejects_non_query_statements() {
    let source = memory_source("CREATE TABLE t (x INT);");
    let stream = tokenize(&source).unwrap();
    let error = parse_sql_query(&stream, 0).unwrap_err();
    assert_eq!(error.diagnostic().code.as_str(), "AVENGER-SQL-008");
}

#[test]
fn token_semicolon_belongs_to_the_outer_dsl_not_the_query() {
    let source = memory_source("SELECT ';' AS \"value\" /* ; */; SELECT 2;");
    let stream = tokenize(&source).unwrap();
    let parsed = parse_sql_query(&stream, 0).unwrap();
    let separator = first_significant(stream.tokens(), parsed.next_token);
    assert!(matches!(
        stream.tokens()[separator].token(),
        Token::SemiColon
    ));
}

#[test]
fn token_datafusion_expression_drift_corpus_parses() {
    let source = fixture_source("datafusion_expressions.avenger");
    for expression in source.text().lines().filter(|line| !line.is_empty()) {
        let expression_source = memory_source(expression);
        let stream = tokenize(&expression_source).unwrap();
        let parsed = parse_sql_expression(&stream, 0)
            .unwrap_or_else(|error| panic!("DataFusion corpus expression `{expression}`: {error}"));
        assert_eq!(
            first_significant(stream.tokens(), parsed.next_token),
            stream.tokens().len() - 1
        );
    }
}

#[test]
fn token_kernel_sql_island_boundaries_share_one_stream() {
    let source = fixture_source("kernel_islands.avenger");
    let stream = tokenize(&source).unwrap();

    let x = stream
        .tokens()
        .iter()
        .position(|token| matches!(token.token(), Token::Word(word) if word.value == "x"))
        .unwrap();
    let x_colon = first_significant(stream.tokens(), x + 1);
    assert!(matches!(stream.tokens()[x_colon].token(), Token::Colon));
    let expression_start = first_significant(stream.tokens(), x_colon + 1);
    let expression = parse_sql_expression(&stream, expression_start).unwrap();
    let expression_end = first_significant(stream.tokens(), expression.next_token);
    assert!(matches!(
        stream.tokens()[expression_end].token(),
        Token::LBrace
    ));

    let sql = stream.tokens().iter().position(|token| {
        matches!(token.token(), Token::Word(word) if word.value.eq_ignore_ascii_case("sql"))
    }).unwrap();
    let sql_colon = first_significant(stream.tokens(), sql + 1);
    let query_start = first_significant(stream.tokens(), sql_colon + 1);
    let query = parse_sql_query(&stream, query_start).unwrap();
    let query_end = first_significant(stream.tokens(), query.next_token);
    assert!(matches!(
        stream.tokens()[query_end].token(),
        Token::SemiColon
    ));
    assert_eq!(query.bindings.len(), 1);
    assert_eq!(query.bindings[0].path, ["minimum"]);
}

#[test]
fn token_reserved_helper_names_and_domain_range_spelling_are_pinned() {
    for name in ["EXISTS", "interval", "Struct", "trim"] {
        assert!(is_reserved_helper_name(name));
    }
    for name in DOMAIN_RANGE_HELPERS {
        assert!(!is_reserved_helper_name(name));
        let source = memory_source(&format!("{name}(1, 2)"));
        let stream = tokenize(&source).unwrap();
        assert!(parse_sql_expression(&stream, 0).is_ok());
    }
    let source = memory_source("interval(1, 2)");
    let stream = tokenize(&source).unwrap();
    let parsed = parse_sql_expression(&stream, 0).unwrap();
    assert!(
        !matches!(parsed.ast, Expr::Function(_)),
        "interval(...) must not enter the ordinary helper-call grammar"
    );
}

#[test]
fn token_classification_covers_all_normative_classes() {
    let source = fixture_source("token_classes.avenger");
    let stream = tokenize(&source).unwrap();
    let classes: Vec<_> = stream.tokens().iter().map(|token| token.class()).collect();
    assert!(classes.contains(&TokenClass::Word));
    assert!(classes.contains(&TokenClass::QuotedIdentifier));
    assert!(classes.contains(&TokenClass::String));
    assert!(classes.contains(&TokenClass::Number));
    assert!(classes.contains(&TokenClass::Punctuation));
    assert!(classes.contains(&TokenClass::Binding));
    assert!(classes.contains(&TokenClass::TemporalVersion));
    assert!(
        classes
            .iter()
            .any(|class| matches!(class, TokenClass::Comment(_)))
    );
    assert!(
        classes
            .iter()
            .any(|class| matches!(class, TokenClass::Whitespace(_)))
    );
    assert_eq!(classes.last(), Some(&TokenClass::Eof));
}
