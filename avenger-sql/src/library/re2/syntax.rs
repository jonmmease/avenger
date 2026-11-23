//! The translation of a pattern of RE2 into the `regex` pattern of its meaning, in three steps.
//!
//! 1. A pass over RE2's syntax rewrites what `regex` reads otherwise or rejects, so that the parse
//!    after it reads RE2's meaning: `\Q…\E` becomes escaped text, an octal escape such as
//!    `\101` a hexadecimal one, `\p{^Greek}` `\P{Greek}`, a `{` that starts no repetition `\{`,
//!    and an escaped punctuation character, such as `\<`, the character itself. A class is read by
//!    RE2's rules, where `[`, `&`, `-`, and `~` are characters rather than nested classes and set
//!    operations, and written with each character escaped. The names of named groups go, since
//!    RE2 allows names that `regex` doesn't, such as two groups of one name. The pass rejects what
//!    RE2 rejects there: a backreference such as `\1`, an escape that RE2 lacks, a flag other than
//!    `i`, `m`, `s`, and `U`, a property name other than `Any`, a general category, or a script, by
//!    its exact name, and `\C`, any byte, which a pattern over strings can't match alone.
//! 2. regex-syntax parses the result, and a walk rejects the repetitions that RE2 rejects: an
//!    operator right after another, such as `a**`, a count over 1000, and nested counts whose
//!    product is over 1000.
//! 3. The Perl classes and the word boundaries become ASCII, as they are in RE2.

use regex_syntax::ast::{
    self, parse::Parser, print::Printer, Assertion, AssertionKind, Ast, ClassAscii, ClassAsciiKind,
    ClassBracketed, ClassPerl, ClassPerlKind, ClassSet, ClassSetItem, ClassSetUnion, Flag, Flags,
    FlagsItem, FlagsItemKind, Group, GroupKind, Literal, LiteralKind, RepetitionKind,
    RepetitionRange, Span, SpecialLiteralKind,
};

type Result<T> = std::result::Result<T, String>;

/// The property names of RE2, exact and case-sensitive: `Any`, the general categories, and the
/// scripts, as `tests/data/re2_oracle.py` records them from RE2.
const PROPERTIES: &str = include_str!("properties.txt");

/// The names of RE2's POSIX classes, such as `[:alpha:]`.
const POSIX: &[&str] = &[
    "alnum", "alpha", "ascii", "blank", "cntrl", "digit", "graph", "lower", "print", "punct",
    "space", "upper", "word", "xdigit",
];

/// A class that matches no character, for a surrogate, which no string holds.
const NEVER: &str = "[^\\x{0}-\\x{10FFFF}]";

/// The `regex` pattern of the meaning of the RE2 pattern `pattern`.
pub(super) fn translate(pattern: &str) -> Result<String> {
    let lexed = lex(pattern)?;
    let mut ast = Parser::new()
        .parse(&lexed)
        .map_err(|error| error.to_string())?;
    check(&ast)?;
    ascii(&mut ast);
    let mut out = String::new();
    Printer::new()
        .print(&ast, &mut out)
        .map_err(|error| error.to_string())?;
    Ok(out)
}

/// The character `c` as `regex` reads it literally: an ASCII punctuation character escaped, and
/// any other as itself.
fn literal(c: char) -> String {
    match c.is_ascii() && !c.is_ascii_alphanumeric() {
        true => format!("\\x{{{:X}}}", c as u32),
        false => c.to_string(),
    }
}

/// The character of the code point `code`, or `None` for a surrogate, which no string holds.
fn character(code: u32) -> Option<char> {
    char::from_u32(code)
}

/// What an escape or a character of a class stands for.
enum Atom {
    /// A character, by its code point, which can be a surrogate.
    Char(u32),
    /// A class, such as `\d` or `\p{Greek}`, as `regex` writes it.
    Class(String),
}

/// Step 1: the pattern with RE2's syntax rewritten for `regex`.
fn lex(pattern: &str) -> Result<String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if chars.get(i + 1) == Some(&'Q') => {
                // Literal text, up to `\E` or the end.
                i += 2;
                while i < chars.len() && !(chars[i] == '\\' && chars.get(i + 1) == Some(&'E')) {
                    out.push_str(&literal(chars[i]));
                    i += 1;
                }
                i = (i + 2).min(chars.len().max(i));
            }
            '\\' if matches!(chars.get(i + 1), Some('b' | 'B')) => {
                // In a group of its own, so that a `{` after it is no word boundary of `regex`.
                out.push_str(&format!("(?:\\{})", chars[i + 1]));
                i += 2;
            }
            '\\' => {
                let (atom, end) = escape(&chars, i, false)?;
                match atom {
                    Atom::Class(class) => out.push_str(&class),
                    Atom::Char(code) => match character(code) {
                        Some(c) => out.push_str(&literal(c)),
                        None => out.push_str(NEVER),
                    },
                }
                i = end;
            }
            '[' => i = class(&chars, i, &mut out)?,
            '{' => match repetition(&chars, i) {
                Some(end) => {
                    out.extend(&chars[i..end]);
                    i = end;
                }
                None => {
                    out.push_str("\\{");
                    i += 1;
                }
            },
            '(' if chars.get(i + 1) == Some(&'?') => i = group(&chars, i, &mut out)?,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
}

/// The text of `chars` from `start` to `end`, for an error.
fn text(chars: &[char], start: usize, end: usize) -> String {
    chars[start..end.min(chars.len())].iter().collect()
}

/// The escape at `start` of `chars`, which starts with a backslash, in a class or not, and the
/// index after it.
fn escape(chars: &[char], start: usize, in_class: bool) -> Result<(Atom, usize)> {
    let invalid = |end: usize| {
        Err(format!(
            "invalid escape sequence: {}",
            text(chars, start, end)
        ))
    };
    let Some(&c) = chars.get(start + 1) else {
        return Err("trailing \\ at end of regular expression".into());
    };
    let at = start + 2;
    Ok(match c {
        // An octal escape: `\0` and up to two more octal digits, or a digit from 1 to 7 before
        // another octal digit, as a lone one is a backreference.
        '0'..='7' => {
            let octal = |i: usize| chars.get(i).filter(|c| ('0'..='7').contains(*c));
            if c != '0' && octal(at).is_none() {
                return invalid(at);
            }
            let mut code = c as u32 - '0' as u32;
            let mut end = at;
            while end < start + 4 {
                match octal(end) {
                    Some(digit) => code = code * 8 + (*digit as u32 - '0' as u32),
                    None => break,
                }
                end += 1;
            }
            (Atom::Char(code), end)
        }
        'x' => match chars.get(at) {
            Some('{') => {
                let close = (at..chars.len()).find(|&i| chars[i] == '}');
                let Some(close) = close else {
                    return invalid(chars.len());
                };
                let digits = text(chars, at + 1, close);
                match u32::from_str_radix(&digits, 16) {
                    Ok(code) if !digits.is_empty() && code <= 0x10FFFF => {
                        (Atom::Char(code), close + 1)
                    }
                    _ => return invalid(close),
                }
            }
            _ => {
                let digits = text(chars, at, at + 2);
                match u32::from_str_radix(&digits, 16) {
                    Ok(code) if digits.chars().count() == 2 => (Atom::Char(code), at + 2),
                    _ => return invalid(at + digits.chars().count()),
                }
            }
        },
        'p' | 'P' => property(chars, start)?,
        'd' | 'D' | 'w' | 'W' | 's' | 'S' => (Atom::Class(format!("\\{c}")), at),
        'a' => (Atom::Char(0x07), at),
        'f' => (Atom::Char(0x0C), at),
        't' => (Atom::Char(0x09), at),
        'n' => (Atom::Char(0x0A), at),
        'r' => (Atom::Char(0x0D), at),
        'v' => (Atom::Char(0x0B), at),
        'A' | 'z' if !in_class => (Atom::Class(format!("\\{c}")), at),
        'C' if !in_class => {
            return Err(
                "`\\C` matches one byte, which a pattern over strings can't match alone".into(),
            )
        }
        // An escaped punctuation character is itself.
        c if c.is_ascii() && !c.is_ascii_alphanumeric() => (Atom::Char(c as u32), at),
        _ => return invalid(at),
    })
}

/// The property escape at `start`, `\pX`, `\p{Name}`, or its negation, of a name that RE2 has.
fn property(chars: &[char], start: usize) -> Result<(Atom, usize)> {
    let negated = chars[start + 1] == 'P';
    let (name, end) = match chars.get(start + 2) {
        Some('{') => {
            let close = (start + 3..chars.len()).find(|&i| chars[i] == '}');
            let Some(close) = close else {
                return Err(format!(
                    "invalid character class range: {}",
                    text(chars, start, chars.len())
                ));
            };
            (text(chars, start + 3, close), close + 1)
        }
        Some(c) => (c.to_string(), start + 3),
        None => {
            return Err(format!(
                "invalid escape sequence: {}",
                text(chars, start, start + 2)
            ))
        }
    };
    let (negated, name) = match name.strip_prefix('^') {
        Some(name) => (!negated, name.to_string()),
        None => (negated, name),
    };
    if !PROPERTIES.split_whitespace().any(|known| known == name) {
        return Err(format!(
            "invalid character class range: {}",
            text(chars, start, end)
        ));
    }
    let p = if negated { 'P' } else { 'p' };
    Ok((Atom::Class(format!("\\{p}{{{name}}}")), end))
}

/// The class at `start` of `chars`, read by RE2's rules and written to `out` with each character
/// escaped, and the index after it.
fn class(chars: &[char], start: usize, out: &mut String) -> Result<usize> {
    let missing = || Err(format!("missing ]: {}", text(chars, start, chars.len())));
    let mut i = start + 1;
    let negated = chars.get(i) == Some(&'^');
    if negated {
        i += 1;
    }
    let mut items = String::new();
    let mut first = true;
    loop {
        let Some(&c) = chars.get(i) else {
            return missing();
        };
        if c == ']' && !first {
            i += 1;
            break;
        }
        first = false;
        if c == '[' && chars.get(i + 1) == Some(&':') {
            let close = (i + 2..chars.len().saturating_sub(1))
                .find(|&j| chars[j] == ':' && chars[j + 1] == ']');
            let Some(close) = close else {
                return missing();
            };
            let name = text(chars, i + 2, close);
            if !POSIX.contains(&name.trim_start_matches('^')) {
                return Err(format!(
                    "invalid character class range: {}",
                    text(chars, i, close + 2)
                ));
            }
            items.push_str(&format!("[:{name}:]"));
            i = close + 2;
            continue;
        }
        let range_start = i;
        let (lo, end) = class_atom(chars, i)?;
        i = end;
        let range = chars.get(i) == Some(&'-') && !matches!(chars.get(i + 1), Some(']') | None);
        match (lo, range) {
            (Atom::Char(lo), true) => {
                let (hi, end) = class_atom(chars, i + 1)?;
                let Atom::Char(hi) = hi else {
                    return Err(format!(
                        "invalid escape sequence: {}",
                        text(chars, i + 1, end)
                    ));
                };
                if hi < lo {
                    return Err(format!(
                        "invalid character class range: {}",
                        text(chars, range_start, end)
                    ));
                }
                i = end;
                // A surrogate end clamps to the characters around the surrogates.
                let lo = if (0xD800..=0xDFFF).contains(&lo) {
                    0xE000
                } else {
                    lo
                };
                let hi = if (0xD800..=0xDFFF).contains(&hi) {
                    0xD7FF
                } else {
                    hi
                };
                if let (Some(lo), Some(hi)) = (character(lo), character(hi)) {
                    if lo <= hi {
                        items.push_str(&format!("{}-{}", literal(lo), literal(hi)));
                    }
                }
            }
            (Atom::Char(code), false) => {
                if let Some(c) = character(code) {
                    items.push_str(&literal(c));
                }
            }
            (Atom::Class(class), _) => items.push_str(&class),
        }
    }
    match (items.is_empty(), negated) {
        (false, _) => {
            let caret = if negated { "^" } else { "" };
            out.push_str(&format!("[{caret}{items}]"));
        }
        // Only surrogates, which no string holds.
        (true, false) => out.push_str(NEVER),
        (true, true) => out.push_str("(?s:.)"),
    }
    Ok(i)
}

/// A character or an escape of a class at `start`, and the index after it.
fn class_atom(chars: &[char], start: usize) -> Result<(Atom, usize)> {
    match chars[start] {
        '\\' => escape(chars, start, true),
        c => Ok((Atom::Char(c as u32), start + 1)),
    }
}

/// The end of the repetition `{n}`, `{n,}`, or `{n,m}` at `start`, or `None` for a `{` that starts
/// none, which RE2 reads as a character.
fn repetition(chars: &[char], start: usize) -> Option<usize> {
    let digits = |from: usize| {
        (from..chars.len())
            .take_while(|&i| chars[i].is_ascii_digit())
            .count()
    };
    let mut i = start + 1;
    let n = digits(i);
    if n == 0 {
        return None;
    }
    i += n;
    if chars.get(i) == Some(&',') {
        i += 1;
        i += digits(i);
    }
    (chars.get(i) == Some(&'}')).then_some(i + 1)
}

/// The group at `start`, which starts with `(?`: a named group, written without its name, or a
/// group of RE2's flags, `i`, `m`, `s`, and `U`, and the index after its opening.
fn group(chars: &[char], start: usize, out: &mut String) -> Result<usize> {
    let at = start + 2;
    let named = match (chars.get(at), chars.get(at + 1)) {
        (Some('P'), Some('<')) => Some(at + 2),
        (Some('<'), Some(next)) if *next != '=' && *next != '!' => Some(at + 1),
        _ => None,
    };
    if let Some(name) = named {
        let close = (name..chars.len()).find(|&i| chars[i] == '>');
        return match close {
            Some(close) if close > name => {
                out.push('(');
                Ok(close + 1)
            }
            _ => Err(format!(
                "invalid named capture group: {}",
                text(chars, start, close.map_or(chars.len(), |close| close + 1))
            )),
        };
    }
    let bad = |end: usize| {
        Err(format!(
            "invalid perl operator: {}",
            text(chars, start, end)
        ))
    };
    let mut i = at;
    let (mut negated, mut flag_after_negation) = (false, false);
    let mut flags = String::new();
    loop {
        match chars.get(i) {
            Some(c @ ('i' | 'm' | 's' | 'U')) => {
                flags.push(*c);
                flag_after_negation |= negated;
            }
            Some('-') if !negated => {
                negated = true;
                flags.push('-');
            }
            Some(close @ (')' | ':')) => {
                if negated && !flag_after_negation {
                    return bad(i + 1);
                }
                match (*close, flags.is_empty()) {
                    // `(?)` sets no flags.
                    (')', true) => {}
                    (close, _) => out.push_str(&format!("(?{flags}{close}")),
                }
                return Ok(i + 1);
            }
            _ => return bad(i + 1),
        }
        i += 1;
    }
}

/// Step 2: the repetitions of `ast` that RE2 rejects.
fn check(ast: &Ast) -> Result<()> {
    match ast {
        Ast::Repetition(repetition) => {
            check(&repetition.ast)?;
            if let Ast::Repetition(inner) = repetition.ast.as_ref() {
                return Err(format!(
                    "bad repetition operator: {}{}",
                    operator(inner),
                    operator(repetition)
                ));
            }
            if let RepetitionKind::Range(range) = &repetition.op.kind {
                let (n, m) = match range {
                    RepetitionRange::Exactly(n) => (*n, *n),
                    RepetitionRange::AtLeast(n) => (*n, *n),
                    RepetitionRange::Bounded(n, m) => (*n, *m),
                };
                if n > 1000 || m > 1000 || bound(ast, 1000) == 0 {
                    return Err(format!("invalid repetition size: {}", operator(repetition)));
                }
            }
            Ok(())
        }
        Ast::Group(group) => check(&group.ast),
        Ast::Alternation(alternation) => alternation.asts.iter().try_for_each(check),
        Ast::Concat(concat) => concat.asts.iter().try_for_each(check),
        _ => Ok(()),
    }
}

/// RE2's bound on nested counts: `budget` divided by the count of each counted repetition on the
/// way down, the least over the paths of `ast`, which is 0 when a product is over the budget.
fn bound(ast: &Ast, budget: u32) -> u32 {
    match ast {
        Ast::Repetition(repetition) => {
            let budget = match &repetition.op.kind {
                RepetitionKind::Range(range) => {
                    let count = match range {
                        RepetitionRange::Exactly(n) | RepetitionRange::AtLeast(n) => *n,
                        RepetitionRange::Bounded(_, m) => *m,
                    };
                    match count {
                        0 => budget,
                        count => budget / count,
                    }
                }
                _ => budget,
            };
            budget.min(bound(&repetition.ast, budget))
        }
        Ast::Group(group) => bound(&group.ast, budget),
        Ast::Alternation(alternation) => (alternation.asts.iter())
            .map(|ast| bound(ast, budget))
            .min()
            .unwrap_or(budget),
        Ast::Concat(concat) => (concat.asts.iter())
            .map(|ast| bound(ast, budget))
            .min()
            .unwrap_or(budget),
        _ => budget,
    }
}

/// The text of the operator of `repetition`, as RE2's errors write it.
fn operator(repetition: &ast::Repetition) -> String {
    let op = match &repetition.op.kind {
        RepetitionKind::ZeroOrOne => "?".to_string(),
        RepetitionKind::ZeroOrMore => "*".to_string(),
        RepetitionKind::OneOrMore => "+".to_string(),
        RepetitionKind::Range(RepetitionRange::Exactly(n)) => format!("{{{n}}}"),
        RepetitionKind::Range(RepetitionRange::AtLeast(n)) => format!("{{{n},}}"),
        RepetitionKind::Range(RepetitionRange::Bounded(n, m)) => format!("{{{n},{m}}}"),
    };
    match repetition.greedy {
        true => op,
        false => format!("{op}?"),
    }
}

/// A span of no place, for the nodes that the translation makes, which the printer ignores.
fn span() -> Span {
    let position = ast::Position::new(0, 1, 1);
    Span::new(position, position)
}

/// `ast` with RE2's ASCII Perl classes and word boundaries.
fn ascii(ast: &mut Ast) {
    match ast {
        Ast::ClassPerl(perl) => {
            *ast = Ast::class_bracketed(ClassBracketed {
                span: span(),
                negated: perl.kind == ClassPerlKind::Space && perl.negated,
                kind: ClassSet::Item(perl_item(perl, false)),
            });
        }
        Ast::ClassBracketed(class) => ascii_set(&mut class.kind),
        Ast::Assertion(assertion)
            if matches!(
                assertion.kind,
                AssertionKind::WordBoundary | AssertionKind::NotWordBoundary
            ) =>
        {
            let assertion = Assertion {
                span: span(),
                kind: assertion.kind.clone(),
            };
            *ast = Ast::group(Group {
                span: span(),
                kind: GroupKind::NonCapturing(Flags {
                    span: span(),
                    items: vec![
                        FlagsItem {
                            span: span(),
                            kind: FlagsItemKind::Negation,
                        },
                        FlagsItem {
                            span: span(),
                            kind: FlagsItemKind::Flag(Flag::Unicode),
                        },
                    ],
                }),
                ast: Box::new(Ast::assertion(assertion)),
            });
        }
        Ast::Repetition(repetition) => ascii(&mut repetition.ast),
        Ast::Group(group) => ascii(&mut group.ast),
        Ast::Alternation(alternation) => alternation.asts.iter_mut().for_each(ascii),
        Ast::Concat(concat) => concat.asts.iter_mut().for_each(ascii),
        _ => {}
    }
}

/// The set `set` of a bracketed class with RE2's ASCII Perl classes.
fn ascii_set(set: &mut ClassSet) {
    match set {
        ClassSet::Item(item) => ascii_item(item),
        ClassSet::BinaryOp(op) => {
            ascii_set(&mut op.lhs);
            ascii_set(&mut op.rhs);
        }
    }
}

fn ascii_item(item: &mut ClassSetItem) {
    match item {
        ClassSetItem::Perl(perl) => *item = perl_item(perl, true),
        ClassSetItem::Bracketed(class) => ascii_set(&mut class.kind),
        ClassSetItem::Union(union) => union.items.iter_mut().for_each(ascii_item),
        _ => {}
    }
}

/// The ASCII item of RE2's Perl class `perl`: `[:digit:]`, `[:word:]`, or RE2's white space,
/// whose negation is a class of its own inside a bracketed class, `in_class`, and outside one is
/// the negation of the class around it.
fn perl_item(perl: &ClassPerl, in_class: bool) -> ClassSetItem {
    let ascii = |kind| {
        ClassSetItem::Ascii(ClassAscii {
            span: span(),
            kind,
            negated: perl.negated,
        })
    };
    match perl.kind {
        ClassPerlKind::Digit => ascii(ClassAsciiKind::Digit),
        ClassPerlKind::Word => ascii(ClassAsciiKind::Word),
        ClassPerlKind::Space => {
            let literal = |kind: Option<SpecialLiteralKind>, c| {
                ClassSetItem::Literal(Literal {
                    span: span(),
                    kind: kind.map_or(LiteralKind::Verbatim, LiteralKind::Special),
                    c,
                })
            };
            let space = ClassSetItem::Union(ClassSetUnion {
                span: span(),
                items: vec![
                    literal(Some(SpecialLiteralKind::Tab), '\t'),
                    literal(Some(SpecialLiteralKind::LineFeed), '\n'),
                    literal(Some(SpecialLiteralKind::FormFeed), '\x0C'),
                    literal(Some(SpecialLiteralKind::CarriageReturn), '\r'),
                    literal(None, ' '),
                ],
            });
            match perl.negated && in_class {
                true => ClassSetItem::Bracketed(Box::new(ClassBracketed {
                    span: span(),
                    negated: true,
                    kind: ClassSet::Item(space),
                })),
                false => space,
            }
        }
    }
}
