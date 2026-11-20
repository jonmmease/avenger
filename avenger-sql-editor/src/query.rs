//! The functions that avenger-sql adds, and the outcome of a chart script's query: its result and
//! plan, or its error with the range of the text that the error points to, and its lifted form.

use std::ops::Range;

use datafusion::{common::DataFusionError, prelude::DataFrame};

use crate::highlight::Chars;
use crate::result::QueryResult;

/// A function that avenger-sql adds, with its documentation.
#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub doc: String,
}

/// The functions that avenger-sql adds, in the groups of the data sidebar.
#[derive(Clone, Debug, Default)]
pub struct Functions {
    pub marks: Vec<Function>,
    pub compositions: Vec<Function>,
    pub constructors: Vec<Function>,
    pub transforms: Vec<Function>,
}

/// The functions that avenger-sql adds to GoogleSQL's, by their families in its library: the marks,
/// the compositions, and the constructors of avenger-chart, and the transforms.
pub fn functions() -> Functions {
    use avenger_sql::library::{Family, Library};
    let library = Library::new();
    let of = |family: Family| -> Vec<Function> {
        let mut functions: Vec<Function> = (library.functions())
            .filter(|function| function.family == family)
            .map(|function| Function {
                name: function.name.clone(),
                doc: function.summary.clone(),
            })
            .collect();
        functions.sort_by(|a, b| a.name.cmp(&b.name));
        functions
    };
    Functions {
        marks: of(Family::Mark),
        compositions: of(Family::Composition),
        constructors: of(Family::Constructor),
        transforms: of(Family::Transform),
    }
}

/// The outcome of a query that ran: its result, and its optimized logical plan.
pub struct Outcome {
    pub result: QueryResult,
    pub plan: String,
}

/// The error of a query, and the range of characters of the text that it points to, when it
/// points to one.
#[derive(Clone, Debug)]
pub struct QueryError {
    pub message: String,
    pub range: Option<Range<usize>>,
}

impl QueryError {
    /// The error `error` of the query `text`.
    pub fn new(error: &DataFusionError, text: &str) -> Self {
        // A script's errors read without DataFusion's prefix of an external error.
        let message = match error.find_root() {
            DataFusionError::External(external) => external.to_string(),
            error => error.to_string(),
        };
        Self {
            message,
            range: error_range(error, text),
        }
    }
}

/// The outcome of `frame`, the planned query `text`: its optimized plan, and its collected
/// result.
pub async fn outcome(frame: DataFrame, text: &str) -> Result<Outcome, QueryError> {
    let error = |error: DataFusionError| QueryError::new(&error, text);
    let plan = frame
        .clone()
        .into_optimized_plan()
        .map_err(error)?
        .display_indent()
        .to_string();
    let schema = frame.schema().inner().clone();
    let batches = frame.collect().await.map_err(error)?;
    let result = QueryResult::new(schema, batches).map_err(|chart| QueryError {
        message: chart.to_string(),
        range: None,
    })?;
    Ok(Outcome { result, plan })
}

/// The lifted form of a query ([`avenger_sql::lift`]): its text, and the ranges of characters of
/// the text that avenger-sql wrote.
#[derive(Clone, Debug)]
pub struct LiftedText {
    pub text: String,
    pub generated: Vec<Range<usize>>,
}

/// The query `text` with each mark call that computes its statistics written as the stages of its
/// lifting, a query that the user could type, from `lifted`, the lifted form of `text` or the
/// error of its rewrites.
pub fn lifted(
    lifted: Result<avenger_sql::analysis::Lifted, DataFusionError>,
    text: &str,
) -> Result<LiftedText, QueryError> {
    let lifted = lifted.map_err(|error| QueryError::new(&error, text))?;
    let chars = |byte: usize| lifted.text[..byte].chars().count();
    let generated = (lifted.generated.iter())
        .map(|range| chars(range.start().into())..chars(range.end().into()))
        .collect();
    Ok(LiftedText {
        text: lifted.text,
        generated,
    })
}

/// The range of characters that `error` points to: the range of the first diagnostic of a
/// script's error.
fn error_range(error: &DataFusionError, text: &str) -> Option<Range<usize>> {
    let DataFusionError::External(external) = error else {
        return None;
    };
    let script = external.downcast_ref::<avenger_sql::ScriptError>()?;
    let diagnostic = script.diagnostics.first()?;
    Some(Chars::new(text).range(diagnostic.range))
}
