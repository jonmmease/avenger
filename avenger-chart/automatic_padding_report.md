# Automatic Padding Determination in Visualization Libraries

## Executive Summary

Automatic padding determination is a critical feature in visualization libraries that ensures charts are properly sized to accommodate all visual elements (axes, labels, titles, legends) while maintaining proper alignment in multi-chart layouts. This report analyzes how major visualization libraries handle this challenge and provides recommendations for avenger-chart using Taffy as the layout engine.

## Core Challenges

1. **Text Measurement**: Accurate measurement of text dimensions depends on font loading and rendering context
2. **Cascading Dependencies**: Padding affects scale ranges, which affects tick generation, which affects padding
3. **Multi-Chart Alignment**: Subplots must align on data regions, not outer boundaries
4. **Performance**: Layout calculations can be expensive, especially with many subplots
5. **Convergence**: Iterative algorithms may not always reach a stable solution

## Library Analysis

### 1. Matplotlib

#### Tight Layout Algorithm
```python
# Simplified algorithm from matplotlib/_tight_layout.py
def auto_adjust_subplotpars(fig, renderer, nrows_ncols, num1num2_list, 
                           subplot_list, ax_bbox_list=None,
                           pad=1.08, h_pad=None, w_pad=None, rect=None):
    """
    1. Get bounding boxes of all artists (axes, labels, etc.)
    2. Calculate required space on each side
    3. Adjust subplot parameters to prevent overlaps
    4. Validate that solution fits within figure
    """
```

**Key Insights:**
- Renders all text elements to get exact dimensions
- Uses padding as fraction of font size (default 1.08)
- Iterative approach that may not converge
- Separate handling for colorbar padding

#### Constrained Layout Engine
- Modern replacement for tight_layout
- Uses constraint solver (similar to CSS flexbox)
- Each element has a "layoutbox" with constraints
- Better handles complex nested layouts

**Algorithm:**
```
1. Create layoutbox hierarchy
2. Add constraints (e.g., "colorbar width = 0.05 * axes width")
3. Solve constraint system
4. Apply solution to actual elements
```

### 2. D3.js

#### Margin Convention
```javascript
// Standard D3 margin convention
const margin = {top: 20, right: 30, bottom: 40, left: 50};
const width = outerWidth - margin.left - margin.right;
const height = outerHeight - margin.top - margin.bottom;

const svg = d3.select("body").append("svg")
    .attr("width", outerWidth)
    .attr("height", outerHeight);

const g = svg.append("g")
    .attr("transform", `translate(${margin.left},${margin.top})`);
```

**Dynamic Margin Calculation:**
```javascript
// Measure text to determine required margins
function measureAxisMargin(axis, scale) {
    const tempG = svg.append("g").style("visibility", "hidden");
    const tempAxis = tempG.append("g").call(axis.scale(scale));
    const bbox = tempAxis.node().getBBox();
    tempG.remove();
    return bbox;
}
```

### 3. Vega-Lite

#### Autosize Configuration
```json
{
  "autosize": {
    "type": "fit",      // or "pad", "none"
    "contains": "padding", // what to include in size
    "resize": true      // allow dynamic resizing
  }
}
```

**Multi-View Alignment:**
```json
{
  "concat": [...],
  "resolve": {
    "scale": {"x": "shared"},
    "axis": {"x": "independent"}
  },
  "spacing": 10,
  "align": "all"  // or "each", "none"
}
```

### 4. ggplot2

#### gtable System
```r
# Simplified gtable layout process
build_plot <- function(plot) {
  # 1. Convert layers to grobs
  panel_grobs <- lapply(panels, ggplotGrob)
  
  # 2. Create gtable with appropriate dimensions
  gt <- gtable(widths = unit(rep(1, ncol), "null"),
               heights = unit(rep(1, nrow), "null"))
  
  # 3. Add panels with consistent alignment
  for (i in seq_along(panel_grobs)) {
    gt <- gtable_add_grob(gt, panel_grobs[[i]], ...)
  }
  
  # 4. Add axes, strips, legends
  # 5. Calculate final dimensions
}
```

### 5. Plotly

#### Margin Autoexpansion
```javascript
// Plotly's automargin implementation
layout: {
  margin: {l: 50, r: 50, t: 50, b: 50},
  xaxis: {automargin: true},  // Expand margin if needed
  yaxis: {automargin: true}
}
```

## Common Patterns and Algorithms

### 1. Text Measurement Strategies

#### Browser-Based (Canvas API)
```javascript
function measureText(text, font) {
  const canvas = document.createElement('canvas');
  const context = canvas.getContext('2d');
  context.font = font;
  const metrics = context.measureText(text);
  return {
    width: metrics.width,
    height: metrics.actualBoundingBoxAscent + 
            metrics.actualBoundingBoxDescent
  };
}
```

#### SVG-Based
```javascript
function measureSVGText(text, styles) {
  const svg = d3.create('svg');
  const textEl = svg.append('text')
    .style(styles)
    .text(text);
  document.body.appendChild(svg.node());
  const bbox = textEl.node().getBBox();
  svg.remove();
  return bbox;
}
```

### 2. Iterative Layout Algorithm

```python
def iterative_layout(charts, max_iterations=5):
    """Generic iterative layout algorithm"""
    for iteration in range(max_iterations):
        # Step 1: Measure all elements
        measurements = []
        for chart in charts:
            m = measure_chart_elements(chart)
            measurements.append(m)
        
        # Step 2: Calculate required padding
        padding = calculate_unified_padding(measurements)
        
        # Step 3: Apply padding and check convergence
        changed = False
        for chart, m in zip(charts, measurements):
            if apply_padding(chart, padding):
                changed = True
        
        if not changed:
            break
    
    return padding
```

### 3. Constraint-Based Layout

```python
class LayoutConstraint:
    """Constraint-based layout similar to CSS Flexbox"""
    
    def __init__(self):
        self.constraints = []
        self.variables = {}
    
    def add_constraint(self, constraint_type, params):
        # e.g., "left_margin >= text_width + 10"
        self.constraints.append((constraint_type, params))
    
    def solve(self):
        # Use linear programming solver
        # Return optimal variable values
        pass
```

### 4. Multi-Chart Alignment Strategies

#### Maximum Padding Strategy
```python
def align_charts_max_padding(charts):
    """Align by using maximum padding across all charts"""
    max_padding = {'left': 0, 'right': 0, 'top': 0, 'bottom': 0}
    
    # Find maximum required padding
    for chart in charts:
        padding = calculate_padding(chart)
        for side in max_padding:
            max_padding[side] = max(max_padding[side], padding[side])
    
    # Apply uniform padding
    for chart in charts:
        apply_padding(chart, max_padding)
```

#### Grid-Based Alignment
```python
def align_grid_layout(charts, rows, cols):
    """Align charts in grid with shared dimensions"""
    # Calculate padding per row/column
    row_padding = [{'top': 0, 'bottom': 0} for _ in range(rows)]
    col_padding = [{'left': 0, 'right': 0} for _ in range(cols)]
    
    # Measure all charts
    for i, chart in enumerate(charts):
        row = i // cols
        col = i % cols
        padding = calculate_padding(chart)
        
        row_padding[row]['top'] = max(row_padding[row]['top'], padding['top'])
        row_padding[row]['bottom'] = max(row_padding[row]['bottom'], padding['bottom'])
        col_padding[col]['left'] = max(col_padding[col]['left'], padding['left'])
        col_padding[col]['right'] = max(col_padding[col]['right'], padding['right'])
    
    # Apply row/column-specific padding
    for i, chart in enumerate(charts):
        row = i // cols
        col = i % cols
        apply_padding(chart, {
            'top': row_padding[row]['top'],
            'bottom': row_padding[row]['bottom'],
            'left': col_padding[col]['left'],
            'right': col_padding[col]['right']
        })
```

## Recommendations for Avenger-Chart Using Taffy

### 1. Leverage Taffy as the Layout Engine

Since avenger-chart already uses Taffy and has geometry measurement via avenger-geometry, we can express the padding problem entirely within Taffy's constraint system:

- **Taffy's measure functions** for dynamic content sizing (axes, labels, legends)
- **CSS Grid/Flexbox** for chart component layout
- **Intrinsic sizing** for text elements via avenger-geometry measurements
- **Automatic space distribution** between fixed and flexible elements

### 2. Implementation Strategy with Taffy

```rust
use taffy::prelude::*;
use avenger_geometry::BoundingBox;

/// Context for measuring chart elements
pub enum ChartNodeContext {
    Title { text: String },
    AxisLabel { labels: Vec<String>, rotation: f32 },
    Legend { items: Vec<LegendItem> },
    ChartArea { aspect_ratio: Option<f32> },
    TickLabels { ticks: Vec<TickLabel> },
}

/// Measure function that integrates with avenger-geometry
pub fn chart_measure_function(
    known_dimensions: Size<Option<f32>>,
    available_space: Size<AvailableSpace>,
    node_context: Option<&mut ChartNodeContext>,
    scene_graph: &SceneGraph,  // From avenger-scenegraph
) -> Size<f32> {
    match node_context {
        Some(ChartNodeContext::AxisLabel { labels, rotation }) => {
            // Use avenger-geometry to measure text bounding boxes
            let mut max_width = 0.0;
            let mut total_height = 0.0;
            
            for label in labels {
                let bbox = measure_text_bounds(label, rotation, scene_graph);
                max_width = max_width.max(bbox.width());
                total_height += bbox.height();
            }
            
            Size { width: max_width, height: total_height }
        },
        Some(ChartNodeContext::Legend { items }) => {
            // Measure legend based on items
            measure_legend_bounds(items, scene_graph)
        },
        Some(ChartNodeContext::ChartArea { aspect_ratio }) => {
            // Maintain aspect ratio while fitting available space
            match (aspect_ratio, known_dimensions.width, known_dimensions.height) {
                (Some(ratio), Some(width), None) => Size { width, height: width / ratio },
                (Some(ratio), None, Some(height)) => Size { width: height * ratio, height },
                _ => Size { width: 0.0, height: 0.0 },
            }
        },
        _ => Size::ZERO,
    }
}
```

### 3. Chart Layout Structure with Taffy

```rust
pub fn build_chart_layout(plot: &Plot) -> (TaffyTree<ChartNodeContext>, NodeId) {
    let mut tree = TaffyTree::new();
    
    // Root container (outer dimensions)
    let root_style = Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::length(plot.width),
            height: Dimension::length(plot.height),
        },
        ..Default::default()
    };
    
    // Title node with intrinsic sizing
    let title_node = tree.new_leaf_with_context(
        Style {
            // Title uses measure function for height
            ..Default::default()
        },
        ChartNodeContext::Title { text: plot.title.clone() },
    ).unwrap();
    
    // Main area (contains y-axis, chart, legend)
    let main_area_style = Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,  // Take remaining space
        ..Default::default()
    };
    
    // Y-axis with intrinsic width from labels
    let y_axis_node = tree.new_leaf_with_context(
        Style {
            // Width determined by measure function
            ..Default::default()
        },
        ChartNodeContext::AxisLabel { 
            labels: plot.get_y_tick_labels(),
            rotation: 0.0,
        },
    ).unwrap();
    
    // Chart area with aspect ratio constraint
    let chart_area_node = tree.new_leaf_with_context(
        Style {
            flex_grow: 1.0,  // Take remaining width
            aspect_ratio: plot.aspect_ratio,
            ..Default::default()
        },
        ChartNodeContext::ChartArea { 
            aspect_ratio: plot.aspect_ratio,
        },
    ).unwrap();
    
    // Build the tree
    let main_area = tree.new_with_children(main_area_style, &[
        y_axis_node,
        chart_area_node,
        // legend_node if needed
    ]).unwrap();
    
    let root = tree.new_with_children(root_style, &[
        title_node,
        main_area,
        // x_axis_node
    ]).unwrap();
    
    // Set measure functions
    tree.set_measure_func(title_node, chart_measure_function);
    tree.set_measure_func(y_axis_node, chart_measure_function);
    // ... set for other nodes
    
    (tree, root)
}
```

### 4. Multi-Chart Alignment with Taffy

For subplot layouts, Taffy's grid system naturally handles alignment:

```rust
pub fn build_subplot_grid(plots: Vec<Plot>, rows: usize, cols: usize) -> Layout {
    let mut tree = TaffyTree::new();
    
    // Grid container
    let grid_style = Style {
        display: Display::Grid,
        grid_template_columns: vec![fr(1.0); cols],
        grid_template_rows: vec![fr(1.0); rows],
        gap: Size { 
            width: length(20.0),  // Horizontal gap
            height: length(20.0), // Vertical gap
        },
        ..Default::default()
    };
    
    // Create subplot nodes
    let mut subplot_nodes = Vec::new();
    for (i, plot) in plots.iter().enumerate() {
        let (subplot_tree, subplot_root) = build_chart_layout(plot);
        
        // Taffy automatically aligns grid children
        // The chart areas will align because padding is computed
        // independently for each subplot
        subplot_nodes.push(subplot_root);
    }
    
    // For shared axes alignment, use CSS Grid's alignment features
    let aligned_grid_style = Style {
        display: Display::Grid,
        grid_template_columns: vec![auto(), fr(1.0), auto()], // [y-axis, chart, legend]
        grid_template_rows: vec![auto(), fr(1.0), auto()],    // [title, chart, x-axis]
        // Align all chart areas
        align_items: Some(AlignItems::Stretch),
        ..grid_style
    };
}
```

### 5. Advantages of Using Taffy

1. **No Custom Constraint Solver Needed**: Taffy is a proven, optimized layout engine
2. **Standards Compliant**: Uses CSS Flexbox/Grid semantics familiar to developers
3. **Automatic Reflow**: Changes propagate automatically through the layout tree
4. **Performance**: Highly optimized with caching and incremental layout
5. **Measure Functions**: Perfect integration point for avenger-geometry measurements

### 6. Integration Pattern

```rust
impl Plot {
    /// Compute the final layout including automatic padding
    pub fn compute_layout(&self) -> ComputedLayout {
        // 1. Build Taffy tree structure
        let (mut tree, root) = build_chart_layout(self);
        
        // 2. Measure all text elements using avenger-geometry
        // This happens automatically via measure functions
        
        // 3. Compute layout
        tree.compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(self.width),
                height: AvailableSpace::Definite(self.height),
            },
        ).unwrap();
        
        // 4. Extract computed positions
        let layout = tree.layout(root).unwrap();
        let chart_area_bounds = extract_chart_area_bounds(&tree, chart_area_node);
        
        ComputedLayout {
            chart_area: chart_area_bounds,
            scale_x_range: (chart_area_bounds.left, chart_area_bounds.right),
            scale_y_range: (chart_area_bounds.bottom, chart_area_bounds.top),
            // ... other computed values
        }
    }
}
```

### 7. Handling Edge Cases

Taffy provides tools for common chart layout challenges:

- **Minimum readable sizes**: Use `min_size` constraints
- **Overlapping labels**: Detect via bounding box intersection, adjust with margins
- **Responsive sizing**: Use percentage-based dimensions and flex properties
- **Legend overflow**: Use `overflow: Scroll` or automatic wrapping

### 8. Key Implementation Insights

1. **Measure Function Architecture**: Each chart component (axis, legend, title) becomes a Taffy node with a measure function that calls avenger-geometry
2. **Cascading Layout**: Taffy handles the complex dependency resolution automatically
3. **Multi-Chart Alignment**: Use CSS Grid with `align-items: stretch` to align chart areas across subplots
4. **Performance**: Taffy's caching means layout is only recomputed when inputs change

### 9. Example: Complete Chart Layout

```rust
// Example of a full chart layout tree
//
// Root (Flex Column)
// ├── Title (Measure: text height)
// ├── Main Area (Flex Row, flex: 1)
// │   ├── Y-Axis Label (Measure: rotated text)
// │   ├── Y-Axis Ticks (Measure: max label width)
// │   ├── Chart Content (Flex Column, flex: 1)
// │   │   ├── Chart Area (aspect ratio constraint)
// │   │   └── X-Axis Area (Flex Row)
// │   │       ├── Spacer (matches y-axis width)
// │   │       ├── X-Axis Ticks (Measure: label heights)
// │   │       └── Spacer (matches legend width)
// │   └── Legend (Measure: content size)
// └── X-Axis Label (Measure: text height)
```

## Conclusion

Using Taffy for automatic padding determination in avenger-chart provides a robust, performant solution that:

1. **Eliminates manual padding calculations** through measure functions
2. **Handles complex multi-chart layouts** via CSS Grid
3. **Integrates seamlessly** with existing avenger-geometry measurements
4. **Provides standard behavior** familiar to web developers
5. **Scales efficiently** to complex dashboards and layouts

The key insight is that by modeling chart components as Taffy nodes with appropriate measure functions, the entire padding and alignment problem becomes a standard layout problem that Taffy is designed to solve.