# Grid Facet Implementation - Detailed Technical Analysis

## Overview

Grid faceting (FacetGrid) creates a 2D arrangement of subplots where data is partitioned along two dimensions (rows and columns). Unlike nested faceting (FacetRow/FacetColumn), grid facets enforce equal-sized cells and provide sophisticated mechanisms for:
- Coordinated scale sharing across dimensions
- Unified axis guides across rows/columns
- Intelligent legend alignment
- Overflow measurement and spacing calculation

## Key Features of Grid Facets

### 1. Equal-Sized Cell Layout
Grid facets enforce a uniform grid where all cells have the same dimensions, unlike nested row/column faceting which allows unequal sizes. This is enforced through:
- **Band scale layout**: Both row and column dimensions use band scales with `padding_inner_px` dynamically calculated
- **Shared bandwidth**: All cells in a column have the same width, all cells in a row have the same height
- **Two-pass rendering**: Measurement pass determines required spacing, then scales are rebuilt with proper padding

### 2. Required Channels
Grid facets require TWO channels:
- `"row"`: Determines which rows each data point appears in
- `"column"`: Determines which columns each data point appears in

These are enforced via:
```rust
fn required_channels(&self) -> &'static [&'static str] {
    &["row", "column"]
}
```

### 3. Coordinate System (FacetGrid)
Located in `avenger-chart/src/facet/coord.rs`, the FacetGrid coordinate system:
- **Inherits from CoordinateSystem trait** with `type Guide = GridFacetGuide`
- **Creates transform** that produces SubplotGeometry (list of SubplotRect structures)
- **Handles 2D grid transformation** via the transform() method that:
  - Computes row positions and bandwidth from row scale
  - Computes column positions and bandwidth from column scale
  - Creates N×M rectangles (where N=num_rows, M=num_cols)
  - Stores row_index and col_index on each rect for later lookup

#### Transform Method Details
```rust
fn transform(
    position_channels: &HashMap<&str, ScalarOrArray<f32>>,
    position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
    plot_width: f32,
    plot_height: f32,
) -> Result<Box<dyn PlotGeometry>, AvengerChartError>
```

Nested loop creates rects in row-major order:
```
for row_idx in 0..num_rows:
    for col_idx in 0..num_cols:
        create SubplotRect(row_value, col_value, row_idx, col_idx, x, y, width, height)
```

Each SubplotRect stores:
- Row and column facet values
- (row_idx, col_idx) indices for 2D positioning
- (x, y) origin position
- (width, height) dimensions

## Cell Sizing and Positioning

### 1. Bandwidth Calculation
Grid facets use `compute_band_layout()` to calculate cell dimensions:

```rust
fn compute_band_layout(positions: &[f32], extent: f32, padding_px: Option<f32>) -> (Vec<f32>, f32)
```

Process:
1. Find minimum gap between consecutive position values (sorted)
2. Use gap as base_bandwidth (or full extent if single cell)
3. Subtract padding_px from bandwidth to get effective_bandwidth
4. Return starting positions (from scale) and effective bandwidth

### 2. Dynamic Padding (Two-Pass Algorithm)

**Pass 1 (Measurement)**:
- Extract initial row/col positions from scales (with padding_inner_px = 0)
- Call coord.transform() to get initial SubplotRect geometry
- Measure overflow for ALL cells:
  - **guide_only_grid**: Axis-only overflow (used for legend alignment)
  - **total_overflow_grid**: Full overflow including legends (used for spacing)
- Calculate required padding:
  - Row padding: max(bottom_overflow[i] + top_overflow[i+1]) for all row gaps
  - Col padding: max(right_overflow[j] + left_overflow[j+1]) for all col gaps

**Between Passes**:
- Rebuild row and column band scales with calculated padding_inner_px
- This increases the gap between cells by the measured overflow amount

**Pass 2 (Rendering)**:
- Extract updated row/col positions from rebuilt scales
- Call coord.transform() again to get final SubplotRect geometry
- Render all cells with final geometry

### 3. Cell Position Storage
Each SubplotRect includes:
- `value`: Row facet value (ScalarValue)
- `col_value`: Column facet value (ScalarValue)
- `row_index`: Index into row domain (0..num_rows)
- `col_index`: Index into column domain (0..num_cols)
- `x, y`: Computed position from transform
- `width, height`: Computed dimensions

## Shared Axes/Guides

### 1. FacetContext - Per-Subplot Context
Each subplot receives a FacetContext containing:
- **position**: (row_idx, col_idx) - position in grid
- **grid_dimensions**: (num_rows, num_cols) - total grid size
- **unified_channels**: Set of channels unified at facet level (e.g., {"x", "y"} for GridFacet)
- **scale_sharing**: Per-channel sharing mode (Shared/Free/SharedInRow/SharedInColumn)

Context is serialized to JSON and passed via params as `__facet_context`.

### 2. Scale Sharing Modes (4 Options)
Determined per-channel via `ScaleSharing` enum:

1. **Shared**: One scale for all cells
   - Data: Union of all cells
   - Label visibility: Only on grid edges
   - Use: Consistent domain across grid (e.g., year scale 2020-2023)

2. **Free**: Independent scale per cell
   - Data: Filtered to specific cell (row_idx, col_idx)
   - Label visibility: All subplots
   - Use: Each subplot has its own value domain

3. **SharedInRow**: One scale per row, shared across columns
   - Data: Filtered to row value (include all column values)
   - Label visibility: Left/right edges for y-axis, top/bottom edges for x-axis
   - Use: Compare across columns within a row

4. **SharedInColumn**: One scale per column, shared across rows
   - Data: Filtered to column value (include all row values)
   - Label visibility: Top/bottom edges for x-axis, left/right edges for y-axis
   - Use: Compare across rows within a column

### 3. ScaleGrouping - Efficient Scale Building
Located in `avenger-chart/src/facet/scale_grouping.rs`:

```rust
pub struct ScaleGrouping {
    channel_groupings: HashMap<String, ChannelGrouping>,
    fallback_builder: ScaleBuilder,
}
```

For each channel:
1. Determine sharing mode (Shared/Free/SharedInRow/SharedInColumn)
2. Compute unique group keys based on mode
3. Build one ScaleBuilder per group from filtered data
4. During rendering, look up appropriate builder for position

GroupKey encoding:
- Shared: {row: None, col: None} (one builder)
- Free: {row: Some(r), col: Some(c)} (N×M builders)
- SharedInRow: {row: Some(r), col: None} (N builders)
- SharedInColumn: {row: None, col: Some(c)} (M builders)

### 4. GridFacetGuide - Coordinate Guide
Located in `avenger-chart/src/facet/guide.rs`:

```rust
pub struct GridFacetGuide {
    facet_sources: Vec<FacetSource>,
    
    // Row dimension
    pub row_title: Option<String>,
    unified_y_title: Option<String>,
    unifiable_row_channel: Option<String>,
    
    // Column dimension
    pub col_title: Option<String>,
    unified_x_title: Option<String>,
    unifiable_col_channel: Option<String>,
}
```

Responsibilities:
- Render row labels on the left (showing unique row values)
- Render column labels on top/bottom (showing unique column values)
- Measure overflow for facet label slabs
- Derive titles from facet channels and unifiable subplot channels

## Overflow Measurement and Coordination

### 1. OverflowSpaceRequirement
Represents space required around a subplot:
```rust
pub struct OverflowSpaceRequirement {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}
```

Tracks:
- Axis ticks and labels
- Legend dimensions
- Title space

### 2. Two-Grid Measurement Strategy
During Pass 1, grid facets measure overflow into TWO grids:

**guide_only_grid** (axes only, no legends):
- Used for legend alignment
- All subplots use global max overflow from this grid
- Ensures legends align vertically across columns and horizontally across rows

**total_overflow_grid** (axes + legends):
- Used for spacing calculation
- Includes actual legend dimensions
- Ensures sufficient space between cells for legends

### 3. Padding Calculation Functions

**calculate_row_padding()**:
```
For each row gap i:
    max_bottom = max(overflow[i][j].bottom for all j)
    max_top = max(overflow[i+1][j].top for all j)
    gap_needed = max_bottom + max_top
row_padding = max(all gaps)
```

**calculate_col_padding()**:
```
For each col gap j:
    max_right = max(overflow[i][j].right for all i)
    max_left = max(overflow[i][j+1].left for all i)
    gap_needed = max_right + max_left
col_padding = max(all gaps)
```

### 4. Unified Overflow for Legend Alignment
Only subplots with legends get unified overflow:

```rust
// Pass unified overflow ONLY for sides where legends are positioned
if legend_positions.contains(&LegendPosition::Right) {
    merged_params.insert(
        "__unified_overflow_right".to_string(),
        ScalarValue::Float32(Some(global_max_overflow.right)),
    );
}
// Similar for Left, Top, Bottom
```

This ensures:
- All cells reserve same space on legend sides
- Legends naturally align across grid
- Axis overflow varies by cell (edge subplots have ticks, inner ones don't)
- No wasted space from over-unifying

## Grid Facet Evaluation Pipeline

### CompiledFacetGrid
```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetGrid {
    pub state: CompiledMarkState,
    pub compiled_subplot: Arc<CompiledPlot>,
    pub row_title: Option<String>,
    pub col_title: Option<String>,
    pub facet_spacing: Option<f32>,
}
```

### evaluate_from_data Flow

1. **Extract domain values** from data:
   - Get unique row values
   - Get unique column values
   - Sort for deterministic ordering

2. **Determine scale sharing**:
   - Collect all channels from all marks
   - For each channel, determine sharing mode (Shared/Free/SharedInRow/SharedInColumn)
   - Build ScaleGrouping with builders per group

3. **Pass 1 - Measurement**:
   - Extract initial row/col positions (padding_inner_px = 0)
   - Call coord.transform() for initial geometry
   - Build SubplotIterator for rows and columns
   - Call measure_grid_overflow() to get 2D grids
   - Compute global max overflow from guide_only_grid
   - Calculate row_padding_px and col_padding_px from total_overflow_grid
   - Extract row/col overflow vectors

4. **Between Passes**:
   - Update coord with measured padding
   - Rebuild row and column band scales with new padding_inner_px
   - Rebuild all scales in context

5. **Pass 2 - Rendering**:
   - Extract updated row/col positions
   - Call coord.transform() for final geometry
   - For each cell (row_idx, col_idx):
     - Merge row and col FacetContexts
     - Add unified overflow params for legend sides
     - Filter data to cell-specific rows and columns
     - Look up scales for position (via ScaleGrouping)
     - Build plot components (marks, guides, legends)
     - Wrap groups with translations and clipping
   - Return all scene marks and layout updates

### merge_grid_facet_contexts()
Combines row and column iteration contexts:

```rust
pub fn merge_grid_facet_contexts(
    row_iteration: &SubplotIteration,
    col_iteration: &SubplotIteration,
    num_rows: usize,
    num_cols: usize,
) -> IndexMap<String, ScalarValue>
```

Creates unified context with:
- position: (row_idx, col_idx)
- grid_dimensions: (num_rows, num_cols)
- unified_channels: {"x", "y"} (both dimensions unified in grid facets)
- scale_sharing: from either iteration (should match)

## Data Flow Coordination

### Subplot Iteration
SubplotIterator tracks:
- facet_value: The unique value for this dimension
- index: Position in the dimension
- params: Context including FacetContext and scale_sharing

Two iterators (row and column) are built and nested during evaluation.

### Data Filtering
Each cell receives filtered data:
```
cell_data = full_data
    .filter(row_expr == row_value)
    .filter(col_expr == col_value)
```

This filtered data is:
- Passed to subplot.measure_with_scales() during Pass 1
- Passed to subplot.build_plot_components() during Pass 2

### Empty Cell Handling
Empty cells (no matching rows):
1. Count data: if count == 0, cell_is_empty = true
2. During rendering: build scales from fallback_builder
3. Skip legend rendering for empty cells
4. Still render guides and data marks (empty result)

## Layout Updates and Guide Integration

### LayoutUpdates Structure
evaluate_from_data returns:
```rust
Ok((
    marks,
    crate::layout::LayoutUpdates::new(
        updated_scales,      // Scales with measured padding
        Some(row_overflow),  // 1D overflow vector per row
        Some(col_overflow),  // 1D overflow vector per column
    ),
))
```

These overflow vectors are used by GridFacetGuide to:
- Position facet label slabs below legend marks
- Account for legend dimensions when placing labels
- Ensure no overlap between grid marks and facet labels

### Unified vs. Actual Overflow
- Subplots receive **unified overflow** on legend sides (for alignment)
- Guide measurement uses **actual per-cell overflow** to size label space
- This asymmetry ensures:
  - Legends align across grid (unified on render)
  - Label space accounts for actual overflow (measured independently)

## Key Architectural Decisions

### 1. Index-Based Cell Lookup
SubplotRect stores row_index and col_index explicitly:
```rust
for rect in final_rects {
    let row_idx = rect.row_index.expect("missing row_index");
    let col_idx = rect.col_index.expect("missing col_index");
    let row_iteration = &row_iterations[row_idx];
    let col_iteration = &col_iterations[col_idx];
}
```

Advantages:
- No reliance on rect ordering
- Supports arbitrary transform implementations
- Clear intent of cell lookup

### 2. Two-Grid Overflow Measurement
Separate guide_only_grid and total_overflow_grid:
- Guides alignment needs axis overflow (guide_only)
- Spacing needs full overflow including legends (total)
- Prevents wasted space from over-unifying

### 3. ScaleGrouping Reuse
Built once in Pass 1, reused in Pass 2 and guide measurement:
- Efficient: builders cached by group key
- Consistent: same data subsets in both passes
- Flexible: builders adapted to different dimensions

### 4. Per-Cell FacetContext
Each cell gets merged context from row and column iterations:
- Guides receive full context (position and grid_dimensions)
- Context used to determine label visibility
- Unified channels indicate facet-level unification

## Testing and Debugging

### Debug Mode
Set `AVENGER_CHART_DEBUG_LAYOUT=1` to log:
```
GRID SUBPLOT r=X c=Y: x=... y=... w=... h=... 
    overflowT=... overflowB=... overflowL=... overflowR=...
```

### Key Assertions in Code
- Row positions length == num_rows
- Col positions length == num_cols
- Final rects count == num_rows × num_cols
- Row/col iteration counts match dimensions
- rect.row_index and col_index match iteration indices

## Performance Considerations

### Scale Building
- Per-channel builders cached in ScaleGrouping
- Fallback builder built once, reused for empty cells
- Builders for shared scales built once globally
- Builders for free scales built per-cell (N×M builders)

### Overflow Measurement
- Measure all cells once in Pass 1
- Computing global max is O(rows×cols)
- Padding calculation is O(rows) + O(cols)

### Memory
- Two 2D overflow grids: O(rows × cols)
- Row/col iteration vectors: O(rows + cols)
- ScaleGrouping builders: O(num_groups) per channel

## Migration Path: Nested to Grid Facets

Grid facets differ from nested row/col faceting:

| Aspect | FacetRow/Col (Nested) | FacetGrid |
|--------|----------------------|-----------|
| Cell sizes | Unequal (based on scale) | Equal (all same) |
| Channels | Single (row OR col) | Two (row AND col) |
| Scale sharing | Boolean (shared or free) | Per-channel 4-mode enum |
| Overflow measurement | Single pass | Two pass |
| Guide type | FacetRowGuide / FacetColGuide | GridFacetGuide |
| Coordinate system | FacetRow / FacetColumn | FacetGrid |

Grid facets are NOT nested rows + nested columns; they're a unified 2D system with coordinated scale sharing and overflow measurement.
