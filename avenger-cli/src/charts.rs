//! `avenger charts`: the chart scripts of a project and the `{avenger-sql}` cells with values of
//! its Quarto documents, by the names that `render` and `view` take, each with the columns of charts of its
//! result. The queries plan and do not run, so the columns come from each query's schema, and a
//! query that does not plan lists its error.

use std::path::Path;

use avenger_chart::results::is_chart_field;
use avenger_sql_project::{ChartError, Loader};
use serde_json::json;

use crate::{
    project::open,
    render::{cell_aim, Pick},
    report::Output,
    Status,
};

/// One chart script or cell, as the command lists it.
struct Listed {
    /// The name that `render` and `view` take, relative to the current directory: a cell by its
    /// label, or by its number without one.
    target: String,
    /// The cell's number, from 1, and its label.
    cell: Option<(usize, Option<String>)>,
    /// The names of the columns of charts, or the error of the query's plan.
    charts: Result<Vec<String>, String>,
}

/// Lists the charts of the project of `project`, or of the current directory.
pub async fn run(project: Option<&Path>, output: &Output) -> Result<Status, String> {
    let opened = open(project).await?;
    let loader = &opened.loader;
    let mut listed = Vec::new();
    for path in opened.files.chart_scripts() {
        if let Some(document) = opened.analysis.project().document(path) {
            listed.push(Listed {
                target: output.display(&opened.root.join(path)),
                cell: None,
                charts: chart_columns(loader, path, document.text()).await,
            });
        }
    }
    for (document, qmd) in opened.analysis.documents() {
        let file = output.display(&opened.root.join(document));
        let statements = crate::cells::statements(qmd);
        for (index, cell) in qmd.cells.iter().enumerate() {
            // A cell without a value, which only defines what later cells read, lists nothing.
            if crate::cells::value(&statements, index).is_none() {
                continue;
            }
            let pick = Pick::Name((index + 1).to_string());
            let charts = match cell_aim(qmd, document, &pick) {
                Ok(aim) => chart_columns(loader, document, &aim.masked).await,
                Err(error) => Err(error),
            };
            let name = (cell.options.label.clone()).unwrap_or_else(|| (index + 1).to_string());
            listed.push(Listed {
                target: format!("{file}#{name}"),
                cell: Some((index + 1, cell.options.label.clone())),
                charts,
            });
        }
    }
    let failed = listed.iter().any(|listed| listed.charts.is_err());
    if output.json() {
        let charts: Vec<serde_json::Value> = (listed.iter())
            .map(|listed| {
                let (cell, label) = match &listed.cell {
                    Some((number, label)) => (json!(number), json!(label)),
                    None => (json!(null), json!(null)),
                };
                json!({
                    "target": listed.target,
                    "cell": cell,
                    "label": label,
                    "charts": listed.charts.as_ref().ok(),
                    "error": listed.charts.as_ref().err(),
                })
            })
            .collect();
        let mut fields = serde_json::Map::new();
        fields.insert("charts".into(), json!(charts));
        output.object(fields);
    } else {
        for listed in &listed {
            match &listed.charts {
                Ok(charts) if charts.is_empty() => println!("{}: no charts", listed.target),
                Ok(charts) => println!("{}: {}", listed.target, charts.join(", ")),
                Err(error) => println!("{}: error: {error}", listed.target),
            }
        }
        if listed.is_empty() {
            eprintln!("the project has no chart scripts and no `{{avenger-sql}}` cells");
        }
    }
    Ok(match failed {
        true => Status::Failed,
        false => Status::Ok,
    })
}

/// The names of the columns of charts of the chart script at `path` with `text`, from its
/// frame's schema, unrun, or the first line of the error that keeps it from planning.
async fn chart_columns(loader: &Loader, path: &str, text: &str) -> Result<Vec<String>, String> {
    match loader.run_chart(path, text).await {
        Ok(query) => Ok((query.schema().fields().iter())
            .filter(|field| is_chart_field(field))
            .map(|field| field.name().clone())
            .collect()),
        Err(ChartError::Run(error)) | Err(ChartError::Unloaded { error, .. }) => {
            Err(first_line(&error.to_string()))
        }
    }
}

/// The first line of `text`.
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_string()
}
