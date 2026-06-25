# Suggested Commands

## Build Commands

### Basic Building
```bash
# Build entire workspace
cargo build --release

# Build specific crate
cd avenger-scenegraph && cargo build --release

# Build for WASM
cd examples/iris-pan-zoom
wasm-pack build --target web --release
```

## Code Quality Commands

### Formatting
```bash
# Format all code (workspace-wide)
cargo fmt --all

# Check formatting without modifying
cargo fmt --all -- --check

# Via pixi
pixi run fmt-rs
```

### Linting
```bash
# Run clippy on all targets
cargo clippy --release --all-targets

# Via pixi
pixi run clippy

# Strict mode (used in CI)
RUSTFLAGS="-D warnings" cargo clippy --release
RUSTFLAGS="-D warnings" cargo clippy --release --all-targets
```

### Type Checking
```bash
# Check code compilation without building
cargo check --release
cargo check --release --tests

# Strict mode (warnings as errors, used in CI)
RUSTFLAGS="-D warnings" cargo check --release
RUSTFLAGS="-D warnings" cargo check --release --tests
```

## Testing Commands

### Basic Testing
```bash
# Run all tests
cargo test --release
cargo test --release --workspace

# Run tests with output
cargo test --release -- --nocapture

# Run specific crate tests
cd avenger-wgpu && cargo test --release

# Run specific test
cargo test --release test_name -- --nocapture

# Run doc tests
cargo test --release --doc
```

### Visual Regression Tests
```bash
# Run visual regression tests (avenger-chart)
cargo test --release -p avenger-chart --test visual_regression

# Run specific visual test
cargo test --release -p avenger-chart --test visual_regression test_name -- --nocapture

# With visual debug layout
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test --release -p avenger-chart --test visual_regression test_name -- --nocapture

# With logging
RUST_LOG=avenger_chart=debug cargo test --release -- --nocapture

# Combined debugging (recommended)
AVENGER_CHART_DEBUG_LAYOUT=1 RUST_LOG=avenger_chart::layout=debug cargo test --release -- --nocapture
```

### Debugging Tests
```bash
# Enable layout debug visualization (magenta rectangles)
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test --release

# Enable debug logging
RUST_LOG=avenger_chart=debug cargo test --release -- --nocapture

# Module-specific logging
RUST_LOG=avenger_chart::layout=trace,avenger_chart::legend=debug cargo test --release -- --nocapture

# With backtrace
RUST_BACKTRACE=1 cargo test --release
RUST_BACKTRACE=full cargo test --release
```

## Running Examples

### Native Examples
```bash
# Run iris pan-zoom example
cd examples/iris-pan-zoom
cargo run --release

# Run other examples
cd examples/wgpu-winit
cargo run --release
```

### WASM Examples
```bash
# Build and open in browser
cd examples/iris-pan-zoom
wasm-pack build --target web --release
# Then open index.html in browser
```

## Python Development

### Building Python Bindings
```bash
# Develop Python package (editable install)
pixi run dev-py

# Build Python package
pixi run build-py
```

## Version Management

```bash
# Bump version numbers
pixi run bump-version
```

## Publishing (Maintainers Only)

```bash
# Publish Rust crates (in dependency order)
pixi run publish-rs
```

## Utility Commands

### Darwin-specific (macOS)
Standard Unix commands work on macOS:
- `ls`, `cd`, `grep`, `find`, `git`, etc.
- Use `rg` (ripgrep) for faster searching
- Use `sg` (ast-grep) for structural code searches

### Code Search
```bash
# Text search with ripgrep
rg "pattern"

# Structural search with ast-grep
sg --lang rust -p 'pattern'
```
