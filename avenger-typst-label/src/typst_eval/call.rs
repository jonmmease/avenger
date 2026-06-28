use crate::typst_library::Color;
use crate::typst_library::text::content::TextMarkupKind;

pub(crate) fn text_span_kind(name: &str) -> Option<TextMarkupKind> {
    match name {
        "underline" => Some(TextMarkupKind::Underline),
        "strike" => Some(TextMarkupKind::Strike),
        "overline" => Some(TextMarkupKind::Overline),
        "sub" => Some(TextMarkupKind::Subscript),
        "super" => Some(TextMarkupKind::Superscript),
        "highlight" => Some(TextMarkupKind::Highlight),
        "lower" => Some(TextMarkupKind::Lower),
        "upper" => Some(TextMarkupKind::Upper),
        "smallcaps" => Some(TextMarkupKind::Smallcaps),
        "emph" => Some(TextMarkupKind::Emph),
        "strong" => Some(TextMarkupKind::Strong),
        "raw" => Some(TextMarkupKind::Raw),
        _ => None,
    }
}

pub(crate) fn is_retained_markup_name(name: &str) -> bool {
    text_span_kind(name).is_some()
        || matches!(name, "auto" | "true" | "false" | "none" | "sym" | "emoji")
        || named_color(name).is_some()
}

pub(crate) fn named_color(name: &str) -> Option<Color> {
    Some(match name {
        "black" => Color::rgba(0.0, 0.0, 0.0, 1.0),
        "white" => Color::rgba(1.0, 1.0, 1.0, 1.0),
        "red" => Color::rgba(1.0, 0.0, 0.0, 1.0),
        "green" => Color::rgba(0.0, 0.5, 0.0, 1.0),
        "blue" => Color::rgba(0.0, 0.0, 1.0, 1.0),
        "yellow" => Color::rgba(1.0, 1.0, 0.0, 1.0),
        "orange" => Color::rgba(1.0, 0.65, 0.0, 1.0),
        "purple" => Color::rgba(0.5, 0.0, 0.5, 1.0),
        "maroon" => Color::rgba(0.5, 0.0, 0.0, 1.0),
        "gray" | "grey" => Color::rgba(0.5, 0.5, 0.5, 1.0),
        "silver" => Color::rgba(0.75, 0.75, 0.75, 1.0),
        "teal" => Color::rgba(0.0, 0.5, 0.5, 1.0),
        "aqua" | "cyan" => Color::rgba(0.0, 1.0, 1.0, 1.0),
        "navy" => Color::rgba(0.0, 0.0, 0.5, 1.0),
        "lime" => Color::rgba(0.0, 1.0, 0.0, 1.0),
        "olive" => Color::rgba(0.5, 0.5, 0.0, 1.0),
        "fuchsia" | "magenta" => Color::rgba(1.0, 0.0, 1.0, 1.0),
        _ => return None,
    })
}

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
        || is_math_call_name(name, has_predefined_operator, has_named_accent)
        || is_unsupported_math_table_call_name(name)
        || has_named_symbol(name)
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
        "ceil.l" | "floor.l" | "paren.l" | "brace.l" | "bracket.l" | "chevron.l" | "bar.double"
    )
}

pub(crate) fn is_math_accent_call_name(name: &str, named_accent: impl Fn(&str) -> bool) -> bool {
    name == "accent" || named_accent(name)
}
