//! The classes of the tokens of a script, and the characters of the language crate's byte ranges.

use std::sync::Arc;

use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library};
use avenger_sql_editor::highlight::{spans, syntax, Chars, Class};
use avenger_sql_language::{Document, HighlightRange, Role, TextRange};

/// The text and class of each token of `text` by `highlights`.
fn classes(text: &str, highlights: &[HighlightRange]) -> Vec<(String, Class)> {
    let chars: Vec<char> = text.chars().collect();
    spans(text, highlights)
        .into_iter()
        .map(|span| (chars[span.range].iter().collect(), span.class))
        .collect()
}

/// A pipe query with named arguments, by the highlights of its syntax: keywords, a table, the
/// pipe operator, a function, its argument names, a column, a string, a number, and a comment.
#[test]
fn a_pipe_query_by_its_syntax() {
    let text =
        "-- sizes\nFROM penguins\n|> AGGREGATE mark_bar(x => species, y => 'n', size => 2) AS chart";
    use Class::*;
    let expected = [
        ("-- sizes", Comment),
        ("FROM", Keyword),
        ("penguins", Table),
        ("|>", Operator),
        ("AGGREGATE", Keyword),
        ("mark_bar", Function),
        ("(", Plain),
        ("x", Argument),
        ("=>", Operator),
        ("species", Column),
        (",", Plain),
        ("y", Argument),
        ("=>", Operator),
        ("'n'", String),
        (",", Plain),
        ("size", Argument),
        ("=>", Operator),
        ("2", Number),
        (")", Plain),
        ("AS", Keyword),
        ("chart", Plain),
    ];
    let expected: Vec<(std::string::String, Class)> = expected
        .into_iter()
        .map(|(text, class)| (text.to_string(), class))
        .collect();
    assert_eq!(classes(text, &syntax(text)), expected);
}

/// A string that is still being typed stays a string, and the tokens before it keep their classes.
#[test]
fn an_unterminated_string_keeps_its_class() {
    let text = "FROM penguins\n|> WHERE species = 'Ade";
    let found = classes(text, &syntax(text));
    assert_eq!(found.last().unwrap(), &("'Ade".to_string(), Class::String));
    assert_eq!(found[1], ("penguins".to_string(), Class::Table));
}

/// The analysis tells a mark from another function, which the syntax cannot.
#[test]
fn the_analysis_colors_a_mark_as_a_chart_function() {
    let text = "FROM penguins |> AGGREGATE mark_bar(x => species, y => COUNT(*)) AS chart";
    let document = Document::new(
        text,
        Role::Script,
        Arc::new(MemoryCatalog::new()),
        Arc::new(Library::new()),
        Options::default(),
    );
    let found = classes(text, &document.highlights());
    assert!(
        found.contains(&("mark_bar".to_string(), Class::Chart)),
        "{found:?}"
    );
    assert!(
        found.contains(&("COUNT".to_string(), Class::Function)),
        "{found:?}"
    );
}

/// A byte range of a text with multi-byte characters is a range of characters, and back.
#[test]
fn byte_ranges_are_ranges_of_characters() {
    let text = "-- é ✓\nFROM t";
    let chars = Chars::new(text);
    let from = text.find("FROM").unwrap();
    let range = TextRange::new((from as u32).into(), ((from + 4) as u32).into());
    let found = chars.range(range);
    assert_eq!(
        text.chars().skip(found.start).take(4).collect::<String>(),
        "FROM"
    );
    assert_eq!(chars.offset(found.start), from);
    assert_eq!(chars.index(text.len() + 10), text.chars().count());
}

/// The format of files is a constant and a URI a path, in the options of a data script's files.
#[test]
fn a_format_is_a_constant_and_a_uri_a_path() {
    let text = "LOAD DATA OVERWRITE sales FROM FILES (format = 'CSV', uris = ['data/sales.csv'])";
    let highlights = avenger_sql_syntax::highlight(&avenger_sql_syntax::parse_data(text).syntax());
    let found = classes(text, &highlights);
    assert!(
        found.contains(&("sales".to_string(), Class::Table)),
        "{found:?}"
    );
    assert!(
        found.contains(&("'CSV'".to_string(), Class::Constant)),
        "{found:?}"
    );
    assert!(
        found.contains(&("'data/sales.csv'".to_string(), Class::Path)),
        "{found:?}"
    );
}
