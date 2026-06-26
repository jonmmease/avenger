use crate::delimiter::{parse_segments, MathDelimiterOptions, ParsedSegment};
use crate::error::MathTypesetError;

use super::ast::{
    OwnedEmojiAlias, OwnedLine, OwnedLineNode, OwnedMathSpan, OwnedPlainText, OwnedTextSpan,
    OwnedTextSpanKind,
};

pub(crate) fn parse_owned_line(
    source: &str,
    delimiters: &MathDelimiterOptions,
) -> Result<OwnedLine, MathTypesetError> {
    let mut nodes = Vec::new();

    for segment in parse_segments(source, delimiters)? {
        match segment {
            ParsedSegment::Plain { text, range } => {
                parse_plain_markup(&text, range.start, &mut nodes)?;
            }
            ParsedSegment::Math {
                source,
                source_range,
                delimiter,
            } => {
                nodes.push(OwnedLineNode::Math(OwnedMathSpan {
                    source,
                    source_range,
                    delimiter,
                }));
            }
        }
    }

    Ok(OwnedLine {
        source: source.to_string(),
        nodes,
    })
}

fn parse_plain_markup(
    source: &str,
    offset: usize,
    nodes: &mut Vec<OwnedLineNode>,
) -> Result<(), MathTypesetError> {
    let mut plain = String::new();
    let mut plain_start = offset;
    let mut pos = 0usize;

    while let Some((idx, ch)) = next_char(source, pos) {
        if ch == '\\' {
            let next_pos = idx + ch.len_utf8();
            if let Some((_, next)) = next_char(source, next_pos) {
                if next == '#' || next == '\\' {
                    plain.push(next);
                    pos = next_pos + next.len_utf8();
                    continue;
                }
            }
        }

        if ch == '#' {
            push_plain(nodes, &mut plain, plain_start, offset + idx);
            let command = read_hash_command(source, offset, idx)?;
            pos = command.end;
            plain_start = offset + pos;
            nodes.push(command.node);
            continue;
        }

        plain.push(ch);
        pos = idx + ch.len_utf8();
    }

    push_plain(nodes, &mut plain, plain_start, offset + source.len());
    Ok(())
}

struct ParsedCommand {
    node: OwnedLineNode,
    end: usize,
}

fn read_hash_command(
    source: &str,
    offset: usize,
    hash_idx: usize,
) -> Result<ParsedCommand, MathTypesetError> {
    let name_start = hash_idx + 1;
    let Some((name_end, name)) = read_command_name(source, name_start) else {
        return Err(unsupported(
            offset + hash_idx,
            "expected static Typst-shaped command after #",
        ));
    };

    if name == "emoji" {
        return read_emoji_alias(source, offset, hash_idx, name_end);
    }

    let Some(kind) = text_span_kind(name) else {
        return Err(unsupported(
            offset + hash_idx,
            "unsupported static text command",
        ));
    };

    match next_char(source, name_end) {
        Some((idx, '[')) => {
            let (body, close_idx) = read_bracket_body(source, idx)?;
            let mut body_nodes = Vec::new();
            parse_plain_markup(&body, offset + idx + 1, &mut body_nodes)?;
            Ok(ParsedCommand {
                node: OwnedLineNode::TextSpan(OwnedTextSpan {
                    kind,
                    body: body_nodes,
                    byte_range: offset + hash_idx..offset + close_idx + 1,
                    body_range: offset + idx + 1..offset + close_idx,
                }),
                end: close_idx + 1,
            })
        }
        Some((_, '(')) => Err(unsupported(
            offset + hash_idx,
            "static text commands do not support Typst-style options",
        )),
        _ => Err(unsupported(
            offset + hash_idx,
            "static text command expects bracketed content",
        )),
    }
}

fn read_emoji_alias(
    source: &str,
    offset: usize,
    hash_idx: usize,
    emoji_name_end: usize,
) -> Result<ParsedCommand, MathTypesetError> {
    let Some((dot_idx, '.')) = next_char(source, emoji_name_end) else {
        return Err(unsupported(
            offset + hash_idx,
            "emoji alias expects a dotted name",
        ));
    };

    let alias_start = dot_idx + 1;
    let alias_end = read_dotted_name_end(source, alias_start);
    if alias_end == alias_start {
        return Err(unsupported(
            offset + hash_idx,
            "emoji alias expects a dotted name",
        ));
    }

    let name = &source[alias_start..alias_end];
    let Some(emoji) = emoji_alias(name) else {
        return Err(unsupported(offset + hash_idx, "unknown emoji alias"));
    };

    Ok(ParsedCommand {
        node: OwnedLineNode::Emoji(OwnedEmojiAlias {
            name: name.to_string(),
            emoji,
            byte_range: offset + hash_idx..offset + alias_end,
        }),
        end: alias_end,
    })
}

fn read_command_name(source: &str, start: usize) -> Option<(usize, &str)> {
    let end = read_identifier_end(source, start);
    (end > start).then(|| (end, &source[start..end]))
}

fn read_identifier_end(source: &str, start: usize) -> usize {
    let mut end = start;
    while let Some((idx, ch)) = next_char(source, end) {
        if ch == '_' || ch == '-' || ch.is_ascii_alphanumeric() {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    end
}

fn read_dotted_name_end(source: &str, start: usize) -> usize {
    let mut end = start;
    while let Some((idx, ch)) = next_char(source, end) {
        if ch == '.' || ch == '_' || ch == '-' || ch.is_ascii_alphanumeric() {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    end
}

fn read_bracket_body(source: &str, open_idx: usize) -> Result<(String, usize), MathTypesetError> {
    let mut body = String::new();
    let mut depth = 0usize;
    let mut pos = open_idx;

    while let Some((idx, ch)) = next_char(source, pos) {
        if ch == '\\' {
            let next_pos = idx + ch.len_utf8();
            if let Some((_, next)) = next_char(source, next_pos) {
                body.push(next);
                pos = next_pos + next.len_utf8();
                continue;
            }
        }

        match ch {
            '[' => {
                if depth > 0 {
                    body.push(ch);
                }
                depth += 1;
            }
            ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok((body, idx));
                }
                body.push(ch);
            }
            _ => body.push(ch),
        }

        pos = idx + ch.len_utf8();
    }

    Err(unsupported(
        open_idx,
        "static text command has unterminated bracketed content",
    ))
}

fn text_span_kind(name: &str) -> Option<OwnedTextSpanKind> {
    match name {
        "underline" => Some(OwnedTextSpanKind::Underline),
        "strike" => Some(OwnedTextSpanKind::Strike),
        "overline" => Some(OwnedTextSpanKind::Overline),
        "sub" => Some(OwnedTextSpanKind::Subscript),
        "super" => Some(OwnedTextSpanKind::Superscript),
        "highlight" => Some(OwnedTextSpanKind::Highlight),
        _ => None,
    }
}

fn emoji_alias(name: &str) -> Option<&'static str> {
    match name {
        "face" => Some("😀"),
        "rocket" => Some("🚀"),
        "chart.up" => Some("📈"),
        _ => None,
    }
}

fn push_plain(nodes: &mut Vec<OwnedLineNode>, plain: &mut String, start: usize, end: usize) {
    if !plain.is_empty() {
        nodes.push(OwnedLineNode::Plain(OwnedPlainText {
            text: std::mem::take(plain),
            byte_range: start..end,
        }));
    }
}

fn next_char(source: &str, start: usize) -> Option<(usize, char)> {
    source[start..]
        .char_indices()
        .next()
        .map(|(offset, ch)| (start + offset, ch))
}

fn unsupported(position: usize, message: &'static str) -> MathTypesetError {
    MathTypesetError::UnsupportedSyntax { position, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> OwnedLine {
        parse_owned_line(source, &MathDelimiterOptions::default()).unwrap()
    }

    #[test]
    fn parses_plain_and_math_spans() {
        let line = parse("Price \\$7, score $R^2$ = 0.94");

        assert_eq!(line.nodes.len(), 3);
        assert!(
            matches!(&line.nodes[0], OwnedLineNode::Plain(plain) if plain.text == "Price $7, score ")
        );
        assert!(matches!(&line.nodes[1], OwnedLineNode::Math(math) if math.source == "R^2"));
        assert!(matches!(&line.nodes[2], OwnedLineNode::Plain(plain) if plain.text == " = 0.94"));
    }

    #[test]
    fn parses_static_text_spans() {
        let line = parse("This is #underline[important] and #strike[old]");

        assert_eq!(line.nodes.len(), 4);
        assert!(matches!(
            &line.nodes[1],
            OwnedLineNode::TextSpan(span)
                if span.kind == OwnedTextSpanKind::Underline
                    && matches!(&span.body[..], [OwnedLineNode::Plain(plain)] if plain.text == "important")
        ));
        assert!(matches!(
            &line.nodes[3],
            OwnedLineNode::TextSpan(span)
                if span.kind == OwnedTextSpanKind::Strike
                    && matches!(&span.body[..], [OwnedLineNode::Plain(plain)] if plain.text == "old")
        ));
    }

    #[test]
    fn parses_nested_static_text_spans() {
        let line = parse("#underline[important #super[2]]");

        let OwnedLineNode::TextSpan(span) = &line.nodes[0] else {
            panic!("expected outer span");
        };
        assert_eq!(span.kind, OwnedTextSpanKind::Underline);
        assert_eq!(span.body.len(), 2);
        assert!(
            matches!(&span.body[1], OwnedLineNode::TextSpan(inner) if inner.kind == OwnedTextSpanKind::Superscript)
        );
    }

    #[test]
    fn parses_named_emoji_aliases() {
        let line = parse("Revenue #emoji.rocket #emoji.chart.up");

        assert_eq!(line.nodes.len(), 4);
        assert!(
            matches!(&line.nodes[1], OwnedLineNode::Emoji(alias) if alias.name == "rocket" && alias.emoji == "🚀")
        );
        assert!(
            matches!(&line.nodes[3], OwnedLineNode::Emoji(alias) if alias.name == "chart.up" && alias.emoji == "📈")
        );
    }

    #[test]
    fn rejects_static_command_options() {
        let err = parse_owned_line(
            "#underline(stroke: red)[important]",
            &MathDelimiterOptions::default(),
        )
        .unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "static text commands do not support Typst-style options"
            }
        );
    }

    #[test]
    fn rejects_unknown_hash_commands() {
        let err = parse_owned_line("#let x = 1", &MathDelimiterOptions::default()).unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "unsupported static text command"
            }
        );
    }

    #[test]
    fn rejects_unknown_emoji_aliases() {
        let err =
            parse_owned_line("#emoji.not.real", &MathDelimiterOptions::default()).unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "unknown emoji alias"
            }
        );
    }
}
