use std::collections::BTreeMap;

use crate::{
    SourceFile, SourceId, SourceOrigin,
    print::print_file,
    sql::{CommentKind, TokenClass, TokenStream, tokenize},
};

use super::{ParseError, ParsedFile, parse_file};

pub fn format_source(source: &SourceFile) -> Result<String, ParseError> {
    parse_file(source).map(|parsed| format_parsed(&parsed))
}

pub fn format_parsed(parsed: &ParsedFile) -> String {
    let canonical = print_file(&parsed.ast);
    let canonical_source = SourceFile::new(
        SourceId::new(parsed.concrete.source().id.get()),
        SourceOrigin::Memory("<canonical-format>".into()),
        canonical.clone(),
    );
    let canonical_tokens = tokenize(&canonical_source)
        .expect("canonical semantic output must tokenize under the Avenger dialect");
    let anchors = collect_anchors(parsed.concrete.tokens());
    reinsert_comments(&canonical, &canonical_tokens, &anchors)
}

#[derive(Clone, Copy)]
enum Side {
    Before,
    AfterLine,
}

struct CommentAnchor {
    text: String,
    kind: CommentKind,
    side: Side,
    window: Vec<String>,
    pivot: usize,
    window_ordinal: usize,
    token: String,
    token_ordinal: usize,
}

struct Significant<'a> {
    stream_index: usize,
    signature: String,
    token: &'a crate::sql::LanguageToken,
}

fn collect_anchors(stream: &TokenStream) -> Vec<CommentAnchor> {
    let significant = significant(stream);
    let mut anchors = Vec::new();
    for (comment_index, comment) in stream.tokens().iter().enumerate() {
        let TokenClass::Comment(kind) = comment.class() else {
            continue;
        };
        let text = stream.raw(comment).trim().to_owned();
        if text.starts_with("-- |") {
            continue;
        }
        let previous = significant
            .iter()
            .rposition(|token| token.stream_index < comment_index);
        let next = significant
            .iter()
            .position(|token| token.stream_index > comment_index);
        let trailing = previous.is_some_and(|position| {
            let token = significant[position].token;
            !stream.text()[token.span().range.end..comment.span().range.start].contains('\n')
        });
        let (side, pivot_position, window_start, pivot) = if trailing {
            let position = previous.expect("trailing comment has a preceding token");
            let start = position.saturating_sub(2);
            (Side::AfterLine, position, start, position - start)
        } else if let Some(position) = next {
            (Side::Before, position, position, 0)
        } else if let Some(position) = previous {
            let start = position.saturating_sub(2);
            (Side::AfterLine, position, start, position - start)
        } else {
            continue;
        };
        let window = significant[window_start..(window_start + 3).min(significant.len())]
            .iter()
            .map(|token| token.signature.clone())
            .collect::<Vec<_>>();
        let window_ordinal = window_ordinal(&significant, &window, window_start);
        let token = significant[pivot_position].signature.clone();
        let token_ordinal = significant[..=pivot_position]
            .iter()
            .filter(|candidate| candidate.signature == token)
            .count()
            - 1;
        anchors.push(CommentAnchor {
            text,
            kind,
            side,
            window,
            pivot,
            window_ordinal,
            token,
            token_ordinal,
        });
    }
    anchors
}

fn reinsert_comments(canonical: &str, stream: &TokenStream, anchors: &[CommentAnchor]) -> String {
    let significant = significant(stream);
    let mut insertions: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for anchor in anchors {
        let pivot = find_window(&significant, &anchor.window, anchor.window_ordinal)
            .and_then(|start| significant.get(start + anchor.pivot))
            .or_else(|| {
                significant
                    .iter()
                    .filter(|token| token.signature == anchor.token)
                    .nth(anchor.token_ordinal)
            });
        let Some(pivot) = pivot else {
            continue;
        };
        let (offset, insertion) = match anchor.side {
            Side::Before => {
                let offset = pivot.token.span().range.start;
                let line_start = canonical[..offset].rfind('\n').map_or(0, |index| index + 1);
                let indent = &canonical[line_start..offset];
                (offset, format!("{}\n{indent}", normalized_comment(anchor)))
            }
            Side::AfterLine => {
                let token_end = pivot.token.span().range.end;
                let offset = canonical[token_end..]
                    .find('\n')
                    .map_or(canonical.len(), |relative| token_end + relative);
                (offset, format!(" {}", normalized_comment(anchor)))
            }
        };
        insertions.entry(offset).or_default().push(insertion);
    }

    let mut output = canonical.to_owned();
    for (offset, values) in insertions.into_iter().rev() {
        output.insert_str(offset, &values.join("\n"));
    }
    output
}

fn significant(stream: &TokenStream) -> Vec<Significant<'_>> {
    stream
        .tokens()
        .iter()
        .enumerate()
        .filter(|(_, token)| {
            !matches!(
                token.class(),
                TokenClass::Whitespace(_) | TokenClass::Comment(_) | TokenClass::Eof
            )
        })
        .map(|(stream_index, token)| Significant {
            stream_index,
            signature: signature(token),
            token,
        })
        .collect()
}

fn signature(token: &crate::sql::LanguageToken) -> String {
    format!(
        "{:?}:{}",
        token.class(),
        token.token().to_string().to_lowercase()
    )
}

fn window_ordinal(significant: &[Significant<'_>], window: &[String], target: usize) -> usize {
    (0..=target)
        .filter(|start| window_matches(significant, window, *start))
        .count()
        .saturating_sub(1)
}

fn find_window(
    significant: &[Significant<'_>],
    window: &[String],
    ordinal: usize,
) -> Option<usize> {
    (0..significant.len())
        .filter(|start| window_matches(significant, window, *start))
        .nth(ordinal)
}

fn window_matches(significant: &[Significant<'_>], window: &[String], start: usize) -> bool {
    significant
        .get(start..start + window.len())
        .is_some_and(|tokens| {
            tokens
                .iter()
                .map(|token| &token.signature)
                .eq(window.iter())
        })
}

fn normalized_comment(anchor: &CommentAnchor) -> String {
    match anchor.kind {
        CommentKind::Line => anchor.text.clone(),
        CommentKind::Block => anchor.text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        SourceFile, SourceId, SourceOrigin,
        print::print_file,
        sql::{CommentKind, TokenClass, tokenize},
        syntax::parse_file,
    };

    use super::format_source;

    fn source(id: u32, text: impl Into<String>) -> SourceFile {
        SourceFile::new(
            SourceId::new(id),
            SourceOrigin::Memory(format!("format-{id}.avenger")),
            text.into(),
        )
    }

    #[test]
    fn format_is_a_fixpoint_and_moves_property_comments_with_the_property() {
        let original = source(
            1,
            r#"avenger 1;
chart cartesian {
 z: 2; -- last property
 -- comment for a
 a: 1;
 mark symbol { /* x channel */ x: "x"; }
}
"#,
        );
        let formatted = format_source(&original).unwrap();
        assert!(formatted.find("comment for a").unwrap() < formatted.find("a: 1;").unwrap());
        assert!(formatted.find("a: 1;").unwrap() < formatted.find("z: 2;").unwrap());
        assert_eq!(
            formatted,
            format_source(&source(2, formatted.clone())).unwrap()
        );
        assert_eq!(
            strip_ordinary_comments(&formatted),
            print_file(&parse_file(&original).unwrap().ast)
        );
    }

    #[test]
    fn format_without_ordinary_comments_is_the_semantic_printer() {
        let input = source(
            1,
            "avenger 1; chart cartesian { z: 2; -- | documented\n mark symbol {} a: 1; }",
        );
        let parsed = parse_file(&input).unwrap();
        assert_eq!(format_source(&input).unwrap(), print_file(&parsed.ast));
    }

    #[test]
    fn format_preserves_sql_comments_as_valid_source() {
        let input = source(
            1,
            "avenger 1; chart cartesian { table sql as t { sql: SELECT * -- rows\n FROM $items; } }",
        );
        let formatted = format_source(&input).unwrap();
        assert!(formatted.contains("-- rows"));
        parse_file(&source(2, formatted)).unwrap();
    }

    #[test]
    fn format_anchors_comments_across_imports_and_sibling_module_items() {
        let input = source(
            1,
            r#"avenger 1;
-- imported charts
import { chart as example } from './library.avenger'; -- exact module

-- first item
export chart cartesian as first {}

-- second item
chart polar as second {}
"#,
        );
        let formatted = format_source(&input).unwrap();
        assert!(formatted.find("imported charts").unwrap() < formatted.find("import {").unwrap());
        assert!(formatted.find("import {").unwrap() < formatted.find("exact module").unwrap());
        assert!(formatted.find("first item").unwrap() < formatted.find("as first").unwrap());
        assert!(formatted.find("second item").unwrap() < formatted.find("as second").unwrap());
        assert_eq!(
            formatted,
            format_source(&source(2, formatted.clone())).unwrap()
        );
    }

    fn strip_ordinary_comments(source_text: &str) -> String {
        let source = source(99, source_text);
        let tokens = tokenize(&source).unwrap();
        let mut ranges = Vec::new();
        for token in tokens.tokens() {
            if !matches!(token.class(), TokenClass::Comment(_))
                || tokens.raw(token).starts_with("-- |")
            {
                continue;
            }
            let mut start = token.span().range.start;
            let mut end = token.span().range.end;
            let line_start = source_text[..start]
                .rfind('\n')
                .map_or(0, |index| index + 1);
            let own_line = source_text[line_start..start].trim().is_empty();
            if own_line {
                start = line_start;
                if source_text.as_bytes().get(end) == Some(&b'\n') {
                    end += 1;
                }
            } else if source_text[..start].ends_with(' ') {
                start -= 1;
            }
            if !own_line
                && matches!(token.class(), TokenClass::Comment(CommentKind::Line))
                && source_text[..end].ends_with('\n')
            {
                end -= 1;
                if source_text[..end].ends_with('\r') {
                    end -= 1;
                }
            }
            ranges.push(start..end);
        }
        let mut stripped = source_text.to_owned();
        for range in ranges.into_iter().rev() {
            stripped.replace_range(range, "");
        }
        stripped
    }
}
