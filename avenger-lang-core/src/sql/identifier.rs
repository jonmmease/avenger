/// Returns whether `character` may begin an unquoted Avenger name.
///
/// This predicate is shared by bare DSL names, unquoted SQL identifiers, and
/// binding path segments. It intentionally does not apply SQL case folding or
/// Unicode normalization.
pub(crate) fn is_unquoted_identifier_start(character: char) -> bool {
    character.is_alphabetic() || character == '_'
}

/// Returns whether `character` may continue an unquoted Avenger name.
///
/// Only ASCII digits are admitted. In particular, combining marks and
/// non-ASCII numeric characters are not silently normalized or accepted.
pub(crate) fn is_unquoted_identifier_part(character: char) -> bool {
    is_unquoted_identifier_start(character) || character.is_ascii_digit()
}

pub(crate) fn is_unquoted_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if is_unquoted_identifier_start(first))
        && characters.all(is_unquoted_identifier_part)
}

#[cfg(test)]
mod tests {
    use super::is_unquoted_identifier;

    #[test]
    fn identifier_profile_is_unicode_alphabetic_with_ascii_digits() {
        for valid in ["_", "sales_2026", "café", "Δvalue"] {
            assert!(
                is_unquoted_identifier(valid),
                "expected `{valid}` to be valid"
            );
        }
        for invalid in [
            "",
            "2sales",
            "sales$value",
            "sales@start",
            "sales#value",
            "cafe\u{301}",
            "value٢",
        ] {
            assert!(
                !is_unquoted_identifier(invalid),
                "expected `{invalid}` to be invalid"
            );
        }
    }
}
