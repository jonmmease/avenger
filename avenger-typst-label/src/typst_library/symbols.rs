//! Retained Typst symbol and emoji definitions.
//!
//! Upstream Typst wires symbols through `typst-library/src/symbols.rs`, backed
//! by the generated `codex` symbol tree and `typst-library/src/foundations/symbol.rs`
//! for modifier resolution. Labels keep a compact owned table here, with the
//! same public concept: a root symbol name plus dot modifiers.

/// Resolve a retained `sym.*` name or bare math symbol name.
pub(crate) fn named_symbol(name: &str) -> Option<&'static str> {
    lookup_symbol(name, RETAINED_SYMBOLS)
}

/// Resolve a retained `emoji.*` name.
pub(crate) fn named_emoji(name: &str) -> Option<&'static str> {
    lookup_symbol(name, RETAINED_EMOJI)
}

/// Resolve retained math accent function names.
pub(crate) fn named_accent_char(name: &str) -> Option<char> {
    match name {
        "grave" => Some('\u{0300}'),
        "acute" => Some('\u{0301}'),
        "hat" => Some('\u{0302}'),
        "tilde" => Some('\u{0303}'),
        "macron" => Some('\u{0304}'),
        "dash" => Some('\u{0305}'),
        "breve" => Some('\u{0306}'),
        "dot" => Some('\u{0307}'),
        "dot.double" | "ddot" | "diaer" => Some('\u{0308}'),
        "dot.triple" => Some('\u{20db}'),
        "dot.quad" => Some('\u{20dc}'),
        "circle" => Some('\u{030a}'),
        "acute.double" => Some('\u{030b}'),
        "caron" => Some('\u{030c}'),
        "arrow" | "arrow.r" => Some('\u{20d7}'),
        "arrow.l" => Some('\u{20d6}'),
        "arrow.l.r" => Some('\u{20e1}'),
        "harpoon" => Some('\u{20d1}'),
        "harpoon.lt" => Some('\u{20d0}'),
        _ => None,
    }
}

/// Resolve a literal accent value to a combining mark.
pub(crate) fn normalize_accent_text(value: &str) -> Option<char> {
    named_accent_char(value).or_else(|| {
        ACCENT_ALIASES
            .iter()
            .find_map(|(accent, aliases)| aliases.contains(&value).then_some(*accent))
            .or_else(|| value.parse::<char>().ok())
    })
}

fn lookup_symbol(name: &str, table: &[(&'static str, &'static str)]) -> Option<&'static str> {
    table
        .iter()
        .find_map(|(candidate, value)| (*candidate == name).then_some(*value))
        .or_else(|| {
            let (base, modifiers) = split_symbol_name(name)?;
            table.iter().find_map(|(candidate, value)| {
                let (candidate_base, candidate_modifiers) = split_symbol_name(candidate)?;
                (candidate_base == base
                    && modifiers.len() == candidate_modifiers.len()
                    && modifiers
                        .iter()
                        .all(|modifier| candidate_modifiers.contains(modifier)))
                .then_some(*value)
            })
        })
}

fn split_symbol_name(name: &str) -> Option<(&str, Vec<&str>)> {
    let (base, modifiers) = name.split_once('.')?;
    let modifiers = modifiers.split('.').collect::<Vec<_>>();
    (!base.is_empty() && modifiers.iter().all(|modifier| !modifier.is_empty()))
        .then_some((base, modifiers))
}

const ACCENT_ALIASES: &[(char, &[&str])] = &[
    ('\u{0300}', &["`"]),
    ('\u{0301}', &["´"]),
    ('\u{0302}', &["^", "ˆ"]),
    ('\u{0303}', &["~", "∼", "˜"]),
    ('\u{0304}', &["¯"]),
    ('\u{0305}', &["-", "–", "‾", "−"]),
    ('\u{0306}', &["˘"]),
    ('\u{0307}', &[".", "˙", "⋅"]),
    ('\u{0308}', &["¨"]),
    ('\u{030a}', &["∘", "○"]),
    ('\u{030b}', &["˝"]),
    ('\u{030c}', &["ˇ"]),
    ('\u{20d6}', &["←"]),
    ('\u{20d7}', &["→", "⟶"]),
    ('\u{20e1}', &["↔", "↔\u{fe0e}", "⟷"]),
    ('\u{20d0}', &["↼"]),
    ('\u{20d1}', &["⇀"]),
];

const RETAINED_EMOJI: &[(&str, &str)] = &[("chart.up", "📈"), ("face", "😀"), ("rocket", "🚀")];

const RETAINED_SYMBOLS: &[(&str, &str)] = &[
    ("CC", "ℂ"),
    ("Delta", "Δ"),
    ("Gamma", "Γ"),
    ("Lambda", "Λ"),
    ("NN", "ℕ"),
    ("Omega", "Ω"),
    ("Phi", "Φ"),
    ("Pi", "Π"),
    ("Psi", "Ψ"),
    ("QQ", "ℚ"),
    ("RR", "ℝ"),
    ("Sigma", "Σ"),
    ("Theta", "Θ"),
    ("Upsilon", "Υ"),
    ("Xi", "Ξ"),
    ("ZZ", "ℤ"),
    ("aleph", "א"),
    ("alpha", "α"),
    ("angle", "∠"),
    ("approx", "≈"),
    ("approx.not", "≉"),
    ("arrow.b", "↓"),
    ("arrow.l", "←"),
    ("arrow.l.bar", "↤"),
    ("arrow.l.double", "⇐"),
    ("arrow.l.double.long", "⟸"),
    ("arrow.l.long", "⟵"),
    ("arrow.l.not", "↚"),
    ("arrow.l.r", "↔"),
    ("arrow.l.r.double", "⇔"),
    ("arrow.l.r.double.long", "⟺"),
    ("arrow.l.r.long", "⟷"),
    ("arrow.r", "→"),
    ("arrow.r.bar", "↦"),
    ("arrow.r.double", "⇒"),
    ("arrow.r.double.long", "⟹"),
    ("arrow.r.long", "⟶"),
    ("arrow.r.not", "↛"),
    ("arrow.t", "↑"),
    ("bar.v", "|"),
    ("bar.v.double", "‖"),
    ("beta", "β"),
    ("chi", "χ"),
    ("degree", "°"),
    ("delta", "δ"),
    ("div", "÷"),
    ("dot", "⋅"),
    ("dot.c", "·"),
    ("dot.op", "⋅"),
    ("dots", "…"),
    ("dots.h", "…"),
    ("dots.h.c", "⋯"),
    ("dots.v", "⋮"),
    ("ell", "ℓ"),
    ("emptyset", "∅"),
    ("epsilon", "ε"),
    ("eq", "="),
    ("eq.not", "≠"),
    ("eq.triple", "≡"),
    ("eq.triple.not", "≢"),
    ("equiv", "≡"),
    ("equiv.not", "≢"),
    ("eta", "η"),
    ("exists", "∃"),
    ("forall", "∀"),
    ("gamma", "γ"),
    ("gradient", "∇"),
    ("gt", ">"),
    ("gt.eq", "≥"),
    ("gt.eq.not", "≱"),
    ("gt.not", "≯"),
    ("in", "∈"),
    ("in.not", "∉"),
    ("in.rev", "∋"),
    ("in.rev.not", "∌"),
    ("infinity", "∞"),
    ("integral", "∫"),
    ("inter", "∩"),
    ("inter.big", "⋂"),
    ("iota", "ι"),
    ("kappa", "κ"),
    ("lambda", "λ"),
    ("lt", "<"),
    ("lt.eq", "≤"),
    ("lt.eq.not", "≰"),
    ("lt.not", "≮"),
    ("minus", "−"),
    ("minus.plus", "∓"),
    ("mu", "μ"),
    ("nabla", "∇"),
    ("nothing", "∅"),
    ("nu", "ν"),
    ("omega", "ω"),
    ("oo", "∞"),
    ("parallel", "∥"),
    ("partial", "∂"),
    ("perp", "⟂"),
    ("phi", "φ"),
    ("pi", "π"),
    ("plus", "+"),
    ("plus.minus", "±"),
    ("prod", "∏"),
    ("product", "∏"),
    ("prop", "∝"),
    ("psi", "ψ"),
    ("rho", "ρ"),
    ("sigma", "σ"),
    ("slash", "/"),
    ("subset", "⊂"),
    ("subset.eq", "⊆"),
    ("subset.eq.not", "⊈"),
    ("subset.neq", "⊊"),
    ("subset.not", "⊄"),
    ("sum", "∑"),
    ("supset", "⊃"),
    ("supset.eq", "⊇"),
    ("supset.eq.not", "⊉"),
    ("supset.neq", "⊋"),
    ("supset.not", "⊅"),
    ("tau", "τ"),
    ("theta", "θ"),
    ("times", "×"),
    ("times.big", "⨉"),
    ("union", "∪"),
    ("union.big", "⋃"),
    ("union.plus", "⊎"),
    ("upsilon", "υ"),
    ("xi", "ξ"),
    ("zeta", "ζ"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_retained_symbol_aliases() {
        assert_eq!(named_symbol("alpha"), Some("α"));
        assert_eq!(named_symbol("arrow.r"), Some("→"));
        assert_eq!(named_symbol("gt.eq.not"), Some("≱"));
        assert_eq!(named_symbol("subset.eq"), Some("⊆"));
    }

    #[test]
    fn retained_symbol_modifiers_are_order_insensitive() {
        assert_eq!(named_symbol("arrow.double.r"), Some("⇒"));
        assert_eq!(named_symbol("gt.not.eq"), Some("≱"));
        assert_eq!(named_symbol("eq.not.triple"), Some("≢"));
    }

    #[test]
    fn resolves_retained_emoji_aliases() {
        assert_eq!(named_emoji("face"), Some("😀"));
        assert_eq!(named_emoji("chart.up"), Some("📈"));
    }

    #[test]
    fn rejects_unknown_symbol_and_modifier_aliases() {
        assert_eq!(named_symbol("arrow.diagonal"), None);
        assert_eq!(named_symbol("arrow.r.double.long.extra"), None);
        assert_eq!(named_emoji("face.halo"), None);
    }
}
