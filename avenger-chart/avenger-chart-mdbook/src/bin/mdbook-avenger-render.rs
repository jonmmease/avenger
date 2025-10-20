//! Subprocess binary for rendering individual code snippets.
//!
//! This binary is invoked by the main mdbook-avenger preprocessor to render
//! individual code snippets in isolation. This allows stdout from user code
//! (like println!) to be captured separately from the preprocessor's JSON output.

use anyhow::{anyhow, Result};
use avenger_chart_mdbook::render_snippets::RENDER_ENTRIES;
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        return Err(anyhow!("Usage: mdbook-avenger-render <slug> <output_path>"));
    }

    let slug = &args[1];
    let output_path = &args[2];

    // Find the render entry for this slug
    let entry = RENDER_ENTRIES
        .iter()
        .find(|e| e.slug == slug)
        .ok_or_else(|| anyhow!("No render entry found for slug: {}", slug))?;

    // Call the render function
    // Any println!/eprintln! from the user's code will go to this process's stdout/stderr,
    // which the parent process will capture
    (entry.render)(Path::new(output_path))
        .map_err(|e| anyhow!("Failed to render snippet {}: {}", slug, e))?;

    Ok(())
}
