# Multi-Dimensional Coordinate Systems

## Overview

Extends Avenger's coordinate system to support multi-dimensional visualizations like Parallel Coordinates and Radar Charts using repeated position channels (`.y()`, `.r()`). This maintains API consistency while supporting variable-dimension visualizations.

## Core Concept

Instead of introducing new APIs, extend the existing channel pattern to support repetition:

```rust
// Traditional scatter plot - single x and y
.mark(Symbol::new()
    .x(col("weight"))
    .y(col("mpg")))

// Parallel coordinates - multiple y channels
.mark(ParallelLine::new()
    .y(col("mpg"))        // First axis
    .y(col("cylinders"))  // Second axis
    .y(col("weight"))     // Third axis
    .stroke(col("brand")))
```

### Key Principles

1. **Order Matters**: The sequence of channel calls determines axis order
2. **Each Gets Configuration**: Every channel can have its own scale and axis
3. **Consistent API**: Uses familiar `.y_with()` pattern for configuration
4. **Type Safety**: Compile-time checking where possible, runtime validation where needed

## Channel Storage

```rust
pub struct ParallelLine {
    y_channels: Vec<PositionChannelConfig>,  // Ordered list
    stroke: Option<ColorChannelConfig>,
    stroke_width: Option<f32>,
}

impl ParallelLine {
    pub fn y(mut self, expr: impl Into<Expr>) -> Self {
        self.y_channels.push(PositionChannelConfig::new(expr.into()));
        self
    }

    pub fn y_with<F>(mut self, expr: impl Into<Expr>, f: F) -> Self
    where F: FnOnce(PositionChannelConfig) -> PositionChannelConfig
    {
        let config = f(PositionChannelConfig::new(expr.into()));
        self.y_channels.push(config);
        self
    }
}
```

## Scale Registry

```rust
impl ParallelCoords {
    fn register_scales(&self, mark: &ParallelLine) -> HashMap<String, ConfiguredScale> {
        let mut scales = HashMap::new();

        for (idx, y_config) in mark.y_channels.iter().enumerate() {
            // Create unique channel name for each dimension
            let channel_name = y_config.expr
                .as_column_name()
                .unwrap_or_else(|| format!("y_{}", idx));

            scales.insert(channel_name, y_config.scale.clone());
        }

        scales
    }
}
```

## Coordinate System Implementation

```rust
impl CoordinateSystem for ParallelCoords {
    type Guide = ParallelGuide;

    fn transform_to_visual(
        &self,
        data: &DataView,
        mark: &dyn Mark<Self>,
        plot_width: f32,
        plot_height: f32,
    ) -> VisualGeometry {
        let y_channels = mark.get_y_channels();
        let num_axes = y_channels.len();

        // Calculate x positions for each axis
        let axis_spacing = plot_width / (num_axes - 1) as f32;
        let x_positions: Vec<f32> = (0..num_axes)
            .map(|i| i as f32 * axis_spacing)
            .collect();

        // Transform each data record to a polyline
        let mut polylines = Vec::new();
        for row in data.rows() {
            let mut vertices = Vec::new();

            for (idx, y_channel) in y_channels.iter().enumerate() {
                let value = y_channel.expr.evaluate(row);
                let scale = &scales[&y_channel.scale_name];
                let y = scale.transform(value);

                vertices.push((x_positions[idx], y));
            }

            polylines.push(vertices);
        }

        VisualGeometry::Polylines(polylines)
    }
}
```

## Guide System

```rust
pub struct ParallelGuide {
    axes: Vec<ParallelAxis>,
    axis_spacing: f32,
    plot_background_color: Option<[f32; 4]>,
}

impl ParallelGuide {
    fn create_from_marks(marks: &[Box<dyn Mark<ParallelCoords>>]) -> Self {
        let mut axes = Vec::new();

        if let Some(mark) = marks.first() {
            for y_channel in mark.get_y_channels() {
                axes.push(ParallelAxis {
                    title: y_channel.axis.title.clone(),
                    grid: y_channel.axis.grid,
                    // ... other axis properties
                });
            }
        }

        Self {
            axes,
            axis_spacing: 120.0,
            plot_background_color: None,
        }
    }
}
```

## Usage Examples

### Parallel Coordinates

```rust
let plot = Plot::<ParallelCoords>::new()
    .data(cars_df)
    .guide(|g| g
        .axis_spacing(100.0)
        .plot_background_color([0.98, 0.98, 0.98, 1.0]))
    .mark(
        ParallelLine::new()
            .y_with(col("mpg"), |c| c
                .scale(|s| s.linear().domain([0.0, 50.0]))
                .axis(|a| a.title("Miles per Gallon").grid(true)))

            .y_with(col("cylinders"), |c| c
                .scale(|s| s.linear().nice(false))
                .axis(|a| a.title("Cylinders")))

            .y_with(col("horsepower") / col("weight"), |c| c
                .scale(|s| s.sqrt())
                .axis(|a| a.title("Power/Weight")))

            .stroke_with(col("origin"), |c| c
                .scale(|s| s.categorical())
                .legend(true))
            .opacity(0.6)
    );
```

### Radar Chart

```rust
let plot = Plot::<Radar>::new()
    .data(pokemon_stats)
    .guide(|g| g
        .grid_style(GridStyle::Polygonal)
        .grid_levels(5))
    .mark(
        RadarArea::new()
            .r_with(col("hp"), |c| c
                .scale(|s| s.linear().domain([0.0, 255.0]))
                .axis(|a| a.title("HP")))

            .r_with(col("attack"), |c| c
                .scale(|s| s.linear().domain([0.0, 255.0]))
                .axis(|a| a.title("Attack")))

            .r_with(col("defense"), |c| c
                .scale(|s| s.linear().domain([0.0, 255.0]))
                .axis(|a| a.title("Defense")))

            .r_with(col("speed"), |c| c
                .scale(|s| s.linear().domain([0.0, 255.0]))
                .axis(|a| a.title("Speed")))

            .fill_with(col("type"), |c| c
                .scale(|s| s.categorical())
                .legend(true))
            .opacity(0.5)
    );
```

## Implementation Plan

### Phase 1: Core Infrastructure
1. Add `ChannelMultiplicity` trait to distinguish single vs multiple channels
2. Update `Mark` trait to support channel introspection
3. Extend scale registry to handle indexed channel names

### Phase 2: Parallel Coordinates
1. Implement `ParallelCoords` coordinate system
2. Create `ParallelLine` mark
3. Design `ParallelGuide` and `ParallelAxis`
4. Add transform logic for polyline generation

### Phase 3: Radar Charts
1. Implement `Radar` coordinate system (extends polar)
2. Create `RadarArea` and `RadarLine` marks
3. Design `RadarGuide` with radial axes
4. Add transform logic for polygon generation

### Phase 4: Testing & Polish
1. Visual regression tests
2. Performance optimization for many dimensions
3. Interactive features (axis reordering, brushing)
4. Documentation and examples

## Edge Cases

### Mixed Mark Types
What if a plot has marks with different numbers of dimensions?
- **Solution**: Error at runtime, all marks must have same dimension count

### Missing Values
How to handle nulls in parallel coordinates?
- **Solution**: Break the line, or provide option to interpolate/skip

### Dimension Scaling
Should all dimensions share a scale or have independent scales?
- **Solution**: Default to independent, provide normalization option

### Large Number of Dimensions
Performance with 50+ dimensions?
- **Solution**: Implement dimension sampling, progressive rendering

### Interactive Features
- **Axis reordering**: Drag to reorder parallel axes
- **Brushing**: Select ranges on each axis
- **Highlighting**: Hover to highlight individual polylines

## Future Extensions

- **Spider/Kiviat diagrams**: Variant of radar charts
- **Coordinate parallel plots**: 3D version with perspective
- **Star glyphs**: Mini radar charts as point marks
- **Andrew's curves**: Fourier transform visualization
- **Radviz**: Radial visualization of multi-dimensional data

## Conclusion

The repeated channel pattern provides a natural extension of Avenger's API to support multi-dimensional coordinate systems. By reusing familiar concepts (channels, scales, axes) in a new way (repetition to indicate dimensions), we maintain consistency while enabling powerful visualizations.

The key insight: multi-dimensional coordinate systems are just systems where marks can have multiple instances of the same position channel type, transformed into connected shapes rather than individual points.
