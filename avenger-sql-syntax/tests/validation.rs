//! The checks of the validation pass: the rules that GoogleSQL checks after its parser, with
//! the message and the range of each diagnostic.

mod common;

use avenger_sql_syntax::{parse, parse_data, parse_module, DiagnosticCode, Parse};
use common::*;

#[test]
fn rejects_what_googlesql_rejects_after_parsing() {
    for (text, message, at) in VALIDATED {
        rejects(text, message, at);
    }
    for (text, message, at) in VALIDATED_MODULES {
        rejects_module(text, message, at);
    }
    for (text, message, at) in VALIDATED_DATA {
        rejects_data(text, message, at);
    }
}

#[test]
fn the_parser_accepts_what_only_validation_rejects() {
    // A text that only validation rejects has a tree without ERROR nodes.
    let parses = (VALIDATED.iter().map(|(text, _, _)| (*text, parse(text))))
        .chain((VALIDATED_MODULES.iter()).map(|(text, _, _)| (*text, parse_module(text))))
        .chain((VALIDATED_DATA.iter()).map(|(text, _, _)| (*text, parse_data(text))));
    for (text, parse) in parses {
        let root = parse.syntax();
        assert!(
            nodes(&root, avenger_sql_syntax::SyntaxKind::ERROR).is_empty(),
            "{text}: {root:#?}"
        );
    }
}

/// The one diagnostic of `parse`, the parse of `text`.
fn only_diagnostic(text: &str, parse: &Parse) -> avenger_sql_syntax::Diagnostic {
    let diagnostics = parse.diagnostics();
    let described: Vec<String> = diagnostics.iter().map(|d| describe(text, d)).collect();
    assert_eq!(diagnostics.len(), 1, "{text}: {described:#?}");
    diagnostics[0].clone()
}

#[test]
fn a_statement_out_of_its_role_has_one_error_that_names_its_place() {
    // The statements of a data script in a script or a module, whose help names the data
    // scripts. GoogleSQL takes them in a script, so they are unsupported there.
    let data_statements = [
        "CREATE PUBLIC EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['t.csv'])",
        "CREATE PRIVATE VIEW IF NOT EXISTS v AS FROM t",
        "CREATE OR REPLACE SCHEMA s OPTIONS (description = 'x')",
        "LOAD DATA OVERWRITE t FROM FILES (format = 'CSV', uris = ['t.csv'])",
    ];
    for statement in data_statements {
        for (text, parse) in [
            (statement.to_string(), parse(statement)),
            (
                format!("MODULE m; {statement}"),
                parse_module(&format!("MODULE m; {statement}")),
            ),
        ] {
            let diagnostic = only_diagnostic(&text, &parse);
            assert_eq!(diagnostic.code, DiagnosticCode::Unsupported, "{text}");
            let help = diagnostic.help.unwrap_or_default();
            assert!(help.contains("data scripts"), "{text}: {help}");
        }
    }
    // A script's own tables of files are TEMP, and the help of the forms without TEMP names
    // the TEMP form.
    for (text, form) in [
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['t.csv'])",
            "CREATE TEMP EXTERNAL TABLE",
        ),
        (
            "LOAD DATA OVERWRITE t FROM FILES (format = 'CSV', uris = ['t.csv'])",
            "LOAD DATA OVERWRITE TEMP TABLE",
        ),
    ] {
        let diagnostic = only_diagnostic(text, &parse(text));
        assert!(diagnostic.message.contains("without TEMP"), "{text}");
        assert!(diagnostic.help.unwrap_or_default().contains(form), "{text}");
    }
    let diagnostic = only_diagnostic(
        "CREATE TABLE t (a INT64)",
        &parse("CREATE TABLE t (a INT64)"),
    );
    assert!(diagnostic
        .message
        .contains("without TEMP belongs to a data script"));
    assert!(diagnostic
        .help
        .unwrap_or_default()
        .contains("CREATE TEMP TABLE"));
    // The statements of a script in a data script, whose help names the chart scripts.
    for text in [
        "FROM t",
        "DECLARE x DEFAULT 1",
        "SET x = 1",
        "CREATE TABLE t (a INT64)",
    ] {
        let diagnostic = only_diagnostic(text, &parse_data(text));
        assert_eq!(diagnostic.code, DiagnosticCode::InvalidStatement, "{text}");
        let help = diagnostic.help.unwrap_or_default();
        assert!(help.contains("chart script"), "{text}: {help}");
    }
    // A DECLARE after another statement has its role's error alone.
    let text = "CREATE SCHEMA s; DECLARE x INT64";
    only_diagnostic(text, &parse_data(text));
}

#[test]
fn a_chart_script_takes_its_own_tables_of_files() {
    for text in [
        "CREATE TEMP EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['t.csv']); FROM t",
        "CREATE TEMPORARY EXTERNAL TABLE IF NOT EXISTS t (a INT64) OPTIONS (format = 'CSV', uris = ['t.csv'])",
        "LOAD DATA OVERWRITE TEMP TABLE t FROM FILES (format = 'CSV', uris = ['t.csv']); FROM t",
        "LOAD DATA INTO TEMP TABLE t (a INT64) FROM FILES (format = 'CSV', uris = ['t.csv'])",
    ] {
        accepts(text);
    }
    // A module holds no tables, TEMP or not.
    let text = "MODULE m; CREATE TEMP EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['t.csv'])";
    assert!(!parse_module(text).ok());
}

#[test]
fn a_data_script_takes_tables_with_or_without_temp() {
    for text in [
        "CREATE TABLE t AS (FROM u)",
        "CREATE TEMP TABLE t (a INT64) AS FROM u",
        "CREATE OR REPLACE TABLE t AS FROM u",
        "CREATE TEMP TABLE FUNCTION f(x INT64) AS FROM u |> LIMIT x",
        "CREATE TEMPORARY AGGREGATE FUNCTION f(x INT64) AS (SUM(x))",
        "IMPORT MODULE charts.scales",
    ] {
        accepts_data(text);
    }
}

#[test]
fn an_unclosed_string_adds_no_errors_of_its_literal() {
    // The open quote ends at the line end, and the lexer reads `b' AS s` as the start of an
    // unclosed bytes literal. Each unclosed part has its lexer error, and the literal has no
    // others.
    let text = "FROM t |> SELECT 'a\nb' AS s";
    let messages: Vec<String> = parse(text)
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect();
    assert_eq!(
        messages,
        [
            "this quoted literal has no closing quote",
            "this quoted literal has no closing quote"
        ]
    );
}
