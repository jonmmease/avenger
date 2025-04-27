use ariadne::{Color, Config, Label, Report, ReportKind, Source};
use sqlparser::parser::ParserError;

#[test]
fn test_parse_query() {
    use sqlparser::dialect::GenericDialect;
    use sqlparser::parser::Parser;

    let src = r#"
SELECT a, b, 123, myfunc(b)
FROM table_1 as "asdf
WHERE a AND b < 100
ORDER BY a DESC, b"#;

    // Compute line lengths, add 1 for the newline between lines
    let lines = src.lines().collect::<Vec<_>>();
    let line_lens = lines.iter().map(|line| line.len() + 1).collect::<Vec<_>>();
    println!("line_lens: {:?}", line_lens);

    let dialect = GenericDialect {}; // or AnsiDialect, or your own dialect ...

    match Parser::parse_sql(&dialect, src) {
        Ok(ast) => {
            // Process the AST here
            println!("AST: {:?}", ast);
        }
        Err(ParserError::ParserError(err) | ParserError::TokenizerError(err)) => {
            if let Some(parts) = ParserErrorParts::from_error_message(&err) {
                let span_start =
                    line_lens[..parts.line - 1].iter().sum::<usize>() + parts.column - 1;

                Report::build(ReportKind::Error, span_start..span_start)
                    .with_message("Parsing error")
                    .with_label(
                        Label::new(span_start..span_start + parts.len - 1).with_message(&parts.msg),
                    )
                    .finish()
                    .print(Source::from(src))
                    .unwrap();
            } else {
                // error with no line number information
                Report::build(ReportKind::Error, 0..0)
                    .with_message("Parsing error")
                    .finish()
                    .print(Source::from(src))
                    .unwrap();
            }
        }
        r => {
            // Extract parts of parser error like
            // "Expected: end of statement, found: > at Line: 4, Column: 9"

            println!("Other error: {:?}", r);
        }
    }
}

#[test]
fn try_diagnostic_message() {
    let src = r#"
def five = match () in {
	() => 5,
	() => "5",
        }

def six =
    five
    + 1
"#;
    Report::build(ReportKind::Error, 34..34)
        .with_message("Incompatible types")
        .with_label(Label::new(32..33).with_message("This is of type Nat"))
        .with_label(Label::new(42..45).with_message("This is of type Str"))
        .finish()
        .print(Source::from(src))
        .unwrap();

    const SOURCE: &str = "a b c d e f";
    // also supports labels with no messages to only emphasis on some areas
    Report::build(ReportKind::Error, 2..3)
        .with_message("Incompatible types")
        .with_config(Config::default().with_compact(true))
        .with_label(Label::new(0..1).with_color(Color::Red))
        .with_label(
            Label::new(2..3)
                .with_color(Color::Blue)
                .with_message("`b` for banana")
                .with_order(1),
        )
        .with_label(Label::new(4..5).with_color(Color::Green))
        .with_label(
            Label::new(7..9)
                .with_color(Color::Cyan)
                .with_message("`e` for emerald"),
        )
        .finish()
        .print(Source::from(SOURCE))
        .unwrap();
}

use regex::Regex;

#[derive(Debug, Clone)]
struct ParserErrorParts {
    pub msg: String,
    pub line: usize,
    pub column: usize,
    pub len: usize,
}

impl ParserErrorParts {
    fn from_error_message(message: &str) -> Option<Self> {
        // Check for length of found item in message
        let found_re = Regex::new(r"found: ([^ ]+)").unwrap();
        let len = found_re
            .captures(message)
            .map(|captures| captures.get(1).unwrap().as_str().len())
            .unwrap_or(1);

        // Check for line and column information
        let re = Regex::new(r"(.*) at Line: (\d+), Column: (\d+)").unwrap();
        re.captures(message).map(|captures| ParserErrorParts {
            msg: captures.get(1).unwrap().as_str().to_string(),
            line: captures.get(2).unwrap().as_str().parse().unwrap(),
            column: captures.get(3).unwrap().as_str().parse().unwrap(),
            len,
        })
    }
}
