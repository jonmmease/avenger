# Polar Coordinate System Implementation Plan

## Executive Summary
This plan outlines the implementation of polar coordinate support in avenger-chart with meaningful checkpoints for testing and validation at key milestones.

## Implementation Milestones

## Milestone 1: Core Polar Infrastructure with Basic Rendering
**Testable Output**: Working polar scatter plots with proper coordinate transformation

### Components:
1. **PolarAxis Type Definition** (`avenger-chart/src/axis.rs`)
   - Complete PolarAxis struct with all needed fields
   - Implement AxisTrait properly from the start
   - Include PolarAxisType (Radial/Angular) and configuration options

2. **Polar CoordinateSystem Implementation** (`avenger-chart/src/coords.rs`)
   - Update type Axis = PolarAxis
   - Implement proper transform_expressions with center calculation
   - Add center injection mechanism (either via trait method or struct fields)
   - Return empty axes from create_default_axes initially (not a hack - genuinely no axes yet)

3. **PlotRenderer Center Injection** (`avenger-chart/src/render.rs`)
   - Calculate proper center based on plot dimensions
   - Pass center to Polar coordinate system for transformation
   - This is the correct long-term approach, not a hack

### Testing Checkpoint 1:
```rust
// Create a polar scatter plot
let plot = Plot::new(Polar)
    .scale_r(|s| s.domain_interval(lit(0.0), lit(100.0)))
    .scale_theta(|s| s.domain_interval(lit(0.0), lit(2.0 * PI)))
    .mark(Symbol::new()
        .r(col("radius"))
        .theta(col("angle"))
        .fill(col("category")));

// Should render symbols in correct polar positions
// No axes or grid yet, but coordinate system fully functional
```

**Validation**: 
- Symbols appear at correct polar positions
- Centering works for different canvas sizes
- All scales (color, size, shape) work correctly
- No temporary code - this is the foundation

---

## Milestone 2: Grid and Scale Integration
**Testable Output**: Polar plots with scale-driven grid lines (no labels yet)

### Components:
1. **Grid Rendering in Polar.render_axes()** (`avenger-chart/src/coords.rs`)
   - Implement render_axes to create grid marks
   - Use scale.ticks() to determine grid positions
   - Create concentric circles for radial grid
   - Create radial lines for angular grid
   - Respect grid boolean flag from axes

2. **Default Axes Creation** (`avenger-chart/src/coords.rs`)
   - Implement create_default_axes() properly
   - Create PolarAxis instances for r and theta channels
   - Set sensible defaults (grid=true for continuous scales)
   - Extract titles from mark encodings

3. **Scale Range Updates**
   - Ensure r scale gets proper range based on plot radius
   - Ensure theta scale defaults to [0, 2π] for continuous scales

### Testing Checkpoint 2:
```rust
// Test with different scale types
let linear_plot = Plot::new(Polar)
    .scale_r(|s| s.scale_type("linear").nice(true))
    .scale_theta(|s| s.scale_type("linear"))
    .axis_r(|a| a.grid(true))
    .axis_theta(|a| a.grid(true))
    .mark(Symbol::new()...);

let ordinal_plot = Plot::new(Polar)
    .scale_theta(|s| s.scale_type("ordinal")
        .domain_discrete(vec!["N", "E", "S", "W"]))
    .mark(Symbol::new()...);

// Should show appropriate grid patterns
// Grid positions match scale ticks
```

**Validation**:
- Grid circles appear at radial scale tick positions
- Grid lines appear at angular scale tick positions  
- Different scale types produce appropriate grids
- Grid can be toggled on/off via axis configuration

---

## Milestone 3: Full Axis Implementation with Labels
**Testable Output**: Complete polar axes with ticks, labels, and titles

### Components:
1. **Polar Axis Modules in avenger-guides**
   - `avenger-guides/src/axis/polar_radial.rs`
     - Tick marks along a baseline (horizontal line from center)
     - Labels positioned along baseline
     - Title below labels
     - Grid circles (when enabled)
   
   - `avenger-guides/src/axis/polar_angular.rs`
     - Tick marks on outer circle
     - Labels outside circle with smart positioning
     - Grid lines from center (when enabled)
     - Handle different scale types appropriately

2. **Update Polar.render_axes()** (`avenger-chart/src/coords.rs`)
   - Call appropriate polar axis functions from avenger-guides
   - Pass correct parameters (center, radius, scales)
   - Combine radial and angular axis marks

3. **Axis Configuration API**
   - Add axis_r() and axis_theta() methods to Plot<Polar>
   - Support all PolarAxis configuration options
   - Ensure axis specs work like Cartesian (transformation functions)

### Testing Checkpoint 3:
```rust
let plot = Plot::new(Polar)
    .scale_r(|s| s.domain_interval(lit(0.0), lit(100.0)))
    .scale_theta(|s| s.domain_interval(lit(0.0), lit(360.0)))
    .axis_r(|a| a
        .title("Distance (km)")
        .format_number(".1f")
        .grid(true))
    .axis_theta(|a| a
        .title("Direction")
        .format_number(".0f°")
        .grid(true))
    .mark(Symbol::new()...);

// Should show complete axes with labels and titles
```

**Validation**:
- Tick marks appear at correct positions
- Labels are readable and properly positioned
- Number formatting works
- Titles appear in sensible locations
- Smart label positioning prevents overlaps

---

## Milestone 4: Layout System Integration
**Testable Output**: Polar plots with dynamic layout, legends, and titles

### Components:
1. **Enable Dynamic Layout** (`avenger-chart/src/coords.rs`)
   - Set supports_dynamic_layout() to return true
   - This enables Taffy layout system

2. **Polar Layout Measurements** (`avenger-chart/src/chart_layout.rs`)
   - Implement measure_polar_axes() (or adapt existing measurement)
   - Calculate axis overflow (how far labels extend beyond circle)
   - Determine required padding for each side

3. **Legend and Title Support**
   - Legends should work automatically (same as Cartesian)
   - Titles/subtitles should position correctly
   - Test multiple legend positions

### Testing Checkpoint 4:
```rust
let plot = Plot::new(Polar)
    .title("Wind Speed and Direction")
    .subtitle("2024 Data")
    .scale_r(|s| s.domain_interval(lit(0.0), lit(50.0)))
    .scale_theta(|s| s.domain_interval(lit(0.0), lit(360.0)))
    .scale_fill(|s| s.scale_type("ordinal"))
    .legend_fill(|l| l.position(LegendPosition::Right))
    .mark(Symbol::new()
        .r(col("speed"))
        .theta(col("direction"))
        .fill(col("category")));

// Full layout with all components properly positioned
```

**Validation**:
- Layout adjusts to accommodate axis labels
- Legends appear in correct positions
- Title and subtitle are properly placed
- Plot remains centered within available space
- Different canvas sizes work correctly

---

## Milestone 5: Polish and Additional Features
**Testable Output**: Production-ready polar plots with advanced features

### Components:
1. **Advanced Configuration**
   - Start angle configuration (where 0° is)
   - Direction (clockwise/counterclockwise)
   - Custom radius origin (not always 0)
   - Partial angular ranges (e.g., semicircle)

2. **Visual Polish**
   - Fine-tune label positioning algorithms
   - Add subtle visual improvements
   - Optimize performance if needed

3. **Additional Mark Support** (Optional)
   - Polar line marks (for radar charts)
   - Polar area marks (for rose charts)

### Testing Checkpoint 5:
```rust
let plot = Plot::new(Polar)
    .polar_config(|p| p
        .start_angle(90.0)  // Start at top
        .direction(PolarDirection::Clockwise))
    .scale_theta(|s| s.domain_interval(lit(0.0), lit(180.0))) // Semicircle
    .mark(Symbol::new()...);

// Advanced polar configurations work correctly
```

---

## Development Approach

### Key Principles:
1. **No temporary hacks** - Each milestone produces production-quality code
2. **Complete components** - When we add something, we add it properly
3. **Testable milestones** - Each milestone has meaningful visual output
4. **Progressive enhancement** - Each milestone builds on solid foundation

### Testing Strategy:
- **Unit tests**: Add tests for each new component
- **Visual tests**: Create visual regression tests at each milestone
- **Integration tests**: Ensure previous functionality remains intact
- **Example evolution**: Build one example that grows with each milestone

### File Organization:
```
avenger-chart/
├── src/
│   ├── axis.rs          (Add PolarAxis in M1)
│   ├── coords.rs        (Update Polar in M1-M4)
│   └── render.rs        (Update in M1, M4)
├── tests/
│   └── test_polar.rs    (Add tests at each milestone)
└── examples/
    └── polar-plot/      (Evolving example)

avenger-guides/
└── src/
    └── axis/
        ├── polar_radial.rs   (Add in M3)
        └── polar_angular.rs  (Add in M3)
```

## Risk Mitigation

### Potential Challenges:
1. **Center injection mechanism**: Design carefully in M1 to avoid refactoring
2. **Label positioning**: May need iteration in M3, but core functionality works
3. **Layout measurements**: M4 might reveal edge cases, but won't break core
4. **Scale tick generation**: Leverage existing scale infrastructure

### Fallback Options:
- After M1: Have working polar plots (even without axes)
- After M2: Have visual reference grid
- After M3: Have complete polar visualization
- After M4: Have production-ready system

## Success Metrics

Each milestone should achieve:
1. **Functional**: The targeted feature works correctly
2. **Clean**: No temporary code or hacks
3. **Tested**: Comprehensive test coverage
4. **Documented**: Clear examples and documentation
5. **Integrated**: Works with rest of the system

## Conclusion

This plan provides five meaningful checkpoints where we can validate progress without introducing temporary hacks. Each milestone delivers real value and builds toward a complete, production-ready polar coordinate system. The implementation is clean and follows existing patterns, ensuring the code is maintainable and extensible.