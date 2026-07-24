//! Inspect the public state interface of every chart in a source module.

use std::path::PathBuf;

use avenger_lang::Compiler;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let module = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: inspect_interface <module.avenger>")?;
    let module = std::fs::canonicalize(module)?;
    let root = module.parent().ok_or("module has no parent directory")?;
    let compiler = Compiler::builder().project_root(root).build()?;
    let compiled = compiler.compile_module(&module).await?;

    for chart in compiled.charts.values() {
        let label = chart
            .name
            .clone()
            .unwrap_or_else(|| format!("{:?}", chart.id.selector));
        println!(
            "{}\n{}",
            label,
            serde_json::to_string_pretty(&chart.interface)?,
        );
    }
    Ok(())
}
