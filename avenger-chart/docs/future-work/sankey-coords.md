# Sankey Coordinate System

## Overview

Implements Sankey diagrams as a coordinate system where nodes form the coordinate space domain, node rectangles serve as guides, and flow ribbons are marks. This maintains consistency with other coordinate system patterns.

## Conceptual Model

- **Coordinate Space**: A topology of nodes arranged in stages
- **Domain**: The set of nodes and their relationships
- **Guides**: Node rectangles that visualize coordinate space structure
- **Marks**: Flow ribbons connecting nodes, representing data

### Relationship to Multi-Dimensional Systems

| Aspect | Parallel Coordinates | Sankey |
|--------|---------------------|---------|
| **Space** | Multiple parallel axes | Node topology |
| **Domain** | Variable dimensions from data | Node set from edge list |
| **Guides** | Vertical axes | Node rectangles |
| **Marks** | Polylines across axes | Flow ribbons between nodes |
| **Data Mapping** | 1 record → 1 polyline | 1 record → 1 flow ribbon |

## Coordinate System Definition

```rust
pub struct SankeyCoord {
    stage_assignment: StageAssignment,
    node_ordering: NodeOrdering,
    layout_method: LayoutMethod,
    iterations: u32,
}

pub enum StageAssignment {
    /// Automatically compute stages using topological sort
    Automatic,
    /// Manually specify which nodes belong to which stage
    Manual(Vec<Vec<String>>),
    /// Use a data column to determine stages
    FromColumn(String),
}

pub enum NodeOrdering {
    OptimizeCrossings,
    Alphabetical,
    Manual(HashMap<usize, Vec<String>>),
    DataOrder,
}

pub enum LayoutMethod {
    Sugiyama { iterations: u32 },
    ForceDirected { strength: f32 },
    Manual(HashMap<String, (f32, f32)>),
}
```

## Guide System

```rust
pub struct SankeyGuide {
    // Node appearance
    node_width: f32,
    node_padding: f32,
    node_fill: Option<ColorOrGradient>,
    node_stroke: Option<Color>,
    node_stroke_width: f32,

    // Node labels
    node_label_config: NodeLabelConfig,

    // Stage labels (optional)
    stage_labels: Option<Vec<String>>,
    stage_label_config: StageLabelConfig,

    // Computed node positions
    node_positions: HashMap<String, NodePosition>,
}

pub struct NodePosition {
    stage: usize,
    order: usize,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    incoming_flows: Vec<FlowAnchor>,
    outgoing_flows: Vec<FlowAnchor>,
}

pub struct FlowAnchor {
    node_id: String,
    flow_id: String,
    y_start: f32,
    y_end: f32,
    value: f32,
}
```

## Mark Definition

```rust
pub struct FlowRibbon {
    // Flow specification (required)
    source_channel: Option<ChannelValue>,
    target_channel: Option<ChannelValue>,
    value_channel: Option<ChannelValue>,

    // Visual encoding channels
    fill: Option<ColorChannelConfig>,
    stroke: Option<ColorChannelConfig>,
    opacity: Option<OpacityChannelConfig>,
    stroke_width: Option<f32>,

    // Interaction
    tooltip_fields: Vec<Expr>,
    hover_opacity: Option<f32>,

    // Flow path style
    path_style: FlowPathStyle,
}

pub enum FlowPathStyle {
    Bezier { curvature: f32 },
    Linear,
    Step,
    Custom(Box<dyn Fn(FlowAnchor, FlowAnchor) -> Path>),
}
```

## Usage Examples

### Basic Energy Flow Sankey

```rust
let plot = Plot::<SankeyCoord>::new()
    .data(energy_flows_df)

    .coord(|c| c
        .stages(StageAssignment::Automatic)
        .ordering(NodeOrdering::OptimizeCrossings)
        .layout(LayoutMethod::Sugiyama { iterations: 32 })
    )

    .guide(|g| g
        .node_width(15.0)
        .node_padding(10.0)
        .node_fill([0.3, 0.3, 0.3, 1.0])
        .node_stroke([0.0, 0.0, 0.0, 1.0])
        .node_stroke_width(1.0)

        .node_labels(|l| l
            .font_size(10.0)
            .color([0.2, 0.2, 0.2, 1.0])
            .anchor("left")
            .offset(5.0))
    )

    .mark(
        FlowRibbon::new()
            .source(col("source"))
            .target(col("target"))
            .value(col("amount"))

            .fill_with(col("source"), |c| c
                .scale(|s| s.categorical())
                .legend(true)
                .opacity(0.7))

            .tooltip(vec![
                col("source"),
                col("target"),
                col("amount"),
            ])
            .hover_opacity(1.0)
    );
```

### Multi-Stage Survey Flow (Alluvial Diagram)

```rust
let plot = Plot::<SankeyCoord>::new()
    .data(survey_responses_df)

    .coord(|c| c
        .stages(StageAssignment::Manual(vec![
            vec!["Strongly Agree", "Agree", "Neutral", "Disagree", "Strongly Disagree"],
            vec!["Strongly Agree", "Agree", "Neutral", "Disagree", "Strongly Disagree"],
            vec!["Strongly Agree", "Agree", "Neutral", "Disagree", "Strongly Disagree"],
        ]))
    )

    .guide(|g| g
        .stage_labels(vec!["Year 1", "Year 2", "Year 3"])
        .stage_label_config(|c| c
            .font_size(12.0)
            .font_weight(600)
            .position(StageLabelPosition::Top))

        .node_width(20.0)
        .node_padding(5.0)
    )

    .mark(
        FlowRibbon::new()
            .from_stages(vec![
                col("response_year1"),
                col("response_year2"),
                col("response_year3"),
            ])
            .value(col("respondent_count"))

            .fill_with(col("response_year1"), |c| c
                .scale(|s| s.ordinal()
                    .range(["#d73027", "#fc8d59", "#fee090", "#91bfdb", "#4575b4"]))
                .opacity(0.8))
    );
```

### Circular Sankey (with cycles)

```rust
let plot = Plot::<SankeyCoord>::new()
    .data(trade_flows_df)

    .coord(|c| c
        .layout(LayoutMethod::ForceDirected { strength: 0.5 })
        .allow_cycles(true)
    )

    .guide(|g| g
        .arrangement(NodeArrangement::Circular { radius: 200.0 })
        .node_width(30.0)
    )

    .mark(
        FlowRibbon::new()
            .source(col("exporter"))
            .target(col("importer"))
            .value(col("trade_volume"))

            .path_style(FlowPathStyle::Bezier { curvature: 0.7 })

            .fill_with(
                when(col("exporter").lt(col("importer")))
                    .then(lit("#4169E1"))
                    .otherwise(lit("#DC143C")),
                |c| c.opacity(0.6)
            )
    );
```

## Coordinate Transform

```rust
impl CoordinateSystem for SankeyCoord {
    type Guide = SankeyGuide;

    fn transform_to_visual(
        &self,
        data: &DataView,
        mark: &dyn Mark<Self>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<VisualGeometry, AvengerChartError> {
        let flow_mark = mark.as_flow_ribbon()
            .ok_or("SankeyCoord requires FlowRibbon mark")?;

        // Step 1: Build node domain from edge data
        let node_domain = self.extract_nodes(data, flow_mark)?;

        // Step 2: Assign stages (x positions)
        let staged_nodes = match &self.stage_assignment {
            StageAssignment::Automatic => self.topological_stages(&node_domain),
            StageAssignment::Manual(stages) => stages.clone(),
            StageAssignment::FromColumn(col) => self.stages_from_column(data, col),
        };

        // Step 3: Order nodes within stages (y positions)
        let ordered_nodes = self.order_nodes_in_stages(staged_nodes, &node_domain);

        // Step 4: Compute node positions and sizes
        let node_positions = self.layout_nodes(
            ordered_nodes,
            plot_width,
            plot_height
        );

        // Step 5: Transform each record to a flow ribbon
        let mut ribbons = Vec::new();
        let mut flow_offsets: HashMap<String, f32> = HashMap::new();

        for row_idx in 0..data.num_rows() {
            let source = flow_mark.source_channel.evaluate_row(data, row_idx)?;
            let target = flow_mark.target_channel.evaluate_row(data, row_idx)?;
            let value = flow_mark.value_channel.evaluate_row(data, row_idx)?;

            let source_anchor = self.get_flow_anchor(
                &source, value, &mut flow_offsets,
                &node_positions, FlowDirection::Outgoing
            );

            let target_anchor = self.get_flow_anchor(
                &target, value, &mut flow_offsets,
                &node_positions, FlowDirection::Incoming
            );

            let path = self.create_flow_path(
                source_anchor, target_anchor, &flow_mark.path_style
            );

            ribbons.push(FlowRibbonGeometry {
                source_node: source,
                target_node: target,
                value,
                path,
                source_anchor,
                target_anchor,
            });
        }

        Ok(VisualGeometry::FlowRibbons {
            ribbons,
            nodes: node_positions,
        })
    }
}
```

## Visual Geometry

```rust
pub enum VisualGeometry {
    Points { x: Vec<f32>, y: Vec<f32> },
    Polylines(Vec<Vec<(f32, f32)>>),

    FlowRibbons {
        ribbons: Vec<FlowRibbonGeometry>,
        nodes: HashMap<String, NodePosition>,
    },
}

pub struct FlowRibbonGeometry {
    source_node: String,
    target_node: String,
    value: f32,
    path: Path,
    source_anchor: FlowAnchor,
    target_anchor: FlowAnchor,
}
```

## Advanced Features

### Node Constraints

```rust
.guide(|g| g
    .pin_node("Coal", stage: 0, position: 0)
    .pin_node("Electricity", stage: 1, position: 0)

    .group_nodes(["Coal", "Oil", "Gas"], "Fossil Fuels")

    .min_flow_height(2.0)
)
```

### Flow Aggregation

```rust
.coord(|c| c
    .aggregate_flows(|flows| {
        flows.filter(|f| f.value < threshold)
             .group_by(|f| (f.source, f.target))
             .sum()
    })

    .duplicate_edge_strategy(DuplicateStrategy::Sum)
)
```

### Interactive Features

```rust
.mark(
    FlowRibbon::new()
        .source(col("from"))
        .target(col("to"))
        .value(col("amount"))

        .on_hover(|state| state
            .highlight_connected(true)
            .dim_others(0.2))

        .on_click(|state| state
            .filter_to_path(true))
)
```

### Animation Support

```rust
.mark(
    FlowRibbon::new()
        .source(col("from"))
        .target(col("to"))
        .value(col("amount"))

        .animate_by(col("year"))
        .animation_duration(2000)
        .animation_easing("cubic-in-out")
)
```

## Layout Algorithms

### Sugiyama Method (Default)

1. **Stage Assignment**: Topological sort or manual
2. **Cross Minimization**: Barycenter or median heuristic
3. **Coordinate Assignment**: Minimize edge bends
4. **Flow Stacking**: Bottom-up or center-aligned

### Force-Directed

For non-hierarchical or circular flows:
1. **Node Repulsion**: Prevent overlaps
2. **Edge Attraction**: Minimize edge lengths
3. **Stage Constraints**: Optional x-position constraints
4. **Convergence**: Iterate until stable

## Performance Considerations

### Level-of-Detail
```rust
.coord(|c| c
    .lod_threshold(100)
    .lod_strategy(LODStrategy::AggregateSmall)
)
```

### Progressive Rendering
```rust
.coord(|c| c
    .progressive(true)
    .initial_iterations(5)
    .refine_on_idle(true)
)
```

### WebGL Acceleration
- Render ribbons as instanced meshes
- Use GPU for path tessellation
- Batch updates for animations

## Edge Cases

### Cycles in Data
```rust
.coord(|c| c
    .handle_cycles(CycleStrategy::BackEdges)  // or Break, or Allow
)
```

### Missing Nodes
```rust
.coord(|c| c
    .handle_missing_nodes(MissingNodeStrategy::Create)  // or Skip, or Error
)
```

### Zero-Value Flows
```rust
.mark(
    FlowRibbon::new()
        .min_flow_height(1.0)
        .zero_flow_style(|s| s.stroke_dasharray([2, 2]))
)
```

## Future Extensions

1. **Hierarchical Sankey**: Nested node groups
2. **Temporal Sankey**: Animated flows over time
3. **Geographic Sankey**: Flows on maps
4. **3D Sankey**: Perspective depth for additional dimension
5. **Hybrid Layouts**: Combining Sankey with other coordinate systems

## Conclusion

Sankey coordinate system fits naturally into Avenger's architecture by treating nodes as the coordinate space domain and guides. This design maintains consistency with other coordinate systems, enables flexible layout algorithms, and provides extensibility for various data formats. The key insight: Sankey diagrams define a topological coordinate space where positions are determined by graph layout algorithms rather than numeric scales.
