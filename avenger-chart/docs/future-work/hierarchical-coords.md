# Hierarchical Coordinate Systems

## Overview

Implements hierarchical visualizations (Treemap, Sunburst) as coordinate systems where parent containers serve as guides and leaf nodes are marks. Uses repeated `.level()` calls to define hierarchy, maintaining consistency with multi-dimensional coordinate system patterns.

## Conceptual Model

- **Coordinate Space**: A hierarchy of nested containers
- **Domain**: The tree structure with parent-child relationships
- **Guides**: Parent containers that provide visual structure
- **Marks**: Leaf nodes that represent actual data points

### Relationship to Other Coordinate Systems

| System | Repeated Method | Creates | Guides | Marks |
|--------|----------------|---------|--------|-------|
| **Parallel** | `.y()` | Axes | Vertical axes | Polylines |
| **Radar** | `.r()` | Dimensions | Radial axes | Polygons |
| **Sankey** | `.flow()` | Edges | Node rectangles | Flow ribbons |
| **Treemap** | `.level()` | Hierarchy | Parent containers | Leaf rectangles |
| **Sunburst** | `.level()` | Hierarchy | Parent arcs | Leaf arcs |

## The `.level()` Pattern

Just as parallel coordinates use repeated `.y()` calls to define dimensions, hierarchical systems use repeated `.level()` calls to define nesting:

```rust
// Parallel Coordinates - each y() creates an axis
.mark(ParallelLine::new()
    .y(col("mpg"))
    .y(col("cylinders"))
    .y(col("weight")))

// Treemap - each level() creates a hierarchy level
.mark(TreemapRect::new()
    .level(col("continent"))    // Outermost containers
    .level(col("country"))      // Nested within continents
    .level(col("city"))         // Nested within countries (leaves)
    .size(col("population")))   // Determines rectangle area
```

## Treemap Coordinate System

```rust
pub struct TreemapCoord {
    layout_method: TreemapLayout,
    nest_padding: f32,
    aspect_ratio_target: f32,
}

pub enum TreemapLayout {
    Squarified { iterations: u32 },
    SliceAndDice,
    Binary,
    OrderPreserving,
}
```

### Treemap Guide

```rust
pub struct TreemapGuide {
    // Container appearance
    container_fill: Option<ColorOrGradient>,
    container_stroke: Option<Color>,
    container_stroke_width: f32,
    container_corner_radius: f32,

    // Container labels
    container_label_config: ContainerLabelConfig,

    // Nesting visualization
    nesting_style: NestingStyle,

    // Breadcrumb navigation
    breadcrumb_config: Option<BreadcrumbConfig>,

    // Computed container hierarchy
    containers: HierarchicalContainers,
}

pub enum NestingStyle {
    Padded { padding_per_level: f32 },
    BordersOnly,
    DepthFade { opacity_range: (f32, f32) },
    PerLevel(Box<dyn Fn(usize) -> ContainerStyle>),
}

pub struct ContainerLabelConfig {
    position: LabelPosition,
    font_size: f32,
    font_weight: u16,
    min_container_area: f32,
    abbreviate: Option<AbbreviateConfig>,
}
```

### Treemap Mark

```rust
pub struct TreemapRect {
    hierarchy_levels: Vec<Expr>,
    size_channel: Option<ChannelValue>,

    // Visual encoding channels for leaves
    fill: Option<ColorChannelConfig>,
    stroke: Option<ColorChannelConfig>,
    stroke_width: Option<f32>,
    opacity: Option<OpacityChannelConfig>,

    // Leaf labels
    label: Option<TextChannelConfig>,

    // Interaction
    tooltip_fields: Vec<Expr>,
    hover_highlight: Option<HoverConfig>,
}

impl TreemapRect {
    pub fn level(mut self, expr: impl Into<Expr>) -> Self {
        self.hierarchy_levels.push(expr.into());
        self
    }

    pub fn size(mut self, expr: impl Into<Expr>) -> Self {
        self.size_channel = Some(ChannelValue::new(expr.into()));
        self
    }
}
```

## Usage Examples

### Basic Sales Treemap

```rust
let plot = Plot::<TreemapCoord>::new()
    .data(sales_df)

    .coord(|c| c
        .layout(TreemapLayout::Squarified { iterations: 10 })
        .nest_padding(3.0)
        .aspect_ratio_target(1.618))  // Golden ratio

    .guide(|g| g
        .container_stroke([0.6, 0.6, 0.6, 1.0])
        .container_stroke_width(2.0)
        .container_fill("none")

        .container_labels(|l| l
            .position(LabelPosition::TopLeft)
            .font_size(12.0)
            .font_weight(600)
            .min_container_area(1000.0))

        .breadcrumbs(|b| b
            .separator(" › ")
            .font_size(10.0)
            .interactive(true)))

    .mark(
        TreemapRect::new()
            .level(col("region"))
            .level(col("category"))
            .level(col("product"))

            .size(col("sales_amount"))

            .fill_with(col("category"), |c| c
                .scale(|s| s.categorical())
                .legend(true))

            .stroke([1.0, 1.0, 1.0, 1.0])
            .stroke_width(1.0)

            .label_with(col("product"), |l| l
                .font_size(10.0)
                .min_area(500.0))

            .tooltip(vec![
                col("product"),
                col("sales_amount").format("$,.0f"),
                col("growth_rate").format("+.1%"),
            ])
    );
```

### File System Treemap

```rust
let plot = Plot::<TreemapCoord>::new()
    .data(file_system_df)

    .coord(|c| c
        .layout(TreemapLayout::SliceAndDice))

    .guide(|g| g
        .container_style_by_level(|level, style| {
            match level {
                0 => style.stroke([0.0, 0.0, 0.0, 1.0]).stroke_width(3.0),
                1 => style.stroke([0.3, 0.3, 0.3, 1.0]).stroke_width(2.0),
                2 => style.stroke([0.6, 0.6, 0.6, 1.0]).stroke_width(1.0),
                _ => style.stroke("none"),
            }
        }))

    .mark(
        TreemapRect::new()
            .level(col("drive"))
            .level(col("folder1"))
            .level(col("folder2"))
            .level(col("folder3"))
            .level(col("filename"))

            .size(col("bytes"))

            .fill_with(col("extension"), |c| c
                .scale(|s| s.categorical()
                    .domain([".rs", ".js", ".css", ".html", ".json"])
                    .range(["#CE422B", "#F0DB4F", "#264DE4", "#E34C26", "#000000"]))
                .legend(true))
    );
```

### Sunburst Budget Visualization

```rust
let plot = Plot::<SunburstCoord>::new()
    .data(budget_df)

    .coord(|c| c
        .inner_radius(60.0)
        .padding_angle(0.02)
        .start_angle(-90.0))

    .guide(|g| g
        .container_stroke([0.5, 0.5, 0.5, 1.0])
        .container_stroke_width(1.5)

        .container_labels(|l| l
            .curved(true)
            .orientation(LabelOrientation::Radial)
            .min_angle(10.0))

        .center_text(|t| t
            .content("FY 2024\nTotal Budget")
            .font_size(14.0)
            .font_weight(700)))

    .mark(
        SunburstArc::new()
            .level(col("department"))
            .level(col("division"))
            .level(col("team"))
            .level(col("project"))

            .size(col("budget_amount"))

            .fill_with(col("department"), |c| c
                .scale(|s| s.categorical())
                .legend(true))

            .fill_modifier(|base_color, level| {
                darken(base_color, level as f32 * 0.15)
            })

            .on_click(|node| {
                if node.has_children() {
                    zoom_to_subtree(node)
                }
            })
    );
```

## Coordinate Transform

```rust
impl CoordinateSystem for TreemapCoord {
    type Guide = TreemapGuide;

    fn transform_to_visual(
        &self,
        data: &DataView,
        mark: &dyn Mark<Self>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<VisualGeometry, AvengerChartError> {
        let treemap_mark = mark.as_treemap_rect()
            .ok_or("TreemapCoord requires TreemapRect mark")?;

        // Step 1: Build hierarchy from data using level expressions
        let hierarchy = self.build_hierarchy_from_levels(
            data,
            &treemap_mark.hierarchy_levels,
            &treemap_mark.size_channel
        )?;

        // Step 2: Compute cumulative sizes (sum of children)
        let sized_hierarchy = self.compute_sizes(hierarchy);

        // Step 3: Apply layout algorithm
        let root_bounds = Rectangle {
            x: 0.0, y: 0.0,
            width: plot_width,
            height: plot_height,
        };

        let layout = match self.layout_method {
            TreemapLayout::Squarified { iterations } => {
                squarified_layout(&sized_hierarchy, root_bounds, iterations)
            }
            TreemapLayout::SliceAndDice => {
                slice_and_dice_layout(&sized_hierarchy, root_bounds)
            }
            // ...
        };

        // Step 4: Separate containers (guides) from leaves (marks)
        let (containers, leaves) = self.separate_containers_and_leaves(layout);

        Ok(VisualGeometry::HierarchicalRectangles {
            containers,
            leaves,
        })
    }
}

fn build_hierarchy_from_levels(
    &self,
    data: &DataView,
    levels: &[Expr],
    size_expr: &Expr,
) -> Result<HierarchyNode, AvengerChartError> {
    let mut root = HierarchyNode::new("root");

    for row_idx in 0..data.num_rows() {
        let mut current_node = &mut root;

        for level_expr in levels {
            let level_value = level_expr.evaluate_row(data, row_idx)?;

            current_node = current_node
                .children
                .entry(level_value.to_string())
                .or_insert_with(|| HierarchyNode::new(&level_value));
        }

        current_node.size = size_expr.evaluate_row(data, row_idx)?;
    }

    Ok(root)
}
```

## Visual Geometry

```rust
pub enum VisualGeometry {
    Points { x: Vec<f32>, y: Vec<f32> },
    Polylines(Vec<Vec<(f32, f32)>>),

    HierarchicalRectangles {
        containers: Vec<Container>,
        leaves: Vec<LeafRectangle>,
    },

    HierarchicalArcs {
        containers: Vec<ContainerArc>,
        leaves: Vec<LeafArc>,
    },
}

pub struct Container {
    level: usize,
    name: String,
    bounds: Rectangle,
    children_bounds: Vec<Rectangle>,
}

pub struct LeafRectangle {
    bounds: Rectangle,
    path: Vec<String>,
    value: f32,
    data_row: usize,
}
```

## Guide Rendering

```rust
impl Guide for TreemapGuide {
    fn render(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &Padding,
        theme: &Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut marks = Vec::new();

        for container in &self.containers {
            if container.is_leaf() {
                continue;
            }

            marks.push(SceneMark::Rect(RectMark {
                x: container.bounds.x + padding.left,
                y: container.bounds.y + padding.top,
                width: container.bounds.width,
                height: container.bounds.height,
                fill: self.container_fill.clone(),
                stroke: self.container_stroke.clone(),
                stroke_width: self.container_stroke_width,
                corner_radius: self.container_corner_radius,
                z_index: -(container.level as i32),
            }));

            if self.should_show_label(container) {
                marks.push(self.create_container_label(container));
            }
        }

        if let Some(breadcrumb_config) = &self.breadcrumb_config {
            marks.extend(self.render_breadcrumbs(breadcrumb_config));
        }

        Ok(marks)
    }
}
```

## Advanced Features

### Dynamic Level Configuration

```rust
.mark(
    TreemapRect::new()
        .level(col("category"))
        .level(col("subcategory"))

        .optional_level(col("detail"))
        .optional_level(col("subdetail"))

        .size(col("value"))
)
```

### Level-Specific Configuration

```rust
.mark(
    TreemapRect::new()
        .level_with(col("region"), |l| l
            .label_position(LabelPosition::Center)
            .label_size(14.0))

        .level_with(col("country"), |l| l
            .label_position(LabelPosition::TopLeft)
            .label_size(11.0))

        .level_with(col("city"), |l| l
            .label_position(LabelPosition::Hidden))

        .size(col("population"))
)
```

### Hierarchy from Nested Data

```rust
// From parent-child relationships
.mark(
    TreemapRect::new()
        .hierarchy_from_edges(|h| h
            .parent(col("parent_id"))
            .child(col("id"))
            .root_value(null()))
        .size(col("value"))
)

// From path strings
.mark(
    TreemapRect::new()
        .hierarchy_from_path(|h| h
            .path(col("file_path"))
            .separator("/"))
        .size(col("file_size"))
)

// From nested JSON
.mark(
    TreemapRect::new()
        .hierarchy_from_json(col("nested_data"))
        .size_field("value")
)
```

### Interactive Features

```rust
.guide(|g| g
    .enable_zoom(|z| z
        .on_container_click(ZoomAction::DrillDown)
        .on_escape_key(ZoomAction::ZoomOut)
        .animated(true)
        .duration(300))

    .on_container_hover(|h| h
        .highlight_subtree(true)
        .dim_others(0.3))
)

.mark(
    TreemapRect::new()
        // ... hierarchy setup ...

        .on_hover(|h| h
            .stroke_width(3.0)
            .stroke([0.0, 0.0, 0.0, 1.0]))

        .on_click(|c| c
            .emit_event("leaf_selected")
            .with_data(["path", "value"]))
)
```

### Layout Stability

```rust
.coord(|c| c
    .layout(TreemapLayout::OrderPreserving)

    .stable_layout(|s| s
        .key(col("id"))
        .transition_duration(500)
        .easing("cubic-in-out"))
)
```

## Sunburst Coordinate System

```rust
pub struct SunburstCoord {
    inner_radius: f32,
    padding_angle: f32,
    start_angle: f32,
    direction: Direction,
}

pub struct SunburstArc {
    hierarchy_levels: Vec<Expr>,
    size_channel: Option<ChannelValue>,

    fill: Option<ColorChannelConfig>,
    stroke: Option<ColorChannelConfig>,

    corner_radius: Option<f32>,
}
```

The transform maps hierarchy to nested arcs instead of rectangles:

```rust
impl CoordinateSystem for SunburstCoord {
    fn transform_to_visual(...) -> Result<VisualGeometry, ...> {
        let layout = radial_partition_layout(
            hierarchy,
            self.inner_radius,
            outer_radius,
            self.start_angle,
            total_angle
        );

        Ok(VisualGeometry::HierarchicalArcs {
            containers: container_arcs,
            leaves: leaf_arcs,
        })
    }
}
```

## Performance Considerations

### Large Hierarchies

```rust
.coord(|c| c
    .min_leaf_size(0.001)
    .aggregate_small_as("Other")

    .max_depth(4)

    .progressive_detail(|p| p
        .initial_depth(2)
        .expand_on_zoom(true))
)
```

### Layout Caching

```rust
.coord(|c| c
    .cache_layout(true)
    .layout_dependency(["size", "hierarchy"])
)
```

### WebGL Optimization

- Render rectangles as instanced geometry
- Use texture atlases for labels
- Implement level-of-detail for text

## Future Extensions

1. **Icicle Plots**: Horizontal/vertical hierarchy bars
2. **Circle Packing**: Hierarchical circles
3. **Dendrograms**: Tree diagrams with edges
4. **Partition Layouts**: Various space-filling approaches
5. **Hybrid Layouts**: Combining treemap with other visualizations

## Conclusion

The hierarchical coordinate system maintains consistency with Avenger's architecture using the `.level()` pattern as a natural extension of repeated method patterns. Parent containers as guides provide clear separation of structure from data, while maintaining flexible configuration and batch processing like other coordinate systems. The key insight: parent containers are the natural guides for hierarchical coordinate systems, providing visual structure for navigating nested space.
