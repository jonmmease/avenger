//! Editor features of avenger-sql scripts: diagnostics, semantic highlighting, the outline,
//! folding and selection ranges, go to definition, references, rename, hover, completion, and
//! signature help.
//!
//! A [`Document`] parses and analyzes the text of a script once, and each of its requests reads
//! that parse and analysis. The requests mirror those of the Language Server Protocol, without
//! the protocol: positions and ranges are byte offsets of the text, [`Offset`] and
//! [`TextRange`], and a host converts them to its own units, such as the UTF-16 positions of the
//! protocol or the character indices of a text widget. A request takes an offset at a character
//! boundary of the text, and an offset past the end reads as the end.
//!
//! ```
//! use std::sync::Arc;
//!
//! use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library};
//! use avenger_sql_language::{CompletionKind, Document};
//! use datafusion::arrow::datatypes::{DataType, Field, Schema};
//!
//! let schema = Schema::new(vec![Field::new("species", DataType::Utf8, true)]);
//! let catalog = MemoryCatalog::new().with_schema(&["penguins"], Arc::new(schema));
//! let text = "FROM penguins |> WHERE ";
//! let document = Document::new(text, Arc::new(catalog), Arc::new(Library::new()), Options::default());
//! let completions = document.completions((text.len() as u32).into());
//! let species = completions.items.iter().find(|item| item.label == "species").unwrap();
//! assert_eq!(species.kind, CompletionKind::Column);
//! assert_eq!(species.detail.as_deref(), Some("STRING"));
//! ```

mod completion;
mod hover;
mod navigation;
mod outline;
mod signature;

use std::sync::Arc;

use avenger_sql::{
    analysis::{Analysis, Options},
    catalog::Catalog,
    library::Library,
};
use avenger_sql_syntax::{
    ast, ast::AstNode, highlight, Parse, SyntaxKind, SyntaxNode, SyntaxToken,
};

pub use avenger_sql_syntax::{
    DiagnosticCode, Fix, Highlight, HighlightRange, Offset, TextEdit, TextRange,
};
pub use completion::{CompletionItem, CompletionKind, Completions};
pub use hover::Hover;
pub use navigation::{DefinitionLink, Reference};
pub use outline::{FoldingRange, Symbol, SymbolKind};
pub use signature::{ParameterInfo, SignatureHelp, SignatureInfo};

/// The text of a script, its parse, and its analysis, which the requests of an editor read.
///
/// A `Document` is `Send` and `Sync`, so a host can analyze a text off its main thread and answer
/// requests from several threads. Every edit of the text makes a new `Document`.
pub struct Document {
    text: String,
    parse: Parse,
    analysis: Analysis,
    catalog: Arc<dyn Catalog + Send + Sync>,
    library: Arc<Library>,
    options: Options,
}

// A `Document` crosses threads. It keeps the green tree of its parse, and each request makes the
// root node of the tree, whose nodes cannot cross threads.
const _: fn() = || {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Document>();
};

/// An error of a script, of its syntax or of its analysis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: TextRange,
    pub message: String,
    /// What to do about the error, beyond its message.
    pub help: Option<String>,
    pub source: Source,
    /// The code of an error of the syntax. An error of the analysis has none.
    pub code: Option<DiagnosticCode>,
    /// The edits that fix the error, which an editor offers as code actions.
    pub fixes: Vec<Fix>,
}

/// What found an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// The lexer, the parser, or the validation of the syntax tree.
    Syntax,
    /// The analysis, which resolves names and types and plans the queries.
    Analysis,
}

impl Document {
    /// Parses and analyzes `text`, whose tables come from `catalog` and whose functions come
    /// from `library`.
    pub fn new(
        text: impl Into<String>,
        catalog: Arc<dyn Catalog + Send + Sync>,
        library: Arc<Library>,
        options: Options,
    ) -> Self {
        let text = text.into();
        let parse = avenger_sql_syntax::parse(&text);
        let analysis = avenger_sql::analyze(&parse.syntax(), catalog.as_ref(), &library, &options);
        Self {
            text,
            parse,
            analysis,
            catalog,
            library,
            options,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The analysis of the text: the plans of its statements, which a host runs, and its model.
    pub fn analysis(&self) -> &Analysis {
        &self.analysis
    }

    /// The errors of the text, in the order of their ranges. An error of the analysis inside a
    /// statement with an error of the syntax is left out: the analysis reads what the parser
    /// could make of the statement, so its errors there repeat the syntax's. An error of the
    /// syntax belongs to the statement before it, as an error at the `;` that ends it does.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let syntax = self.parse.diagnostics();
        let starts: Vec<Offset> = statements(&self.root())
            .map(|stmt| stmt.syntax().text_range().start())
            .collect();
        let end = Offset::from(self.text.len() as u32);
        let broken: Vec<TextRange> = (starts.iter().enumerate())
            .map(|(i, &start)| TextRange::new(start, starts.get(i + 1).copied().unwrap_or(end)))
            .filter(|range| {
                (syntax.iter()).any(|diagnostic| range.contains_inclusive(diagnostic.range.start()))
            })
            .collect();
        let mut diagnostics: Vec<Diagnostic> = (syntax.iter())
            .map(|diagnostic| Diagnostic {
                range: diagnostic.range,
                message: diagnostic.message.clone(),
                help: diagnostic.help.clone(),
                source: Source::Syntax,
                code: Some(diagnostic.code),
                fixes: diagnostic.fixes.clone(),
            })
            .collect();
        diagnostics.extend(
            (self.analysis.diagnostics.iter())
                .filter(|diagnostic| {
                    !(broken.iter()).any(|range| range.intersect(diagnostic.range).is_some())
                })
                .map(|diagnostic| Diagnostic {
                    range: diagnostic.range,
                    message: diagnostic.message.clone(),
                    help: None,
                    source: Source::Analysis,
                    code: None,
                    fixes: Vec::new(),
                }),
        );
        diagnostics.sort_by_key(|diagnostic| diagnostic.range.start());
        diagnostics
    }

    /// The classes of the tokens, for semantic highlighting: the classes of the syntax, which the
    /// analysis refines where it resolved a name, a function, or a value of a set.
    pub fn highlights(&self) -> Vec<HighlightRange> {
        let mut ranges = highlight(&self.root());
        self.analysis.model.refine_highlights(&mut ranges);
        ranges
    }

    /// The root of the syntax tree, made for each request.
    fn root(&self) -> SyntaxNode {
        self.parse.syntax()
    }

    /// `offset`, or the end of the text for an offset past it.
    fn clamp(&self, offset: Offset) -> Offset {
        offset.min(Offset::from(self.text.len() as u32))
    }

    fn slice(&self, range: TextRange) -> &str {
        &self.text[std::ops::Range::<usize>::from(range)]
    }
}

/// The statements of the script `root`.
fn statements(root: &SyntaxNode) -> impl Iterator<Item = ast::Stmt> {
    ast::Script::cast(root.clone())
        .into_iter()
        .flat_map(|script| script.stmts())
}

/// The range of `node` from its first token to its last, without the whitespace and comments
/// around them, which a statement's node holds.
fn trimmed(node: &SyntaxNode) -> TextRange {
    let mut tokens = (node.descendants_with_tokens())
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia());
    match tokens.next() {
        Some(first) => {
            let last = tokens.last().unwrap_or_else(|| first.clone());
            TextRange::new(first.text_range().start(), last.text_range().end())
        }
        None => node.text_range(),
    }
}

/// The last token of `root` that is not whitespace or a comment and ends at or before `offset`.
fn token_before(root: &SyntaxNode, offset: Offset) -> Option<SyntaxToken> {
    if offset == Offset::from(0) {
        return None;
    }
    let mut token = root.token_at_offset(offset).left_biased()?;
    loop {
        if !token.kind().is_trivia() && token.text_range().end() <= offset {
            return Some(token);
        }
        token = token.prev_token()?;
    }
}

/// The node of `root` whose range is `range`, of the kind `kind`.
fn node_at(root: &SyntaxNode, range: TextRange, kind: SyntaxKind) -> Option<SyntaxNode> {
    let element = root.covering_element(range);
    let node = match element {
        avenger_sql_syntax::SyntaxElement::Node(node) => node,
        avenger_sql_syntax::SyntaxElement::Token(token) => token.parent()?,
    };
    node.ancestors()
        .take_while(|node| node.text_range() == range)
        .find(|node| node.kind() == kind)
}

/// `name` as a script writes it: as it is when it is a plain name, and in backticks when it is
/// not, or when it is a reserved word.
pub(crate) fn quote(name: &str) -> String {
    let plain = (name.chars().next()).is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    match plain && !avenger_sql_syntax::is_reserved(name) {
        true => name.to_string(),
        false => format!("`{}`", escape(name)),
    }
}

/// `name` as the inside of a quoted name writes it, with `\` before a backslash or a backtick.
pub(crate) fn escape(name: &str) -> String {
    name.replace('\\', "\\\\").replace('`', "\\`")
}

/// The first line of `text`, without the whitespace around it, shortened to `max` characters.
fn first_line(text: &str, max: usize) -> String {
    let line = text.trim().lines().next().unwrap_or_default().trim();
    match line.chars().count() > max {
        true => format!("{}…", line.chars().take(max).collect::<String>()),
        false => line.to_string(),
    }
}
