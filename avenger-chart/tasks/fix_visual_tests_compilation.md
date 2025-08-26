# Fix Visual Tests Compilation Errors

This document tracks the progress of fixing compilation errors in visual test files after removing plot-level scale and axis methods.

## Files to Fix

- [x] test_bar_scale_color.rs
- [x] test_eight_types_symbol.rs
- [x] test_formatting.rs
- [x] test_grid_zindex.rs
- [x] test_legend_background.rs
- [x] test_legend_titles.rs
- [x] test_line_legend.rs
- [x] test_line_multi_series.rs
- [x] test_multiple_legends.rs
- [x] test_multiple_legends_background.rs
- [x] test_plot_level_config.rs
- [x] test_polar_scatter.rs
- [x] test_rect_legend.rs
- [x] test_right_axis.rs
- [x] test_symbol_padding.rs
- [x] test_title.rs

## Common Patterns to Apply

### 1. Move scale configuration to channel
```rust
// OLD:
.scale_x(|s| s.domain((0.0, 10.0)))

// NEW:
.x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
```

### 2. Move axis configuration to channel
```rust
// OLD:
.axis_x(|a| a.title("X"))

// NEW:
.x_with(col("x"), |c| c.axis(|a: DefaultCartesianAxis| a.title("X")))
```

### 3. Combine scale and axis in one channel
```rust
// OLD:
.scale_x(|s| s.domain((0.0, 10.0)))
.axis_x(|a| a.title("X"))

// NEW:
.x_with(col("x"), |c| c
    .scale(|s| s.domain((0.0, 10.0)))
    .axis(|a: DefaultCartesianAxis| a.title("X"))
)
```

### 4. Handle Band scales
```rust
// OLD:
.scale_x_with::<Band>(|s| s.domain_discrete(vec![...]))

// NEW:
.x_with(col("x"), |c| c.scale_with::<Band, _>(|s| s.domain_discrete(vec![...])))
```

### 5. Add required imports
```rust
use avenger_chart::cartesian::DefaultCartesianAxis;
```

## Progress Notes

Starting with test_bar_scale_color.rs...