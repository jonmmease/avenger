# External Implementation Tests

This crate demonstrates how external crates can extend avenger-chart with custom implementations of:
- **Scales**: Custom data transformations and scale types
- **Marks**: Custom visualization marks
- **Coordinate Systems**: Custom coordinate projections and transformations

## Structure

This test crate is organized into three modules, each demonstrating a different extension point:

### 1. Custom Scales (`src/external_scale.rs`)
- Implements `SmoothLogScale`: A logarithmic scale with configurable smoothing
- Defines `SmoothLog` marker type implementing `ScaleSpec`
- Provides `SmoothLogExt` extension trait for typed methods
- Shows how to work around Rust's orphan rule for inherent impls

### 2. Custom Marks (`src/external_mark.rs`)
- Implements `HexBin`: A custom hexagonal binning mark
- Uses avenger-chart's mark macros for boilerplate
- Demonstrates channel definitions and mark rendering

### 3. Custom Coordinate Systems (`src/external_coord_system.rs`)
- Implements `Isometric`: A 3D isometric projection coordinate system
- Shows custom axis implementation
- Demonstrates coordinate transformation and layout

## Key Discoveries

### Rust's Orphan Rule for Inherent Impls
We discovered that you cannot add inherent methods to `Scale<YourType>` even when `YourType` is local:
```rust
// ❌ This does NOT work in external crates:
impl Scale<SmoothLog> {  // Error E0116
    pub fn smoothing(self, value: f32) -> Self { ... }
}

// ✅ Use an extension trait instead:
pub trait SmoothLogExt {
    fn smoothing(self, value: f32) -> Self;
}
impl SmoothLogExt for Scale<SmoothLog> { ... }
```

## Tests

Each module has corresponding integration tests in the `tests/` directory:
- `tests/test_custom_scale.rs` - Tests scale implementation and usage
- `tests/test_custom_mark.rs` - Tests mark implementation  
- `tests/test_custom_coord_system.rs` - Tests coordinate system implementation

Run all tests with:
```bash
cargo test
```

## Usage Examples

### Custom Scale
```rust
use avenger_chart_external_test::external_scale::{SmoothLog, SmoothLogExt};

let scale = Scale::<SmoothLog>::new()
    .smoothing(0.5)  // Extension trait method
    .domain((0.1, 100.0));
```

### Custom Mark
```rust
use avenger_chart_external_test::external_mark::HexBin;

let plot = Plot::new(Cartesian)
    .mark(HexBin::new().x("value").y("count").fill("category"));
```

### Custom Coordinate System
```rust
use avenger_chart_external_test::external_coord_system::Isometric;

let plot = Plot::new(Isometric::new())
    .mark(Symbol3D::new().x("x").y("y").z("z"));
```

This crate demonstrates that avenger-chart's extension points are fully functional and can be used by external crates to add custom functionality
