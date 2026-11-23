//! The Typst markup in strings: the highlights of a title's markup through escapes, raw and
//! adjacent strings, a plain title, the Typst syntax errors at their places in the file, and the
//! strings in a language of their own.

mod common;

use avenger_sql_language::{Document, Embedded, Highlight, Source, TypstTag};

/// A chart whose title is `title`, an expression.
fn chart(title: &str) -> String {
    format!("FROM penguins |> AGGREGATE mark_bar(x => species, y => body_mass_g, title => {title}) AS chart")
}

/// The highlights of the document, after a check that they are in order without overlaps, as
/// their text and class.
fn highlights(document: &Document) -> Vec<(String, Highlight)> {
    let ranges = document.highlights();
    for pair in ranges.windows(2) {
        assert!(pair[0].range.end() <= pair[1].range.start(), "{pair:?}");
    }
    let text = document.text();
    (ranges.into_iter())
        .map(|range| (text[range.range].to_string(), range.highlight))
        .collect()
}

/// The text of each Typst range.
fn typst(highlights: &[(String, Highlight)]) -> Vec<(&str, TypstTag)> {
    (highlights.iter())
        .filter_map(|(text, highlight)| match highlight {
            Highlight::Typst(tag) => Some((text.as_str(), *tag)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_title_s_markup_takes_typst_classes_through_its_escapes() {
    let document = common::document(&chart(r"'é #emph[Mass] of #value in $x^2$'"));
    assert_eq!(document.diagnostics(), Vec::new());
    let highlights = highlights(&document);
    let typst = typst(&highlights);
    // Typst's classes hold the `#` of a call and of a variable.
    assert!(typst.contains(&("#emph", TypstTag::Function)), "{typst:?}");
    assert!(
        typst.contains(&("#value", TypstTag::Interpolated)),
        "{typst:?}"
    );
    assert!(typst.contains(&("$", TypstTag::MathDelimiter)), "{typst:?}");
    assert!(typst.contains(&("^", TypstTag::MathOperator)), "{typst:?}");
    // The escape before the markup, the quotes, and the plain words stay a string.
    let strings: Vec<&str> = (highlights.iter())
        .filter(|(_, highlight)| *highlight == Highlight::String)
        .map(|(text, _)| text.as_str())
        .collect();
    assert!(strings[0].starts_with(r"'é "), "{strings:?}");
    assert!(strings.last().unwrap().ends_with('\''), "{strings:?}");
}

#[test]
fn a_raw_string_and_adjacent_strings_hold_the_same_markup() {
    let tags = |title: &str| -> Vec<TypstTag> {
        let highlights = highlights(&common::document(&chart(title)));
        let typst = typst(&highlights);
        // No range holds the quotes between two strings.
        assert!(
            typst.iter().all(|(text, _)| !text.contains('\'')),
            "{typst:?}"
        );
        typst.into_iter().map(|(_, tag)| tag).collect()
    };
    let plain = tags("'#emph[Mass]'");
    assert!(!plain.is_empty());
    assert_eq!(tags("r'#emph[Mass]'"), plain);
    assert_eq!(tags("'#emph[' 'Mass]'"), plain);
}

#[test]
fn a_plain_title_is_a_string() {
    let document = common::document(&chart("title('#emph[Mass]', syntax => 'plain')"));
    let highlights = highlights(&document);
    assert_eq!(typst(&highlights), Vec::new());
    assert!(highlights.contains(&("'#emph[Mass]'".to_string(), Highlight::String)));
    assert_eq!(document.diagnostics(), Vec::new());
}

#[test]
fn a_typst_syntax_error_is_at_its_place_in_the_string() {
    let text = chart("title('Mass', subtitle => 'in $x')");
    let document = common::document(&text);
    let diagnostics = document.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.source, Source::Typst);
    assert!(
        diagnostic.message.ends_with("in Typst markup"),
        "{diagnostic:?}"
    );
    // The error is inside the subtitle's string, not over the whole call.
    let subtitle = text.find("'in $x'").unwrap();
    let start = usize::from(diagnostic.range.start());
    assert!((subtitle..subtitle + 7).contains(&start), "{diagnostic:?}");
    assert!(!diagnostic.range.is_empty());
}

/// The text and the language of each string in a language of its own.
fn embedded(document: &Document) -> Vec<(String, Embedded)> {
    let text = document.text();
    (document.embedded_strings().into_iter())
        .map(|string| (text[string.range].to_string(), string.language))
        .collect()
}

#[test]
fn the_strings_of_markup_and_of_typed_literals_are_in_languages_of_their_own() {
    let document = common::document(&chart("'*Mass* of ' 'penguins'"));
    assert_eq!(
        embedded(&document),
        [("'*Mass* of ' 'penguins'".to_string(), Embedded::Typst)]
    );
    // A plain title is no markup, and its subtitle neither.
    let plain = chart("title('*Mass*', subtitle => '_g_', syntax => 'plain')");
    assert_eq!(embedded(&common::document(&plain)), Vec::new());
    // A chart's format and the format of a function.
    let formats = "FROM penguins |> EXTEND FORMAT_DATE('%Y', CURRENT_DATE()) AS y |> AGGREGATE mark_text(x => species, y => body_mass_g, text => body_mass_g, format => '%.1f') AS chart";
    assert_eq!(
        embedded(&common::document(formats)),
        [
            ("'%Y'".to_string(), Embedded::Format),
            ("'%.1f'".to_string(), Embedded::Format),
        ]
    );
    // The values of typed literals, and not a plain string or the keywords.
    let typed = "FROM penguins |> EXTEND DATE '2024-01-03' AS d, NUMERIC '1.25' AS n, 'plain' AS p";
    assert_eq!(
        embedded(&common::document(typed)),
        [
            ("'2024-01-03'".to_string(), Embedded::TypedValue),
            ("'1.25'".to_string(), Embedded::TypedValue),
        ]
    );
}
