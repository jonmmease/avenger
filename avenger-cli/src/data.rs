//! `avenger tables` and `avenger schema`: the tables and the views that a project's data scripts
//! register. `tables` lists their names and their data scripts, and `schema` the columns of the
//! tables that it names, or of every table, with their types as GoogleSQL names and their row
//! counts, which run a view's query. The data scripts load as `check` loads them, and the errors
//! of a data script that did not load go to standard error, or with `--json`, to the result's
//! `diagnostics`.

use std::path::Path;

use avenger_sql::types::googlesql_name;
use avenger_sql_project::{Status as LoadStatus, Table};
use datafusion::arrow::datatypes::DataType;
use serde_json::json;

use crate::{
    project::{open, Opened},
    report::Output,
    Status,
};

/// Lists the names of the tables of the project of `project`, or of the current directory, with
/// their data scripts.
pub async fn tables(project: Option<&Path>, output: &Output) -> Result<Status, String> {
    let opened = open(project).await?;
    let errors = opened.load_errors().await;
    let status = match errors.is_empty() {
        true => Status::Ok,
        false => Status::Failed,
    };
    let tables: Vec<(String, &Table, String)> = (opened.tables())
        .map(|(script, table)| {
            let script = output.display(&opened.root.join(script));
            (table.names.join("."), table, script)
        })
        .collect();
    if output.json() {
        let tables: Vec<serde_json::Value> = (tables.iter())
            .map(|(name, table, script)| json!({"name": name, "path": table.names, "script": script}))
            .collect();
        let mut fields = serde_json::Map::new();
        fields.insert("tables".into(), json!(tables));
        fields.insert(
            "diagnostics".into(),
            json!(output.diagnostic_values(errors)),
        );
        output.object(fields);
        return Ok(status);
    }
    let width = (tables.iter())
        .map(|(name, _, _)| name.chars().count())
        .max()
        .unwrap_or(0);
    for (name, _, script) in &tables {
        let pad = " ".repeat(width - name.chars().count());
        println!("{name}{pad}  {script}");
    }
    if tables.is_empty() {
        eprintln!("the project's data scripts register no tables");
    }
    output.write_diagnostics(&mut std::io::stderr().lock(), errors);
    Ok(status)
}

/// Prints the columns and the row counts of the tables `names` of the project of `project`, or
/// of the current directory, or of every table without one.
pub async fn schema(
    names: &[String],
    project: Option<&Path>,
    output: &Output,
) -> Result<Status, String> {
    let opened = open(project).await?;
    let (tables, errors) = match names.is_empty() {
        // Every table, with the errors of the data scripts that did not load, whose tables are
        // missing.
        true => (
            opened.tables().collect::<Vec<_>>(),
            opened.load_errors().await,
        ),
        false => {
            let mut tables = Vec::new();
            for name in names {
                match opened.table(name).await {
                    Ok(found) => tables.push(found),
                    Err(error) => return Err(missing(&opened, output, error)),
                }
            }
            (tables, Vec::new())
        }
    };
    let status = match errors.is_empty() {
        true => Status::Ok,
        false => Status::Failed,
    };
    let mut counted = Vec::new();
    for (script, table) in tables {
        let rows = opened.loader.row_count(&table.names).await;
        counted.push((script, table, rows.map_err(|error| error.to_string())));
    }
    if output.json() {
        let tables: Vec<serde_json::Value> = (counted.iter())
            .map(|(script, table, rows)| {
                let mut value = table_value(&opened, output, script, table);
                value["rows"] = rows.as_ref().map_or(json!(null), |rows| json!(rows));
                value["error"] = rows.as_ref().err().map_or(json!(null), |e| json!(e));
                value
            })
            .collect();
        let mut fields = serde_json::Map::new();
        fields.insert("tables".into(), json!(tables));
        fields.insert(
            "diagnostics".into(),
            json!(output.diagnostic_values(errors)),
        );
        output.object(fields);
        return Ok(status);
    }
    for (index, (script, table, rows)) in counted.iter().enumerate() {
        if index > 0 {
            println!();
        }
        let rows = match rows {
            Ok(1) => "1 row".to_string(),
            Ok(rows) => format!("{rows} rows"),
            Err(error) => format!("rows: {error}"),
        };
        let script = output.display(&opened.root.join(script));
        println!("{} ({rows}, {script})", table.names.join("."));
        for line in column_lines(&table.columns) {
            println!("  {line}");
        }
    }
    if counted.is_empty() {
        eprintln!("the project's data scripts register no tables");
    }
    output.write_diagnostics(&mut std::io::stderr().lock(), errors);
    Ok(status)
}

/// The error of a table that the project lacks, `error`, with the data scripts that did not
/// load, whose tables are missing.
fn missing(opened: &Opened, output: &Output, error: String) -> String {
    let unloaded: Vec<String> = (opened.state.scripts.iter())
        .filter(|script| script.status != LoadStatus::Loaded)
        .map(|script| output.display(&opened.root.join(&script.path)))
        .collect();
    match unloaded.is_empty() {
        true => error,
        false => format!(
            "{error}; these data scripts did not load, so their tables are missing: {} (run \
             `avenger check`)",
            unloaded.join(", ")
        ),
    }
}

/// The JSON of `table`, of the data script `script`: its name, its path, its script, and its
/// columns.
fn table_value(opened: &Opened, output: &Output, script: &str, table: &Table) -> serde_json::Value {
    let columns: Vec<serde_json::Value> = (table.columns.iter())
        .map(|(name, data_type)| json!({"name": name, "type": googlesql_name(data_type)}))
        .collect();
    json!({
        "name": table.names.join("."),
        "path": table.names,
        "script": output.display(&opened.root.join(script)),
        "columns": columns,
    })
}

/// The lines of `columns`: each name, padded to the longest, and its type.
fn column_lines(columns: &[(String, DataType)]) -> Vec<String> {
    let width = (columns.iter()).map(|(name, _)| name.chars().count()).max();
    (columns.iter())
        .map(|(name, data_type)| {
            let pad = width.unwrap_or(0) - name.chars().count();
            format!("{name}{}  {}", " ".repeat(pad), googlesql_name(data_type))
        })
        .collect()
}
