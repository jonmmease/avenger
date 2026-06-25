#![cfg(feature = "vendor-typst")]

use typst_syntax::{parse_math, SyntaxKind};

#[test]
fn vendored_typst_syntax_parses_common_math_fragment() {
    let root = parse_math("sqrt(x^2 + y^2)");
    let (errors, warnings) = root.errors_and_warnings();

    assert_eq!(root.kind(), SyntaxKind::Math);
    assert!(errors.is_empty(), "unexpected parse errors: {errors:?}");
    assert!(
        warnings.is_empty(),
        "unexpected parse warnings: {warnings:?}"
    );
}

#[test]
fn vendored_typst_syntax_reports_invalid_math_fragment() {
    let root = parse_math("x^");
    assert!(root.diagnosis().errors);
}
