# Crate Updates Task List

## Complex Updates Requiring Investigation

### 1. PyO3 and Pythonize Update (0.23 → 0.25)
**Challenge**: Major version bump with breaking changes
- pyo3: 0.23.2 → 0.25.1
- pythonize: 0.23.0 → 0.25.0
- These need to be updated together
- Check PyO3 migration guide: https://pyo3.rs/v0.25.1/migration
- May require changes to Python bindings code

### ~~3. Ordered Float Update (4.5.0 → 5.0.0)~~ ✓ Completed
**Challenge**: Major version bump, potential breaking changes in float ordering
- ~~Used for ordered floating point comparisons~~
- ~~Check if the ordering semantics have changed~~
- ~~May affect sorting and comparison operations~~
- **Update completed successfully**: The codebase only uses `OrderedFloat` for hashing f32/f64 values, not `NotNan`. The breaking changes (Hash only for f32/f64, NotNan operator changes) don't affect our usage.

### ~~4. Resvg/Usvg Update (0.44.0 → 0.45.1)~~ ✓ Completed
**Challenge**: Major version bump in SVG rendering libraries
- ~~resvg: 0.44.0 → 0.45.1~~
- ~~usvg: 0.44.0 → 0.45.1~~
- ~~These must be updated together~~
- ~~Check for API changes in SVG parsing and rendering~~
- **Update completed successfully**: No code changes required. The project's usage of resvg/usvg APIs remained compatible across the version update.

### ~~5. Strum Update (0.26 → 0.27)~~ ✓ Completed
**Challenge**: Major version bump in enum utilities
- ~~May have changes to derive macros~~
- ~~Check if enum string conversion APIs have changed~~
- **Update completed successfully**: No code changes required. The codebase only uses `VariantNames` trait, which was not affected by the breaking changes in strum 0.27.

## Inconsistencies to Fix

### ~~1. Pollster Version Mismatch~~ ✓ Fixed
- ~~Workspace specifies 0.3~~ Updated to 0.4.0
- ~~avenger-wgpu directly uses 0.4.0~~
- ~~Should align these versions~~

### ~~2. WASM32 wgpu Override~~ ✓ Fixed
- ~~The wasm32 target override in avenger-wgpu/Cargo.toml specifies wgpu 23.0.1~~ Updated to 25.0.2
- ~~Workspace uses 25.0.2~~
- ~~This needs to be updated to match~~

## Notes

- Arrow crates use "*" version which allows any version - consider pinning to specific versions for reproducibility
- rstar uses git dependency - consider if a released version is now available