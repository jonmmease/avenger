# Code Style & Conventions

## Rust Style Guidelines

### Formatting
- **Tool**: `rustfmt` (standard Rust formatter)
- **Command**: `cargo fmt --all`
- **CI Check**: `cargo fmt --all -- --check`
- No custom rustfmt.toml configuration (uses Rust defaults)

### Standard Rust Conventions
Following standard Rust style guide:
- Snake case for functions, variables, modules: `my_function`, `data_value`
- Pascal case for types, structs, enums: `SceneGraph`, `ScaleDomain`
- SCREAMING_SNAKE_CASE for constants: `MAX_BUFFER_SIZE`
- Module names are lowercase with underscores

### Linting
- **Tool**: `clippy` (Rust linter)
- **Standard check**: `cargo clippy --all-targets`
- **CI standard**: `RUSTFLAGS="-D warnings" cargo clippy`
- All clippy warnings must be resolved before merging
- No custom .clippy.toml configuration

### Error Handling
- Use `thiserror` crate for error types (workspace dependency)
- Propagate errors with `?` operator
- Return `Result<T, E>` for fallible operations

### Code Organization
- Each crate has clear separation of concerns
- Public API in `lib.rs` or exposed modules
- Internal implementation details in private modules
- Tests in `tests/` directory or inline with `#[cfg(test)]`

## Documentation

### Code Documentation
- Document public APIs with `///` doc comments
- Include examples in doc comments where helpful
- Run doc tests: `cargo test --doc`

### Module Documentation
- Use `//!` for module-level documentation
- Explain module purpose and key concepts

## Testing Conventions

### Test Organization
- Unit tests: inline with `#[cfg(test)]` mod tests
- Integration tests: in `tests/` directory
- Visual regression tests: in avenger-wgpu/tests/test_image_baselines.rs
- Test naming: descriptive, prefixed with `test_`

### Test Output
- Use `-- --nocapture` to see println!/debug output
- Visual tests compare against baseline images
- Use debugging tools for layout/rendering issues

## Dependencies

### Adding Dependencies
- Add to `[workspace.dependencies]` in root Cargo.toml
- Reference in individual crate Cargo.toml with `workspace = true`
- Keep versions consistent across workspace

### Dependency Guidelines
- Prefer workspace dependencies
- Use crates.io versions (avoid git dependencies except when necessary, e.g., rstar)
- Keep dependencies minimal and well-maintained

## Type System Usage

### Traits
- Implement standard traits where appropriate: Debug, Clone, PartialEq, etc.
- Use derive macros from serde for serialization
- Custom trait implementations for extensibility points

### Generics
- Use generics for flexible, reusable code
- Common pattern: `SceneGraphBuilder<State>`
- Keep trait bounds clear and minimal

### Ownership
- Prefer owned types over borrowed when possible in public APIs
- Use `&str` for string parameters, `String` for owned strings
- Recent refactoring eliminated interior mutability (Arc<Mutex>)

## Performance Considerations

### Optimization Practices
- Profile before optimizing
- Use release builds for performance testing
- Instanced rendering for repeated marks
- Spatial indexing for hit testing
- Pre-tessellation of geometry

### Memory Management
- Avoid unnecessary allocations
- Use `Vec::with_capacity` when size is known
- Pool resources where appropriate (GPU buffers, etc.)

## Git Practices

### Commit Messages
- Descriptive commit messages
- Reference issue numbers when applicable
- Recent example: "Fix non-deterministic facet rendering by sorting domain values"

### Branch Names
- Descriptive branch names
- Format: `username/feature-description` or `fix/issue-description`
- Example: `jonmmease/avenger-chart3`

## CI/CD Expectations

### Before Merging
All CI checks must pass:
1. `cargo fmt --all -- --check` (formatting)
2. `cargo check --tests` (no compiler warnings)
3. `cargo clippy` (no linter warnings)
4. `cargo build --workspace` (successful build)
5. `cargo test --workspace --exclude avenger-wgpu` (tests pass)
6. `cargo test --doc` (doc tests pass)

Note: GPU tests (avenger-wgpu) are excluded from CI due to MakeWgpuAdapterError on Linux