//! Inspect the public state interface of every chart in a project.

use std::path::PathBuf;

use avenger_lang::Compiler;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: inspect_interface <project-directory>")?;
    let root = std::fs::canonicalize(root)?;
    let compiler = Compiler::builder().project_root(&root).build()?;
    let project = compiler.compile_project(&root).await?;

    for chart in project.charts.values() {
        println!(
            "{}\n{}",
            chart.name.as_deref().unwrap_or(chart.id.as_str()),
            serde_json::to_string_pretty(&chart.interface)?,
        );
    }
    Ok(())
}
