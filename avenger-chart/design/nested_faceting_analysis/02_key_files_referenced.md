# Key Files Referenced in Nested Faceting Architecture

## Core Faceting Implementation

### 1. /Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/src/facet/marks/facet.rs
**Contains**: Facet mark definitions and compilation logic

Key Structures:
- `Facet<InnerC>`: Generic facet mark type parameterized by inner coordinate system
- `CompiledFacetRow`: Compiled facet mark for row faceting
- `CompiledFacetCol`: Compiled facet mark for column faceting  
- `CompiledFacetGrid`: Compiled facet mark for 2D grid faceting

Key Methods:
- `Facet::compile()`: Compiles subplot once at plot level (currently doesn't handle nesting)
- `CompiledFacetRow::evaluate_from_data()`: Two-pass layout/rendering
- `CompiledFacetCol::evaluate_from_data()`: Two-pass layout/rendering
- `CompiledFacetGrid::evaluate_from_data()`: Complex 2D grid layout

**Problem Location**: Lines 146-160 and 344-356 where `subplot.compile()` is called eagerly without filtered data

### 2. /Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/src/facet/marks/facet_evaluation.rs
**Contains**: Generic two-pass facet evaluation algorithm shared between FacetRow and FacetCol

Key Function:
- `evaluate_facet<DimConfig>()`: Generic parameterized function implementing two-pass algorithm:
  - Pass 1: Measures guide overflow for spacing calculation
  - Pass 2: Renders subplots with final spacing

**Design Pattern**: Accepts orientation-specific closures for `subplot_dims` and `group_origin` to parameterize behavior

**Data Filtering Logic**: Uses `df.filter(facet_expr.eq(lit(facet_value)))` to filter data per facet value during evaluation

### 3. /Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/src/plot/plot.rs
**Contains**: Plot specification and compilation

Key Methods:
- `Plot::compile()`: Main compilation entry point that compiles marks
- Mark compilation happens here, where facets eagerly compile subplots

**Modification Point**: May need `compile_with_data()` method for lazy compilation variant

### 4. /Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/src/facet/
**Directory Structure**:
- `marks/`: Facet mark implementations (facet.rs, facet_evaluation.rs)
- `context.rs`: FacetContext for parameter passing between facet levels
- `dimension_config.rs`: RowDimensionConfig, ColumnDimensionConfig traits
- `coord.rs`: FacetRow, FacetColumn, FacetGrid coordinate system types
- `scale_grouping.rs`: ScaleGrouping for building per-cell scales in GridFacet
- `subplot_iterator.rs`: SubplotIterator for iterating facet domain values

## Data Flow Architecture

### Current Data Flow (Single-Level Faceting)
```
Plot<FacetColumn>::compile(df)
  └─> Mark::compile()
      └─> Facet::compile(df)
          └─> CompiledPlot for inner marks
              └─> Compiled inner marks have schema from df

CompiledFacetCol::evaluate_from_data(full_dataset)
  └─> For each col_value:
      └─> Filter df by col_value → filtered_df
      └─> evaluate_from_data(filtered_df, ..., Cartesian_coord)
          └─> Inner marks render with filtered data
```

### Problematic Data Flow (Nested Faceting)
```
Plot<FacetColumn>::compile(df)
  └─> Facet::compile(df)
      └─> Plot<FacetRow>.compile(df)  ← df, not filtered!
          └─> Facet::compile(df)
              └─> Plot<Cartesian>.compile(df)
                  └─> Stack overflow on recursive nesting
```

## Type System and Traits

### CoordinateSystem Trait Hierarchy
- `CoordinateSystem`: Marker trait for coordinate types
- `FacetColumn`, `FacetRow`, `FacetGrid`: Faceting coordinate types
- `Cartesian`: Default plot coordinate system

### Mark Trait Methods Involved
- `Mark::compile()`: Async, takes SessionContext → Arc<CompiledMark>
- `CompiledMark::evaluate_from_data()`: Async, takes RenderContext and data → Marks + LayoutUpdates

### Data Context
- `DataContext`: Holds DataSource/DataFrame reference
- `CompiledDataContext`: Used after compilation
- Methods: `dataframe_with_context()`, `channels()`

## Testing and Baseline Infrastructure

### Visual Regression Tests
Located in `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/tests/`

**Facet Test Baselines** (currently deleted, regenerated):
- `baselines/facet/grid_facet_*.png`: Grid faceting visual tests
- `baselines/facet_legends/facet_grid_*.png`: Legend interaction tests

**Running Tests**:
```bash
# With debug layout visualization
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test -p avenger-chart

# With trace logging
RUST_LOG=avenger_chart::facet=trace cargo test -p avenger-chart
```

## Configuration and State Management

### FacetContext
- Carries facet-specific state through evaluation pipeline
- Contains: `position`, `grid_dimensions`, `unified_channels`, `scale_sharing`
- Merged at nesting boundaries (GridFacet merges row+col contexts)

### RenderContext
- Carries rendering configuration
- Contains: scales, theme, session_context, plot_width/height, params
- Updated during two-pass rendering (scales rebuilt after Pass 1)

### ScaleGrouping
- Manages per-cell scale building for GridFacet
- Caches shared scale builders to avoid recomputation
- Key for consistent measurement across Pass 1 and Pass 2

## Related Modules

### avenger_scales
- Scale implementation and configuration
- Band scale specifically used for facet dimensions
- `ConfiguredScale`, `ScaleBuilder`, `ScaleImpl` traits

### avenger_scenegraph
- Scene graph mark types (Arc, Area, Path, Symbol, etc.)
- Group marks for positioning and clipping
- Mark composition and scene tree rendering

### datafusion
- Arrow-based data processing
- DataFrame filtering and transformations
- SessionContext for SQL/expression evaluation

## Key Implementation Patterns

### Two-Pass Rendering
1. **Pass 1 (Measurement)**: Measure guides, calculate spacing
2. **Pass 2 (Rendering)**: Rebuild scales with spacing, render final output
3. Both passes must use identical data and parameters

### Serialization (Serde)
- CompiledMark types use `#[typetag::serde]` for polymorphic serialization
- CompiledFacetCol/CompiledFacetRow are serializable
- Needed for caching and persistence

### Async Architecture
- All compilation and evaluation is async
- SessionContext required for DataFusion operations
- Parallel potential in nested compilation (TODO)

