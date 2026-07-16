//! Temporary Phase 0 vertical slice.
//!
//! Phase 5 replaces this programmatic input with `Compiler::compile_file` on a
//! real `.avenger` source while preserving the returned artifact contract.

use avenger_lang::Compiler;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let compiler = Compiler::builder().project_root(".").build()?;
    let artifact = compiler.compile_phase0_example().await?;
    println!(
        "compiled {} with registry profile {}",
        artifact.id.as_str(),
        artifact.native_registry_profile.as_str()
    );
    Ok(())
}
