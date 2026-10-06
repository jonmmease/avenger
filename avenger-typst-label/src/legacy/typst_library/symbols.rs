//! Retained Typst symbol and emoji definitions.
//!
//! Upstream Typst wires symbols through `typst-library/src/symbols.rs`, backed
//! by the generated `codex` symbol tree and `typst-library/src/foundations/symbol.rs`
//! for modifier resolution. Labels mirror the `codex` module data in
//! `symbols/{sym,emoji}.txt` and keep just enough of the `Symbol` model to
//! resolve root names plus dot modifiers.

use std::cmp::Reverse;
use std::sync::LazyLock;

static SYMBOLS: LazyLock<Vec<SymbolEntry>> = LazyLock::new(|| parse_symbol_data(SYM_DATA));
static EMOJI: LazyLock<Vec<SymbolEntry>> = LazyLock::new(|| parse_symbol_data(EMOJI_DATA));

const SYM_DATA: &str = include_str!("symbols/sym.txt");
const EMOJI_DATA: &str = include_str!("symbols/emoji.txt");

/// Resolve a retained `sym.*` name or bare math symbol name.
pub(crate) fn named_symbol(name: &str) -> Option<&'static str> {
    lookup_symbol(name, &SYMBOLS)
}

/// Resolve a retained `emoji.*` name.
pub(crate) fn named_emoji(name: &str) -> Option<&'static str> {
    lookup_symbol(name, &EMOJI)
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

fn lookup_symbol(name: &str, table: &'static [SymbolEntry]) -> Option<&'static str> {
    if let Some(entry) = table.iter().find(|entry| entry.name == name) {
        return entry.best_match(&[]).map(String::as_str);
    }

    table
        .iter()
        .filter_map(|entry| {
            let modifiers = name.strip_prefix(&entry.name)?.strip_prefix('.')?;
            let modifiers = modifiers
                .split('.')
                .filter(|modifier| !modifier.is_empty())
                .collect::<Vec<_>>();
            (!modifiers.is_empty()).then(|| {
                entry
                    .best_match(&modifiers)
                    .map(|value| (entry.name.len(), value))
            })?
        })
        .max_by_key(|(base_len, _)| *base_len)
        .map(|(_, value)| value.as_str())
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

#[derive(Debug)]
struct SymbolEntry {
    name: String,
    variants: Vec<SymbolVariant>,
}

impl SymbolEntry {
    fn best_match(&self, modifiers: &[&str]) -> Option<&String> {
        let mut best = None;
        let mut best_score = None;
        for variant in self.variants.iter().filter(|variant| {
            modifiers
                .iter()
                .all(|modifier| variant.modifiers.iter().any(|item| item == modifier))
        }) {
            let matching = variant
                .modifiers
                .iter()
                .filter(|modifier| modifiers.contains(&modifier.as_str()))
                .count();
            let score = (matching, Reverse(variant.modifiers.len()));
            if best_score.is_none_or(|current| score > current) {
                best = Some(&variant.value);
                best_score = Some(score);
            }
        }
        best
    }
}

#[derive(Debug)]
struct SymbolVariant {
    modifiers: Vec<String>,
    value: String,
}

fn parse_symbol_data(data: &'static str) -> Vec<SymbolEntry> {
    let mut entries: Vec<SymbolEntry> = Vec::new();
    let mut module_stack: Vec<&'static str> = Vec::new();
    let mut current_symbol: Option<usize> = None;

    for raw_line in data.lines() {
        let line = raw_line
            .split_once("//")
            .map_or(raw_line, |(head, _)| head)
            .trim();
        if line.is_empty() || line.starts_with("@deprecated:") {
            continue;
        }
        if line == "}" {
            module_stack.pop();
            current_symbol = None;
            continue;
        }

        let (head, tail) = line
            .split_once(' ')
            .map_or((line, None), |(head, tail)| (head, Some(tail.trim())));
        if tail == Some("{") {
            module_stack.push(head);
            current_symbol = None;
            continue;
        }

        if let Some(modifiers) = head.strip_prefix('.') {
            let Some(symbol_idx) = current_symbol else {
                panic!("symbol variant without preceding symbol in codex data: {line}");
            };
            let value = decode_symbol_value(tail.expect("codex symbol variant has a value"));
            entries[symbol_idx].variants.push(SymbolVariant {
                modifiers: split_modifiers(modifiers),
                value,
            });
            continue;
        }

        let mut name = module_stack.join(".");
        if !name.is_empty() {
            name.push('.');
        }
        name.push_str(head);

        let mut variants = Vec::new();
        if let Some(value) = tail {
            variants.push(SymbolVariant {
                modifiers: Vec::new(),
                value: decode_symbol_value(value),
            });
        }
        entries.push(SymbolEntry { name, variants });
        current_symbol = Some(entries.len() - 1);
    }

    entries
}

fn split_modifiers(modifiers: &str) -> Vec<String> {
    modifiers
        .split('.')
        .map(ToString::to_string)
        .collect::<Vec<_>>()
}

fn decode_symbol_value(mut text: &str) -> String {
    let mut result = String::new();
    loop {
        if let Some(rest) = text.strip_prefix("\\u{") {
            let (code, tail) = rest
                .split_once('}')
                .expect("codex unicode escape is closed");
            result.push(
                u32::from_str_radix(code, 16)
                    .ok()
                    .and_then(|value| char::try_from(value).ok())
                    .expect("codex unicode escape is valid"),
            );
            text = tail;
        } else if let Some(rest) = text.strip_prefix("\\vs{") {
            let (value, tail) = rest.split_once('}').expect("codex VS escape is closed");
            result.push(match value {
                "1" => '\u{fe00}',
                "2" => '\u{fe01}',
                "3" => '\u{fe02}',
                "4" => '\u{fe03}',
                "5" => '\u{fe04}',
                "6" => '\u{fe05}',
                "7" => '\u{fe06}',
                "8" => '\u{fe07}',
                "9" => '\u{fe08}',
                "10" => '\u{fe09}',
                "11" => '\u{fe0a}',
                "12" => '\u{fe0b}',
                "13" => '\u{fe0c}',
                "14" => '\u{fe0d}',
                "15" | "text" => '\u{fe0e}',
                "16" | "emoji" => '\u{fe0f}',
                _ => panic!("unsupported codex variation selector: {value}"),
            });
            text = tail;
        } else if let Some((prefix, tail)) = text.find('\\').map(|idx| text.split_at(idx)) {
            assert!(
                !prefix.is_empty(),
                "unsupported codex escape sequence: {tail}"
            );
            result.push_str(prefix);
            text = tail;
        } else {
            result.push_str(text);
            return result;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_retained_symbol_aliases() {
        assert_eq!(named_symbol("alpha"), Some("α"));
        assert_eq!(named_symbol("arrow.r"), Some("→"));
        assert_eq!(named_symbol("gt.eq.not"), Some("≱"));
        assert_eq!(named_symbol("subset.eq"), Some("⊆"));
        assert_eq!(named_symbol("forces.not"), Some("⊮"));
        assert_eq!(named_symbol("interleave.big"), Some("⫼"));
        assert_eq!(named_symbol("gender.male.stroke.t"), Some("⚨"));
        assert_eq!(named_symbol("control.dc.three"), Some("␓"));
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
        assert_eq!(named_emoji("face.halo"), Some("😇"));
        assert_eq!(named_emoji("chart.up"), Some("📈"));
    }

    #[test]
    fn rejects_unknown_symbol_and_modifier_aliases() {
        assert_eq!(named_symbol("arrow.diagonal"), None);
        assert_eq!(named_symbol("arrow.r.double.long.extra"), None);
        assert_eq!(named_emoji("face.not.real"), None);
    }
}
