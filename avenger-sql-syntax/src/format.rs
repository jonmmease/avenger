//! The formatter of scripts, which writes the style of `avenger-sql/STYLE.md`.
//!
//! The formatter reads the syntax tree alone. It changes the whitespace between tokens, the case
//! of keywords and of the names of functions, and the commas after the last item of a list, and
//! leaves every other token as written. A statement with an error stays exactly as written.
//!
//! Each kind of part has its rule of layout: the pipe operators of a query start lines of their
//! own, a call, an array, or another list in brackets breaks with one item to a line, the items
//! of a pipe operator and of a clause break in the same way after its keyword, and a chain of
//! binary operators breaks before its operators. A comma after the last item of a list keeps the
//! list broken. A part without a rule keeps its tokens on one line, with one space between them
//! where the style puts one. Each comment stays before or after the same tokens: a comment on a
//! line of its own stays on a line of its own, and a comment at the end of a line stays at the
//! end of its line and breaks the parts around it.

use std::collections::HashMap;

use crate::layout::{list_of, Doc, Item, INDENT, PIPE};
use crate::parser::binary_level;
use crate::{parse, Offset, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, TextRange};

use SyntaxKind as K;

/// `text`, a script, in the style. A statement with an error stays as written.
pub fn format(text: &str) -> String {
    let parse = parse(text);
    let root = parse.syntax();
    let formatter = Formatter {
        comments: Comments::new(&root),
        errors: parse.diagnostics().iter().map(|d| d.range).collect(),
    };
    let mut out = formatter.script(&root).render(0);
    if text.ends_with('\n') && !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// The comments of a token: those on lines of their own before it, each with whether a blank line
/// comes before it, and those after it on its line.
#[derive(Default)]
struct Trivia {
    leading: Vec<(String, bool)>,
    /// Whether a blank line comes right before the token, after its leading comments.
    blank: bool,
    trailing: Vec<String>,
}

struct Comments {
    by_token: HashMap<Offset, Trivia>,
    /// The comments after the last token of the script.
    end: Vec<(String, bool)>,
}

impl Comments {
    fn new(root: &SyntaxNode) -> Comments {
        let mut by_token: HashMap<Offset, Trivia> = HashMap::new();
        let mut pending: Vec<(String, bool)> = Vec::new();
        let mut newlines = 0;
        let mut last: Option<Offset> = None;
        for token in root
            .descendants_with_tokens()
            .filter_map(SyntaxElement::into_token)
        {
            match token.kind() {
                K::WHITESPACE => newlines += token.text().matches('\n').count(),
                K::COMMENT | K::HINT => {
                    let text = token.text().trim_end().to_string();
                    match last {
                        Some(last) if newlines == 0 => {
                            by_token.entry(last).or_default().trailing.push(text)
                        }
                        _ => pending.push((text, newlines >= 2)),
                    }
                    newlines = 0;
                }
                _ => {
                    let start = token.text_range().start();
                    let trivia = by_token.entry(start).or_default();
                    trivia.leading = std::mem::take(&mut pending);
                    trivia.blank = newlines >= 2;
                    newlines = 0;
                    last = Some(start);
                }
            }
        }
        Comments {
            by_token,
            end: pending,
        }
    }

    fn of(&self, token: &SyntaxToken) -> Option<&Trivia> {
        self.by_token.get(&token.text_range().start())
    }
}

/// Whether `comment` runs to the end of its line.
fn is_line_comment(comment: &str) -> bool {
    comment.starts_with("--") || comment.starts_with('#')
}

/// The tokens of `node` that are not whitespace or comments.
fn solid_tokens(node: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> {
    (node.descendants_with_tokens())
        .filter_map(SyntaxElement::into_token)
        .filter(|token| !token.kind().is_trivia())
}

/// The first token of `element` that is not whitespace or a comment.
fn first_token(element: &SyntaxElement) -> Option<SyntaxToken> {
    match element {
        SyntaxElement::Token(token) => (!token.kind().is_trivia()).then(|| token.clone()),
        SyntaxElement::Node(node) => solid_tokens(node).next(),
    }
}

fn last_token(element: &SyntaxElement) -> Option<SyntaxToken> {
    match element {
        SyntaxElement::Token(token) => (!token.kind().is_trivia()).then(|| token.clone()),
        SyntaxElement::Node(node) => solid_tokens(node).last(),
    }
}

/// The children of `node` that are not whitespace or comments, and have a token.
fn solid_children(node: &SyntaxNode) -> Vec<SyntaxElement> {
    (node.children_with_tokens())
        .filter(|element| first_token(element).is_some())
        .collect()
}

/// Keywords that a `(` follows without a space, as a call's name is.
const CALL_LIKE: &[SyntaxKind] = &[
    K::CAST_KW,
    K::SAFE_CAST_KW,
    K::EXTRACT_KW,
    K::UNNEST_KW,
    K::STRUCT_KW,
    K::ARRAY_KW,
    K::ROLLUP_KW,
    K::CUBE_KW,
    K::SETS_KW,
    K::OPTIONS_KW,
    K::REPLACE_KW,
    K::REPEATABLE_KW,
    K::MATCH_RECOGNIZE_KW,
];

/// The kinds whose `<` and `>` hold types, without spaces inside.
const ANGLES: &[SyntaxKind] = &[
    K::ARRAY_TYPE,
    K::STRUCT_TYPE,
    K::RANGE_TYPE,
    K::TABLE_TYPE,
    K::ARRAY_EXPR,
    K::STRUCT_EXPR,
    K::TYPED_LITERAL,
];

/// A clause that goes on a line of its own when its part breaks: its node, the keywords that can
/// start it, and whether its items go on the lines after its keywords, as those of `GROUP BY` do,
/// when they do not fit on its line.
pub(crate) struct Clause {
    pub(crate) kind: SyntaxKind,
    pub(crate) keywords: &'static [&'static str],
    pub(crate) list: bool,
}

/// The clauses of a pipe operator, at the depth of the operator's keyword.
pub(crate) const OPERATOR_CLAUSES: &[Clause] = &[
    clause(K::GROUP_BY, &["GROUP"], true),
    clause(K::WINDOW_CLAUSE, &["WINDOW"], true),
];

/// The clauses in the brackets of a call, a window, or `MATCH_RECOGNIZE`, at the depth of the
/// arguments.
pub(crate) const BRACKET_CLAUSES: &[Clause] = &[
    clause(K::NULL_TREATMENT, &["IGNORE", "RESPECT"], false),
    clause(K::HAVING_CLAUSE, &["HAVING"], false),
    clause(K::ORDER_BY, &["ORDER"], true),
    clause(K::CALL_LIMIT, &["LIMIT"], false),
    clause(K::PARTITION_BY, &["PARTITION"], true),
    clause(K::FRAME, &["ROWS", "RANGE"], false),
    clause(K::MEASURES, &["MEASURES"], true),
    clause(K::AFTER_MATCH_SKIP, &["AFTER"], false),
    clause(K::ROW_PATTERN, &["PATTERN"], false),
    clause(K::DEFINE, &["DEFINE"], true),
    clause(K::OPTIONS, &["OPTIONS"], false),
];

const fn clause(kind: SyntaxKind, keywords: &'static [&'static str], list: bool) -> Clause {
    Clause {
        kind,
        keywords,
        list,
    }
}

/// Whether `kind` is the node of one of `clauses`.
fn is_clause(clauses: &[Clause], kind: SyntaxKind) -> bool {
    clauses.iter().any(|clause| clause.kind == kind)
}

/// Whether `kind` is the node of a clause whose items go on the lines after its keywords.
fn is_list_clause(kind: SyntaxKind) -> bool {
    (OPERATOR_CLAUSES.iter().chain(BRACKET_CLAUSES))
        .any(|clause| clause.kind == kind && clause.list)
}

fn parent_kind(token: &SyntaxToken) -> Option<SyntaxKind> {
    token.parent().map(|parent| parent.kind())
}

/// Whether a space goes between the tokens `prev` and `next`, which follow each other on a line.
fn spaced(prev: &SyntaxToken, next: &SyntaxToken) -> bool {
    let (p, n) = (prev.kind(), next.kind());
    let (pp, np) = (parent_kind(prev), parent_kind(next));
    if matches!(
        n,
        K::COMMA | K::SEMICOLON | K::R_PAREN | K::R_BRACK | K::DOT
    ) || matches!(p, K::L_PAREN | K::L_BRACK | K::DOT)
    {
        return false;
    }
    // A path, and a name with dashes, such as a project's name, have no spaces.
    let path = |token: &SyntaxToken| {
        (token.parent_ancestors().find(|node| node.kind() == K::PATH))
            .or_else(|| token.parent().filter(|node| node.kind() == K::NAME_REF))
    };
    let path_of_next = path(next);
    if path(prev).is_some_and(|path| Some(&path) == path_of_next.as_ref()) {
        return false;
    }
    // A prefix operator takes no space, except before a sign, since `- -x` is not `--x`, a
    // comment.
    if pp == Some(K::PREFIX_EXPR) && matches!(p, K::MINUS | K::PLUS | K::TILDE) {
        return matches!(n, K::MINUS | K::PLUS);
    }
    let angled =
        |token: &SyntaxToken| parent_kind(token).is_some_and(|kind| ANGLES.contains(&kind));
    if (n == K::LT && angled(next)) || (p == K::LT && angled(prev)) || (n == K::GT && angled(next))
    {
        return false;
    }
    if p == K::GT && angled(prev) && matches!(n, K::L_PAREN | K::L_BRACK) {
        return false;
    }
    // A pattern's quantifier, as in `a{2,}`, has no spaces.
    if (next.parent_ancestors()).any(|node| node.kind() == K::QUANTIFIER) {
        return false;
    }
    match n {
        K::L_PAREN => !(matches!(p, K::WORD | K::QUOTED_IDENT) || CALL_LIKE.contains(&p)),
        K::L_BRACK => !(np == Some(K::SUBSCRIPT_EXPR) || p == K::ARRAY_KW),
        _ => true,
    }
}

/// The level of a binary operator, from its tokens, as the parser binds it.
fn precedence(operator: &[SyntaxElement]) -> Option<u8> {
    let tokens: Option<Vec<(SyntaxKind, &str)>> = (operator.iter())
        .map(|element| element.as_token().map(|token| (token.kind(), token.text())))
        .collect();
    binary_level(&tokens?)
}

/// Parts in a row on one line, with spaces between their tokens where the style puts them.
struct Row {
    docs: Vec<Doc>,
    last: Option<SyntaxToken>,
}

impl Row {
    fn new() -> Row {
        Row {
            docs: Vec::new(),
            last: None,
        }
    }

    fn push(&mut self, doc: Doc, first: &SyntaxToken, last: SyntaxToken) {
        if self.last.as_ref().is_some_and(|prev| spaced(prev, first)) {
            self.docs.push(Doc::text(" "));
        }
        self.docs.push(doc);
        self.last = Some(last);
    }

    fn doc(self) -> Doc {
        Doc::Concat(self.docs)
    }
}

struct Formatter {
    comments: Comments,
    errors: Vec<TextRange>,
}

impl Formatter {
    /// The text of `token` in the style: a keyword in upper case, the name of a function in lower
    /// case, and any other token as written.
    fn text(&self, token: &SyntaxToken) -> String {
        if token.kind().is_keyword() {
            return token.text().to_ascii_uppercase();
        }
        if token.kind() == K::WORD && is_function_name(token) {
            return token.text().to_ascii_lowercase();
        }
        token.text().to_string()
    }

    /// The comments of `token` on lines of their own before it, each followed by a line break.
    fn leading(&self, token: &SyntaxToken) -> Option<Doc> {
        let trivia = self.comments.of(token)?;
        if trivia.leading.is_empty() {
            return None;
        }
        let mut docs = Vec::new();
        for (index, (comment, blank)) in trivia.leading.iter().enumerate() {
            if index > 0 && *blank {
                docs.push(Doc::HardLine);
            }
            docs.extend([Doc::text(comment), Doc::HardLine]);
        }
        if trivia.blank {
            docs.push(Doc::HardLine);
        }
        Some(Doc::Concat(docs))
    }

    /// The comments of `token` at the end of its line: a line comment at the end of the line that
    /// the token ends, and a block comment right after the token.
    fn trailing(&self, token: &SyntaxToken) -> Vec<Doc> {
        let Some(trivia) = self.comments.of(token) else {
            return Vec::new();
        };
        (trivia.trailing.iter())
            .flat_map(|comment| match is_line_comment(comment) {
                true => vec![Doc::LineSuffix(format!(" {comment}")), Doc::BreakParent],
                false => vec![Doc::text(format!(" {comment}"))],
            })
            .collect()
    }

    /// `token` in the style, with its comments.
    fn token(&self, token: &SyntaxToken) -> Doc {
        let mut docs: Vec<Doc> = self.leading(token).into_iter().collect();
        docs.push(Doc::text(self.text(token)));
        docs.extend(self.trailing(token));
        Doc::Concat(docs)
    }

    /// The comments of `token`, a token that the layout writes itself, such as a comma, at the
    /// end of the line where the layout puts it.
    fn detached(&self, token: &SyntaxToken) -> Option<Doc> {
        let trivia = self.comments.of(token)?;
        let comments: Vec<Doc> = (trivia.leading.iter().map(|(comment, _)| comment))
            .chain(&trivia.trailing)
            .flat_map(|comment| [Doc::LineSuffix(format!(" {comment}")), Doc::BreakParent])
            .collect();
        (!comments.is_empty()).then_some(Doc::Concat(comments))
    }

    /// Whether a blank line comes before `element`, before its first comment.
    fn blank(&self, element: &SyntaxElement) -> bool {
        let Some(trivia) = first_token(element).and_then(|token| self.comments.of(&token)) else {
            return false;
        };
        match trivia.leading.first() {
            Some((_, blank)) => *blank,
            None => trivia.blank,
        }
    }

    /// Whether comments on lines of their own come before `element`.
    fn commented(&self, element: &SyntaxElement) -> bool {
        (first_token(element).and_then(|token| self.comments.of(&token)))
            .is_some_and(|trivia| !trivia.leading.is_empty())
    }

    fn element(&self, element: &SyntaxElement) -> Doc {
        match element {
            SyntaxElement::Token(token) => self.token(token),
            SyntaxElement::Node(node) => self.node(node),
        }
    }

    /// `elements` in a row.
    fn row(&self, elements: &[SyntaxElement]) -> Doc {
        let mut row = Row::new();
        for element in elements {
            if let (Some(first), Some(last)) = (first_token(element), last_token(element)) {
                row.push(self.element(element), &first, last);
            }
        }
        row.doc()
    }

    /// The statements of a script, each followed by its `;`, with a line break between them.
    fn script(&self, root: &SyntaxNode) -> Doc {
        let mut docs = Vec::new();
        let mut statement: Vec<SyntaxElement> = Vec::new();
        let mut first = true;
        let children = solid_children(root);
        for (index, element) in children.iter().enumerate() {
            let end = element.kind() == K::SEMICOLON;
            if !end {
                statement.push(element.clone());
            }
            if end || index + 1 == children.len() {
                if !statement.is_empty() {
                    if !first {
                        docs.push(Doc::HardLine);
                        if self.blank(&statement[0]) {
                            docs.push(Doc::HardLine);
                        }
                    }
                    first = false;
                    docs.push(self.statement(&statement));
                }
                if end {
                    docs.push(self.element(element));
                }
                statement.clear();
            }
        }
        for (index, (comment, blank)) in self.comments.end.iter().enumerate() {
            if !first || index > 0 {
                docs.push(Doc::HardLine);
                if *blank {
                    docs.push(Doc::HardLine);
                }
            }
            docs.push(Doc::text(comment));
        }
        Doc::Concat(docs)
    }

    /// A statement, from the parts between two `;`: in the style, or as written when it has an
    /// error, as the parts of a statement that the parser could not read into one do.
    fn statement(&self, parts: &[SyntaxElement]) -> Doc {
        let (Some(first), Some(last)) = (
            parts.first().and_then(first_token),
            parts.last().and_then(last_token),
        ) else {
            return Doc::Concat(Vec::new());
        };
        let range = TextRange::new(first.text_range().start(), last.text_range().end());
        let erroneous = self.errors.iter().any(|error| touches(*error, range));
        if let ([SyntaxElement::Node(node)], false) = (parts, erroneous) {
            return self.node(node);
        }
        let root = (first.parent_ancestors().last()).expect("a token has a root");
        let text = root.text().slice(range).to_string();
        let mut docs: Vec<Doc> = self.leading(&first).into_iter().collect();
        docs.push(Doc::text(text));
        docs.extend(self.trailing(&last));
        Doc::Concat(docs)
    }

    fn node(&self, node: &SyntaxNode) -> Doc {
        match node.kind() {
            K::QUERY => self.query(node),
            K::PAREN_QUERY => self.parenthesized(node),
            K::ARG_LIST => self.arguments(node),
            K::ARRAY_EXPR
            | K::STRUCT_EXPR
            | K::TUPLE_EXPR
            | K::IN_LIST
            | K::NAME_LIST
            | K::PARAM_LIST
            | K::COLUMN_LIST
            | K::OPTIONS
            | K::ROLLUP
            | K::CUBE
            | K::GROUPING_SETS
            | K::PIVOT_VALUES
            | K::REPLACE_LIST
            | K::UNNEST_SOURCE => self.bracketed(node),
            // A clause whose part puts its clauses on lines of their own.
            kind if is_list_clause(kind)
                && node.parent().is_some_and(|parent| {
                    matches!(
                        parent.kind(),
                        K::PIPE_AGGREGATE
                            | K::PIPE_ORDER_BY
                            | K::PIPE_SELECT
                            | K::PIPE_EXTEND
                            | K::ARG_LIST
                            | K::WINDOW_SPEC
                            | K::MATCH_RECOGNIZE_CLAUSE
                    )
                }) =>
            {
                self.keyword_list(&solid_children(node))
            }
            K::BIN_EXPR => self.binary(node),
            K::CASE_EXPR => self.case(node),
            K::WINDOW_SPEC => self.window(node),
            K::MATCH_RECOGNIZE_CLAUSE => self.match_recognize(node),
            _ => self.row(&solid_children(node)),
        }
    }

    /// A query: its source, then each pipe operator on a line of its own.
    fn query(&self, node: &SyntaxNode) -> Doc {
        let children = solid_children(node);
        let start = (children.iter())
            .position(|child| is_pipe_operator(child.kind()))
            .unwrap_or(children.len());
        let mut docs = vec![self.row(&children[..start])];
        for operator in &children[start..] {
            docs.push(Doc::HardLine);
            if self.blank(operator) {
                docs.push(Doc::HardLine);
            }
            docs.push(self.pipe(operator));
        }
        Doc::Concat(docs)
    }

    /// A pipe operator: `|>`, then the operator, whose lines that break start at the depth of its
    /// keyword.
    fn pipe(&self, operator: &SyntaxElement) -> Doc {
        let SyntaxElement::Node(node) = operator else {
            return self.element(operator);
        };
        let children = solid_children(node);
        let Some((pipe, rest)) = children.split_first() else {
            return self.row(&children);
        };
        let body = match node.kind() {
            K::PIPE_SELECT
            | K::PIPE_EXTEND
            | K::PIPE_SET
            | K::PIPE_DROP
            | K::PIPE_RENAME
            | K::PIPE_AGGREGATE
            | K::PIPE_WITH
            | K::PIPE_SET_OPERATION => self.keyword_list(rest),
            _ => self.row(rest),
        };
        Doc::concat([self.element(pipe), Doc::text(" "), Doc::nest(PIPE, body)])
    }

    /// A keyword and its list of items, then its clauses, such as a pipe `AGGREGATE` and its
    /// `GROUP BY`: on one line when it fits, and otherwise with each clause on a line of its own
    /// at the depth of the keyword, and then, when the items still do not fit, each item on a
    /// line of its own, two spaces deeper, followed by a comma. A sole item stays on the line of
    /// the keyword and breaks inside itself.
    fn keyword_list(&self, children: &[SyntaxElement]) -> Doc {
        let is_head = |element: &SyntaxElement| match element {
            SyntaxElement::Token(token) => token.kind() != K::COMMA,
            SyntaxElement::Node(node) => matches!(
                node.kind(),
                K::SELECT_AS | K::SET_OP_MODE | K::BY_NAME | K::CORRESPONDING
            ),
        };
        let is_clause = |element: &SyntaxElement| is_clause(OPERATOR_CLAUSES, element.kind());
        let start = children
            .iter()
            .position(|c| !is_head(c))
            .unwrap_or(children.len());
        let end = (children.iter())
            .position(is_clause)
            .unwrap_or(children.len())
            .max(start);
        let (items, trailing) = self.items(&children[start..end]);
        let mut docs = vec![self.row(&children[..start])];
        let sole = items.len() == 1
            && !trailing
            && !self.commented(&children[start])
            && items[0].after_comma.is_none();
        if sole {
            docs.extend([Doc::text(" "), items.into_iter().next().unwrap().doc]);
        } else if !items.is_empty() {
            let count = items.len();
            let mut inner = Vec::new();
            for (index, item) in items.into_iter().enumerate() {
                inner.push(Doc::Line);
                if item.blank && index > 0 {
                    inner.push(Doc::HardLine);
                }
                inner.push(item.doc);
                inner.push(match index + 1 == count {
                    true => Doc::if_break(Doc::text(","), Doc::text("")),
                    false => Doc::text(","),
                });
                inner.extend(item.after_comma);
            }
            let list = Doc::group(Doc::nest(INDENT, Doc::Concat(inner)));
            docs.push(match trailing {
                true => list.broken(),
                false => list,
            });
        }
        for clause in &children[end..] {
            docs.extend([Doc::Line, self.element(clause)]);
        }
        Doc::group(Doc::Concat(docs))
    }

    /// The items of a list, separated by commas, and whether a comma follows the last one.
    fn items(&self, elements: &[SyntaxElement]) -> (Vec<Item>, bool) {
        let mut items: Vec<Item> = Vec::new();
        let mut parts: Vec<SyntaxElement> = Vec::new();
        let mut trailing = false;
        let finish = |parts: &mut Vec<SyntaxElement>, items: &mut Vec<Item>| {
            if let Some(first) = parts.first() {
                items.push(Item {
                    doc: self.row(parts),
                    blank: self.blank(first),
                    after_comma: None,
                });
            }
            parts.clear();
        };
        for element in elements {
            match element {
                SyntaxElement::Token(token) if token.kind() == K::COMMA => {
                    finish(&mut parts, &mut items);
                    if let Some(item) = items.last_mut() {
                        item.after_comma = self.detached(token);
                    }
                    trailing = true;
                }
                _ => {
                    parts.push(element.clone());
                    trailing = false;
                }
            }
        }
        finish(&mut parts, &mut items);
        (items, trailing)
    }

    /// A list in brackets, after the parts before its opener, such as `ARRAY<INT64>` or
    /// `OPTIONS`, and before the parts after its closer, such as an alias.
    fn bracketed(&self, node: &SyntaxNode) -> Doc {
        let children = solid_children(node);
        let open = children
            .iter()
            .position(|c| matches!(c.kind(), K::L_PAREN | K::L_BRACK));
        let close = (children.iter()).rposition(|c| matches!(c.kind(), K::R_PAREN | K::R_BRACK));
        let (Some(open), Some(close)) = (open, close) else {
            return self.row(&children);
        };
        if close < open {
            return self.row(&children);
        }
        let (items, trailing) = self.items(&children[open + 1..close]);
        let list = self.list(
            &children[open],
            items,
            Vec::new(),
            &children[close],
            trailing,
        );
        self.around(
            &children[..open],
            list,
            &children[open],
            &children[close],
            &children[close + 1..],
        )
    }

    /// `list`, between the parts before its opener and after its closer, in a row.
    fn around(
        &self,
        before: &[SyntaxElement],
        list: Doc,
        open: &SyntaxElement,
        close: &SyntaxElement,
        after: &[SyntaxElement],
    ) -> Doc {
        let mut row = Row::new();
        for element in before {
            if let (Some(first), Some(last)) = (first_token(element), last_token(element)) {
                row.push(self.element(element), &first, last);
            }
        }
        if let (Some(first), Some(last)) = (first_token(open), last_token(close)) {
            row.push(list, &first, last);
        }
        for element in after {
            if let (Some(first), Some(last)) = (first_token(element), last_token(element)) {
                row.push(self.element(element), &first, last);
            }
        }
        row.doc()
    }

    /// The layout of a list from its opener, items, clauses, and closer: the comments on lines
    /// of their own before the closer stay at the depth of the items.
    fn list(
        &self,
        open: &SyntaxElement,
        items: Vec<Item>,
        clauses: Vec<Doc>,
        close: &SyntaxElement,
        trailing: bool,
    ) -> Doc {
        let (tail, close) = match close {
            SyntaxElement::Token(token) => {
                let tail = self.leading(token).map(|_| {
                    let trivia = self.comments.of(token).expect("leading comments");
                    let mut docs = Vec::new();
                    for (index, (comment, blank)) in trivia.leading.iter().enumerate() {
                        if index > 0 {
                            docs.push(Doc::HardLine);
                            if *blank {
                                docs.push(Doc::HardLine);
                            }
                        }
                        docs.push(Doc::text(comment));
                    }
                    docs.push(Doc::BreakParent);
                    Doc::Concat(docs)
                });
                let mut docs = vec![Doc::text(self.text(token))];
                docs.extend(self.trailing(token));
                (tail, Doc::Concat(docs))
            }
            SyntaxElement::Node(_) => (None, self.element(close)),
        };
        let list = list_of(self.element(open), items, clauses, tail, close);
        match trailing {
            true => list.broken(),
            false => list,
        }
    }

    /// The arguments of a call: its items, and its clauses, such as `ORDER BY`, on lines of their
    /// own when it breaks.
    fn arguments(&self, node: &SyntaxNode) -> Doc {
        let children = solid_children(node);
        let (Some(open), Some(close)) = (children.first(), children.last()) else {
            return self.row(&children);
        };
        if open.kind() != K::L_PAREN || close.kind() != K::R_PAREN || children.len() < 2 {
            return self.row(&children);
        }
        let inner = &children[1..children.len() - 1];
        let is_clause = |element: &SyntaxElement| is_clause(BRACKET_CLAUSES, element.kind());
        let end = inner.iter().position(is_clause).unwrap_or(inner.len());
        // `DISTINCT` goes with the first argument.
        let (distinct, arguments) = match inner.first() {
            Some(first) if first.kind() == K::DISTINCT_KW => (Some(first), &inner[1..end]),
            _ => (None, &inner[..end]),
        };
        let (mut items, trailing) = self.items(arguments);
        if let (Some(distinct), Some(first)) = (distinct, items.first_mut()) {
            let doc = std::mem::replace(&mut first.doc, Doc::text(""));
            first.doc = Doc::concat([self.element(distinct), Doc::text(" "), doc]);
        }
        let clauses = inner[end..].iter().map(|c| self.element(c)).collect();
        self.list(open, items, clauses, close, trailing)
    }

    /// A query in parentheses: on one line when it fits and has no pipe operators, and otherwise
    /// with the query two spaces deeper than the line that opens it.
    fn parenthesized(&self, node: &SyntaxNode) -> Doc {
        let children = solid_children(node);
        let [open, query, close] = children.as_slice() else {
            return self.row(&children);
        };
        Doc::group(Doc::concat([
            self.element(open),
            Doc::nest(INDENT, Doc::concat([Doc::SoftLine, self.element(query)])),
            Doc::SoftLine,
            self.element(close),
        ]))
    }

    /// A chain of binary operators of one precedence: on one line when it fits, and otherwise
    /// with each operator at the start of a line, two spaces deeper than the line of the chain.
    fn binary(&self, node: &SyntaxNode) -> Doc {
        let mut operands: Vec<SyntaxElement> = Vec::new();
        let mut operators: Vec<Vec<SyntaxElement>> = Vec::new();
        self.chain(node, &mut operands, &mut operators);
        let Some((first, rest)) = operands.split_first() else {
            return self.row(&solid_children(node));
        };
        let mut docs = Vec::new();
        for (operator, operand) in operators.iter().zip(rest) {
            let operator: Vec<Doc> = operator.iter().map(|op| self.element(op)).collect();
            docs.extend([
                Doc::Line,
                Doc::Concat(operator),
                Doc::text(" "),
                self.element(operand),
            ]);
        }
        Doc::group(Doc::concat([
            self.element(first),
            Doc::nest(INDENT, Doc::Concat(docs)),
        ]))
    }

    /// The operands and operators of the chain of `node`, flattened through its left operands of
    /// the same precedence.
    fn chain(
        &self,
        node: &SyntaxNode,
        operands: &mut Vec<SyntaxElement>,
        operators: &mut Vec<Vec<SyntaxElement>>,
    ) {
        let children = solid_children(node);
        let nodes: Vec<usize> = (children.iter().enumerate())
            .filter(|(_, c)| matches!(c, SyntaxElement::Node(_)))
            .map(|(index, _)| index)
            .collect();
        let [lhs, rhs] = nodes.as_slice() else {
            operands.push(SyntaxElement::Node(node.clone()));
            return;
        };
        let operator = children[lhs + 1..*rhs].to_vec();
        if operator.is_empty() {
            operands.push(SyntaxElement::Node(node.clone()));
            return;
        }
        match &children[*lhs] {
            SyntaxElement::Node(left)
                if left.kind() == K::BIN_EXPR
                    && precedence(&operator).is_some()
                    && binary_precedence(left) == precedence(&operator) =>
            {
                self.chain(left, operands, operators)
            }
            left => operands.push(left.clone()),
        }
        operators.push(operator);
        operands.push(children[*rhs].clone());
    }

    /// `CASE`: on one line when it fits, and otherwise with each `WHEN` and the `ELSE` on a line
    /// of its own, two spaces deeper, and `END` back at the depth of the line of `CASE`.
    fn case(&self, node: &SyntaxNode) -> Doc {
        let children = solid_children(node);
        let start = (children.iter())
            .position(|c| matches!(c.kind(), K::WHEN_CLAUSE | K::ELSE_CLAUSE))
            .unwrap_or(children.len());
        let end = (children.iter())
            .rposition(|c| c.kind() == K::END_KW)
            .filter(|end| *end >= start);
        let Some(end) = end else {
            return self.row(&children);
        };
        let mut inner = Vec::new();
        for clause in &children[start..end] {
            inner.extend([Doc::Line, self.element(clause)]);
        }
        Doc::group(Doc::concat([
            self.row(&children[..start]),
            Doc::nest(INDENT, Doc::Concat(inner)),
            Doc::Line,
            self.row(&children[end..]),
        ]))
    }

    /// A window's specification: on one line when it fits, and otherwise with each clause on a
    /// line of its own, two spaces deeper.
    fn window(&self, node: &SyntaxNode) -> Doc {
        let children = solid_children(node);
        self.clauses(&children)
            .unwrap_or_else(|| self.row(&children))
    }

    /// `MATCH_RECOGNIZE (...)`, with its clauses laid out as a window's, and its alias.
    fn match_recognize(&self, node: &SyntaxNode) -> Doc {
        let children = solid_children(node);
        let open = children.iter().position(|c| c.kind() == K::L_PAREN);
        let close = children.iter().rposition(|c| c.kind() == K::R_PAREN);
        let (Some(open), Some(close)) = (open, close) else {
            return self.row(&children);
        };
        let Some(clauses) = self.clauses(&children[open..=close]) else {
            return self.row(&children);
        };
        self.around(
            &children[..open],
            clauses,
            &children[open],
            &children[close],
            &children[close + 1..],
        )
    }

    /// Clauses in parentheses, from `(` to `)`: on one line when they fit, and otherwise with
    /// each clause on a line of its own, two spaces deeper, and `)` back at the depth of the
    /// line. `None` for elements without the parentheses.
    fn clauses(&self, children: &[SyntaxElement]) -> Option<Doc> {
        let (open, rest) = children.split_first()?;
        let (close, clauses) = rest.split_last()?;
        if open.kind() != K::L_PAREN || close.kind() != K::R_PAREN {
            return None;
        }
        let mut inner = Vec::new();
        for (index, clause) in clauses.iter().enumerate() {
            inner.push(match index {
                0 => Doc::SoftLine,
                _ => Doc::Line,
            });
            inner.push(self.element(clause));
        }
        Some(Doc::group(Doc::concat([
            self.element(open),
            Doc::nest(INDENT, Doc::Concat(inner)),
            Doc::SoftLine,
            self.element(close),
        ])))
    }
}

/// The precedence of the operator of `node`, a binary expression.
fn binary_precedence(node: &SyntaxNode) -> Option<u8> {
    let children = solid_children(node);
    let nodes: Vec<usize> = (children.iter().enumerate())
        .filter(|(_, c)| matches!(c, SyntaxElement::Node(_)))
        .map(|(index, _)| index)
        .collect();
    let [lhs, rhs] = nodes.as_slice() else {
        return None;
    };
    precedence(&children[lhs + 1..*rhs])
}

fn is_pipe_operator(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        K::PIPE_SELECT
            | K::PIPE_EXTEND
            | K::PIPE_SET
            | K::PIPE_DROP
            | K::PIPE_RENAME
            | K::PIPE_AS
            | K::PIPE_WHERE
            | K::PIPE_AGGREGATE
            | K::PIPE_DISTINCT
            | K::PIPE_ORDER_BY
            | K::PIPE_LIMIT
            | K::PIPE_SET_OPERATION
            | K::PIPE_JOIN
            | K::PIPE_CALL
            | K::PIPE_SAMPLE
            | K::PIPE_WITH
            | K::PIPE_PIVOT
            | K::PIPE_UNPIVOT
            | K::PIPE_MATCH_RECOGNIZE
    )
}

/// Whether `token` names a function: the last name of the path of a call, a table function in
/// `FROM`, or a pipe `CALL`.
fn is_function_name(token: &SyntaxToken) -> bool {
    let Some(name) = token.parent().filter(|name| name.kind() == K::NAME_REF) else {
        return false;
    };
    if solid_tokens(&name).count() != 1 {
        return false;
    }
    let Some(path) = name.parent().filter(|path| path.kind() == K::PATH) else {
        return false;
    };
    let called = (path.parent())
        .is_some_and(|parent| matches!(parent.kind(), K::CALL_EXPR | K::TVF_SOURCE | K::PIPE_CALL));
    called && path.children().filter(|c| c.kind() == K::NAME_REF).last() == Some(name)
}

/// Whether `error` touches `statement`, as an error at the `;` after a statement does.
fn touches(error: TextRange, statement: TextRange) -> bool {
    error.start() <= statement.end() && statement.start() <= error.end()
}
