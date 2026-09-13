#[cfg(test)]
fn single_atom_text(math: &MathAst) -> Option<String> {
    let [node] = &math.nodes[..] else {
        return None;
    };
    simple_atom(node).map(|atom| atom.styled_text)
}

fn simple_atom(node: &MathNode) -> Option<SimpleMathAtom> {
    match node {
        MathNode::Text(text) => Some(SimpleMathAtom {
            styled_text: style_text_atom(text),
            class: match text.kind {
                MathTextKind::Grapheme => SimpleMathClass::Alphabetic,
                MathTextKind::Number | MathTextKind::Upright => SimpleMathClass::Normal,
            },
            text_operator: false,
        }),
        MathNode::StringLiteral(string) => Some(SimpleMathAtom {
            styled_text: string.text.clone(),
            class: SimpleMathClass::Normal,
            text_operator: false,
        }),
        MathNode::Identifier(identifier) => {
            let text = identifier.symbol.unwrap_or(&identifier.name);
            if let Some(operator) = operator_identifier_text(text) {
                Some(SimpleMathAtom {
                    styled_text: operator.to_string(),
                    class: SimpleMathClass::Large,
                    text_operator: true,
                })
            } else if identifier.symbol.is_some() || text.chars().count() == 1 {
                Some(SimpleMathAtom {
                    styled_text: style_default_math_text(text),
                    class: identifier_class(text),
                    text_operator: false,
                })
            } else {
                None
            }
        }
        MathNode::Operator(operator) => Some(SimpleMathAtom {
            styled_text: operator_text(operator),
            class: operator_class(&operator.operator),
            text_operator: false,
        }),
        MathNode::Shorthand(shorthand) => Some(SimpleMathAtom {
            styled_text: shorthand_text(shorthand),
            class: symbol_class(shorthand.replacement),
            text_operator: false,
        }),
        _ => None,
    }
}

fn layout_operator_atom(
    font: &MathFont,
    text: &str,
    font_size: f32,
) -> Result<LaidOutMathAtom, LabelError> {
    let mut atom = layout_math_text(font, text, font_size)?;
    atom.left_class = SimpleMathClass::Large;
    atom.right_class = SimpleMathClass::Large;
    Ok(atom)
}

fn operator_identifier_text(name: &str) -> Option<&'static str> {
    predefined_operator_text(name)
}

fn style_text_atom(text: &MathText) -> String {
    match text.kind {
        MathTextKind::Grapheme => style_default_math_text(&text.text),
        MathTextKind::Number | MathTextKind::Upright => text.text.clone(),
    }
}

fn style_default_math_text(text: &str) -> String {
    text.chars().map(style_default_math_char).collect()
}

fn style_math_nodes(nodes: &[MathNode], selection: MathStyleSelection) -> Vec<MathNode> {
    nodes
        .iter()
        .flat_map(|node| style_math_node(node, selection))
        .collect()
}

fn style_math_node(node: &MathNode, selection: MathStyleSelection) -> Vec<MathNode> {
    match node {
        MathNode::Space(_)
        | MathNode::Spacing(_)
        | MathNode::Operator(_)
        | MathNode::Shorthand(_) => vec![node.clone()],
        MathNode::StringLiteral(string) => {
            let mut string = string.clone();
            string.text = style_math_text_with_selection(&string.text, selection);
            vec![MathNode::StringLiteral(string)]
        }
        MathNode::Text(text) => vec![MathNode::Text(ast::MathText {
            text: style_math_text_with_selection(&text.text, selection),
            kind: MathTextKind::Number,
            byte_range: text.byte_range.clone(),
        })],
        MathNode::Identifier(identifier) => {
            let text = identifier.symbol.unwrap_or(&identifier.name);
            vec![MathNode::Text(ast::MathText {
                text: style_math_text_with_selection(text, selection),
                kind: MathTextKind::Number,
                byte_range: identifier.byte_range.clone(),
            })]
        }
        MathNode::Group(group) => vec![MathNode::Group(ast::MathGroup {
            left: group.left,
            right: group.right,
            body: style_math_nodes(&group.body, selection),
            byte_range: group.byte_range.clone(),
        })],
        MathNode::Attach(attach) => {
            let base = style_single_math_node(&attach.base, selection);
            vec![MathNode::Attach(ast::MathAttach {
                base: Box::new(base),
                top: style_optional_math_nodes(&attach.top, selection),
                bottom: style_optional_math_nodes(&attach.bottom, selection),
                top_left: style_optional_math_nodes(&attach.top_left, selection),
                top_right: style_optional_math_nodes(&attach.top_right, selection),
                bottom_left: style_optional_math_nodes(&attach.bottom_left, selection),
                bottom_right: style_optional_math_nodes(&attach.bottom_right, selection),
                primes: attach.primes,
                byte_range: attach.byte_range.clone(),
            })]
        }
        MathNode::Fraction(fraction) => {
            vec![MathNode::Fraction(ast::MathFraction {
                numerator: style_math_nodes(&fraction.numerator, selection),
                denominator: style_math_nodes(&fraction.denominator, selection),
                style: fraction.style,
                slash_range: fraction.slash_range.clone(),
                byte_range: fraction.byte_range.clone(),
            })]
        }
        MathNode::Cancel(cancel) => vec![MathNode::Cancel(ast::MathCancel {
            body: style_math_nodes(&cancel.body, selection),
            options: cancel.options.clone(),
            byte_range: cancel.byte_range.clone(),
        })],
        MathNode::Accent(accent) => vec![MathNode::Accent(ast::MathAccent {
            base: style_math_nodes(&accent.base, selection),
            accent: accent.accent,
            size: accent.size,
            dotless: accent.dotless,
            byte_range: accent.byte_range.clone(),
        })],
        MathNode::Call(call) => {
            if let Some(nested) = MathStyleSelection::from_call_name(&call.name) {
                let combined = selection.compose(nested);
                return call
                    .args
                    .iter()
                    .flat_map(|arg| style_math_nodes(&arg.nodes, combined))
                    .collect();
            }

            vec![MathNode::Call(ast::MathCall {
                name: call.name.clone(),
                args: call
                    .args
                    .iter()
                    .map(|arg| ast::MathArg {
                        nodes: style_math_nodes(&arg.nodes, selection),
                        byte_range: arg.byte_range.clone(),
                    })
                    .collect(),
                options: call.options,
                byte_range: call.byte_range.clone(),
            })]
        }
    }
}

fn style_optional_math_nodes(
    nodes: &Option<Vec<MathNode>>,
    selection: MathStyleSelection,
) -> Option<Vec<MathNode>> {
    nodes
        .as_ref()
        .map(|nodes| style_math_nodes(nodes, selection))
}

fn style_single_math_node(node: &MathNode, selection: MathStyleSelection) -> MathNode {
    let mut styled = style_math_node(node, selection);
    if styled.len() == 1 {
        styled.remove(0)
    } else {
        MathNode::Group(ast::MathGroup {
            left: '(',
            right: ')',
            body: styled,
            byte_range: node.byte_range(),
        })
    }
}

fn style_math_text_with_selection(text: &str, selection: MathStyleSelection) -> String {
    text.chars()
        .flat_map(|ch| {
            let style = MathAlphabetStyle::select(ch, selection);
            style_math_char(ch, style)
                .into_iter()
                .filter(|styled| *styled != '\0')
        })
        .collect()
}

fn operator_text(operator: &MathOperator) -> String {
    match operator.operator.as_str() {
        "-" => "−".to_string(),
        _ => operator.operator.clone(),
    }
}

fn shorthand_text(shorthand: &MathShorthand) -> String {
    shorthand.replacement.to_string()
}

fn identifier_class(text: &str) -> SimpleMathClass {
    if matches!(text, "∑" | "∏" | "∫") {
        SimpleMathClass::Large
    } else if let Some(ch) = single_char(text) {
        simple_math_class_for_char(ch)
    } else {
        SimpleMathClass::Alphabetic
    }
}

fn operator_class(text: &str) -> SimpleMathClass {
    match text {
        "=" | "<" | ">" | ":" => SimpleMathClass::Relation,
        "," => SimpleMathClass::Punctuation,
        "(" | "[" | "{" => SimpleMathClass::Opening,
        ")" | "]" | "}" => SimpleMathClass::Closing,
        "|" => SimpleMathClass::Fence,
        "+" | "-" | "*" | "!" | "&" => SimpleMathClass::Vary,
        _ => SimpleMathClass::Normal,
    }
}

fn symbol_class(text: &str) -> SimpleMathClass {
    single_char(text)
        .map(simple_math_class_for_char)
        .unwrap_or(SimpleMathClass::Normal)
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

fn simple_math_class_for_char(ch: char) -> SimpleMathClass {
    use unicode_math_class::MathClass;

    let class = match ch {
        ':' => Some(MathClass::Relation),
        '⋯' | '⋱' | '⋰' | '⋮' => Some(MathClass::Normal),
        '.' | '/' => Some(MathClass::Normal),
        '\u{22A5}' => Some(MathClass::Normal),
        '⅋' => Some(MathClass::Binary),
        '⎰' | '⟅' => Some(MathClass::Opening),
        '⎱' | '⟆' => Some(MathClass::Closing),
        '⟇' => Some(MathClass::Binary),
        '،' => Some(MathClass::Punctuation),
        c => unicode_math_class::class(c),
    };

    match class {
        Some(MathClass::Alphabetic) => SimpleMathClass::Alphabetic,
        Some(MathClass::Binary) => SimpleMathClass::Binary,
        Some(MathClass::Closing) => SimpleMathClass::Closing,
        Some(MathClass::Fence) => SimpleMathClass::Fence,
        Some(MathClass::Large) => SimpleMathClass::Large,
        Some(MathClass::Opening) => SimpleMathClass::Opening,
        Some(MathClass::Punctuation) => SimpleMathClass::Punctuation,
        Some(MathClass::Relation) => SimpleMathClass::Relation,
        Some(MathClass::Unary) => SimpleMathClass::Unary,
        Some(MathClass::Vary) => SimpleMathClass::Vary,
        _ => SimpleMathClass::Normal,
    }
}

fn simple_math_class_from_name(name: &str) -> Option<SimpleMathClass> {
    match name {
        "normal" => Some(SimpleMathClass::Normal),
        "alphabetic" => Some(SimpleMathClass::Alphabetic),
        "binary" => Some(SimpleMathClass::Binary),
        "unary" => Some(SimpleMathClass::Unary),
        "vary" => Some(SimpleMathClass::Vary),
        "relation" => Some(SimpleMathClass::Relation),
        "opening" => Some(SimpleMathClass::Opening),
        "closing" => Some(SimpleMathClass::Closing),
        "fence" => Some(SimpleMathClass::Fence),
        "punctuation" => Some(SimpleMathClass::Punctuation),
        "large" => Some(SimpleMathClass::Large),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathSizeCall {
    Display,
    Inline,
    Script,
    ScriptScript,
}

impl MathSizeCall {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "display" => Some(Self::Display),
            "inline" => Some(Self::Inline),
            "script" => Some(Self::Script),
            "sscript" => Some(Self::ScriptScript),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathLineCall {
    Below,
    Above,
}

impl MathLineCall {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "underline" => Some(Self::Below),
            "overline" => Some(Self::Above),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathUnderOverPosition {
    Below,
    Above,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MathUnderOverCall {
    position: MathUnderOverPosition,
    shape: MathUnderOverShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathUnderOverShape {
    Brace,
    Bracket,
    Paren,
    Shell,
}

impl MathUnderOverCall {
    fn from_name(name: &str) -> Option<Self> {
        let (position, shape) = match name {
            "underbrace" => (MathUnderOverPosition::Below, MathUnderOverShape::Brace),
            "overbrace" => (MathUnderOverPosition::Above, MathUnderOverShape::Brace),
            "underbracket" => (MathUnderOverPosition::Below, MathUnderOverShape::Bracket),
            "overbracket" => (MathUnderOverPosition::Above, MathUnderOverShape::Bracket),
            "underparen" => (MathUnderOverPosition::Below, MathUnderOverShape::Paren),
            "overparen" => (MathUnderOverPosition::Above, MathUnderOverShape::Paren),
            "undershell" => (MathUnderOverPosition::Below, MathUnderOverShape::Shell),
            "overshell" => (MathUnderOverPosition::Above, MathUnderOverShape::Shell),
            _ => return None,
        };
        Some(Self { position, shape })
    }
}
