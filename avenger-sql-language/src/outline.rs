//! The outline of a script, and its folding and selection ranges, from its syntax tree.

use avenger_sql_syntax::{
    ast::{self, AstNode, CreateStmt},
    SyntaxKind, SyntaxNode, SyntaxToken, TextRange,
};

use crate::{first_line, statements, trimmed, Document, Offset};

/// An item of the outline of a script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    /// The name, never empty.
    pub name: String,
    pub kind: SymbolKind,
    /// More on the item, such as the type of a variable or the output columns of an operator.
    pub detail: Option<String>,
    /// The range of the item's text.
    pub range: TextRange,
    /// The range of the item's name, inside `range`.
    pub name_range: TextRange,
    pub children: Vec<Symbol>,
}

/// What an item of the outline is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// A variable of a `DECLARE`.
    Variable,
    /// A temporary table.
    Table,
    /// A temporary function.
    Function,
    /// A module of an `IMPORT MODULE`.
    Module,
    /// A query of the script, or a pipe `WITH` name.
    Query,
    /// A pipe operator.
    Operator,
}

/// A range that an editor can fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldingRange {
    pub range: TextRange,
    /// Whether the range is a comment.
    pub comment: bool,
}

/// The longest name of a query, in characters, which the first line of its text gives.
const QUERY_NAME: usize = 60;

impl Document {
    /// The outline of the script: its variables, temporary tables and functions, imports, and
    /// queries, and under a query, its pipe operators, and under a pipe `WITH`, its names.
    pub fn symbols(&self) -> Vec<Symbol> {
        let mut symbols = Vec::new();
        for stmt in statements(&self.root()) {
            let range = stmt.syntax().text_range();
            match stmt {
                ast::Stmt::Query(query) => symbols.extend(self.query_symbol(&query)),
                ast::Stmt::DeclareStmt(declare) => {
                    let detail = declare.ty().map(|ty| ty.syntax().to_string());
                    for name in declare.names() {
                        symbols.push(Symbol {
                            name: name.syntax().to_string(),
                            kind: SymbolKind::Variable,
                            detail: detail.clone(),
                            range,
                            name_range: name.syntax().text_range(),
                            children: Vec::new(),
                        });
                    }
                }
                ast::Stmt::CreateTableStmt(create) => {
                    let Some(path) = create.name_path() else {
                        continue;
                    };
                    symbols.push(Symbol {
                        name: path.syntax().to_string(),
                        kind: SymbolKind::Table,
                        detail: None,
                        range,
                        name_range: path.syntax().text_range(),
                        children: (create.query())
                            .map(|query| self.operator_symbols(&query))
                            .unwrap_or_default(),
                    });
                }
                ast::Stmt::CreateFunctionStmt(create) => {
                    let Some(path) = create.name_path() else {
                        continue;
                    };
                    let detail =
                        (create.param_list()).map(|params| collapse(&params.syntax().to_string()));
                    symbols.push(Symbol {
                        name: path.syntax().to_string(),
                        kind: SymbolKind::Function,
                        detail,
                        range,
                        name_range: path.syntax().text_range(),
                        children: Vec::new(),
                    });
                }
                ast::Stmt::ImportStmt(import) => {
                    let Some(path) = import.path() else {
                        continue;
                    };
                    symbols.push(Symbol {
                        name: path.syntax().to_string(),
                        kind: SymbolKind::Module,
                        detail: (import.alias().and_then(|alias| alias.name()))
                            .map(|name| format!("AS {}", name.syntax())),
                        range,
                        name_range: path.syntax().text_range(),
                        children: Vec::new(),
                    });
                }
                ast::Stmt::SetStmt(_) | ast::Stmt::CreateConstantStmt(_) => {}
            }
        }
        symbols
    }

    /// The symbol of a query, named by the first line of its text.
    fn query_symbol(&self, query: &ast::Query) -> Option<Symbol> {
        let range = query.syntax().text_range();
        let name = first_line(&query.syntax().to_string(), QUERY_NAME);
        if name.is_empty() {
            return None;
        }
        let text = self.slice(range);
        let start = range.start() + Offset::from((text.len() - text.trim_start().len()) as u32);
        let line = text
            .trim_start()
            .lines()
            .next()
            .unwrap_or_default()
            .trim_end();
        let name_range = TextRange::at(start, Offset::from(line.len() as u32));
        Some(Symbol {
            name,
            kind: SymbolKind::Query,
            detail: None,
            range,
            name_range,
            children: self.operator_symbols(query),
        })
    }

    /// The symbols of the pipe operators of `query`, each named by its keywords, with its output
    /// columns as its detail, and the names of a pipe `WITH` under it.
    fn operator_symbols(&self, query: &ast::Query) -> Vec<Symbol> {
        let mut symbols = Vec::new();
        for operator in query.pipe_operators() {
            let range = operator.syntax().text_range();
            let keywords = keywords(operator.syntax());
            let (Some(first), Some(last)) = (keywords.first(), keywords.last()) else {
                continue;
            };
            let name: Vec<String> = (keywords.iter())
                .map(|token| token.text().to_ascii_uppercase())
                .collect();
            let detail = (self.analysis.model.operators.iter())
                .rfind(|scope| scope.range == range)
                .map(|scope| {
                    (scope.output.iter())
                        .filter(|column| !column.name.is_empty())
                        .map(|column| column.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|detail| !detail.is_empty());
            let children = match &operator {
                ast::PipeOperator::PipeWith(with) => (with.named_queries())
                    .filter_map(|named| {
                        let name = named.name()?;
                        Some(Symbol {
                            name: name.syntax().to_string(),
                            kind: SymbolKind::Query,
                            detail: None,
                            range: named.syntax().text_range(),
                            name_range: name.syntax().text_range(),
                            children: Vec::new(),
                        })
                    })
                    .collect(),
                _ => Vec::new(),
            };
            symbols.push(Symbol {
                name: name.join(" "),
                kind: SymbolKind::Operator,
                detail,
                range,
                name_range: TextRange::new(first.text_range().start(), last.text_range().end()),
                children,
            });
        }
        symbols
    }

    /// The statements, parenthesized queries, and argument lists that span lines, and the block
    /// comments and runs of line comments that span lines, in the order of the text.
    pub fn folding_ranges(&self) -> Vec<FoldingRange> {
        let root = self.root();
        let mut ranges: Vec<FoldingRange> = statements(&root)
            .map(|stmt| stmt.syntax().clone())
            .chain(root.descendants().filter(|node| {
                matches!(node.kind(), SyntaxKind::PAREN_QUERY | SyntaxKind::ARG_LIST)
            }))
            .map(|node| trimmed(&node))
            .filter(|range| self.slice(*range).contains('\n'))
            .map(|range| FoldingRange {
                range,
                comment: false,
            })
            .collect();
        // Line comments fold as a run of comments with no blank line between them.
        let mut run: Option<(TextRange, usize)> = None;
        let close = |run: &mut Option<(TextRange, usize)>, ranges: &mut Vec<FoldingRange>| {
            if let Some((range, count)) = run.take() {
                if count > 1 {
                    ranges.push(FoldingRange {
                        range,
                        comment: true,
                    });
                }
            }
        };
        for token in root
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
        {
            match token.kind() {
                SyntaxKind::COMMENT if token.text().starts_with("/*") => {
                    close(&mut run, &mut ranges);
                    if token.text().contains('\n') {
                        ranges.push(FoldingRange {
                            range: token.text_range(),
                            comment: true,
                        });
                    }
                }
                SyntaxKind::COMMENT => {
                    run = Some(match run {
                        Some((range, count)) => (range.cover(token.text_range()), count + 1),
                        None => (token.text_range(), 1),
                    });
                }
                SyntaxKind::WHITESPACE if token.text().matches('\n').count() <= 1 => {}
                _ => close(&mut run, &mut ranges),
            }
        }
        close(&mut run, &mut ranges);
        ranges.sort_by_key(|range| (range.range.start(), std::cmp::Reverse(range.range.end())));
        ranges
    }

    /// The ranges of the token at `offset` and of the nodes around it, innermost first, without
    /// repeats, for an editor's command that grows a selection.
    pub fn selection_ranges(&self, offset: Offset) -> Vec<TextRange> {
        let offset = self.clamp(offset);
        let root = self.root();
        let tokens = root.token_at_offset(offset);
        // A word or a literal over punctuation, and punctuation over whitespace.
        let rank = |token: &SyntaxToken| match token.kind().is_trivia() {
            true => 0,
            false if token.text().chars().any(char::is_alphanumeric) => 2,
            false => 1,
        };
        let Some(token) = tokens.max_by_key(rank) else {
            return Vec::new();
        };
        let mut ranges = vec![token.text_range()];
        for node in token.parent_ancestors() {
            if ranges.last() != Some(&node.text_range()) {
                ranges.push(node.text_range());
            }
        }
        ranges
    }
}

/// The `|>` and the keywords at the start of the pipe operator `node`, such as `|> ORDER BY`.
fn keywords(node: &SyntaxNode) -> Vec<SyntaxToken> {
    (node.descendants_with_tokens())
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .take_while(|token| token.kind() == SyntaxKind::PIPE || token.kind().is_keyword())
        .collect()
}

/// `text` with each run of whitespace as one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
