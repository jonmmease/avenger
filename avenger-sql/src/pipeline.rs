//! The pipeline of a script: [`sql`] parses it, loads the tables that it names from a session,
//! analyzes it, and plans its last query.

use std::fmt;

use avenger_sql_syntax::{
    ast::{self, AstNode, CreateStmt, Scope},
    line_column, SyntaxNode,
};

use datafusion::{
    common::{DataFusionError, Result},
    dataframe::DataFrame,
    prelude::SessionContext,
};

use crate::{
    analysis::{analyze, Analysis, Diagnostic, DiagnosticCode, Lifted, Options},
    catalog::{references, Catalog, SessionCatalog},
    library::Library,
};

/// The errors of a script: the errors of its parse, or else of its analysis, each at a range of
/// the text.
#[derive(Clone, Debug)]
pub struct ScriptError {
    pub diagnostics: Vec<Diagnostic>,
    /// The text of the script, for the lines and columns of the messages.
    text: String,
}

impl ScriptError {
    /// The error of the diagnostics `diagnostics` of the script `text`.
    pub fn new(text: &str, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            diagnostics,
            text: text.to_string(),
        }
    }
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            let (line, column) = line_column(&self.text, diagnostic.range.start());
            write!(f, "{} [at {line}:{column}]", diagnostic.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for ScriptError {}

/// The rows of the last query of the script `text`, as a DataFrame of `ctx`: the tables that the
/// script names come from `ctx`, and each variable before the query evaluates first and binds
/// its value. A script with an error returns a [`ScriptError`] of every diagnostic, as
/// [`DataFusionError::External`].
pub async fn sql(ctx: &SessionContext, text: &str) -> Result<DataFrame> {
    sql_with_modules(ctx, text, &|_| None).await
}

/// The rows of the last query of the script `text`, as [`sql`] plans them, with the modules that
/// `modules` gives for its `IMPORT MODULE`s: the text of the module at a path, or `None` for a
/// path without one. A script with its own tables of files, `CREATE TEMP EXTERNAL TABLE` and
/// `LOAD DATA ... TEMP TABLE`, is an error at each, since a project runs it, as
/// [`crate::data::ScriptTables`] builds the tables of its files.
pub async fn sql_with_modules(
    ctx: &SessionContext,
    text: &str,
    modules: &dyn Fn(&[String]) -> Option<String>,
) -> Result<DataFrame> {
    let root = script_tree(text)?;
    let references = references(&root);
    let catalog = SessionCatalog::load(ctx, &references)
        .await?
        .with_modules(&references, modules);
    session_analysis(text, &root, &catalog)?
        .dataframe(ctx)
        .await
}

/// The tree of the script `text`, or a [`ScriptError`] of the errors of its parse, as
/// [`DataFusionError::External`].
pub fn script_tree(text: &str) -> Result<SyntaxNode> {
    let parse = avenger_sql_syntax::parse(text);
    if !parse.ok() {
        let diagnostics = (parse.diagnostics().iter())
            .map(|diagnostic| Diagnostic {
                range: diagnostic.range,
                message: diagnostic.message.clone(),
                code: None,
            })
            .collect();
        return Err(script_error(text, diagnostics));
    }
    Ok(parse.syntax())
}

/// The analysis of the script `text`, whose tree is `root`, with the tables and the modules of
/// `catalog`, or a [`ScriptError`] of the errors of the analysis. A host with its own catalog,
/// such as a project's, plans a script with [`script_tree`] and this, as [`sql_with_modules`]
/// does with the tables of a session.
pub fn script_analysis(text: &str, root: &SyntaxNode, catalog: &dyn Catalog) -> Result<Analysis> {
    let analysis = analyze(root, catalog, Library::shared(), &Options::default());
    match analysis.diagnostics.is_empty() {
        true => Ok(analysis),
        false => Err(script_error(text, analysis.diagnostics)),
    }
}

/// The script `text` as the script that its lifted calls plan, as [`Analysis::lift`] prints it,
/// with the tables of `ctx`. A script with an error returns a [`ScriptError`] of every diagnostic.
///
/// [`Analysis::lift`]: crate::analysis::Analysis::lift
pub async fn lift(ctx: &SessionContext, text: &str) -> Result<Lifted> {
    lift_with_modules(ctx, text, &|_| None).await
}

/// The script `text` as the script that its lifted calls plan, as [`lift`] prints it, with the
/// modules that `modules` gives for its `IMPORT MODULE`s, as [`sql_with_modules`] takes them.
pub async fn lift_with_modules(
    ctx: &SessionContext,
    text: &str,
    modules: &dyn Fn(&[String]) -> Option<String>,
) -> Result<Lifted> {
    let root = script_tree(text)?;
    let references = references(&root);
    let catalog = SessionCatalog::load(ctx, &references)
        .await?
        .with_modules(&references, modules);
    let analysis = session_analysis(text, &root, &catalog)?;
    Ok(analysis.lift(text).expect("a script without errors lifts"))
}

fn script_error(text: &str, diagnostics: Vec<Diagnostic>) -> DataFusionError {
    DataFusionError::External(Box::new(ScriptError::new(text, diagnostics)))
}

/// The analysis of the script `text`, whose tree is `root`, with the tables of a session that
/// `catalog` holds, as [`script_analysis`] gives it, with an error at each of the script's own
/// tables of files, since their URIs are relative to the root of a project, and no URI resolves
/// against the working directory of the process. The error takes the place of the analysis's
/// error that the catalog reads no files, and the other errors of the analysis stay.
fn session_analysis(text: &str, root: &SyntaxNode, catalog: &dyn Catalog) -> Result<Analysis> {
    let mut analysis = analyze(root, catalog, Library::shared(), &Options::default());
    let outside = outside_project(root);
    if !outside.is_empty() {
        analysis.diagnostics.retain(|diagnostic| {
            diagnostic.code != Some(DiagnosticCode::NoFiles)
                || !(outside.iter()).any(|outside| outside.range.contains_range(diagnostic.range))
        });
        analysis.diagnostics.extend(outside);
        analysis
            .diagnostics
            .sort_by_key(|diagnostic| diagnostic.range.start());
    }
    match analysis.diagnostics.is_empty() {
        true => Ok(analysis),
        false => Err(script_error(text, analysis.diagnostics)),
    }
}

/// The errors of the script whose tree is `root` at its own tables of files: one at each `CREATE
/// TEMP EXTERNAL TABLE` and `LOAD DATA ... TEMP TABLE`, since a project runs such a chart. These
/// statements without `TEMP` are errors of the parse.
fn outside_project(root: &SyntaxNode) -> Vec<Diagnostic> {
    (root.children())
        .filter(|node| {
            let external = ast::CreateExternalTableStmt::cast(node.clone())
                .is_some_and(|create| create.scope_kind() == Some(Scope::Temp));
            external
                || ast::LoadDataStmt::cast(node.clone()).is_some_and(|load| load.scope().is_some())
        })
        .map(|node| Diagnostic {
            range: node.text_range(),
            message: "A chart's own table of files reads URIs relative to the root of a project, so a project runs the chart, as avenger-sql-project's loader does".to_string(),
            code: None,
        })
        .collect()
}
