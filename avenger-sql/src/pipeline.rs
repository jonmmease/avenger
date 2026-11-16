//! The pipeline of a script: [`sql`] parses it, loads the tables that it names from a session,
//! analyzes it, and plans its last query.

use std::{fmt, sync::OnceLock};

use avenger_sql_syntax::line_column;

use datafusion::{
    common::{DataFusionError, Result},
    dataframe::DataFrame,
    prelude::SessionContext,
};

use crate::{
    analysis::{analyze, Diagnostic, Lifted, Options},
    catalog::{references, SessionCatalog},
    library::Library,
};

/// The library of every script, which holds no state of a session.
fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(Library::new)
}

/// The errors of a script: the errors of its parse, or else of its analysis, each at a range of
/// the text.
#[derive(Clone, Debug)]
pub struct ScriptError {
    pub diagnostics: Vec<Diagnostic>,
    /// The text of the script, for the lines and columns of the messages.
    text: String,
}

impl ScriptError {
    fn new(text: &str, diagnostics: Vec<Diagnostic>) -> Self {
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
/// path without one.
pub async fn sql_with_modules(
    ctx: &SessionContext,
    text: &str,
    modules: &dyn Fn(&[String]) -> Option<String>,
) -> Result<DataFrame> {
    let parse = avenger_sql_syntax::parse(text);
    if !parse.ok() {
        let diagnostics = (parse.diagnostics().iter())
            .map(|diagnostic| Diagnostic {
                range: diagnostic.range,
                message: diagnostic.message.clone(),
            })
            .collect();
        return Err(script_error(text, diagnostics));
    }
    let root = parse.syntax();
    let references = references(&root);
    let catalog = SessionCatalog::load(ctx, &references)
        .await?
        .with_modules(&references, modules);
    let analysis = analyze(&root, &catalog, library(), &Options::default());
    if !analysis.diagnostics.is_empty() {
        return Err(script_error(text, analysis.diagnostics));
    }
    analysis.dataframe(ctx).await
}

/// The script `text` as the script that its lifted calls plan, as [`Analysis::lift`] prints it,
/// with the tables of `ctx`. A script with an error returns a [`ScriptError`] of every diagnostic.
///
/// [`Analysis::lift`]: crate::analysis::Analysis::lift
pub async fn lift(ctx: &SessionContext, text: &str) -> Result<Lifted> {
    let parse = avenger_sql_syntax::parse(text);
    if !parse.ok() {
        let diagnostics = (parse.diagnostics().iter())
            .map(|diagnostic| Diagnostic {
                range: diagnostic.range,
                message: diagnostic.message.clone(),
            })
            .collect();
        return Err(script_error(text, diagnostics));
    }
    let root = parse.syntax();
    let references = references(&root);
    let catalog = SessionCatalog::load(ctx, &references).await?;
    let analysis = analyze(&root, &catalog, library(), &Options::default());
    match analysis.lift(text) {
        Some(lifted) => Ok(lifted),
        None => Err(script_error(text, analysis.diagnostics)),
    }
}

fn script_error(text: &str, diagnostics: Vec<Diagnostic>) -> DataFusionError {
    DataFusionError::External(Box::new(ScriptError::new(text, diagnostics)))
}
