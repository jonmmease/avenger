//! Retained math function vocabulary and option schemas.
//!
//! This module mirrors the small `typst-library` math surface Avenger labels
//! support. Evaluation lowers syntax into these retained math concepts, but the
//! question of which math functions belong to the supported label subset lives
//! here with the math library model.

pub(crate) fn is_base_math_call_name(name: &str) -> bool {
    matches!(
        name,
        "frac"
            | "sqrt"
            | "root"
            | "binom"
            | "abs"
            | "norm"
            | "floor"
            | "ceil"
            | "round"
            | "lr"
            | "mid"
            | "class"
            | "underline"
            | "overline"
            | "underbrace"
            | "overbrace"
            | "underbracket"
            | "overbracket"
            | "underparen"
            | "overparen"
            | "undershell"
            | "overshell"
            | "bb"
            | "cal"
            | "frak"
            | "sans"
            | "mono"
            | "serif"
            | "scr"
            | "upright"
            | "italic"
            | "bold"
            | "display"
            | "inline"
            | "script"
            | "sscript"
            | "stretch"
    )
}

pub(crate) fn is_builtin_math_control_name(name: &str) -> bool {
    matches!(name, "op" | "attach" | "cancel" | "scripts" | "limits")
}

pub(crate) fn is_retained_math_name(
    name: &str,
    has_predefined_operator: impl Fn(&str) -> bool,
    has_named_accent: impl Fn(&str) -> bool,
    has_named_symbol: impl Fn(&str) -> bool,
) -> bool {
    is_builtin_math_control_name(name)
        || is_math_spacing_name(name)
        || is_math_differential_name(name)
        || is_math_call_name(name, has_predefined_operator, has_named_accent)
        || is_unsupported_math_table_call_name(name)
        || has_named_symbol(name)
}

pub(crate) fn is_math_spacing_name(name: &str) -> bool {
    matches!(name, "thin" | "med" | "thick" | "quad" | "wide")
}

pub(crate) fn is_math_differential_name(name: &str) -> bool {
    matches!(name, "dif" | "Dif")
}

pub(crate) fn is_math_call_name(
    name: &str,
    has_predefined_operator: impl Fn(&str) -> bool,
    has_named_accent: impl Fn(&str) -> bool,
) -> bool {
    is_base_math_call_name(name)
        || has_predefined_operator(name)
        || is_math_accent_call_name(name, has_named_accent)
        || is_math_delimiter_symbol_call_name(name)
}

pub(crate) fn is_unsupported_math_table_call_name(name: &str) -> bool {
    matches!(name, "mat" | "vec" | "cases")
}

pub(crate) fn is_math_size_call_name(name: &str) -> bool {
    matches!(name, "display" | "inline" | "script" | "sscript")
}

pub(crate) fn is_math_delimiter_helper_call_name(name: &str) -> bool {
    matches!(name, "abs" | "norm" | "floor" | "ceil" | "round")
}

pub(crate) fn is_math_delimiter_symbol_call_name(name: &str) -> bool {
    matches!(
        name,
        "ceil.l"
            | "floor.l"
            | "paren.l"
            | "brace.l"
            | "bracket.l"
            | "chevron.l"
            | "bar"
            | "bar.double"
    )
}

pub(crate) fn is_math_accent_call_name(name: &str, named_accent: impl Fn(&str) -> bool) -> bool {
    name == "accent" || named_accent(name)
}

pub(crate) fn is_math_under_over_call_name(name: &str) -> bool {
    matches!(
        name,
        "underbrace"
            | "overbrace"
            | "underbracket"
            | "overbracket"
            | "underparen"
            | "overparen"
            | "undershell"
            | "overshell"
    )
}
