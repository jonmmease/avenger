# Enhanced Facet Resolution Design for avenger-chart

## Resolution System Overview

Based on research into axis and legend sharing across grammar of graphics libraries, I'm designing a Vega-Lite-style resolution system with enhanced 4-option resolution that includes row and column-specific sharing.

## Key Insights: Axis and Legend Sharing

### **Axis Sharing** (Visual Layout)
- **Shared axes**: Single set of axis labels/ticks physically shared across plots (space-efficient)
- **Independent axes**: Each plot has its own axis labels/ticks (more space, but flexible)
- **Relationship**: Shared scales enable shared axes; independent scales require independent axes

### **Legend Sharing** (Visual Layout)
- **Shared legends**: Single legend applies to all plots (consistent encoding)
- **Independent legends**: Each plot has its own legend (different categories per plot)
- **Relationship**: Shared scales with same domain → shared legend; independent scales → independent legends

## Enhanced Resolution Options

### Four-Option Resolution System

```rust
/// Enhanced resolution options with row/column specificity
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Completely shared across all facets (same domain/range)
    Shared,
    
    /// Independent per facet (each facet has its own domain/range)
    Independent,
    
    /// Shared within rows, independent across rows
    /// (facet_grid: each row has consistent domain, different rows can differ)
    SharedRows,
    
    /// Shared within columns, independent across columns  
    /// (facet_grid: each column has consistent domain, different columns can differ)
    SharedCols,
}

impl Default for Resolution {
    fn default() -> Self {
        Resolution::Shared  // Matches ggplot2/Vega-Lite statistical graphics defaults
    }
}
```

### Resolution Behavior in Different Facet Types

#### **Wrap Faceting**
- `SharedRows` / `SharedCols`: Depends on flow direction and panel arrangement
- Most useful for `facet_wrap` with `ncol` or `nrow` specified

#### **Grid Faceting**  
- `SharedRows`: All panels in same row share domain (different rows can differ)
- `SharedCols`: All panels in same column share domain (different columns can differ)
- Natural fit for 2D grid layout

## Core Types Implementation

### Resolution Configuration

```rust
/// Fine-grained resolution control for faceted plots
#[derive(Debug, Clone)]  
pub struct FacetResolve {
    /// Scale resolution per channel (data mapping)
    scales: HashMap<String, Resolution>,
    
    /// Axis resolution per positional channel (visual layout)
    axes: HashMap<String, Resolution>,
    
    /// Legend resolution per non-positional channel (visual layout)
    legends: HashMap<String, Resolution>,
}

impl FacetResolve {
    pub fn new() -> Self {
        Self {
            scales: HashMap::new(),
            axes: HashMap::new(), 
            legends: HashMap::new(),
        }
    }
    
    /// Configure scale resolution for a channel
    pub fn scale<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.scales.insert(channel.into(), resolution);
        self
    }
    
    /// Configure axis resolution for a positional channel (x, y, r, theta)
    pub fn axis<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        let channel = channel.into();
        if is_positional_channel(&channel) {
            self.axes.insert(channel, resolution);
        }
        // Silently ignore non-positional channels (or we could warn/error)
        self
    }
    
    /// Configure legend resolution for a non-positional channel (color, size, shape, etc.)
    pub fn legend<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        let channel = channel.into();
        if !is_positional_channel(&channel) {
            self.legends.insert(channel, resolution);
        }
        // Silently ignore positional channels (or we could warn/error)
        self
    }
    
    /// Get effective resolution for a channel type
    pub fn get_scale_resolution(&self, channel: &str) -> Resolution {
        self.scales.get(channel).copied().unwrap_or(Resolution::Shared)
    }
    
    pub fn get_axis_resolution(&self, channel: &str) -> Resolution {
        self.axes.get(channel).copied().unwrap_or_else(|| {
            // Default: axes follow scales unless explicitly overridden
            self.get_scale_resolution(channel)
        })
    }
    
    pub fn get_legend_resolution(&self, channel: &str) -> Resolution {
        self.legends.get(channel).copied().unwrap_or_else(|| {
            // Default: legends follow scales unless explicitly overridden
            self.get_scale_resolution(channel)
        })
    }
}

fn is_positional_channel(channel: &str) -> bool {
    matches!(channel, "x" | "y" | "r" | "theta")
}
```

### Enhanced Facet Specification

```rust
/// Enhanced faceting specification with full resolution control
#[derive(Debug, Clone)]
pub enum FacetSpec {
    Wrap {
        column: String,
        columns: Option<usize>,
        
        // Resolution system
        resolve: FacetResolve,
        
        // Layout configuration
        spacing: Option<f64>,
        strip: Option<StripConfig>,
    },
    Grid {
        row: Option<String>,
        column: Option<String>,
        
        // Resolution system
        resolve: FacetResolve,
        
        // Layout configuration  
        spacing: Option<(f64, f64)>,    // (row_spacing, col_spacing)
        strip: Option<StripConfig>,
    },
}

impl Default for FacetSpec {
    fn default() -> Self {
        FacetSpec::Wrap {
            column: String::new(),
            columns: None,
            resolve: FacetResolve::new(),  // All defaults to Shared
            spacing: None,
            strip: None,
        }
    }
}
```

### Builder Implementation

```rust
/// Enhanced facet wrap builder with resolution control
pub struct FacetWrapBuilder {
    column: String,
    columns: Option<usize>,
    resolve: FacetResolve,
    spacing: Option<f64>,
    strip: Option<StripConfig>,
}

impl FacetWrapBuilder {
    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = Some(columns);
        self
    }
    
    /// Set complete resolution configuration
    pub fn resolve(mut self, resolve: FacetResolve) -> Self {
        self.resolve = resolve;
        self
    }
    
    /// Quick scale resolution for a channel
    pub fn scale<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.scale(channel, resolution);
        self
    }
    
    /// Quick axis resolution for a positional channel
    pub fn axis<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.axis(channel, resolution);
        self
    }
    
    /// Quick legend resolution for a non-positional channel
    pub fn legend<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.legend(channel, resolution);
        self
    }
    
    pub fn spacing(mut self, spacing: f64) -> Self {
        self.spacing = Some(spacing);
        self
    }
    
    pub fn build(self) -> FacetSpec {
        FacetSpec::Wrap {
            column: self.column,
            columns: self.columns,
            resolve: self.resolve,
            spacing: self.spacing,
            strip: self.strip,
        }
    }
}

/// Enhanced facet grid builder
pub struct FacetGridBuilder {
    row: Option<String>,
    column: Option<String>,
    resolve: FacetResolve,
    spacing: Option<(f64, f64)>,
    strip: Option<StripConfig>,
}

impl FacetGridBuilder {
    pub fn row<S: Into<String>>(mut self, variable: S) -> Self {
        self.row = Some(variable.into());
        self
    }
    
    pub fn column<S: Into<String>>(mut self, variable: S) -> Self {
        self.column = Some(variable.into());
        self
    }
    
    /// Set complete resolution configuration
    pub fn resolve(mut self, resolve: FacetResolve) -> Self {
        self.resolve = resolve;
        self
    }
    
    /// Quick scale resolution for a channel
    pub fn scale<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.scale(channel, resolution);
        self
    }
    
    /// Quick axis resolution for a positional channel
    pub fn axis<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.axis(channel, resolution);
        self
    }
    
    /// Quick legend resolution for a non-positional channel
    pub fn legend<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.legend(channel, resolution);
        self
    }
    
    pub fn spacing(mut self, row_spacing: f64, col_spacing: f64) -> Self {
        self.spacing = Some((row_spacing, col_spacing));
        self
    }
    
    pub fn build(self) -> FacetSpec {
        FacetSpec::Grid {
            row: self.row,
            column: self.column,
            resolve: self.resolve,
            spacing: self.spacing,
            strip: self.strip,
        }
    }
}

impl Facet {
    pub fn wrap<S: Into<String>>(column: S) -> FacetWrapBuilder {
        FacetWrapBuilder {
            column: column.into(),
            columns: None,
            resolve: FacetResolve::new(),
            spacing: None,
            strip: None,
        }
    }
    
    pub fn grid() -> FacetGridBuilder {
        FacetGridBuilder {
            row: None,
            column: None,
            resolve: FacetResolve::new(),
            spacing: None,
            strip: None,
        }
    }
}
```

## Usage Examples

### Basic Resolution Control

```rust
// Default: everything shared (good for comparison)
let plot = Plot::new(Cartesian)
    .data(df)
    .facet_wrap("continent")  // All default to Resolution::Shared
    .mark(Symbol::new().x("gdp").y("life"));

// Independent scales (good for individual focus)
let plot = Plot::new(Cartesian)
    .data(df)
    .facet_wrap("continent")
        .scale("x", Resolution::Independent)
        .scale("y", Resolution::Independent)
    .mark(Symbol::new().x("gdp").y("life"));
```

### Row/Column-Specific Sharing

```rust
// Grid: shared within rows, independent across rows
let plot = Plot::new(Cartesian)
    .data(df)
    .facet_grid()
        .row("continent")
        .column("year")
        .scale("y", Resolution::SharedRows)  // Each row has same y-scale
        .scale("x", Resolution::SharedCols)  // Each column has same x-scale
    .mark(Symbol::new().x("gdp").y("life"));

// Wrap: useful with explicit column layout
let plot = Plot::new(Cartesian)
    .data(df)
    .facet_wrap("metric")
        .columns(3)  // 3 columns of facets
        .scale("y", Resolution::SharedCols)  // Each column shares y-scale
    .mark(Line::new().x("date").y("value"));
```

### Advanced Resolution Scenarios

```rust
// Mixed resolution: compare patterns (shared x) but see individual ranges (independent y)
let plot = Plot::new(Cartesian)
    .data(df)
    .facet_wrap("continent")
        .resolve(FacetResolve::new()
            .scale("x", Resolution::Shared)         // Same time axis
            .scale("y", Resolution::Independent)    // Different value ranges
            .scale("color", Resolution::Shared)     // Consistent color meaning
            .axis("x", Resolution::Shared)          // Single x-axis (space-efficient)
            .axis("y", Resolution::Independent)     // Multiple y-axes (necessary)
            .legend("color", Resolution::Shared))   // Single legend (consistent)
    .mark(Line::new()
        .x("year")
        .y("gdp_per_capita")
        .color("development_status"));

// Grid with sophisticated resolution
let plot = Plot::new(Cartesian)
    .data(economic_data)
    .facet_grid()
        .row("region")       // Regions as rows
        .column("metric")    // GDP, unemployment, etc. as columns
        .resolve(FacetResolve::new()
            .scale("x", Resolution::Shared)         // Same time axis everywhere
            .scale("y", Resolution::SharedCols)     // Same metric scale per column
            .scale("color", Resolution::SharedRows) // Same country colors per region
            .axis("x", Resolution::Shared)          // Single time axis
            .axis("y", Resolution::SharedCols)      // Metric-specific y-axes
            .legend("color", Resolution::SharedRows)) // Region-specific legends
    .mark(Line::new()
        .x("year")
        .y("value")
        .color("country"));
```

### Convenience Methods

```rust
// Common patterns as convenience methods on the plot
impl<C: CoordinateSystem> Plot<C> {
    /// Create facet wrap with independent scales (ggplot2 scales="free")
    pub fn facet_wrap_free<S: Into<String>>(mut self, column: S) -> Self {
        self.facet_spec = Some(FacetSpec::Wrap {
            column: column.into(),
            columns: None,
            resolve: FacetResolve::new()
                .scale("x", Resolution::Independent)
                .scale("y", Resolution::Independent),
            spacing: None,
            strip: None,
        });
        self
    }
    
    /// Create facet wrap with free y-scales (ggplot2 scales="free_y")
    pub fn facet_wrap_free_y<S: Into<String>>(mut self, column: S) -> Self {
        self.facet_spec = Some(FacetSpec::Wrap {
            column: column.into(),
            columns: None,
            resolve: FacetResolve::new()
                .scale("y", Resolution::Independent),
            spacing: None,
            strip: None,
        });
        self
    }
}
```

## Implementation Considerations

### Scale Domain Calculation

```rust
/// Helper for calculating domains with different resolution strategies
impl FacetResolve {
    pub fn calculate_domains<T>(&self, facet_data: &HashMap<FacetKey, DataFrame>) -> HashMap<String, ScaleDomain> {
        let mut domains = HashMap::new();
        
        for (channel, resolution) in &self.scales {
            let domain = match resolution {
                Resolution::Shared => {
                    // Union all data across all facets
                    union_domains(facet_data.values(), channel)
                }
                Resolution::Independent => {
                    // Each facet gets its own domain (handled per-facet)
                    continue;
                }
                Resolution::SharedRows => {
                    // Union within rows, calculate separately
                    calculate_row_domains(facet_data, channel)
                }
                Resolution::SharedCols => {
                    // Union within columns, calculate separately  
                    calculate_col_domains(facet_data, channel)
                }
            };
            domains.insert(channel.clone(), domain);
        }
        
        domains
    }
}
```

### Rendering Pipeline Integration

The resolution system integrates with the rendering pipeline:
1. **Domain Calculation**: Calculate domains based on resolution strategy
2. **Scale Creation**: Create scales per facet group (shared/independent/row/col)
3. **Axis Rendering**: Render axes based on axis resolution (shared vs independent layout)
4. **Legend Rendering**: Render legends based on legend resolution (single vs multiple)

## Benefits of This Design

1. **Flexible Control**: Four resolution options cover all common use cases
2. **Vega-Lite Compatibility**: Familiar resolution concept with enhancements
3. **Grid-Optimized**: SharedRows/SharedCols natural for 2D grid layouts
4. **Clear Separation**: Distinct control over scales (data) vs axes/legends (presentation)
5. **Progressive Enhancement**: Simple defaults with advanced control available
6. **Type Safety**: Rust's type system prevents invalid configurations

This design provides a powerful and intuitive system for controlling how scales, axes, and legends are shared across faceted plots, giving users precise control over both data mapping and visual presentation.