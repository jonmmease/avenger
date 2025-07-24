# Interactivity Research Report for Avenger-Chart

## Executive Summary

This report analyzes interactivity systems across major visualization libraries to inform the design of Avenger-Chart's interactivity architecture. The proposed approach leverages DataFusion's `Expr::Placeholder` API as a novel parameterization mechanism, integrates with the existing avenger-eventstream framework, and provides high-level interaction components through a Controller abstraction.

This design incorporates lessons learned from an earlier avenger-chart prototype that successfully demonstrated a controller-based interaction system with sophisticated state management and scale integration.

## Research Findings

### 1. Vega-Lite: Declarative Interaction Grammar

#### Core Concepts
- **Parameters**: Basic building blocks for interaction
  - **Variables**: Simple values that can be reused throughout the specification
  - **Selections**: Data queries driven by user input (point, interval)
- **Event Streams**: Sophisticated event handling model with operators and transformations
- **Conditional Encoding**: Visual properties change based on parameter values

#### Architecture
```json
{
  "params": [{
    "name": "brush",
    "select": {"type": "interval"}
  }],
  "mark": "point",
  "encoding": {
    "color": {
      "condition": {"param": "brush", "field": "category"},
      "value": "gray"
    }
  }
}
```

#### Key Features
- **Scale Binding**: Interval selections can bind to scales for pan/zoom
- **Input Binding**: Parameters can bind to UI widgets (sliders, dropdowns)
- **Multi-view Coordination**: Selections can be shared across views
- **Event Stream Syntax**: Complex patterns like drag interactions

### 2. Observable Plot: Simplified Interactivity

#### Current State
- **Pointer Transform**: Dynamic filtering to show data near cursor
- **Tip Mark**: Automatic tooltips
- **Event Listeners**: Can attach to plot element
- **Limitations**: No built-in brushing, limited individual mark events

#### Example
```javascript
Plot.dot(data, {
  x: "weight",
  y: "height",
  tip: true,
  channels: {name: "name"}
}).plot();

// Listen to pointer events
plot.addEventListener("input", (event) => {
  console.log(plot.value); // Current pointed value
});
```

### 3. D3.js: Low-Level Control

#### Interaction Patterns
- **Zoom Behavior**: `d3.zoom()` with scale/translate transformations
- **Brush**: `d3.brush()` for rectangular selections
- **Drag**: `d3.drag()` for individual element manipulation
- **Full Control**: Complete customization of all interactions

#### Implementation
```javascript
const zoom = d3.zoom()
  .scaleExtent([0.5, 32])
  .on("zoom", zoomed);

svg.call(zoom);
```

### 4. ggplot2 + Plotly: Static to Interactive

#### Approach
- ggplot2 creates static plots with grammar of graphics
- `ggplotly()` converts to interactive Plotly visualizations
- Automatic addition of hover, zoom, pan functionality
- Limited customization of interaction behavior

### 5. Altair: Python Declarative Interactions

#### Features
- Python API wrapping Vega-Lite
- Parameters and selections with Python syntax
- JupyterChart for bidirectional communication
- Dashboard integration capabilities

```python
brush = alt.selection_interval()

chart = alt.Chart(data).mark_point().encode(
    x='x',
    y='y',
    color=alt.when(brush).then("category").otherwise(alt.value("gray"))
).add_params(brush)
```

## DataFusion Expr Placeholders: A Novel Approach

### Core Concept
Use DataFusion's `Expr::Placeholder` API to parameterize every aspect of a visualization, staying entirely in DataFrame land without SQL strings.

### Current DataFusion Support
```rust
use datafusion::prelude::*;

// Create placeholders as expressions
let threshold_param = Expr::Placeholder(Placeholder {
    id: "$threshold".to_string(),
    data_type: Some(DataType::Float64),
});

// Use in DataFrame operations
let filtered_df = df
    .filter(col("value").gt(threshold_param))?
    .filter(col("category").eq(placeholder("$category")))?;
```

### Proposed Extension for Visualizations

#### 1. Parameterized Data Queries
```rust
// Define parameterized data transformations
let filtered_data = df
    .filter(col("date").between(
        placeholder("$start_date"),
        placeholder("$end_date")
    ))?
    .filter(col("region").eq(placeholder("$region")))?
    .filter(col("value").gt(placeholder("$threshold")))?;

// Parameters updated by interactions
let params = ParameterValues::new()
    .set("start_date", ScalarValue::Date32(Some(start)))
    .set("end_date", ScalarValue::Date32(Some(end)))
    .set("region", ScalarValue::Utf8(Some(selected_region)))
    .set("threshold", ScalarValue::Float64(Some(100.0)));
```

#### 2. Parameterized Scales
```rust
// Dynamic scale domains via DataFrame expressions
let scale_domain = df
    .aggregate(
        vec![],
        vec![
            // Conditional min based on zoom level
            when(placeholder("$zoom_level").gt(lit(1.0)),
                placeholder("$pan_x") - (placeholder("$domain_width") / placeholder("$zoom_level"))
            ).otherwise(min(col("value"))).alias("x_min"),
            
            // Conditional max based on zoom level
            when(placeholder("$zoom_level").gt(lit(1.0)),
                placeholder("$pan_x") + (placeholder("$domain_width") / placeholder("$zoom_level"))
            ).otherwise(max(col("value"))).alias("x_max"),
        ]
    )?;
```

#### 3. Parameterized Visual Properties
```rust
// Conditional encoding via DataFrame expressions
let styled_marks = marks_df
    .with_column(
        // Dynamic color based on selection
        when(col("id").in_list(placeholder("$selected_ids"), false),
            placeholder("$highlight_color")
        ).otherwise(placeholder("$default_color"))
        .alias("color")
    )?
    .with_column(
        // Dynamic opacity based on brush
        when(col("value").between(
            placeholder("$brush_min"),
            placeholder("$brush_max")
        ), lit(1.0))
        .otherwise(lit(0.3))
        .alias("opacity")
    )?;
```

## Proposed Architecture

### Four-Layer Design

Based on the successful prototype implementation, the architecture consists of four layers:

#### 1. Core Parameter System (Inside Avenger)
```rust
use datafusion::{logical_expr::expr::Placeholder, prelude::Expr, scalar::ScalarValue};

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: ScalarValue,
    pub stream: Option<Arc<dyn ParamStream>>,
}

impl Param {
    pub fn new<S: Into<String>, T: Into<ScalarValue>>(name: S, default: T) -> Self {
        Self {
            name: name.into(),
            default: default.into(),
            stream: None,
        }
    }

    pub fn expr(&self) -> Expr {
        Expr::Placeholder(Placeholder {
            id: format!("${}", self.name),
            data_type: Some(self.default.data_type()),
        })
    }
}

// Parameter values collection used at runtime
pub struct ParameterValues {
    values: HashMap<String, ScalarValue>,
}
```

#### 2. ParamStream Pattern (Inside Avenger)
The ParamStream trait provides a structured way to map events to parameter updates:

```rust
pub struct ParamStreamContext<'a> {
    pub event: &'a SceneGraphEvent,
    pub params: &'a HashMap<String, ScalarValue>,
    pub scales: &'a [ConfiguredScale],
    pub group_path: &'a [usize],
    pub rtree: &'a SceneGraphRTree,
    pub details: &'a HashMap<Vec<usize>, RecordBatch>,
}

pub trait ParamStream: Debug + Send + Sync + 'static {
    /// Controller state type
    type State: Clone + Send + Sync + 'static;
    
    fn stream_config(&self) -> &EventStreamConfig;
    fn input_params(&self) -> &[String];
    fn input_scales(&self) -> &[Scale];
    
    /// Update with mutable access to state
    fn update(
        &self,
        context: ParamStreamContext,
        state: &mut Self::State,
    ) -> (HashMap<String, ScalarValue>, UpdateStatus);
}
```

#### 3. Controller Abstraction (Inside Avenger)
Controllers organize related parameters and interaction logic:

```rust
// Scale modification abstraction
pub enum ScaleTarget {
    Named(String),              // Target a specific named scale
    Encoding(EncodingChannel),  // Target all scales used for an encoding (x, y, color, etc.)
    All,                       // Target all scales
}

pub struct ScaleModifier {
    pub target: ScaleTarget,
    pub modify: Box<dyn Fn(Scale) -> Scale + Send + Sync>,
}

pub trait Controller: Debug + Send + Sync + 'static {
    /// State type for this controller
    type State: Clone + Default + Send + Sync + 'static;
    
    /// Controller name that's unique in the group
    fn name(&self) -> &str;
    
    /// Declare state requirements based on chart configuration
    fn state_mode(&self, chart_config: &ChartConfig) -> StateMode {
        // Default: follow resolve configuration
        match chart_config.resolve() {
            Resolution::Shared => StateMode::Shared,
            Resolution::Independent => StateMode::PerFacet,
            Resolution::SharedRows => StateMode::PerRow,
            Resolution::SharedCols => StateMode::PerColumn,
        }
    }
    
    /// Create param streams with state passed in
    fn create_param_streams(
        &self,
        scale_registry: &ScaleRegistry,
        state_map: &StateMap<Self::State>,
    ) -> Vec<Arc<dyn ParamStream>>;
    
    /// Generate params based on state configuration
    fn generate_params(&self, state_map: &StateMap<Self::State>) -> Vec<Param>;
    
    /// Generate scale modifiers based on state configuration
    fn generate_scale_modifiers(
        &self,
        scale_registry: &ScaleRegistry,
        state_map: &StateMap<Self::State>,
    ) -> Vec<ScaleModifier>;
    
    /// Optional marks that this controller provides (e.g., tooltips)
    fn marks(&self) -> Vec<MarkOrGroup> {
        Vec::new()
    }
}

pub enum StateMode {
    /// Single state instance shared across all facets
    Shared,
    /// Separate state instance per facet
    PerFacet,
    /// State instance per row (for grid facets)
    PerRow,
    /// State instance per column (for grid facets)
    PerColumn,
    /// Custom grouping logic
    Custom(Box<dyn Fn(&FacetInfo, &FacetInfo) -> bool>),
}
```

#### 4. Event Integration Bridge (Inside Avenger)
The ParamEventStreamHandler bridges controllers to the EventStreamHandler trait:

```rust
struct ParamEventStreamHandler {
    input_param_names: Vec<String>,
    input_scales: Vec<Scale>,
    param_stream: Arc<dyn ParamStream>,
}

#[async_trait]
impl EventStreamHandler<ChartState> for ParamEventStreamHandler {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartState,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        // Gather input parameters
        let input_params = self.input_param_names.iter()
            .map(|name| (name.clone(), state.param_values[name].clone()))
            .collect();
        
        // Evaluate input scales
        let mut scales = Vec::new();
        for scale in &self.input_scales {
            scales.push(state.eval_scale(scale).await);
        }
        
        let context = ParamStreamContext {
            event,
            params: &input_params,
            scales: &scales,
            group_path: &[0],
            rtree,
            details: &state.details,
        };
        
        let (new_params, update_status) = self.param_stream.update(context);
        
        // Update state with new parameter values
        for (name, value) in new_params {
            state.param_values.insert(name, value);
        }
        
        update_status
    }
}
```

### Declarative Controller Design

The new design uses a Scale Modification Pipeline that allows controllers to declaratively modify scales without creating them:

### High-Level Controllers (Outside Core)

Controllers now provide declarative scale modifications:

```rust
// State type for PanZoom controller
#[derive(Clone, Debug, Default)]
pub struct PanZoomState {
    anchor_position: Option<[f32; 2]>,
    anchor_x_domain: Option<ScalarValue>,
    anchor_y_domain: Option<ScalarValue>,
    is_panning: bool,
}

// Stateless PanZoomController
#[derive(Debug, Clone)]
pub struct PanZoomController {
    // Configuration only - no state!
    x_target: ScaleTarget,
    y_target: ScaleTarget,
}

impl PanZoomController {
    pub fn new() -> Self {
        Self {
            x_target: ScaleTarget::Encoding(EncodingChannel::X),
            y_target: ScaleTarget::Encoding(EncodingChannel::Y),
        }
    }
    
    // Builder methods for customization
    pub fn x_scale(mut self, name: &str) -> Self {
        self.x_target = ScaleTarget::Named(name.to_string());
        self
    }
    
    pub fn y_scale(mut self, name: &str) -> Self {
        self.y_target = ScaleTarget::Named(name.to_string());
        self
    }
}

impl Controller for PanZoomController {
    type State = PanZoomState;
    
    fn name(&self) -> &str {
        "pan-zoom"
    }
    
    // State follows resolve configuration by default
    fn state_mode(&self, chart_config: &ChartConfig) -> StateMode {
        match chart_config.resolve() {
            Resolution::Shared => StateMode::Shared,
            Resolution::Independent => StateMode::PerFacet,
            Resolution::SharedRows => StateMode::Custom(Box::new(|a, b| a.row == b.row)),
            Resolution::SharedCols => StateMode::Custom(Box::new(|a, b| a.col == b.col)),
        }
    }
    
    fn generate_params(&self, state_map: &StateMap<PanZoomState>) -> Vec<Param> {
        let mut params = vec![];
        
        // Generate parameters for each state instance
        for state_key in state_map.keys() {
            params.push(Param::new(
                format!("panzoom_{}_x_domain", state_key),
                ScalarValue::Null,
            ));
            params.push(Param::new(
                format!("panzoom_{}_y_domain", state_key),
                ScalarValue::Null,
            ));
        }
        
        params
    }
    
    fn generate_scale_modifiers(
        &self,
        scale_registry: &ScaleRegistry,
        state_map: &StateMap<PanZoomState>,
    ) -> Vec<ScaleModifier> {
        let mut modifiers = vec![];
        
        // For each scale, find its state key and create appropriate modifier
        for (scale_id, scale_info) in scale_registry.iter() {
            let state_key = state_map.state_key_for_facet(scale_info.facet_id.as_ref());
            
            if scale_info.matches_target(&self.x_target) {
                let param_name = format!("panzoom_{}_x_domain", state_key);
                modifiers.push(ScaleModifier {
                    target: ScaleTarget::Specific(scale_id.clone()),
                    modify: Box::new(move |scale| {
                        scale.raw_domain(Param::ref_by_name(&param_name))
                    }),
                });
            }
            
            if scale_info.matches_target(&self.y_target) {
                let param_name = format!("panzoom_{}_y_domain", state_key);
                modifiers.push(ScaleModifier {
                    target: ScaleTarget::Specific(scale_id.clone()),
                    modify: Box::new(move |scale| {
                        scale.raw_domain(Param::ref_by_name(&param_name))
                    }),
                });
            }
        }
        
        modifiers
    }
    
    fn create_param_streams(
        &self,
        scale_registry: &ScaleRegistry,
        state_map: &StateMap<PanZoomState>,
    ) -> Vec<Arc<dyn ParamStream>> {
        let mut streams = vec![];
        
        // Group scales by state key
        for (state_key, scale_group) in scale_registry.group_by_state(state_map) {
            let x_scales: Vec<_> = scale_group.iter()
                .filter(|s| s.matches_target(&self.x_target))
                .collect();
            let y_scales: Vec<_> = scale_group.iter()
                .filter(|s| s.matches_target(&self.y_target))
                .collect();
            
            // Create param streams for this state group
            for (x_scale, y_scale) in x_scales.iter().zip(y_scales.iter()) {
                streams.push(Arc::new(PanMouseDownParamStream {
                    scales: vec![x_scale.clone(), y_scale.clone()],
                    state_key: state_key.clone(),
                    x_param_name: format!("panzoom_{}_x_domain", state_key),
                    y_param_name: format!("panzoom_{}_y_domain", state_key),
                }));
                
                streams.push(Arc::new(PanMouseMoveParamStream {
                    scales: vec![x_scale.clone(), y_scale.clone()],
                    state_key: state_key.clone(),
                    x_param_name: format!("panzoom_{}_x_domain", state_key),
                    y_param_name: format!("panzoom_{}_y_domain", state_key),
                }));
            }
        }
        
        streams
    }
}
```

### Example ParamStream with State

ParamStreams now receive mutable state access:

#### 1. Pan Mouse Down - Updates State
```rust
#[derive(Debug, Clone)]
pub struct PanMouseDownParamStream {
    scales: Vec<Scale>,
    state_key: StateKey,
    x_param_name: String,
    y_param_name: String,
}

impl PanMouseDownParamStream {
    pub fn new(
        x_scale: Scale,
        y_scale: Scale,
        state_key: StateKey,
        x_param_name: String,
        y_param_name: String,
    ) -> Self {
        Self {
            scales: vec![x_scale, y_scale],
            state_key,
            x_param_name,
            y_param_name,
        }
    }
}

impl ParamStream for PanMouseDownParamStream {
    type State = PanZoomState;
    
    fn stream_config(&self) -> &EventStreamConfig {
        &EventStreamConfig {
            types: vec![SceneGraphEventType::MouseDown],
            filter: Some(vec![EventStreamFilter(Arc::new(|event| {
                matches!(event, SceneGraphEvent::MouseDown(e) if e.button == MouseButton::Left)
            }))]),
            ..Default::default()
        }
    }

    fn input_params(&self) -> &[String] {
        &[]  // No input params needed
    }

    fn input_scales(&self) -> &[Scale] {
        &self.scales
    }

    fn update(
        &self,
        context: ParamStreamContext,
        state: &mut PanZoomState,  // Mutable state access!
    ) -> (HashMap<String, ScalarValue>, UpdateStatus) {
        // Check if this event is for our facet
        let facet_under_cursor = context.rtree.get_facet_at_position(context.event.position());
        if !self.state_key.matches_facet(facet_under_cursor) {
            return (HashMap::new(), UpdateStatus::default());
        }
        
        // Get scales and compute position
        let x_scale = &context.scales[0];
        let y_scale = &context.scales[1];
        let event_position = context.event.position().unwrap();
        let plot_origin = context.rtree.group_origin(context.group_path).unwrap();
        let plot_x = event_position[0] - plot_origin[0];
        let plot_y = event_position[1] - plot_origin[1];

        // Check if cursor is over the plot area
        let (x_start, x_end) = x_scale.numeric_interval_range().unwrap();
        let (y_start, y_end) = y_scale.numeric_interval_range().unwrap();
        if plot_x < x_start || plot_x > x_end || plot_y < y_start || plot_y > y_end {
            return (HashMap::new(), UpdateStatus::default());
        }

        // Update state
        state.anchor_position = Some([plot_x, plot_y]);
        state.anchor_x_domain = Some(x_scale.get_domain_scalar());
        state.anchor_y_domain = Some(y_scale.get_domain_scalar());
        state.is_panning = true;

        // No parameter updates needed - state is stored separately
        (HashMap::new(), UpdateStatus::default())
    }
}
```

#### 2. Pan Mouse Move - Uses State to Update Domains
```rust
#[derive(Debug, Clone)]
pub struct PanMouseMoveParamStream {
    scales: Vec<Scale>,
    state_key: StateKey,
    x_param_name: String,
    y_param_name: String,
}

impl ParamStream for PanMouseMoveParamStream {
    type State = PanZoomState;
    
    fn stream_config(&self) -> &EventStreamConfig {
        &EventStreamConfig {
            types: vec![SceneGraphEventType::CursorMoved],
            // Only active during drag
            between: Some((
                Box::new(EventStreamConfig {
                    types: vec![SceneGraphEventType::MouseDown],
                    filter: Some(vec![EventStreamFilter(Arc::new(|e| {
                        matches!(e, SceneGraphEvent::MouseDown(ev) if ev.button == MouseButton::Left)
                    }))]),
                    ..Default::default()
                }),
                Box::new(EventStreamConfig {
                    types: vec![SceneGraphEventType::MouseUp],
                    filter: Some(vec![EventStreamFilter(Arc::new(|e| {
                        matches!(e, SceneGraphEvent::MouseUp(ev) if ev.button == MouseButton::Left)
                    }))]),
                    ..Default::default()
                }),
            )),
            ..Default::default()
        }
    }

    fn update(
        &self,
        context: ParamStreamContext,
        state: &mut PanZoomState,
    ) -> (HashMap<String, ScalarValue>, UpdateStatus) {
        // Only handle if we're panning and it's our facet
        if !state.is_panning {
            return (HashMap::new(), UpdateStatus::default());
        }
        
        let facet_under_cursor = context.rtree.get_facet_at_position(context.event.position());
        if !self.state_key.matches_facet(facet_under_cursor) {
            return (HashMap::new(), UpdateStatus::default());
        }
        
        // Use stored anchor state
        let Some(anchor_pos) = state.anchor_position else {
            return (HashMap::new(), UpdateStatus::default());
        };
        
        // Calculate pan deltas
        let event_position = context.event.position().unwrap();
        let plot_origin = context.rtree.group_origin(context.group_path).unwrap();
        let plot_x = event_position[0] - plot_origin[0];
        let plot_y = event_position[1] - plot_origin[1];
        
        let dx = plot_x - anchor_pos[0];
        let dy = plot_y - anchor_pos[1];
        
        // Get scales and compute new domains
        let x_scale = &context.scales[0].with_domain(&state.anchor_x_domain.clone().unwrap());
        let y_scale = &context.scales[1].with_domain(&state.anchor_y_domain.clone().unwrap());
        
        let (x_start, x_end) = x_scale.numeric_interval_range().unwrap();
        let x_delta = dx / (x_end - x_start);
        let new_x_domain = x_scale.pan(x_delta).unwrap().get_domain_scalar();
        
        let (y_start, y_end) = y_scale.numeric_interval_range().unwrap();
        let y_delta = dy / (y_end - y_start);
        let new_y_domain = y_scale.pan(-y_delta).unwrap().get_domain_scalar();
        
        // Return updated parameter values
        let new_params = vec![
            (self.x_param_name.clone(), new_x_domain),
            (self.y_param_name.clone(), new_y_domain),
        ]
        .into_iter()
        .collect();

        (new_params, UpdateStatus { rerender: true, rebuild_geometry: false })
    }
}
```

### Faceting Support

The stateless controller design elegantly handles faceting through the state management system:

```rust
// Shared scales = shared state
Chart::new()
    .data(df)
    .facet_by("category")
    .resolve(Resolution::shared())
    .mark(Symbol::new().x("a").y("b"))
    .controller(PanZoom::new())  // One PanZoomState shared across facets

// Independent scales = per-facet state
Chart::new()
    .data(df)
    .facet_by("category")
    .resolve(Resolution::independent())
    .mark(Symbol::new().x("a").y("b"))
    .controller(PanZoom::new())  // Separate PanZoomState per facet

// Mixed resolution (shared rows)
Chart::new()
    .data(df)
    .facet_grid("row", "col")
    .resolve(Resolution::shared_rows())  // Share x scales per row
    .mark(Symbol::new().x("a").y("b"))
    .controller(PanZoom::new())  // State shared per row
```

The controller automatically adapts to the faceting structure:
- With `Resolution::shared`, all facets share the same state instance
- With `Resolution::independent`, each facet gets its own state instance
- With mixed resolutions, state is grouped according to the sharing pattern

### Chart Compilation Process

The compilation process handles all the complexity:

```rust
impl Chart {
    async fn compile(&self) -> CompiledChart {
        // Phase 1: Collect all scales from marks
        let mut scale_registry = self.collect_scales();
        
        // Phase 2: Apply controller modifications
        for controller in &self.controllers {
            for modifier in controller.scale_modifiers() {
                match modifier.target {
                    ScaleTarget::Named(name) => {
                        if let Some(scale) = scale_registry.get_mut(&name) {
                            *scale = (modifier.modify)(scale.clone());
                        }
                    }
                    ScaleTarget::Encoding(channel) => {
                        // Find all scales used for this encoding
                        for (name, scale) in scale_registry.iter_mut() {
                            if scale.used_by_encoding(channel) {
                                *scale = (modifier.modify)(scale.clone());
                            }
                        }
                    }
                    ScaleTarget::All => {
                        for (_, scale) in scale_registry.iter_mut() {
                            *scale = (modifier.modify)(scale.clone());
                        }
                    }
                }
            }
        }
        
        // Phase 3: Create state management for controllers
        for controller in &self.controllers {
            // Determine state mode based on chart configuration
            let state_mode = controller.state_mode(&self.config);
            
            // Create state map based on mode
            let state_map = match state_mode {
                StateMode::Shared => {
                    StateMap::shared(controller.default_state())
                }
                StateMode::PerFacet => {
                    StateMap::per_facet(&self.facets, controller.default_state)
                }
                StateMode::PerRow => {
                    StateMap::per_row(&self.facet_grid, controller.default_state)
                }
                StateMode::Custom(grouping_fn) => {
                    StateMap::custom(&self.facets, grouping_fn, controller.default_state)
                }
            };
            
            // Generate params and modifiers based on state map
            let params = controller.generate_params(&state_map);
            let modifiers = controller.generate_scale_modifiers(&scale_registry, &state_map);
            
            // Apply generated modifiers
            for modifier in modifiers {
                // Apply modifier to registry (as shown in Phase 2)
            }
            
            // Create param streams with state access
            let param_streams = controller.create_param_streams(&scale_registry, &state_map);
            
            // Register everything with the runtime
            runtime.register(controller, state_map, params, param_streams);
        }
        
        // Phase 4: Continue with normal compilation
        // ...
    }
}
```

### Benefits of Scale Modification Pipeline

1. **Declarative**: Users just add `.controller(PanZoom::new())`
2. **Flexible**: Can target scales by name, encoding, or all
3. **Composable**: Multiple controllers can modify same scales
4. **Late Binding**: Scale resolution happens at compile time
5. **Type Safe**: Scale modifications are checked at compile time

### Usage Example

The new declarative design makes charts much simpler to write:

```rust
use avenger_chart::prelude::*;
use datafusion::prelude::*;

// Load data
let df = ctx.read_csv("data.csv", CsvReadOptions::default()).await?;

// Simple case - automatic scale discovery
let chart = Chart::new()
    .data(df)
    .mark(Symbol::new()
        .x("x_column")  // Implicit x scale created
        .y("y_column")  // Implicit y scale created
        .fill("category")
        .size(10.0))
    .controller(PanZoom::new());  // Automatically binds to x/y scales

// The chart compilation process:
// 1. Creates implicit scales for encodings
// 2. Applies controller scale modifiers
// 3. Creates ParamStreams with resolved scales
// 4. Registers event handlers
// 5. Renders with parameter bindings

// Advanced case - explicit scale control
let chart = Chart::new()
    .data(df)
    .scale("custom_x", Scale::linear().domain(0, 100))
    .scale("custom_y", Scale::log())
    .mark(Symbol::new()
        .x(scale_field("custom_x", "x_column"))
        .y(scale_field("custom_y", "y_column")))
    .controller(PanZoom::new()
        .x_scale("custom_x")  // Explicitly target named scales
        .y_scale("custom_y"));

// Multiple marks with shared scales
let chart = Chart::new()
    .data(df)
    .mark(Symbol::new().x("a").y("b"))
    .mark(Line::new().x("a").y("b"))    // Same encodings
    .mark(Area::new().x("a").y2("c"))   // Different y encoding
    .controller(PanZoom::new());  // Affects all x/y encoded scales

// Composing multiple controllers
let chart = Chart::new()
    .data(df)
    .mark(Symbol::new()
        .x("x")
        .y("y")
        .fill("category")
        .opacity(when(col("id").in_list(placeholder("$selected"), false), 1.0).otherwise(0.3)))
    .controller(PanZoom::new())
    .controller(BrushSelect::new()
        .selection_param("selected"));  // Updates $selected placeholder
```

## Advantages of DataFrame Placeholder Approach

### 1. **Unified Parameterization**
- Every aspect of the visualization can be parameterized
- Data, scales, encodings all use the same parameter system
- Natural integration with DataFusion's DataFrame API
- Stay in DataFrame land without SQL string manipulation

### 2. **Performance**
- Logical plans are optimized and cached by DataFusion
- Only affected DataFrame operations re-execute on parameter changes
- Efficient dependency tracking minimizes recomputation
- Type-safe operations without SQL parsing overhead

### 3. **Flexibility**
- Complex conditions via DataFrame `when().then().otherwise()` expressions
- Aggregations and calculations in DataFrame operations
- Can reference multiple parameters in one query
- Composable DataFrame transformations

### 4. **Extensibility**
- New interaction handlers can be added without core changes
- Custom handlers implement EventStreamHandler trait
- Handlers can be shared as separate packages
- Leverage existing avenger-eventstream infrastructure

### 5. **Type Safety and Developer Experience**
- Compile-time type checking for DataFrame operations
- IDE support for method completion
- Rust's ownership system prevents common errors
- No SQL injection concerns

## Implementation Considerations

### 1. **Parameter Types**
Need to support various DataFusion ScalarValue types:
- Scalars: Int64, Float64, Utf8, Date32, TimestampMillisecond
- Lists: ScalarValue::List for multi-selection
- Structs: ScalarValue::Struct for complex state
- Null: ScalarValue::Null for unset parameters

### 2. **Event Coordination**
- Leverage EventStreamConfig for event filtering and prioritization
- Use `consume` flag for exclusive event handling
- `throttle` option for performance-sensitive handlers
- `between` operator for complex event sequences

### 3. **Performance Optimization**
- DataFrame logical plan caching by DataFusion
- Parameter binding at execution time
- UpdateStatus flags control re-rendering vs geometry rebuild
- Event throttling built into EventStreamConfig

### 4. **Integration with Existing Infrastructure**
- EventStreamManager handles event dispatch
- SceneGraphRTree enables efficient spatial queries
- ModifiersState tracks keyboard state
- Window events automatically converted to SceneGraphEvent

## Comparison with Existing Systems

| Feature | Vega-Lite | Observable Plot | D3.js | Avenger (Proposed) |
|---------|-----------|-----------------|-------|-------------------|
| Declarative | ✅ High | ✅ Medium | ❌ Low | ✅ High |
| Flexibility | ✅ High | ⚠️ Limited | ✅ Very High | ✅ Very High |
| Performance | ⚠️ Medium | ✅ Good | ✅ Excellent | ✅ Excellent |
| Extensibility | ⚠️ Limited | ⚠️ Limited | ✅ High | ✅ High |
| Learning Curve | ⚠️ Steep | ✅ Gentle | ⚠️ Steep | ✅ Medium |
| DataFrame Integration | ❌ None | ❌ None | ❌ None | ✅ Native |
| Scale-Aware Interactions | ⚠️ Limited | ❌ None | ⚠️ Manual | ✅ Built-in |
| State Management | ⚠️ Complex | ❌ Limited | ⚠️ Manual | ✅ Structured |

## Key Architectural Insights

The updated controller-based design improves on the prototype by making interactions truly declarative:

### 1. **Declarative Scale Binding**
The Scale Modification Pipeline enables:
- Controllers don't create scales, they modify them
- Automatic discovery via encoding channels
- Explicit targeting when needed
- Late binding at compile time

### 2. **Improved Separation of Concerns**
- **Chart**: Defines data, marks, and encodings
- **Controllers**: Declare scale modifications and parameters
- **Compilation**: Resolves scales and creates ParamStreams
- **Runtime**: Handles events and parameter updates

### 3. **Composable Controllers**
Multiple controllers work together:
```rust
chart
    .controller(PanZoom::new())          // Modifies x/y scale domains
    .controller(BrushSelect::new())      // Updates selection parameter
    .controller(Tooltip::new())          // Shows details on hover
```

### 4. **Progressive Enhancement**
- Simple case: `.controller(PanZoom::new())` just works
- Advanced: Target specific scales by name
- Expert: Custom ParamStreams for novel interactions

## Conclusion

The evolved controller-based interaction system improves upon the prototype with several key innovations:

### Key Design Principles

1. **Stateless Controllers**: Controllers are pure configuration objects that declare their state needs
2. **Declarative Scale Binding**: Scale Modification Pipeline allows controllers to modify scales without creating them
3. **Automatic Facet Support**: State management automatically adapts to resolve configuration
4. **Type-Safe State**: Each controller declares its state type, enabling compile-time safety
5. **Progressive Enhancement**: Simple cases are simple, complex cases are possible

### Architectural Benefits

- **Separation of Concerns**: Controllers declare what they need, runtime manages how it works
- **Testability**: Stateless controllers are easy to test in isolation
- **Composability**: Multiple controllers work together naturally
- **Performance**: State is efficiently managed and parameter updates are optimized
- **Flexibility**: Custom state modes and scale targeting provide full control when needed

### User Experience

The declarative API makes common interactions trivial while preserving power for advanced use cases:

```rust
// Simple: Just works with sensible defaults
chart.controller(PanZoom::new())

// Advanced: Full control when needed
chart.controller(PanZoom::new()
    .x_scale("custom_x")
    .y_scale("custom_y"))
```

This architecture successfully balances the competing demands of simplicity, flexibility, and performance, providing a solid foundation for interactive data visualization in the grammar of graphics tradition.

## Next Steps

1. **Core Implementation**:
   - Port Param and ParamStream traits from prototype
   - Implement Controller trait and registration system
   - Create ParamEventStreamHandler bridge
   - Integrate with chart compilation pipeline

2. **Standard Controllers**:
   - Port PanZoomController with scale integration
   - Implement BrushController for selections
   - Create TooltipController with mark details
   - Build ClickSelectController with modifiers

3. **Scale Integration**:
   - Ensure all scale types support required operations
   - Implement scale inversion for coordinate transforms
   - Add pan/zoom methods to scale trait
   - Create scale-aware hit testing

4. **Documentation and Examples**:
   - Port and update prototype examples
   - Create controller composition examples
   - Document ParamStream patterns
   - Build interactive gallery