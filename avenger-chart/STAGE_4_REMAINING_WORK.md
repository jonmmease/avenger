# Stage 4 Remaining Work

## Completed So Far
1. ✅ Updated `evaluate_facet()` signature to accept `facet_coord` as first parameter
2. ✅ Updated `FacetRow::evaluate_from_data()` to pass `coord.as_ref()`
3. ✅ Updated `FacetColumn::evaluate_from_data()` to pass `coord.as_ref()`

## Remaining Tasks in facet_evaluation.rs

### 1. Replace Pass 1 from_band_positions() call (line ~203-215)

**Current code:**
```rust
let initial_geometry = SubplotGeometry::from_band_positions(
    BandPositionIterator::from_scale(dimension_scale)?,
    if DimConfig::is_row_facet() {
        FacetAxis::Row
    } else {
        FacetAxis::Column
    },
    if DimConfig::is_row_facet() {
        context.plot_width
    } else {
        context.plot_height
    },
);
let initial_rects = &initial_geometry.rects;
```

**Replace with:**
```rust
// Call facet coord transform to get initial geometry
let initial_geometry = facet_coord.transform(
    &position_channels,
    Some(&position_values),
    context.plot_width,
    context.plot_height,
)?;

let initial_rects = initial_geometry
    .as_any()
    .downcast_ref::<SubplotGeometry>()
    .ok_or_else(|| {
        AvengerChartError::InternalError(
            "Expected SubplotGeometry from facet coord transform".into(),
        )
    })?
    .rects
    .clone();
```

### 2. Add with_measured_padding() call (after line ~318)

**After computing `rounded_gap`, add:**
```rust
// Update facet coord with measured padding for Pass 2
let updated_facet_coord = facet_coord.with_measured_padding(
    rounded_gap,
    overflow_measurements.clone(),
);
```

### 3. Replace Pass 2 from_band_positions() call (line ~367-379)

**Current code:**
```rust
let final_geometry = SubplotGeometry::from_band_positions(
    BandPositionIterator::from_scale(final_dimension_scale)?,
    if DimConfig::is_row_facet() {
        FacetAxis::Row
    } else {
        FacetAxis::Column
    },
    if DimConfig::is_row_facet() {
        context.plot_width
    } else {
        context.plot_height
    },
);
let final_rects = final_geometry.rects;
```

**Replace with:**
```rust
// Call updated facet coord transform for final geometry
let final_geometry = updated_facet_coord.transform(
    &position_channels_pass2,
    Some(&position_values_pass2),
    context.plot_width,
    context.plot_height,
)?;

let final_rects = final_geometry
    .as_any()
    .downcast_ref::<SubplotGeometry>()
    .ok_or_else(|| {
        AvengerChartError::InternalError(
            "Expected SubplotGeometry from facet coord transform in Pass 2".into(),
        )
    })?
    .rects
    .clone();
```

### 4. Delete Scale Rebuild Hack (lines ~327-353)

**Delete entire block that starts with:**
```rust
// Rebuild the facet dimension scale with measured padding_inner_px
```

**And ends before the Pass 2 section.**

### 5. Remove Obsolete Imports

Check if these are still used, if not remove:
- `use crate::coords::FacetAxis;`
- `use crate::facet::band_positions::BandPositionIterator;`

### 6. FacetGrid Note

FacetGrid has custom evaluation logic and doesn't use `evaluate_facet()`.
It may need separate updates or can remain using `from_band_positions()` for now.

## Testing After Changes

```bash
# Check compilation
RUSTFLAGS="-D warnings" cargo check -p avenger-chart

# Run tests
cargo test -p avenger-chart

# Visual regression
cargo test -p avenger-chart --test visual_regression
```

## Key Points from GPT-5 Codex

1. The facet coord was already available as `_coord` in `evaluate_from_data()` - now passed through ✅
2. Must use measured padding via `with_measured_padding()` or bandwidth calculations will be wrong
3. Ensure `position_channels` uses exact keys the transform expects (`"row"` or `"column"`)
4. The difference between centers includes the measured gap

## Expected Outcome

After these changes:
- Stages 1-3 infrastructure actually gets used
- Faceting works through `coord.transform()` like other coordinate systems
- Scale rebuilding hack eliminated
- Two-pass flow cleaner via `with_measured_padding()`
- Sets up Phase 7 (removing `cached_edge_overflow`)
