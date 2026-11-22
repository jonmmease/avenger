//! Editor features of avenger-sql scripts: diagnostics, semantic highlighting, the outline,
//! folding and selection ranges, go to definition, references, rename, hover, completion,
//! signature help, and formatting.
//!
//! A [`Document`] parses and analyzes the text of a file once, in the mode of its [`Role`]: a
//! chart script, a data script, or a module. Each of its requests reads that parse and analysis.
//! The requests mirror those of the Language Server Protocol, without the protocol: positions
//! and ranges are byte offsets of the text, [`Offset`] and [`TextRange`], and a host converts
//! them to its own units, such as the UTF-16 positions of the protocol or the character indices
//! of a text widget. A request takes an offset at a character boundary of the text, and an offset
//! past the end reads as the end.
//!
//! ```
//! use std::sync::Arc;
//!
//! use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library};
//! use avenger_sql_language::{CompletionKind, Document, Role};
//! use datafusion::arrow::datatypes::{DataType, Field, Schema};
//!
//! let schema = Schema::new(vec![Field::new("species", DataType::Utf8, true)]);
//! let catalog = MemoryCatalog::new().with_schema(&["penguins"], Arc::new(schema));
//! let text = "FROM penguins |> WHERE ";
//! let document = Document::new(
//!     text,
//!     Role::Script,
//!     Arc::new(catalog),
//!     Arc::new(Library::new()),
//!     Options::default(),
//! );
//! let completions = document.completions((text.len() as u32).into());
//! let species = completions.items.iter().find(|item| item.label == "species").unwrap();
//! assert_eq!(species.kind, CompletionKind::Column);
//! assert_eq!(species.detail.as_deref(), Some("STRING"));
//! ```

mod completion;
mod data;
mod hover;
mod navigation;
mod outline;
mod signature;

use std::sync::Arc;

use avenger_sql::{
    analysis::{Analysis, Model, Options},
    catalog::Catalog,
    data::{DataAnalysis, Definition},
    library::Library,
};
use avenger_sql_syntax::{
    ast, ast::AstNode, highlight, Parse, SyntaxKind, SyntaxNode, SyntaxToken,
};

pub use avenger_sql_syntax::{
    DiagnosticCode, Fix, Highlight, HighlightRange, Offset, Role, TextEdit, TextRange,
};
pub use completion::{CompletionItem, CompletionKind, Completions};
pub use hover::Hover;
pub use navigation::{DefinitionLink, Reference};
pub use outline::{FoldingRange, Symbol, SymbolKind};
pub use signature::{ParameterInfo, SignatureHelp, SignatureInfo};

/// The text of a file, its role, its parse, and its analysis, which the requests of an editor
/// read.
///
/// A `Document` is `Send` and `Sync`, so a host can analyze a text off its main thread and answer
/// requests from several threads. Every edit of the text makes a new `Document`.
pub struct Document {
    text: String,
    role: Role,
    parse: Parse,
    analysis: Analyzed,
    catalog: Arc<dyn Catalog + Send + Sync>,
    library: Arc<Library>,
    options: Options,
    /// The paths of the project's files, relative to its root, which completion offers in a URI.
    paths: Arc<[String]>,
}

/// The analysis of a text by its role.
#[allow(clippy::large_enum_variant)]
enum Analyzed {
    Script(Analysis),
    Data(DataAnalysis),
    /// A module, which avenger-sql analyzes where a script imports it, so that its document reads
    /// its syntax alone.
    Module(Model),
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
    /// The function of the call, table function source, or pipe `CALL` `call`: a constructor of a
    /// chart's option, by the name that the parameter of its argument gives it, or a function of
    /// the library.
    fn called(&self, call: &SyntaxNode) -> Option<avenger_sql::library::context::Called<'_>> {
        use avenger_sql::library::context::{called, Called};
        called(&self.library, call).or_else(|| {
            let function = self.library.get(&hover::callee(call)?)?;
            Some(Called {
                function,
                name: function.name.clone(),
                fills: None,
            })
        })
    }

    /// Parses and analyzes `text`, a file of the role `role`, whose tables come from `catalog`
    /// and whose functions come from `library`. A chart script analyzes with
    /// [`avenger_sql::analyze`], and a data script with [`avenger_sql::analyze_data`], and the
    /// catalog of both gives the tables of their files, as [`Catalog::files`]. A module has the
    /// requests of its syntax: avenger-sql analyzes a module where a script imports it.
    pub fn new(
        text: impl Into<String>,
        role: Role,
        catalog: Arc<dyn Catalog + Send + Sync>,
        library: Arc<Library>,
        options: Options,
    ) -> Self {
        let text = text.into();
        let (parse, analysis) = match role {
            Role::Script => {
                let parse = avenger_sql_syntax::parse(&text);
                let analysis =
                    avenger_sql::analyze(&parse.syntax(), catalog.as_ref(), &library, &options);
                (parse, Analyzed::Script(analysis))
            }
            Role::Data => {
                let parse = avenger_sql_syntax::parse_data(&text);
                let analysis = avenger_sql::analyze_data(
                    &parse.syntax(),
                    catalog.as_ref(),
                    &library,
                    &options,
                );
                (parse, Analyzed::Data(analysis))
            }
            Role::Module => {
                let parse = avenger_sql_syntax::parse_module(&text);
                (parse, Analyzed::Module(Model::default()))
            }
        };
        Self {
            text,
            role,
            parse,
            analysis,
            catalog,
            library,
            options,
            paths: Arc::from([]),
        }
    }

    /// The document with `paths`, the paths of the files of its project relative to the
    /// project's root, separated by `/`, as `data/files/sales.csv`. Completion in a URI of a data
    /// script offers them, and the directories that hold them. The host lists the files, since a
    /// document reads none.
    pub fn with_paths(mut self, paths: Arc<[String]>) -> Self {
        self.paths = paths;
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The role of the text, which its parse and its analysis follow.
    pub fn role(&self) -> Role {
        self.role
    }

    /// The analysis of a chart script: the plans of its statements, which a host runs, and its
    /// model. `None` for a text of another role.
    pub fn analysis(&self) -> Option<&Analysis> {
        match &self.analysis {
            Analyzed::Script(analysis) => Some(analysis),
            Analyzed::Data(_) | Analyzed::Module(_) => None,
        }
    }

    /// The analysis of a data script: the tables, views, and schemas that it defines, the reads
    /// that leave it, and its model. `None` for a text of another role.
    pub fn data_analysis(&self) -> Option<&DataAnalysis> {
        match &self.analysis {
            Analyzed::Data(analysis) => Some(analysis),
            Analyzed::Script(_) | Analyzed::Module(_) => None,
        }
    }

    /// The statements of the text that define a table, a view, or a schema, or load files into a
    /// table, in the order of the text: a data script's, and a chart script's own tables of
    /// files. A module has none.
    pub(crate) fn definitions(&self) -> &[Definition] {
        match &self.analysis {
            Analyzed::Script(analysis) => &analysis.file_tables,
            Analyzed::Data(analysis) => &analysis.definitions,
            Analyzed::Module(_) => &[],
        }
    }

    /// The text of a chart script as the script that its lifted calls plan: each pipe
    /// `AGGREGATE` with a mark call that computes statistics in its channels becomes its staged
    /// pipe query, and the ranges that the printer wrote mark it. `None` for a text with an
    /// error, and for a text of another role.
    pub fn lifted(&self) -> Option<avenger_sql::analysis::Lifted> {
        self.analysis()?.lift(&self.text)
    }

    /// The semantic model of the text's queries.
    fn model(&self) -> &Model {
        match &self.analysis {
            Analyzed::Script(analysis) => &analysis.model,
            Analyzed::Data(analysis) => &analysis.model,
            Analyzed::Module(model) => model,
        }
    }

    /// The errors of the analysis.
    fn analysis_diagnostics(&self) -> &[avenger_sql::analysis::Diagnostic] {
        match &self.analysis {
            Analyzed::Script(analysis) => &analysis.diagnostics,
            Analyzed::Data(analysis) => &analysis.diagnostics,
            Analyzed::Module(_) => &[],
        }
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
            (self.analysis_diagnostics().iter())
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

    /// The edits that format the text in the style of `avenger-sql/STYLE.md`: one edit of the
    /// range between the text's longest common prefix and suffix with the formatted text, or none
    /// when the text is already formatted. A statement with an error stays as written.
    pub fn formatting(&self) -> Vec<TextEdit> {
        let formatted = avenger_sql_syntax::format::format(&self.text, self.role);
        let (old, new) = (self.text.as_bytes(), formatted.as_bytes());
        let mut prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        while !self.text.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let limit = old.len().min(new.len()) - prefix;
        let mut suffix = (old.iter().rev().zip(new.iter().rev()))
            .take(limit)
            .take_while(|(a, b)| a == b)
            .count();
        while !self.text.is_char_boundary(old.len() - suffix) {
            suffix -= 1;
        }
        if prefix == old.len() && prefix == new.len() {
            return Vec::new();
        }
        let range = TextRange::new(
            Offset::from(prefix as u32),
            Offset::from((old.len() - suffix) as u32),
        );
        vec![TextEdit::new(range, &formatted[prefix..new.len() - suffix])]
    }

    /// The classes of the tokens, for semantic highlighting: the classes of the syntax, which the
    /// analysis refines where it resolved a name, a function, or a value of a set.
    pub fn highlights(&self) -> Vec<HighlightRange> {
        let mut ranges = highlight(&self.root());
        self.model().refine_highlights(&mut ranges);
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
/// not, or when it is a reserved word, as completion inserts a name. A host that inserts a name of
/// its own, such as a column that a list of tables shows, writes it the same way.
pub fn quote(name: &str) -> String {
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

/// `text` with each run of whitespace as one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first line of `text`, without the whitespace around it, shortened to `max` characters.
fn first_line(text: &str, max: usize) -> String {
    let line = text.trim().lines().next().unwrap_or_default().trim();
    match line.chars().count() > max {
        true => format!("{}…", line.chars().take(max).collect::<String>()),
        false => line.to_string(),
    }
}
