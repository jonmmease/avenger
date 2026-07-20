//! Compile every chart root in one project and print deterministic identities.

use std::path::PathBuf;

use avenger_lang::Compiler;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: compile_project <project-directory>")?;
    let root = std::fs::canonicalize(root)?;
    let compiler = Compiler::builder().project_root(&root).build()?;
    let project = compiler.compile_project(&root).await?;

    println!("project {}", project.project_fingerprint.as_str());
    for chart in project.charts.values() {
        println!(
            "{}\t{}\tparams={}\tstores={}\tselections={}",
            chart.name.as_deref().unwrap_or(chart.id.as_str()),
            chart.dependency_fingerprint.as_str(),
            chart.interface.params.len(),
            chart.interface.stores.len(),
            chart.interface.selections.len(),
        );
    }
    Ok(())
}
