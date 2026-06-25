# External Implementation Tests

This crate demonstrates how external crates can extend avenger-chart with custom implementations of:
- **Scales**: Custom data transformations and scale types
- **Marks**: Custom visualization marks
- **Compound marks**: External builders that lower to `MarkGroup` and primitives
- **Coordinate Systems**: Custom coordinate projections and transformations
- **Subplot-capable Coordinate Systems**: Coordinate systems that opt into compiling `Subplot` marks
- **Built-in Coordinate Crates**: Direct use of split coordinate crates such as
  `avenger-chart-polar`

## Structure

This test crate is organized into modules that demonstrate different extension points:

### 1. Custom Scales (`src/external_scale.rs`)
- Implements `SmoothLogScale`: A logarithmic scale with configurable smoothing
- Defines `SmoothLog` marker type implementing `ScaleSpec`
- Provides `SmoothLogExt` extension trait for typed methods
- Shows how to work around Rust's orphan rule for inherent impls

### 2. Custom Marks (`src/external_mark.rs`)
- Implements `HexBin`: A custom hexagonal binning mark
- Uses `avenger-chart-core` mark macros for boilerplate
- Demonstrates channel definitions and mark rendering

### 3. Compound Marks (`src/external_compound_mark.rs`)
- Implements `ExternalMeanPoint`: a tiny aggregate-backed compound mark
- Uses `MarkGroup`, `Aggregate`, primitive `Symbol`, and a public scale inference hint
- Demonstrates that compound marks can be authored without depending on `avenger-chart`

### 4. Custom Coordinate Systems (`src/external_coord_system.rs`)
- Implements `Isometric`: A 3D isometric projection coordinate system
- Shows custom axis implementation
- Demonstrates coordinate transformation and layout

### 5. Subplot-Capable Coordinate Systems (`src/external_subplot_coord.rs`)
- Implements `ExternalSubplotCoord`: a minimal coordinate system that compiles `Subplot<ExternalSubplotCoord>`
- Proves the narrow `SubplotContainerCoordinateSystem` hook works from another crate
- Does not expose facet, concat, or layout-container implementation as an external API

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
- `tests/test_external_compound_mark.rs` - Tests compound mark lowering through `MarkGroup`
- `tests/test_custom_coord_system.rs` - Tests coordinate system implementation
- `tests/test_external_subplot_coord.rs` - Tests subplot compilation for an external coordinate system
- `tests/test_builtin_coordinate_crates.rs` - Tests direct imports from built-in coordinate crates

Run all tests with:
```bash
cargo test --release
```

## Usage Examples

### Custom Scale
```rust
use avenger_chart_core::Scale;
use avenger_chart_external_test::external_scale::{SmoothLog, SmoothLogExt};

let scale = Scale::<SmoothLog>::new()
    .smoothing(0.5)  // Extension trait method
    .domain((0.1, 100.0));
```

### Custom Mark
```rust
use avenger_chart::plot::Plot;
use avenger_chart_cartesian::Cartesian;
use avenger_chart_external_test::external_mark::HexBin;

let plot = Plot::<Cartesian>::new()
    .mark(HexBin::new().x("value").y("count").fill("category"));
```

### Compound Mark
```rust
use avenger_chart::plot::Plot;
use avenger_chart_cartesian::Cartesian;
use avenger_chart_external_test::external_compound_mark::ExternalMeanPoint;
use datafusion::prelude::col;

let plot = Plot::<Cartesian>::new()
    .mark(ExternalMeanPoint::new(col("category"), col("value")));
```

### Custom Coordinate System
```rust
use avenger_chart::plot::Plot;
use avenger_chart_external_test::external_coord_system::Isometric;

let plot = Plot::with_coord(Isometric::new())
    .mark(Symbol3D::new().x("x").y("y").z("z"));
```

### Subplot-Capable Coordinate System
```rust
use avenger_chart::plot::Plot;
use avenger_chart_core::ZeroDCoord;
use avenger_chart_external_test::external_subplot_coord::ExternalSubplotCoord;
use avenger_chart_marks::Subplot;

let plot = Plot::<ExternalSubplotCoord>::new()
    .mark(Subplot::new(Plot::<ZeroDCoord>::new()));
```

This crate demonstrates that avenger-chart's supported extension points are
usable from external crates without depending on built-in mark, coordinate, or
legend implementation crates unless those concrete implementations are needed.
