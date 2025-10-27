# Refactoring: Extract iter_band_positions() Helper

**Status**: Proposed
**Priority**: Medium
**Effort**: ~30-60 minutes
**Related**: Issue #2 from FacetRow review

## Problem Statement

The logic for iterating over band scale positions is duplicated across multiple locations in the faceting system. This creates maintenance burden and makes it harder to ensure consistent behavior when positioning faceted subplots and their labels.

### Current Duplication

**Location 1**: `avenger-chart/src/facet/marks/facet.rs:528-553`
```rust
fn iter_band_positions(
    scale: &ConfiguredScaleWithSpec,
) -> Result<Vec<(ScalarValue, (f32, f32))>, AvengerChartError> {
    use crate::scales::ConfiguredScaleLegendExt;
    use avenger_scales::scales::band;

    let configured = scale.configured();
    let domain_vals = configured.domain_values()?;
    let positions = match domain_vals {
        crate::scales::extensions::DomainValues::Discrete(vals) => {
            configured.scale_scalars_to_numeric(&vals)?
        }
        _ => Vec::new(),
    };
    let bandwidth = band::bandwidth(&configured.config)?;

    let mut out = Vec::new();
    if let crate::scales::extensions::DomainValues::Discrete(vals) = configured.domain_values()? {
        for (i, v) in vals.into_iter().enumerate() {
            let pos = positions.get(i).cloned().unwrap_or(0.0);
            out.push((v, (pos, bandwidth)));
        }
    }
    Ok(out)
}
```

**Location 2**: `avenger-chart/src/facet/guide.rs:392-409` (with variations)
```rust
// Create centered scale by modifying band parameter
let mut centered_config = row_scale.config.clone();
centered_config.options.insert(
    "band".to_string(),
    avenger_scales::scalar::Scalar::from_f32(0.5),
);

let centered_scale = avenger_scales::scales::ConfiguredScale {
    scale_impl: row_scale.scale_impl.clone(),
    config: centered_config,
};

// Get positions from centered scale
let positions = match row_scale.domain_values()? {
    crate::scales::extensions::DomainValues::Discrete(vals) => {
        use crate::scales::extensions::ConfiguredScaleLegendExt;
        centered_scale.scale_scalars_to_numeric(&vals)?
    }
    _ => Vec::new(),
};

// Later: iterate with positions.get(i)
for (i, label) in labels.iter().enumerate() {
    let y_center = plot_bounds.y + positions.get(i).cloned().unwrap_or(0.0);
    // ... render at y_center
}
```

### Key Differences

1. **facet.rs** returns `(ScalarValue, (f32, f32))` - domain value with (position, bandwidth)
2. **guide.rs** creates a "centered" scale (band=0.5) and extracts only positions
3. **facet.rs** calls domain_values() twice (lines 536 and 546) - inefficient
4. Both use the same pattern: domain → positions → bandwidth → zip

## Analysis

### What the Function Does

The `iter_band_positions()` function performs these steps:

1. **Extract domain values** from the band scale (e.g., ["setosa", "versicolor", "virginica"])
2. **Convert to numeric positions** using `scale_scalars_to_numeric()` (e.g., [0.0, 100.0, 200.0])
3. **Get bandwidth** from scale config (e.g., 80.0 for 20% padding)
4. **Zip together** domain values with (position, bandwidth) tuples

**Purpose**: Provides the information needed to position each faceted subplot along the row/column dimension.

### Why guide.rs Uses "Centered" Scale

The guide needs label positions at the **center** of each band, not the start. Band scales have a configurable `band` parameter (0.0 = start, 0.5 = center, 1.0 = end).

Current approach:
- Clone the scale config
- Set `band: 0.5` to get center positions
- Call `scale_scalars_to_numeric()` with modified config

This is **more flexible** than facet.rs's approach because:
- facet.rs returns `(position, bandwidth)` and requires callers to compute centers
- guide.rs gets positions directly at centers

### Inefficiency in Current Implementation

**facet.rs line 536 and 546**: Calls `configured.domain_values()` twice:
```rust
let domain_vals = configured.domain_values()?;  // First call
let positions = match domain_vals { ... };
let bandwidth = band::bandwidth(&configured.config)?;

let mut out = Vec::new();
if let crate::scales::extensions::DomainValues::Discrete(vals) = configured.domain_values()? {  // Second call
    for (i, v) in vals.into_iter().enumerate() {
        // ...
    }
}
```

This is wasteful since `domain_values()` likely involves data processing.

## Proposed Solution

### Option 1: Iterator-Based API (Recommended)

Following the pattern established by `SubplotIterator`, create a dedicated module with an iterator type.

**Create**: `avenger-chart/src/facet/band_positions.rs`

```rust
use datafusion::common::ScalarValue;
use crate::scales::ConfiguredScaleWithSpec;
use crate::scales::extensions::{ConfiguredScaleLegendExt, DomainValues};
use crate::error::AvengerChartError;

/// Position information for a single band in a band scale
#[derive(Debug, Clone)]
pub struct BandPosition {
    /// The domain value (e.g., "setosa", "versicolor", "virginica")
    pub value: ScalarValue,
    /// Numeric position of the band start
    pub position: f32,
    /// Width/height of the band
    pub bandwidth: f32,
}

impl BandPosition {
    /// Get the center position of this band
    pub fn center(&self) -> f32 {
        self.position + self.bandwidth / 2.0
    }

    /// Get the end position of this band
    pub fn end(&self) -> f32 {
        self.position + self.bandwidth
    }
}

/// Iterator over band positions from a configured band scale
///
/// Provides consistent iteration over domain values with their corresponding
/// positions and bandwidth for faceted layouts.
///
/// # Example
///
/// ```ignore
/// let iter = BandPositionIterator::from_scale(&row_scale)?;
/// for band_pos in iter {
///     let subplot_y = band_pos.position;
///     let subplot_height = band_pos.bandwidth;
///     // render subplot at (x, subplot_y) with height subplot_height
/// }
/// ```
pub struct BandPositionIterator {
    domain_vals: Vec<ScalarValue>,
    positions: Vec<f32>,
    bandwidth: f32,
    current_index: usize,
}

impl BandPositionIterator {
    /// Create an iterator from a configured band scale
    pub fn from_scale(scale: &ConfiguredScaleWithSpec) -> Result<Self, AvengerChartError> {
        use avenger_scales::scales::band;

        let configured = scale.configured();
        let domain_vals = configured.domain_values()?;

        let positions = match &domain_vals {
            DomainValues::Discrete(vals) => {
                configured.scale_scalars_to_numeric(vals)?
            }
            _ => Vec::new(),
        };

        let bandwidth = band::bandwidth(&configured.config)?;

        let domain_vals = match domain_vals {
            DomainValues::Discrete(vals) => vals,
            _ => Vec::new(),
        };

        Ok(Self {
            domain_vals,
            positions,
            bandwidth,
            current_index: 0,
        })
    }

    /// Create an iterator with custom band parameter (0.0 = start, 0.5 = center, 1.0 = end)
    ///
    /// Useful for positioning labels at band centers:
    /// ```ignore
    /// let iter = BandPositionIterator::from_scale_with_band(&row_scale, 0.5)?;
    /// for band_pos in iter {
    ///     let label_y = band_pos.position;  // Already at center
    /// }
    /// ```
    pub fn from_scale_with_band(
        scale: &ConfiguredScaleWithSpec,
        band_offset: f32,
    ) -> Result<Self, AvengerChartError> {
        use avenger_scales::scales::band;

        let configured = scale.configured();
        let domain_vals = configured.domain_values()?;

        // Create modified config with custom band offset
        let mut centered_config = configured.config.clone();
        centered_config.options.insert(
            "band".to_string(),
            avenger_scales::scalar::Scalar::from_f32(band_offset),
        );

        let centered_scale = avenger_scales::scales::ConfiguredScale {
            scale_impl: configured.scale_impl.clone(),
            config: centered_config,
        };

        let positions = match &domain_vals {
            DomainValues::Discrete(vals) => {
                centered_scale.scale_scalars_to_numeric(vals)?
            }
            _ => Vec::new(),
        };

        let bandwidth = band::bandwidth(&configured.config)?;

        let domain_vals = match domain_vals {
            DomainValues::Discrete(vals) => vals,
            _ => Vec::new(),
        };

        Ok(Self {
            domain_vals,
            positions,
            bandwidth,
            current_index: 0,
        })
    }

    /// Get the number of bands
    pub fn len(&self) -> usize {
        self.domain_vals.len()
    }

    /// Check if iterator is empty
    pub fn is_empty(&self) -> bool {
        self.domain_vals.is_empty()
    }

    /// Get the bandwidth (same for all bands)
    pub fn bandwidth(&self) -> f32 {
        self.bandwidth
    }
}

impl Iterator for BandPositionIterator {
    type Item = BandPosition;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_index >= self.domain_vals.len() {
            return None;
        }

        let index = self.current_index;
        let value = self.domain_vals[index].clone();
        let position = self.positions.get(index).copied().unwrap_or(0.0);

        self.current_index += 1;

        Some(BandPosition {
            value,
            position,
            bandwidth: self.bandwidth,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.domain_vals.len() - self.current_index;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for BandPositionIterator {}
```

### Option 2: Simple Helper Function

Just move the existing function to a shared location and add a variant for centered positions.

**Create**: `avenger-chart/src/facet/band_positions.rs`

```rust
use datafusion::common::ScalarValue;
use crate::scales::ConfiguredScaleWithSpec;
use crate::error::AvengerChartError;

/// Get band positions from a configured band scale
///
/// Returns (domain_value, (position, bandwidth)) tuples
pub fn iter_band_positions(
    scale: &ConfiguredScaleWithSpec,
) -> Result<Vec<(ScalarValue, (f32, f32))>, AvengerChartError> {
    // ... existing implementation from facet.rs
}

/// Get centered band positions from a configured band scale
///
/// Returns (domain_value, center_position) tuples
pub fn iter_centered_band_positions(
    scale: &ConfiguredScaleWithSpec,
) -> Result<Vec<(ScalarValue, f32)>, AvengerChartError> {
    // ... implementation using band=0.5
}
```

## Recommendation

**Choose Option 1** for these reasons:

1. **Consistency**: Matches the `SubplotIterator` pattern already established
2. **Flexibility**: `BandPosition` struct provides `center()` and `end()` helpers
3. **Ergonomics**: Iterator API is more natural for consumption
4. **Efficiency**: Single `domain_values()` call, cached in iterator
5. **Extensibility**: Easy to add methods like `from_scale_with_band()` for custom offsets

## Migration Plan

### Step 1: Create New Module

1. Create `avenger-chart/src/facet/band_positions.rs`
2. Implement `BandPosition` and `BandPositionIterator`
3. Add unit tests (similar to `subplot_iterator.rs` tests)
4. Export from `avenger-chart/src/facet/mod.rs`

### Step 2: Update facet.rs

Replace `iter_band_positions()` function with iterator:

**Before**:
```rust
let band_positions = iter_band_positions(&row_scale)?;
for (facet_value, (pos, bandwidth)) in band_positions {
    // ...
}
```

**After**:
```rust
use crate::facet::band_positions::BandPositionIterator;

let band_iter = BandPositionIterator::from_scale(&row_scale)?;
for band_pos in band_iter {
    let facet_value = band_pos.value;
    let pos = band_pos.position;
    let bandwidth = band_pos.bandwidth;
    // ...
}
```

### Step 3: Update guide.rs

Replace manual centered scale creation with `from_scale_with_band()`:

**Before** (lines 392-409):
```rust
let mut centered_config = row_scale.config.clone();
centered_config.options.insert(
    "band".to_string(),
    avenger_scales::scalar::Scalar::from_f32(0.5),
);

let centered_scale = avenger_scales::scales::ConfiguredScale {
    scale_impl: row_scale.scale_impl.clone(),
    config: centered_config,
};

let positions = match row_scale.domain_values()? {
    crate::scales::extensions::DomainValues::Discrete(vals) => {
        centered_scale.scale_scalars_to_numeric(&vals)?
    }
    _ => Vec::new(),
};

// Later...
for (i, label) in labels.iter().enumerate() {
    let y_center = plot_bounds.y + positions.get(i).cloned().unwrap_or(0.0);
    // ...
}
```

**After**:
```rust
use crate::facet::band_positions::BandPositionIterator;

let band_iter = BandPositionIterator::from_scale_with_band(&row_scale, 0.5)?;
let band_positions: Vec<_> = band_iter.collect();

for (i, label) in labels.iter().enumerate() {
    if let Some(band_pos) = band_positions.get(i) {
        let y_center = plot_bounds.y + band_pos.position;
        // ...
    }
}
```

**Or even better** (zip directly):
```rust
for (label, band_pos) in labels.iter().zip(&band_positions) {
    let y_center = plot_bounds.y + band_pos.position;
    // ...
}
```

### Step 4: Testing

Run visual regression tests:
```bash
RUSTFLAGS="-D warnings" cargo test -p avenger-chart
```

All existing tests should pass without changes since this is a refactoring.

## Benefits

1. **Code Reuse**: Eliminates ~40 lines of duplicated logic
2. **Maintainability**: Single source of truth for band position iteration
3. **Consistency**: Same pattern as `SubplotIterator` makes codebase more predictable
4. **Readability**: `band_pos.center()` is clearer than `pos + bandwidth / 2.0`
5. **Efficiency**: Single `domain_values()` call instead of multiple
6. **Extensibility**: Easy to add FacetCol support with same pattern

## Risks

1. **Low Risk**: Pure refactoring with no behavior changes
2. **Test Coverage**: Existing visual regression tests will catch any issues
3. **Performance**: Iterator pattern has zero-cost abstraction in Rust

## Future Work

Once this is in place:
- Use same pattern for `FacetCol` when implemented
- Could extend to support 2D iteration for grid facets
- Could add caching if band position calculation becomes expensive
