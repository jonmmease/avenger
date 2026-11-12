//! The lexer: every byte belongs to a token, and malformed tokens keep a kind.

use avenger_sql_syntax::{
    lexer::tokenize,
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
fn operators_follow_googlesql() {
    assert_eq!(
        kinds("|> || | => = <="),
        [PIPE, CONCAT, BIT_OR, FAT_ARROW, EQ, LT_EQ]
    );
    assert_eq!(kinds("1.x"), [FLOAT, WORD]);
    assert_eq!(kinds("t.2023 @p.123"), [WORD, DOT, WORD, PARAM, DOT, WORD]);
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
