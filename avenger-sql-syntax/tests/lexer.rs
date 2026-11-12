//! The lexer: every byte belongs to a token, and malformed tokens keep a kind.

use avenger_sql_syntax::{
    lexer::{tokenize, unescape},
    SyntaxKind::{self, *},
};

fn kinds(text: &str) -> Vec<SyntaxKind> {
    let (tokens, _) = tokenize(text);
    tokens
        .into_iter()
        .map(|token| token.kind)
        .filter(|kind| !kind.is_trivia())
        .collect()
}

#[test]
fn tokens_cover_the_text() {
    let text = "FROM t -- c\n|> WHERE a <> b /* x */ AND `q` = 'v' # e";
    let (tokens, diagnostics) = tokenize(text);
    assert!(diagnostics.is_empty());
    let joined: String = tokens.iter().map(|token| &text[token.range]).collect();
    assert_eq!(joined, text);
}

#[test]
fn operators_follow_googlesql() {
    assert_eq!(kinds("a <> b >> c"), [WORD, LT, GT, WORD, GT, GT, WORD]);
    assert_eq!(
        kinds("|> || | => = <="),
        [PIPE, CONCAT, BIT_OR, FAT_ARROW, EQ, LT_EQ]
    );
    assert_eq!(kinds("1.x"), [FLOAT, WORD]);
    assert_eq!(kinds("t.2023 @p.123"), [WORD, DOT, WORD, PARAM, DOT, WORD]);
}

#[test]
fn literals_hold_their_escapes() {
    assert_eq!(
        unescape(r"a\n\x41\u00e9\101", false, false).unwrap(),
        "a\nAéA".as_bytes()
    );
    assert_eq!(unescape(r"\n", true, false).unwrap(), br"\n");
    assert!(unescape(r"\q", false, false).is_err());
    assert!(unescape(r"\u0041", false, true).is_err());
}

#[test]
fn malformed_tokens_keep_a_kind() {
    for (text, kind) in [
        ("'abc", STRING),
        ("`x", QUOTED_IDENT),
        ("/* open", COMMENT),
        ("5abc", WORD),
        ("0x", INTEGER),
        ("@@x", PARAM),
        ("\u{1}", ERROR_TOKEN),
    ] {
        let (tokens, diagnostics) = tokenize(text);
        assert_eq!(tokens[0].kind, kind, "{text:?}");
        assert_eq!(diagnostics.len(), 1, "{text:?}");
    }
}
