# Avenger Chart Documentation System

This memory describes the documentation infrastructure for avenger-chart, including how to write new pages, build the documentation site, and debug build failures.

## Overview

The documentation uses **mdBook** with a custom preprocessor (`mdbook-avenger`) that automatically renders Rust chart code examples to PNG images. The system compiles chart code at build time, renders images, and injects them into the markdown.

## Directory Structure

```
avenger-chart/
├── book/                          # Documentation source
│   ├── book.toml                  # mdBook configuration
│   ├── src/                       # Markdown source files
│   │   ├── SUMMARY.md            # Table of contents
│   │   ├── introduction.md
│   │   ├── docs/                  # Main documentation
│   │   ├── getting-started/       # Getting started guides
│   │   └── .generated/images/    # Generated chart images (gitignored)
│   ├── book/                      # Built output (gitignored)
│   ├── custom.css                 # Custom styling
│   ├── build.sh                   # Fast build (text changes only)
│   ├── rebuild.sh                 # Full rebuild (new examples)
│   └── BUILD.md                   # Build documentation
└── avenger-chart-mdbook/          # Custom preprocessor crate
    ├── Cargo.toml
    ├── build.rs                   # Scans markdown for render blocks
    └── src/
        ├── lib.rs
        └── bin/
            ├── mdbook-avenger.rs       # Main preprocessor
            └── mdbook-avenger-render.rs # Subprocess renderer
```

## Writing Documentation

### Adding a New Page

1. Create a markdown file in `book/src/` (e.g., `book/src/docs/my-topic.md`)
2. Add the page to `book/src/SUMMARY.md`:
   ```markdown
   - [My Topic](./docs/my-topic.md)
   ```
3. Write content with optional rendered chart examples

### Writing Rendered Chart Examples

Use the `rust,render` fence info to create examples that render to images:

```markdown
    ```rust,render
    use avenger_chart::prelude::*;
    use datafusion::prelude::*;

    let ctx = SessionContext::new();
    let df = ctx.read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default()).await?;

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(Symbol::new().x(col("sepal_length")).y(col("sepal_width")));

    let compiled = plot.compile(&ctx).await?;
    let evaluated = compiled.evaluate(&ctx, None).await?;
    Ok(evaluated)
    ```
```

**Key requirements:**
- Must return `Ok(evaluated)` where `evaluated` is an `EvaluatedPlot`
- All imports must be explicit or hidden with `# `
- Use `?` for error handling (async closure wraps the code)
- Available imports: `avenger_chart::prelude::*`, `datafusion::prelude::*`, `avenger_sample_data`, `indexmap::IndexMap`, `datafusion::common::ScalarValue`

### Hidden Code (Boilerplate)

Use `# ` prefix to hide lines in rendered output (user can click eye icon to reveal):

```markdown
    ```rust,render
    use avenger_chart::prelude::*;
    use datafusion::prelude::*;

    # // This comment is hidden
    let ctx = SessionContext::new();
    # let df = ctx.read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default()).await?;

    // Plot code here...
    Ok(evaluated)
    ```
```

### Multiple Images from One Example

Return a tuple to generate multiple images:

```markdown
    ```rust,render
    // ... setup code ...

    let compiled = plot.compile(&ctx).await?;

    let result1 = compiled.evaluate(&ctx, None).await?;
    let result2 = compiled.evaluate(&ctx, Some(params)).await?;

    Ok((result1, result2))  // Generates two images
    ```
```

### Non-Rendered Code Examples

Use standard mdBook fences for code that shouldn't be rendered:
- `rust,no_run` - Compiles but doesn't execute
- `rust,ignore` - Doesn't compile (useful for pseudocode)
- `rust` - Regular Rust code block

## Building Documentation

### Quick Reference

| Command | When to Use | Speed |
|---------|-------------|-------|
| `bash build.sh` | Text-only markdown changes | ⚡ ~5-10 sec |
| `bash rebuild.sh` | Added new `rust,render` examples | 🐢 ~1-2 min |
| `mdbook serve` | Live preview while editing | ⚡ Instant |

### Build Commands

```bash
cd avenger-chart/book

# Fast build (text changes only)
bash build.sh

# Full rebuild (new render examples)
bash rebuild.sh

# Live preview server
mdbook serve --port 3000
```

### How the Build Works

1. **build.rs** (compile time):
   - Scans all `.md` files in `book/src/`
   - Finds `rust,render` code blocks
   - Generates `render_snippets.rs` with functions for each example
   - Each function wraps the code and calls `render_evaluated_plot_to_png()`

2. **mdbook-avenger** (preprocessor):
   - Runs when mdbook builds
   - For each render block, spawns `mdbook-avenger-render` subprocess
   - Subprocess executes the render function, produces PNG
   - Main preprocessor injects `![Rendered plot](.generated/images/...)` markdown

3. **Image naming**:
   - Single image: `{slug}.png` (e.g., `getting_started_first_plot_render00.png`)
   - Tuple images: `{slug}_{index:02}.png` (e.g., `docs_parameters_render00_00.png`, `docs_parameters_render00_01.png`)

## Debugging Build Failures

### New Example Not Rendering

**Symptom:** Added `rust,render` block but no image appears.

**Solution:** Use `rebuild.sh` instead of `build.sh`. The build script cache needs to be cleared to detect new render blocks.

```bash
cd avenger-chart/book
bash rebuild.sh
```

### Compilation Error in Render Block

**Symptom:** Build fails with Rust compilation error.

**Solution:** Check the error message carefully. Common issues:
- Missing imports (add `use` statements)
- Missing `?` on fallible operations
- Wrong return type (must be `Ok(evaluated)` or `Ok((r1, r2, ...))`)

**Debug tip:** Copy the code to a standalone test file and compile to see better error messages.

### Runtime Error (Render Fails)

**Symptom:** Build fails with "failed to render snippet X".

**Debug steps:**
1. Check the stderr in the error output
2. Common issues:
   - Data file not found (use `avenger_sample_data::*` paths)
   - Invalid column names
   - Scale type mismatch

### Images Not Updating

**Symptom:** Changed code but image looks the same.

**Solution:** 
1. Delete the old image: `rm book/src/.generated/images/{slug}.png`
2. Run `bash rebuild.sh`

Or clear all generated images:
```bash
rm -rf book/src/.generated/images/
bash rebuild.sh
```

### stdout Capture

If your code uses `println!()`, the output is captured to a `.stdout` file and displayed as "**Output:**" below the code block. This is useful for demonstrating DataFrame output.

## Configuration

### book.toml

Key settings:
```toml
[preprocessor.avenger]
command = "../../target/release/mdbook-avenger"

[output.html]
additional-css = ["custom.css"]
```

### custom.css

Adds styling for rendered plot images:
```css
img[alt="Rendered plot"] {
    border: 1px solid #ddd;
    box-sizing: border-box;
}
```

## Adding to SUMMARY.md

The table of contents in `book/src/SUMMARY.md` uses mdBook's format:

```markdown
# Summary

[Introduction](./introduction.md)

# Getting Started

- [Installation](./getting-started/installation.md)
- [Your First Plot](./getting-started/first-plot.md)

# Documentation

- [Topic](./docs/topic.md)
  - [Subtopic](./docs/topic/subtopic.md)
```

## Tips for Writing Good Documentation

1. **Start simple**: Show basic usage first, then add complexity
2. **Use realistic data**: Use `avenger_sample_data` datasets (iris, cars, movies, etc.)
3. **Progressive examples**: Each example should build on the previous
4. **Hide boilerplate**: Use `# ` to hide repetitive setup code
5. **Test your examples**: Run `bash rebuild.sh` to verify all examples compile and render
6. **Link related topics**: Cross-reference other documentation pages

## Render System Internals

The render system uses:
- `avenger_chart::doc::render::render_evaluated_plot_to_png()` - Renders at 4x scale
- `avenger_wgpu::canvas::PngCanvas` - GPU-accelerated rendering
- Subprocess isolation to capture stdout/stderr separately from mdbook JSON output

Generated code for each render block looks like:
```rust
fn render_{slug}(output: &Path) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let runtime = Runtime::new()?;
    runtime.block_on(async {
        let evaluated = (async move || -> Result<_, Box<dyn Error + Send + Sync + 'static>> {
            // User's code here
        })().await?;
        render_evaluated_plot_to_png(&evaluated, output.with_extension("png")).await
    })
}
```
