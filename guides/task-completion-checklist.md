# Task Completion Checklist

When completing a coding task in Avenger, follow this checklist:

## 1. Code Quality Checks (Required)

### Format Code
```bash
cargo fmt --all
```
- Ensures consistent formatting across the codebase
- Must pass before committing

### Check for Compiler Warnings
```bash
RUSTFLAGS="-D warnings" cargo check --release --tests
```
- Treats warnings as errors (CI standard)
- Must have zero warnings

### Run Clippy Lints
```bash
RUSTFLAGS="-D warnings" cargo clippy --release --all-targets
```
- Catches common mistakes and non-idiomatic code
- Must pass with zero warnings

### Alternative: Individual Checks
```bash
# 1. Format
cargo fmt --all

# 2. Check compilation
cargo check --release --tests

# 3. Run clippy
cargo clippy --release --all-targets
```

## 2. Testing (Required)

### Run Relevant Tests
```bash
# For most changes
cargo test --release

# For specific crate
cargo test --release -p avenger-scenegraph

# For workspace (excluding GPU tests)
cargo test --release --workspace --exclude avenger-wgpu

# With output for debugging
cargo test --release -- --nocapture
```

### Run Doc Tests
```bash
cargo test --release --doc
```

### Visual Regression Tests (if applicable)
```bash
# For avenger-chart changes
cargo test --release -p avenger-chart --test visual_regression

# With debugging if needed
AVENGER_CHART_DEBUG_LAYOUT=1 RUST_LOG=avenger_chart=debug cargo test --release -p avenger-chart -- --nocapture
```

## 3. Build Verification

```bash
# Verify workspace builds
cargo build --release --workspace

# Verify release build works
cargo build --release
```

## 4. Documentation Updates (if applicable)

- Update README.md if adding new features
- Update doc comments for public APIs
- Update CLAUDE.md if changing development workflows
- Update debugging docs if adding new debug capabilities

## 5. Manual Testing (if applicable)

### For New Features
- Run relevant examples to verify functionality
- Test both native and WASM builds if cross-platform

### For Bug Fixes
- Verify the bug is fixed
- Add regression test if possible

### Examples
```bash
# Run example
cd examples/iris-pan-zoom
cargo run --release

# Build WASM example
cd examples/iris-pan-zoom
wasm-pack build --target web --release
python3 -m http.server 8765
# Open http://localhost:8765/ in browser
```

## 6. Git Hygiene

### Before Committing
```bash
# Check git status
git status

# Review changes
git diff

# Stage files
git add <files>

# Commit with descriptive message
git commit -m "Descriptive message about changes"
```

### Commit Message Guidelines
- Clear, descriptive messages
- Reference issue numbers if applicable
- Example: "Fix non-deterministic facet rendering by sorting domain values"

## Quick Checklist Summary

Before marking a task complete, ensure:
- [ ] `cargo fmt --all` - Code is formatted
- [ ] `RUSTFLAGS="-D warnings" cargo check --release --tests` - No warnings
- [ ] `RUSTFLAGS="-D warnings" cargo clippy --release --all-targets` - Clippy passes
- [ ] `cargo test --release` - Tests pass (or specific tests for your changes)
- [ ] `cargo build --release --workspace` - Workspace builds successfully
- [ ] Documentation updated (if needed)
- [ ] Manual testing performed (if applicable)
- [ ] Changes committed with clear message

## Notes

### GPU Tests
GPU tests in avenger-wgpu are excluded from CI but should be run locally on macOS:
```bash
cd avenger-wgpu
cargo test --release -- --nocapture
```

### Performance Testing
For performance-sensitive changes:
```bash
cargo build --release
# Run performance tests or benchmarks
```

### Python Bindings
If changes affect Python bindings:
```bash
pixi run dev-py
# Test Python functionality
```
