# Facet System Architecture Overview

This document provides a maintainer-level introduction to the avenger-chart facet system. It covers module organization, core types, key traits, and high-level data flow.

## Module Structure

```
avenger-chart/src/facet/
+-- mod.rs                    Public module exports
|
+-- coord.rs                  Coordinate systems: FacetRow, FacetColumn
|                             Implements CoordinateSystem and CoordinateSystemTransform
|
+-- marks/                    Facet mark implementation
|   +-- mod.rs                Re-exports: CompiledFacetRow, CompiledFacetCol
|   +-- facet.rs              Facet<InnerC> generic mark + compiled variants
|   +-- facet_config.rs       FacetOptions, FacetRowChannelConfig, FacetColChannelConfig
|   +-- facet_evaluation.rs   Two-pass algorithm: measure_pass, render_pass, evaluate_facet
|
+-- guide.rs                  Facet guides: FacetRowGuide, FacetColGuide
|                             Implements CompiledGuide for axis/label rendering
|
+-- dimension_config.rs       FacetDimensionConfig trait + Row/Column implementations
|                             Parameterizes row vs column behavior
|
+-- context.rs                FacetContext: subplot position/grid info
|                             Passed via params for axis visibility decisions
|
+-- coordination.rs           FacetCoordinationContext: nested facet coordination
|                             Domain propagation, spacing keys, guide ownership
|
+-- visibility.rs             Label/title visibility computation
|
+-- scale_helpers.rs          Scale building utilities for facet measurement
|
+-- scale_grouping.rs         Scale extent aggregation across facets
|
+-- band_positions.rs         Band scale position computation
|
+-- subplot_iterator.rs       Iterator over subplot cells with data filtering
|
+-- phantom_cells.rs          Empty cell handling for shared domain facets
|
+-- guide_utils.rs            Shared guide measurement utilities
|
+-- guide_measurement.rs      Guide overflow measurement helpers
|
+-- scalar_cmp.rs             Scalar value comparison for domain sorting
|
+-- keys.rs                   Spacing key constants
```

## Core Types

### Coordinate Systems

| Type | Location | Purpose |
|------|----------|---------|
| `FacetRow` | `coord.rs:26` | Vertical stack of subplots (row faceting) |
| `FacetColumn` | `coord.rs:187` | Horizontal row of subplots (column faceting) |

Both implement:
- `CoordinateSystem` trait: defines required channels and associated guide type
- `CoordinateSystemTransform` trait: transforms data to subplot geometry

### Mark Types

| Type | Location | Purpose |
|------|----------|---------|
| `Facet<InnerC>` | `marks/facet.rs:24` | Generic facet mark builder (user-facing API) |
| `CompiledFacetRow` | `marks/facet.rs:123` | Compiled row facet mark |
| `CompiledFacetCol` | `marks/facet.rs:319` | Compiled column facet mark |

### Configuration Types

| Type | Location | Purpose |
|------|----------|---------|
| `FacetOptions` | `marks/facet_config.rs:11` | Builder for title, spacing, scale_sharing |
| `FacetRowChannelConfig` | `marks/facet_config.rs:4` | Row channel configuration |
| `FacetColChannelConfig` | `marks/facet_config.rs:65` | Column channel configuration |
| `ScaleSharing` | `channel/config_traits.rs:152` | Enum: `Shared`, `Free`, `Level(n)` |

### Context Types

| Type | Location | Purpose |
|------|----------|---------|
| `FacetContext` | `context.rs:58` | Subplot position within single facet level |
| `FacetCoordinationContext` | `coordination.rs` | Cross-level nested facet coordination |
| `GuideOwnership` | `coordination.rs:109` | Enum: `Full`, `Edge`, `Suppress` |

### Dimension Configuration

| Type | Location | Purpose |
|------|----------|---------|
| `FacetDimensionConfig` | `dimension_config.rs:16` | Trait abstracting row vs column behavior |
| `RowDimensionConfig` | `dimension_config.rs:106` | Row-specific implementation |
| `ColumnDimensionConfig` | `dimension_config.rs:181` | Column-specific implementation |

## Key Traits

### `FacetDimensionConfig` (dimension_config.rs:16)

Parameterizes row vs column behavior for code sharing. Key methods:

```rust
pub trait FacetDimensionConfig: Clone + Send + Sync + 'static {
    fn channel_name() -> &'static str;              // "row" or "column"
    fn facet_direction() -> FacetDirection;         // Row or Column
    fn unified_channels() -> &'static [&'static str]; // ["y"] or ["x"]
    fn index_to_position(index: usize) -> (usize, usize);
    fn count_to_grid_dimensions(total: usize) -> (usize, usize);
    fn calculate_adjacent_overflow(a: &Overflow, b: &Overflow) -> f32;
    fn is_row_facet() -> bool;
    fn is_col_facet() -> bool;
    fn subplot_dimensions(band_size: f32, plot_width: f32, plot_height: f32) -> (f32, f32);
    fn group_origin(position: f32) -> [f32; 2];
    fn inter_gap_key() -> &'static str;             // "inter_row_gap" or "inter_col_gap"
    // ... additional methods
}
```

### `Mark<C>` and `CompiledMark` (marks/mod.rs)

Facet marks implement these traits from the core mark system:

- `Mark<FacetRow>` for `Facet<InnerC>` - compiles to `CompiledFacetRow`
- `Mark<FacetColumn>` for `Facet<InnerC>` - compiles to `CompiledFacetCol`

The `CompiledMark` trait provides:
- `evaluate_from_data()` - entry point for the two-pass algorithm
- `wants_full_data_batch()` - returns `true` (facets need complete data for filtering)
- `preferred_scale_type()` - returns `Band` scale for facet channels
- `default_scale_options()` - configures band padding/alignment

### `CompiledGuide` (guide.rs)

Facet guides (`FacetRowGuide`, `FacetColGuide`) implement `CompiledGuide`:
- `measure_overflow()` - measures axis/label space requirements
- `build()` - renders guide elements (labels, titles, rules)

## File Responsibilities

| File | Primary Responsibility |
|------|------------------------|
| `coord.rs` | Coordinate system definitions and transforms |
| `marks/facet.rs` | User API + compiled mark implementations |
| `marks/facet_config.rs` | Configuration builder types |
| `marks/facet_evaluation.rs` | **Core algorithm**: two-pass measurement and rendering |
| `guide.rs` | Guide rendering and overflow measurement |
| `dimension_config.rs` | Row/column behavior parameterization |
| `context.rs` | Single-level subplot context |
| `coordination.rs` | Multi-level nested facet coordination |
| `visibility.rs` | Axis label/title visibility rules |
| `scale_helpers.rs` | Scale construction during measurement |
| `scale_grouping.rs` | Data extent aggregation across facets |
| `subplot_iterator.rs` | Iteration over facet cells with data filtering |
| `phantom_cells.rs` | Empty cell placement for shared domains |

## High-Level Data Flow

```
User Code
    |
    v
Facet<InnerC>::new()                    [marks/facet.rs]
    .row(col("species"))
    .subplot(Plot::new()...)
    |
    v
+-------------------+
|   Compilation     |
+-------------------+
    |
    Mark::compile()                     [marks/facet.rs:145]
    |
    v
CompiledFacetRow / CompiledFacetCol
    |
    v
+-------------------+
|   Evaluation      |
+-------------------+
    |
    CompiledMark::evaluate_from_data()  [marks/facet.rs:223]
    |
    v
evaluate_facet<DimConfig>()             [marks/facet_evaluation.rs]
    |
    +---> measure_pass<DimConfig>()     [Pass 1: Measurement]
    |         |
    |         +---> For each subplot:
    |         |         - Filter data by facet value
    |         |         - Build scales (shared/free)
    |         |         - Measure overflow via guide
    |         |
    |         +---> Aggregate spacing needs
    |         +---> Compute final padding
    |
    +---> [Phase 1.5: Coordination]      (if nested facets)
    |         |
    |         +---> Check cross-dimension gaps
    |         +---> Re-run measure_pass with coordinated spacing
    |
    +---> render_pass<DimConfig>()      [Pass 2: Rendering]
              |
              +---> For each subplot:
                        - Build final scales
                        - Render subplot marks
                        - Position at calculated rectangle
    |
    v
Vec<SceneMark>                          [Output]
```

## Scale Sharing Flow

```
ScaleSharing Mode
    |
    +-- Shared ---------> Domain from full dataset
    |                     Same scale for all cells
    |                     Labels only on edges
    |
    +-- Free -----------> Domain from filtered data
    |                     Independent scale per cell
    |                     Labels on all cells
    |
    +-- Level(n) -------> Hierarchical sharing
                          Level(0) = Free
                          Level(1+) = Share with parent
```

## Context Propagation

```
Outer Facet (e.g., FacetColumn)
    |
    +---> Creates FacetCoordinationContext
    |         - inner_domain (if sharing)
    |         - coordinated_spacing
    |         - guide_ownership
    |
    +---> Serializes to params["__facet_coordination"]
    |
    v
Inner Facet (e.g., FacetRow)
    |
    +---> Reads FacetCoordinationContext from params
    |         - Uses passed domain (grid-like layout)
    |         - Uses coordinated gaps
    |
    +---> Creates FacetContext for each cell
    |         - position: (row, col)
    |         - grid_dimensions: (num_rows, num_cols)
    |         - scale_sharing per channel
    |
    +---> Serializes to params["__facet_context"]
    |
    v
Subplot Guides
    |
    +---> Reads FacetContext
    +---> Decides label/title visibility
```

## Cross-References

For detailed algorithm documentation, see:

- **Two-Pass Algorithm**: `two-pass-algorithm.md` (planned)
  - Covers measure_pass, Phase 1.5 coordination, render_pass
  - Line-by-line walkthrough of facet_evaluation.rs

- **Spacing Coordination**: `spacing-coordination.md` (planned)
  - Named spacing keys system
  - Cross-dimension gap computation
  - Overflow aggregation

- **Scale Sharing Internals**: `scale-sharing-internals.md` (planned)
  - Domain propagation mechanisms
  - SharedInRow/SharedInColumn modes
  - Level-based hierarchical sharing

For comprehensive analysis with code examples:

- `guides/measurement-coordination-detailed.md` - 750-line deep dive
- `guides/measurement-architecture-quickref.md` - Quick reference

## Key Constants

Spacing keys (from `keys.rs` and `coordination.rs`):

| Key | Purpose |
|-----|---------|
| `"inter_row_gap"` | Gap between adjacent rows |
| `"inter_col_gap"` | Gap between adjacent columns |
| `"shared_overflow_left"` | Left overflow for alignment |
| `"shared_overflow_right"` | Right overflow for alignment |
| `"shared_overflow_top"` | Top overflow for alignment |
| `"shared_overflow_bottom"` | Bottom overflow for alignment |
| `"legend_right"` | Legend width on right side |
| `"legend_bottom"` | Legend height on bottom |

## Concurrency

The measurement pass uses bounded concurrency:

```rust
// facet_evaluation.rs
const MAX_CONCURRENT_MEASURE: usize = 4;
let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_MEASURE));
```

This prevents overwhelming DataFusion with too many concurrent queries during subplot measurement.

## Debugging

Enable debug output:

```bash
# Visual layout debugging (magenta rectangles)
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test -p avenger-chart

# Logging output
RUST_LOG=avenger_chart::facet=debug cargo test -p avenger-chart -- --nocapture
```

See `avenger-chart/docs/DEBUGGING.md` for comprehensive debugging guidance.
