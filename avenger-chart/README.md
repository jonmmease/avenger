# Avenger Chart

A Rust-based grammar of graphics library built on top of the Avenger visualization engine.

## Overview

Avenger Chart provides a high-level, type-safe API for creating data visualizations using the grammar of graphics approach. It features:

- **Observable Plot-inspired transform system** for data manipulation
- **Enhanced faceting with resolution control** (SharedRows/SharedCols options)
- **Type-safe coordinate system support** (Cartesian, Polar)
- **Post-scale adjustments and derived marks** for reactive geometry
- **Plot-level and mark-level data inheritance**

## Architecture

### Core Components

- **Plot**: Main visualization container with coordinate system support
- **Marks**: Visual elements (Symbol, Line, Rect) with encoding channels
- **Transforms**: Data transformation pipeline (Bin, Group, Stack)
- **Adjustments**: Post-scale position modifications (Jitter, Dodge)
- **Derived Marks**: Generate additional marks from scaled data
- **Faceting**: Multi-panel layouts with enhanced resolution control

### Transform Pipeline

```rust
// Observable Plot-inspired transform chain
Rect::new()
    .data(df)
    .transform(Bin::x("price").width(10.0).aggregate(count(lit(1))))?
    .transform(Stack::y().order(StackOrder::Sum))?
    .adjust(Jitter::new().x(2.0))
    .derive(LabelPoints::new())
```

### Enhanced Faceting

```rust
// Advanced resolution control
Plot::new(Cartesian)
    .data(df)
    .facet(Facet::grid()
        .row("continent")
        .column("year")
        .resolve_scale("x", Resolution::Shared)      // Same time axis
        .resolve_scale("y", Resolution::SharedCols)  // Metric per column
        .resolve_legend("color", Resolution::SharedRows)) // Region legends
    .mark(Symbol::new().x("gdp").y("life").color("country"))
```

## Current Status

This is an active development library. The API design is largely complete with:

- ✅ Core plot and mark system
- ✅ Transform pipeline with channel references
- ✅ Adjust and derive functionality
- ✅ Enhanced facet resolution system
- 🚧 Transform implementations (currently stubs)
- 🚧 Rendering pipeline integration

## Documentation

See the `docs/` directory for detailed design documents:

- `enhanced-facet-resolution-design.md` - Faceting with 4-option resolution system
- `post-scale-transforms-design.md` - Adjust and derive architecture  
- `adjust-api-design.md` - Post-scale adjustment system

## Development

```bash
# Build the library
cargo build

# Run examples
cargo run --example transform_demo
cargo run --example adjust_api
cargo run --example derive_marks

# Run tests
cargo test
```