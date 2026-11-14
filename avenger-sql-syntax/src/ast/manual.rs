//! The accessors that decode text or read tokens together: names, literal values, operators,
//! and the modifiers of `CREATE` statements.

use super::*;
use crate::{escape, SyntaxKind::*};

/// The tokens of `node`, not of its children, other than trivia.
fn tokens(node: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
}

/// The text of a name, without its backticks and with its escapes decoded. A table's dashed
/// first name, such as `my-project-2.` in `my-project-2.dataset`, is its text without the dot
/// that its last number holds.
fn name_text(node: &SyntaxNode) -> Option<String> {
    let parts: Vec<SyntaxToken> = tokens(node).collect();
    match parts.as_slice() {
        [] => None,
        [token] if token.kind() == QUOTED_IDENT => {
            String::from_utf8(escape::value(token.text())?).ok()
        }
        [token] => Some(token.text().to_string()),
        parts => {
            let mut text: String = parts.iter().map(|token| token.text()).collect();
            if parts.last().is_some_and(|token| token.kind() == FLOAT) {
                text = text.trim_end_matches('.').to_string();
            }
            Some(text)
        }
    }
}

impl Name {
    /// The name, without its backticks and with its escapes decoded.
    pub fn text(&self) -> Option<String> {
        name_text(&self.syntax)
    }
}

impl NameRef {
    /// The name, without its backticks and with its escapes decoded.
    pub fn text(&self) -> Option<String> {
        name_text(&self.syntax)
    }
}

impl Path {
    /// The names of the path, in order. A name that the tree lacks, or whose escapes are wrong,
    /// makes the result `None`.
    pub fn names(&self) -> Option<Vec<String>> {
        self.name_refs().map(|name| name.text()).collect()
    }
}

/// The kind of a [`Literal`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LiteralKind {
    Int,
    Float,
    String,
    Bytes,
    Null,
    True,
    False,
}

impl Literal {
    pub fn kind(&self) -> Option<LiteralKind> {
        let kind = match tokens(&self.syntax).next()?.kind() {
            INTEGER => LiteralKind::Int,
            FLOAT => LiteralKind::Float,
            STRING => LiteralKind::String,
            BYTES => LiteralKind::Bytes,
            NULL_KW => LiteralKind::Null,
            TRUE_KW => LiteralKind::True,
            FALSE_KW => LiteralKind::False,
            _ => return None,
        };
        Some(kind)
    }

    /// The value of an integer literal, decimal or hex after `0x`, or `None` when it is larger
    /// than `u64::MAX`. A literal has no sign: `-1` is a `PrefixExpr`.
    pub fn int_value(&self) -> Option<u64> {
        let token = self.int()?;
        let text = token.text();
        match text.get(..2) {
            Some("0x" | "0X") => u64::from_str_radix(&text[2..], 16).ok(),
            _ => text.parse().ok(),
        }
    }

    /// The value of a float literal, such as `1.5`, `1.` or `1e-3`.
    pub fn float_value(&self) -> Option<f64> {
        self.float()?.text().parse().ok()
    }

    /// The value of a string literal: its adjacent strings concatenated, with their escapes
    /// decoded, or `None` when an escape is wrong or the value isn't UTF-8.
    pub fn string_value(&self) -> Option<String> {
        concatenated(&self.syntax, STRING).and_then(|value| String::from_utf8(value).ok())
    }

    /// The value of a bytes literal: its adjacent bytes literals concatenated, with their
    /// escapes decoded.
    pub fn bytes_value(&self) -> Option<Vec<u8>> {
        concatenated(&self.syntax, BYTES)
    }
}

impl TypedLiteral {
    /// The value of the string of the literal, as [`Literal::string_value`] decodes it, such as
    /// `2024-01-01` in `DATE '2024-01-01'`.
    pub fn value(&self) -> Option<String> {
        concatenated(&self.syntax, STRING).and_then(|value| String::from_utf8(value).ok())
    }
}

/// The concatenated values of the tokens of `node` of `kind`, or `None` when there are none or
/// one doesn't decode.
fn concatenated(node: &SyntaxNode, kind: SyntaxKind) -> Option<Vec<u8>> {
    let mut parts = tokens(node).filter(|token| token.kind() == kind).peekable();
    parts.peek()?;
    let mut value = Vec::new();
    for part in parts {
        value.extend(escape::value(part.text())?);
    }
    Some(value)
}

/// The operator of a [`BinExpr`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    /// `!=` or `<>`
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    BitOr,
    BitXor,
    BitAnd,
    ShiftLeft,
    ShiftRight,
    Add,
    Sub,
    Mul,
    Div,
    Concat,
}

impl BinExpr {
    /// The operator, which reads `<>` and `>>` from their two tokens.
    pub fn operator(&self) -> Option<BinaryOp> {
        let mut parts = tokens(&self.syntax);
        let first = parts.next()?;
        let second = parts.next().map(|token| token.kind());
        let op = match first.kind() {
            OR_KW => BinaryOp::Or,
            AND_KW => BinaryOp::And,
            EQ => BinaryOp::Eq,
            NEQ => BinaryOp::NotEq,
            LT if second == Some(GT) => BinaryOp::NotEq,
            LT => BinaryOp::Lt,
            LT_EQ => BinaryOp::LtEq,
            GT if second == Some(GT) => BinaryOp::ShiftRight,
            GT => BinaryOp::Gt,
            GT_EQ => BinaryOp::GtEq,
            BIT_OR => BinaryOp::BitOr,
            CARET => BinaryOp::BitXor,
            AMP => BinaryOp::BitAnd,
            SHL => BinaryOp::ShiftLeft,
            PLUS => BinaryOp::Add,
            MINUS => BinaryOp::Sub,
            STAR => BinaryOp::Mul,
            SLASH => BinaryOp::Div,
            CONCAT => BinaryOp::Concat,
            _ => return None,
        };
        Some(op)
    }
}

/// The operator of a [`PrefixExpr`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrefixOp {
    Neg,
    Plus,
    BitNot,
    Not,
}

impl PrefixExpr {
    pub fn operator(&self) -> Option<PrefixOp> {
        let op = match self.op()?.kind() {
            MINUS => PrefixOp::Neg,
            PLUS => PrefixOp::Plus,
            TILDE => PrefixOp::BitNot,
            NOT_KW => PrefixOp::Not,
            _ => return None,
        };
        Some(op)
    }
}

/// The scope of an object that a `CREATE` statement creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// `TEMP` or `TEMPORARY`, in a script.
    Temp,
    /// `PUBLIC`, in a module.
    Public,
    /// `PRIVATE`, in a module.
    Private,
}

/// The modifiers that the `CREATE` statements share.
pub trait CreateStmt: AstNode {
    /// Whether the statement has `OR REPLACE`.
    fn or_replace(&self) -> bool {
        tokens(self.syntax()).any(|token| token.kind() == REPLACE_KW)
    }

    /// Whether the statement has `IF NOT EXISTS`.
    fn if_not_exists(&self) -> bool {
        tokens(self.syntax()).any(|token| token.kind() == EXISTS_KW)
    }

    fn scope_kind(&self) -> Option<Scope> {
        tokens(self.syntax()).find_map(|token| match token.kind() {
            TEMP_KW | TEMPORARY_KW => Some(Scope::Temp),
            PUBLIC_KW => Some(Scope::Public),
            PRIVATE_KW => Some(Scope::Private),
            _ => None,
        })
    }

    /// The path that the statement names its object by.
    fn name_path(&self) -> Option<Path> {
        support::child(self.syntax(), 0)
    }
}

impl CreateStmt for CreateTableStmt {}
impl CreateStmt for CreateFunctionStmt {}
impl CreateStmt for CreateConstantStmt {}

/// The kind of a function that a [`CreateFunctionStmt`] creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionKind {
    Scalar,
    Aggregate,
    Table,
}

impl CreateFunctionStmt {
    pub fn function_kind(&self) -> FunctionKind {
        if self.aggregate_kw().is_some() {
            FunctionKind::Aggregate
        } else if self.table_kw().is_some() {
            FunctionKind::Table
        } else {
            FunctionKind::Scalar
        }
    }
}

impl crate::Parse {
    /// The root as a [`Script`], when the text was parsed as a script.
    pub fn script(&self) -> Option<Script> {
        Script::cast(self.syntax())
    }

    /// The root as a [`Module`], when the text was parsed as a module.
    pub fn module(&self) -> Option<Module> {
        Module::cast(self.syntax())
    }
}
