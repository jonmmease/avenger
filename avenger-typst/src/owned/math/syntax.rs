use crate::error::MathTypesetError;

use super::ast::{
    OwnedMath, OwnedMathArg, OwnedMathAttach, OwnedMathCall, OwnedMathFraction, OwnedMathGroup,
    OwnedMathIdentifier, OwnedMathNode, OwnedMathOperator, OwnedMathShorthand, OwnedMathSpace,
    OwnedMathStringLiteral, OwnedMathText, OwnedMathTextKind,
};

pub(crate) fn parse_owned_math(source: &str, offset: usize) -> Result<OwnedMath, MathTypesetError> {
    let mut parser = Parser {
        source,
        offset,
        pos: 0,
    };
    let nodes = parser.parse_sequence(&[])?;
    Ok(OwnedMath {
        source: source.to_string(),
        nodes,
    })
}

struct Parser<'a> {
    source: &'a str,
    offset: usize,
    pos: usize,
}

impl Parser<'_> {
    fn parse_sequence(&mut self, stop: &[char]) -> Result<Vec<OwnedMathNode>, MathTypesetError> {
        let mut nodes = Vec::new();

        while let Some((idx, ch)) = self.peek_char() {
            if stop.contains(&ch) {
                break;
            }
            if is_closing_delimiter(ch) {
                return Err(self.unsupported(idx, "unexpected closing math delimiter"));
            }

            if ch.is_whitespace() {
                nodes.push(self.parse_space());
                continue;
            }

            if ch == '/' {
                let numerator = take_fraction_numerator(&mut nodes, self.absolute(idx))?;
                self.consume_char();
                let slash_range = self.absolute(idx)..self.absolute(idx + ch.len_utf8());
                self.consume_spaces();
                let denominator = self.parse_postfix_atom().map_err(|err| match err {
                    MathTypesetError::UnsupportedSyntax { position, .. } => {
                        MathTypesetError::UnsupportedSyntax {
                            position,
                            message: "fraction slash expects a denominator",
                        }
                    }
                    other => other,
                })?;
                let byte_range = numerator.byte_range().start..denominator.byte_range().end;
                nodes.push(OwnedMathNode::Fraction(OwnedMathFraction {
                    numerator: Box::new(numerator),
                    denominator: Box::new(denominator),
                    slash_range,
                    byte_range,
                }));
                continue;
            }

            nodes.push(self.parse_postfix_atom()?);
        }

        Ok(nodes)
    }

    fn parse_postfix_atom(&mut self) -> Result<OwnedMathNode, MathTypesetError> {
        let base = self.parse_atom()?;
        let mut top = None;
        let mut bottom = None;
        let mut primes = 0usize;
        let mut byte_range = base.byte_range();

        while let Some((idx, ch)) = self.peek_char() {
            match ch {
                '^' | '_' => {
                    self.consume_char();
                    let script = self.parse_script_arg()?;
                    byte_range.end = script.byte_range().end;
                    let slot = if ch == '^' { &mut top } else { &mut bottom };
                    if slot.is_some() {
                        return Err(self.unsupported(idx, "duplicate math script attachment"));
                    }
                    *slot = Some(Box::new(script));
                }
                '\'' => {
                    while matches!(self.peek_char(), Some((_, '\''))) {
                        let (_, prime) = self.consume_char().expect("prime should exist");
                        primes += 1;
                        byte_range.end =
                            self.absolute(self.pos - prime.len_utf8()) + prime.len_utf8();
                    }
                }
                _ => break,
            }
        }

        if top.is_none() && bottom.is_none() && primes == 0 {
            Ok(base)
        } else {
            Ok(OwnedMathNode::Attach(OwnedMathAttach {
                base: Box::new(base),
                top,
                bottom,
                primes,
                byte_range,
            }))
        }
    }

    fn parse_script_arg(&mut self) -> Result<OwnedMathNode, MathTypesetError> {
        self.consume_spaces();
        if self.peek_char().is_none() {
            return Err(self.unsupported(self.pos, "math script expects an expression"));
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<OwnedMathNode, MathTypesetError> {
        let Some((idx, ch)) = self.peek_char() else {
            return Err(self.unsupported(self.pos, "expected math expression"));
        };

        if ch == '#' {
            return Err(
                self.unsupported(idx, "embedded Typst code is not allowed in math fragments")
            );
        }

        if let Some(right) = matching_closing_delimiter(ch) {
            return self.parse_group(ch, right);
        }

        if ch == '"' {
            return self.parse_string_literal();
        }

        if let Some((source, replacement)) = self.peek_shorthand() {
            let start = idx;
            self.pos += source.len();
            return Ok(OwnedMathNode::Shorthand(OwnedMathShorthand {
                source: source.to_string(),
                replacement,
                byte_range: self.absolute(start)..self.absolute(self.pos),
            }));
        }

        if ch.is_ascii_digit()
            || (ch == '.'
                && self
                    .peek_next_char()
                    .is_some_and(|next| next.1.is_ascii_digit()))
        {
            return Ok(self.parse_number());
        }

        if is_identifier_start(ch) {
            return self.parse_identifier_or_call();
        }

        if is_operator_char(ch) {
            self.consume_char();
            return Ok(OwnedMathNode::Operator(OwnedMathOperator {
                operator: ch.to_string(),
                byte_range: self.absolute(idx)..self.absolute(idx + ch.len_utf8()),
            }));
        }

        self.consume_char();
        Ok(OwnedMathNode::Text(OwnedMathText {
            text: ch.to_string(),
            kind: OwnedMathTextKind::Grapheme,
            byte_range: self.absolute(idx)..self.absolute(idx + ch.len_utf8()),
        }))
    }

    fn parse_group(&mut self, left: char, right: char) -> Result<OwnedMathNode, MathTypesetError> {
        let start = self.pos;
        self.consume_char();
        let body = self.parse_sequence(&[right])?;
        let Some((close_idx, close)) = self.peek_char() else {
            return Err(self.unsupported(start, "unterminated math group"));
        };
        if close != right {
            return Err(self.unsupported(close_idx, "mismatched math delimiter"));
        }
        self.consume_char();
        Ok(OwnedMathNode::Group(OwnedMathGroup {
            left,
            right,
            body,
            byte_range: self.absolute(start)..self.absolute(self.pos),
        }))
    }

    fn parse_string_literal(&mut self) -> Result<OwnedMathNode, MathTypesetError> {
        let start = self.pos;
        self.consume_char();
        let mut text = String::new();

        while let Some((idx, ch)) = self.peek_char() {
            self.consume_char();
            match ch {
                '"' => {
                    return Ok(OwnedMathNode::StringLiteral(OwnedMathStringLiteral {
                        text,
                        byte_range: self.absolute(start)..self.absolute(self.pos),
                    }));
                }
                '\\' => {
                    if let Some((_, escaped)) = self.consume_char() {
                        text.push(escaped);
                    } else {
                        return Err(self.unsupported(idx, "unterminated math string literal"));
                    }
                }
                _ => text.push(ch),
            }
        }

        Err(self.unsupported(start, "unterminated math string literal"))
    }

    fn parse_number(&mut self) -> OwnedMathNode {
        let start = self.pos;
        let mut seen_dot = false;

        while let Some((_, ch)) = self.peek_char() {
            if ch.is_ascii_digit() {
                self.consume_char();
            } else if ch == '.' && !seen_dot {
                seen_dot = true;
                self.consume_char();
            } else {
                break;
            }
        }

        OwnedMathNode::Text(OwnedMathText {
            text: self.source[start..self.pos].to_string(),
            kind: OwnedMathTextKind::Number,
            byte_range: self.absolute(start)..self.absolute(self.pos),
        })
    }

    fn parse_identifier_or_call(&mut self) -> Result<OwnedMathNode, MathTypesetError> {
        let start = self.pos;
        self.consume_char();

        while let Some((_, ch)) = self.peek_char() {
            if is_identifier_continue(ch) {
                self.consume_char();
            } else {
                break;
            }
        }
        self.consume_known_dotted_symbol_suffixes(start);

        let name = &self.source[start..self.pos];
        if matches!(self.peek_char(), Some((_, '('))) && (is_math_call_name(name) || name == "mat")
        {
            return self.parse_call(start, name.to_string());
        }

        Ok(OwnedMathNode::Identifier(OwnedMathIdentifier {
            name: name.to_string(),
            symbol: named_math_symbol(name),
            byte_range: self.absolute(start)..self.absolute(self.pos),
        }))
    }

    fn consume_known_dotted_symbol_suffixes(&mut self, start: usize) {
        loop {
            let Some((dot_idx, '.')) = self.peek_char() else {
                break;
            };
            let segment_start = dot_idx + 1;
            let segment_end = read_symbol_modifier_end(self.source, segment_start);
            if segment_end == segment_start {
                break;
            }

            let candidate = &self.source[start..segment_end];
            if named_math_symbol(candidate).is_none() {
                break;
            }
            self.pos = segment_end;
        }
    }

    fn parse_call(
        &mut self,
        name_start: usize,
        name: String,
    ) -> Result<OwnedMathNode, MathTypesetError> {
        if name == "mat" {
            return Err(self.unsupported(
                name_start,
                "matrix/table math is not supported in owned Typst subset",
            ));
        }

        self.expect_char('(')?;
        let mut args = Vec::new();

        loop {
            self.consume_spaces();
            let arg_start = self.pos;

            if matches!(self.peek_char(), Some((_, ')'))) {
                self.consume_char();
                break;
            }

            let nodes = self.parse_sequence(&[',', ';', ')'])?;
            let arg_end = self.pos;
            args.push(OwnedMathArg {
                nodes,
                byte_range: self.absolute(arg_start)..self.absolute(arg_end),
            });

            match self.peek_char() {
                Some((_, ',')) => {
                    self.consume_char();
                }
                Some((semi_idx, ';')) => {
                    return Err(self.unsupported(
                        semi_idx,
                        "semicolon math arguments are not supported in owned Typst subset",
                    ));
                }
                Some((_, ')')) => {
                    self.consume_char();
                    break;
                }
                Some((idx, _)) => {
                    return Err(self.unsupported(idx, "expected math call argument separator"));
                }
                None => {
                    return Err(self.unsupported(name_start, "unterminated math call"));
                }
            }
        }

        Ok(OwnedMathNode::Call(OwnedMathCall {
            name,
            args,
            byte_range: self.absolute(name_start)..self.absolute(self.pos),
        }))
    }

    fn parse_space(&mut self) -> OwnedMathNode {
        let start = self.pos;
        while let Some((_, ch)) = self.peek_char() {
            if ch.is_whitespace() {
                self.consume_char();
            } else {
                break;
            }
        }
        OwnedMathNode::Space(OwnedMathSpace {
            byte_range: self.absolute(start)..self.absolute(self.pos),
        })
    }

    fn consume_spaces(&mut self) {
        while let Some((_, ch)) = self.peek_char() {
            if ch.is_whitespace() {
                self.consume_char();
            } else {
                break;
            }
        }
    }

    fn expect_char(&mut self, expected: char) -> Result<(), MathTypesetError> {
        match self.consume_char() {
            Some((_, ch)) if ch == expected => Ok(()),
            Some((idx, _)) => Err(self.unsupported(idx, "unexpected math character")),
            None => Err(self.unsupported(self.pos, "unexpected end of math source")),
        }
    }

    fn peek_char(&self) -> Option<(usize, char)> {
        next_char(self.source, self.pos)
    }

    fn peek_next_char(&self) -> Option<(usize, char)> {
        let (_, ch) = self.peek_char()?;
        next_char(self.source, self.pos + ch.len_utf8())
    }

    fn consume_char(&mut self) -> Option<(usize, char)> {
        let (idx, ch) = self.peek_char()?;
        self.pos = idx + ch.len_utf8();
        Some((idx, ch))
    }

    fn peek_shorthand(&self) -> Option<(&'static str, &'static str)> {
        for (source, replacement) in SHORTHANDS {
            if self.source[self.pos..].starts_with(source) {
                return Some((*source, *replacement));
            }
        }
        None
    }

    fn absolute(&self, position: usize) -> usize {
        self.offset + position
    }

    fn unsupported(&self, position: usize, message: &'static str) -> MathTypesetError {
        MathTypesetError::UnsupportedSyntax {
            position: self.absolute(position),
            message,
        }
    }
}

fn take_fraction_numerator(
    nodes: &mut Vec<OwnedMathNode>,
    slash_position: usize,
) -> Result<OwnedMathNode, MathTypesetError> {
    while matches!(nodes.last(), Some(OwnedMathNode::Space(_))) {
        nodes.pop();
    }
    nodes.pop().ok_or(MathTypesetError::UnsupportedSyntax {
        position: slash_position,
        message: "fraction slash expects a numerator",
    })
}

fn next_char(source: &str, start: usize) -> Option<(usize, char)> {
    source[start..]
        .char_indices()
        .next()
        .map(|(offset, ch)| (start + offset, ch))
}

fn matching_closing_delimiter(ch: char) -> Option<char> {
    match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

fn is_closing_delimiter(ch: char) -> bool {
    matches!(ch, ')' | ']' | '}')
}

fn is_identifier_start(ch: char) -> bool {
    ch == '\\' || ch.is_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    ch.is_alphanumeric()
}

fn read_symbol_modifier_end(source: &str, start: usize) -> usize {
    let mut end = start;
    while let Some((idx, ch)) = next_char(source, end) {
        if ch.is_ascii_alphabetic() {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    end
}

fn is_operator_char(ch: char) -> bool {
    matches!(
        ch,
        '+' | '-' | '*' | '=' | '<' | '>' | '!' | ':' | ',' | '.' | '|' | '&'
    )
}

const SHORTHANDS: &[(&str, &str)] = &[
    ("...", "…"),
    ("<=", "≤"),
    (">=", "≥"),
    ("!=", "≠"),
    ("=>", "⇒"),
    ("->", "→"),
    ("<-", "←"),
    (":=", "≔"),
];

fn is_math_call_name(name: &str) -> bool {
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
            | "cancel"
            | "op"
            | "sin"
            | "cos"
            | "tan"
            | "log"
            | "ln"
            | "lim"
            | "max"
            | "min"
            | "hat"
            | "tilde"
            | "dot"
            | "ddot"
            | "bar"
            | "vec"
            | "arrow"
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
    )
}

fn named_math_symbol(name: &str) -> Option<&'static str> {
    match name {
        "alpha" => Some("α"),
        "beta" => Some("β"),
        "gamma" => Some("γ"),
        "delta" => Some("δ"),
        "epsilon" => Some("ε"),
        "zeta" => Some("ζ"),
        "eta" => Some("η"),
        "theta" => Some("θ"),
        "iota" => Some("ι"),
        "kappa" => Some("κ"),
        "lambda" => Some("λ"),
        "mu" => Some("μ"),
        "nu" => Some("ν"),
        "xi" => Some("ξ"),
        "pi" => Some("π"),
        "rho" => Some("ρ"),
        "sigma" => Some("σ"),
        "tau" => Some("τ"),
        "upsilon" => Some("υ"),
        "phi" => Some("φ"),
        "chi" => Some("χ"),
        "psi" => Some("ψ"),
        "omega" => Some("ω"),
        "Gamma" => Some("Γ"),
        "Delta" => Some("Δ"),
        "Theta" => Some("Θ"),
        "Lambda" => Some("Λ"),
        "Xi" => Some("Ξ"),
        "Pi" => Some("Π"),
        "Sigma" => Some("Σ"),
        "Upsilon" => Some("Υ"),
        "Phi" => Some("Φ"),
        "Psi" => Some("Ψ"),
        "Omega" => Some("Ω"),
        "dot" | "dot.op" => Some("⋅"),
        "dot.c" => Some("·"),
        "dots" | "dots.h" => Some("…"),
        "dots.h.c" => Some("⋯"),
        "dots.v" => Some("⋮"),
        "sum" => Some("∑"),
        "prod" | "product" => Some("∏"),
        "integral" => Some("∫"),
        "oo" | "infinity" => Some("∞"),
        "partial" => Some("∂"),
        "gradient" | "nabla" => Some("∇"),
        "RR" => Some("ℝ"),
        "NN" => Some("ℕ"),
        "ZZ" => Some("ℤ"),
        "QQ" => Some("ℚ"),
        "CC" => Some("ℂ"),
        "plus" => Some("+"),
        "plus.minus" => Some("±"),
        "minus" => Some("−"),
        "minus.plus" => Some("∓"),
        "times" => Some("×"),
        "times.big" => Some("⨉"),
        "div" => Some("÷"),
        "eq" => Some("="),
        "eq.not" => Some("≠"),
        "eq.triple" | "equiv" => Some("≡"),
        "eq.triple.not" | "equiv.not" => Some("≢"),
        "lt" => Some("<"),
        "lt.eq" => Some("≤"),
        "lt.eq.not" => Some("≰"),
        "lt.not" => Some("≮"),
        "gt" => Some(">"),
        "gt.eq" => Some("≥"),
        "gt.eq.not" => Some("≱"),
        "gt.not" => Some("≯"),
        "approx" => Some("≈"),
        "approx.not" => Some("≉"),
        "prop" => Some("∝"),
        "emptyset" | "nothing" => Some("∅"),
        "in" => Some("∈"),
        "in.not" => Some("∉"),
        "in.rev" => Some("∋"),
        "in.rev.not" => Some("∌"),
        "subset" => Some("⊂"),
        "subset.eq" => Some("⊆"),
        "subset.eq.not" => Some("⊈"),
        "subset.neq" => Some("⊊"),
        "subset.not" => Some("⊄"),
        "supset" => Some("⊃"),
        "supset.eq" => Some("⊇"),
        "supset.eq.not" => Some("⊉"),
        "supset.neq" => Some("⊋"),
        "supset.not" => Some("⊅"),
        "union" => Some("∪"),
        "union.big" => Some("⋃"),
        "union.plus" => Some("⊎"),
        "inter" => Some("∩"),
        "inter.big" => Some("⋂"),
        "forall" => Some("∀"),
        "exists" => Some("∃"),
        "angle" => Some("∠"),
        "parallel" => Some("∥"),
        "perp" => Some("⟂"),
        "degree" => Some("°"),
        "aleph" => Some("א"),
        "ell" => Some("ℓ"),
        "arrow.r" => Some("→"),
        "arrow.r.long" => Some("⟶"),
        "arrow.r.bar" => Some("↦"),
        "arrow.r.double" => Some("⇒"),
        "arrow.r.double.long" => Some("⟹"),
        "arrow.r.not" => Some("↛"),
        "arrow.l" => Some("←"),
        "arrow.l.long" => Some("⟵"),
        "arrow.l.bar" => Some("↤"),
        "arrow.l.double" => Some("⇐"),
        "arrow.l.double.long" => Some("⟸"),
        "arrow.l.not" => Some("↚"),
        "arrow.l.r" => Some("↔"),
        "arrow.l.r.long" => Some("⟷"),
        "arrow.l.r.double" => Some("⇔"),
        "arrow.l.r.double.long" => Some("⟺"),
        "arrow.t" => Some("↑"),
        "arrow.b" => Some("↓"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> OwnedMath {
        parse_owned_math(source, 0).unwrap()
    }

    #[test]
    fn parses_core_oracle_fragments() {
        for source in [
            "x",
            "y",
            "t",
            "x^2 + y^2",
            "x_i^2",
            "x'",
            "x''",
            "x_1^2",
            "sqrt(x) / (1 + x^2)",
            "root(3, x)",
            "frac(x + y, z)",
            "binom(n, k)",
            "cancel(x)",
            "a / b",
            "a / (b + c)",
            "J_0(x)",
            "J_n(x)",
            "sum_(i=0)^n i",
            "lim_(x -> oo) f(x)",
            "sin(x)",
            "op(\"custom\")",
            "abs(x)",
            "norm(v)",
            "floor(x)",
            "ceil(x)",
            "round(x)",
            "alpha + beta -> gamma",
            "alpha + pi + sum",
            "x(t)",
            "x(t) = A r^t",
            "R^2 = 0.94",
            "y = sqrt(x) / (1 + x^2)",
        ] {
            parse_owned_math(source, 0)
                .unwrap_or_else(|err| panic!("owned math parser failed for {source:?}: {err:?}"));
        }
    }

    #[test]
    fn parses_slash_fraction_with_parenthesized_denominator() {
        let math = parse("sqrt(x) / (1 + x^2)");

        assert_eq!(math.nodes.len(), 1);
        let OwnedMathNode::Fraction(fraction) = &math.nodes[0] else {
            panic!("expected slash fraction");
        };
        assert!(matches!(
            fraction.numerator.as_ref(),
            OwnedMathNode::Call(call) if call.name == "sqrt"
        ));
        assert!(matches!(
            fraction.denominator.as_ref(),
            OwnedMathNode::Group(group) if group.left == '(' && group.right == ')'
        ));
    }

    #[test]
    fn parses_scripts_and_primes() {
        let math = parse("x_i^2 + x''");

        assert!(matches!(
            &math.nodes[0],
            OwnedMathNode::Attach(attach)
                if attach.top.is_some() && attach.bottom.is_some() && attach.primes == 0
        ));
        assert!(matches!(
            &math.nodes[4],
            OwnedMathNode::Attach(attach) if attach.primes == 2
        ));
    }

    #[test]
    fn parses_symbols_and_shorthands() {
        let math = parse("alpha -> RR + in.not + subset.eq + arrow.r.double");

        assert!(matches!(
            &math.nodes[0],
            OwnedMathNode::Identifier(ident)
                if ident.name == "alpha" && ident.symbol == Some("α")
        ));
        assert!(matches!(
            &math.nodes[2],
            OwnedMathNode::Shorthand(shorthand)
                if shorthand.source == "->" && shorthand.replacement == "→"
        ));
        assert!(matches!(
            &math.nodes[4],
            OwnedMathNode::Identifier(ident) if ident.symbol == Some("ℝ")
        ));
        assert!(matches!(
            &math.nodes[8],
            OwnedMathNode::Identifier(ident)
                if ident.name == "in.not" && ident.symbol == Some("∉")
        ));
        assert!(matches!(
            &math.nodes[12],
            OwnedMathNode::Identifier(ident)
                if ident.name == "subset.eq" && ident.symbol == Some("⊆")
        ));
        assert!(matches!(
            &math.nodes[16],
            OwnedMathNode::Identifier(ident)
                if ident.name == "arrow.r.double" && ident.symbol == Some("⇒")
        ));
    }

    #[test]
    fn dotted_symbol_suffixes_only_consume_known_symbols() {
        let math = parse("arrow.unknown");

        assert!(matches!(
            &math.nodes[..],
            [
                OwnedMathNode::Identifier(identifier),
                OwnedMathNode::Operator(operator),
                OwnedMathNode::Identifier(suffix),
            ] if identifier.name == "arrow"
                && identifier.symbol.is_none()
                && operator.operator == "."
                && suffix.name == "unknown"
        ));
    }

    #[test]
    fn parses_whitelisted_function_calls() {
        let math = parse("frac(x, y) + op(\"custom\") + bb(R) + scr(P)");

        assert!(matches!(
            &math.nodes[0],
            OwnedMathNode::Call(call) if call.name == "frac" && call.args.len() == 2
        ));
        assert!(matches!(
            &math.nodes[4],
            OwnedMathNode::Call(call) if call.name == "op" && call.args.len() == 1
        ));
        assert!(matches!(
            &math.nodes[8],
            OwnedMathNode::Call(call) if call.name == "bb" && call.args.len() == 1
        ));
        assert!(matches!(
            &math.nodes[12],
            OwnedMathNode::Call(call) if call.name == "scr" && call.args.len() == 1
        ));
    }

    #[test]
    fn leaves_unknown_function_like_identifiers_as_groups() {
        let math = parse("f(x)");

        assert!(matches!(
            &math.nodes[..],
            [OwnedMathNode::Identifier(_), OwnedMathNode::Group(_)]
        ));
    }

    #[test]
    fn rejects_matrix_calls() {
        let err = parse_owned_math("mat(1, 2; 3, 4)", 10).unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 10,
                message: "matrix/table math is not supported in owned Typst subset"
            }
        );
    }

    #[test]
    fn rejects_semicolon_arguments() {
        let err = parse_owned_math("frac(1; 2)", 0).unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 6,
                message: "semicolon math arguments are not supported in owned Typst subset"
            }
        );
    }

    #[test]
    fn rejects_unterminated_groups() {
        let err = parse_owned_math("sqrt(x", 0).unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "unterminated math call"
            }
        );
    }
}
