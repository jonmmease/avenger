//! Compile and evaluate one real Avenger chart-language source file.

use std::path::PathBuf;

use avenger_lang::Compiler;
use datafusion::prelude::SessionContext;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: compile_file <chart.avenger>")?;
    let absolute = std::fs::canonicalize(&path)?;
    let project_root = absolute
        .parent()
        .ok_or("chart source has no parent directory")?;
    let compiler = Compiler::builder().project_root(project_root).build()?;
    let artifact = compiler.compile_file(&absolute).await?;
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&SessionContext::new(), None)
        .await?;
    println!(
        "compiled {} with registry profile {} into a {}x{} scene with {} root groups",
        artifact.id.as_str(),
        artifact.native_registry_profile.as_str(),
        evaluated.scene_graph.width,
        evaluated.scene_graph.height,
        evaluated.scene_graph.groups().len(),
    );
    println!("interface: {:#?}", artifact.interface);
    Ok(())
}
