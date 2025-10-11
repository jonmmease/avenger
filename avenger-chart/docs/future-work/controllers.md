# Controllers and Interactivity

## Purpose

Manage interactive behaviors like pan/zoom, selection, and brushing through a controller abstraction with state management.

## Core Architecture

```rust
use datafusion::logical_expr::expr::Placeholder;
use datafusion::logical_expr::Expr;
use datafusion::scalar::ScalarValue;

/// A parameter that can be updated by controllers
/// (Already implemented in avenger-chart/src/param.rs)
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: ScalarValue,
}

impl Param {
    pub fn new<S: Into<String>, T: Into<ScalarValue>>(name: S, default: T) -> Self {
        Self {
            name: name.into(),
            default: default.into(),
        }
    }

    pub fn expr(&self) -> Expr {
        Expr::Placeholder(Placeholder {
            id: format!("${}", self.name),
            data_type: Some(self.default.data_type()),
        })
    }
}

/// Controller trait for organizing interaction logic
pub trait Controller: Debug + Send + Sync + 'static {
    type State: Clone + Default + Send + Sync + 'static;

    fn name(&self) -> &str;
    fn state_mode(&self, chart_config: &ChartConfig) -> StateMode;
    fn create_param_streams(
        &self,
        scale_registry: &ScaleRegistry,
        state_map: &StateMap<Self::State>,
    ) -> Vec<Arc<dyn ParamStream>>;
    fn generate_params(&self, state_map: &StateMap<Self::State>) -> Vec<Param>;
    fn generate_scale_modifiers(
        &self,
        scale_registry: &ScaleRegistry,
        state_map: &StateMap<Self::State>,
    ) -> Vec<ScaleModifier>;
}

#[derive(Debug, Clone, Copy)]
pub enum StateMode {
    Shared,        // Single state for all facets
    PerFacet,      // Independent state per facet
    PerRow,        // Shared state per row
    PerColumn,     // Shared state per column
}
```

## Pan/Zoom Controller Implementation

```rust
#[derive(Debug, Clone)]
pub struct PanZoom {
    x_channel: Option<String>,
    y_channel: Option<String>,
    wheel_zoom: bool,
    drag_pan: bool,
    double_click_reset: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PanZoomState {
    x_domain: Option<(f64, f64)>,
    y_domain: Option<(f64, f64)>,
    x_translate: f64,
    y_translate: f64,
    scale: f64,
}

impl Controller for PanZoom {
    type State = PanZoomState;

    fn name(&self) -> &str {
        "pan-zoom"
    }

    fn state_mode(&self, _chart_config: &ChartConfig) -> StateMode {
        StateMode::Shared  // Usually want consistent zoom across facets
    }

    fn create_param_streams(
        &self,
        scale_registry: &ScaleRegistry,
        state_map: &StateMap<Self::State>,
    ) -> Vec<Arc<dyn ParamStream>> {
        let mut streams = vec![];

        // Create wheel zoom stream
        if self.wheel_zoom {
            streams.push(Arc::new(WheelZoomStream::new(
                self.x_channel.clone(),
                self.y_channel.clone(),
                state_map.clone(),
            )));
        }

        // Create drag pan stream
        if self.drag_pan {
            streams.push(Arc::new(DragPanStream::new(
                self.x_channel.clone(),
                self.y_channel.clone(),
                state_map.clone(),
            )));
        }

        streams
    }

    fn generate_params(&self, state_map: &StateMap<Self::State>) -> Vec<Param> {
        let mut params = vec![];

        for (facet_id, state) in state_map.iter() {
            if let Some((x_min, x_max)) = state.x_domain {
                params.push(Param::new(
                    format!("{}_x_min", facet_id),
                    ScalarValue::Float64(Some(x_min)),
                ));
                params.push(Param::new(
                    format!("{}_x_max", facet_id),
                    ScalarValue::Float64(Some(x_max)),
                ));
            }

            if let Some((y_min, y_max)) = state.y_domain {
                params.push(Param::new(
                    format!("{}_y_min", facet_id),
                    ScalarValue::Float64(Some(y_min)),
                ));
                params.push(Param::new(
                    format!("{}_y_max", facet_id),
                    ScalarValue::Float64(Some(y_max)),
                ));
            }
        }

        params
    }

    fn generate_scale_modifiers(
        &self,
        scale_registry: &ScaleRegistry,
        state_map: &StateMap<Self::State>,
    ) -> Vec<ScaleModifier> {
        let mut modifiers = vec![];

        for (facet_id, state) in state_map.iter() {
            if let Some((x_min, x_max)) = state.x_domain {
                modifiers.push(ScaleModifier {
                    target: ScaleTarget::Named(vec![format!("{}_x", facet_id)]),
                    transform: ScaleTransform::SetDomain(x_min, x_max),
                });
            }

            if let Some((y_min, y_max)) = state.y_domain {
                modifiers.push(ScaleModifier {
                    target: ScaleTarget::Named(vec![format!("{}_y", facet_id)]),
                    transform: ScaleTransform::SetDomain(y_min, y_max),
                });
            }
        }

        modifiers
    }
}
```

## Box Selection Controller

```rust
#[derive(Debug, Clone)]
pub struct BoxSelect {
    channels: Vec<String>,
    selection_param: String,
    clear_on_empty: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BoxSelectState {
    selection_bounds: Option<SelectionBounds>,
    selected_indices: Vec<usize>,
}

#[derive(Debug, Clone)]
struct SelectionBounds {
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
}

impl Controller for BoxSelect {
    type State = BoxSelectState;

    fn generate_params(&self, state_map: &StateMap<Self::State>) -> Vec<Param> {
        use datafusion::prelude::{col, lit};

        let mut params = vec![];

        for (facet_id, state) in state_map.iter() {
            if let Some(ref bounds) = state.selection_bounds {
                // Create selection filter expression using .and() method
                let _filter = col("x")
                    .gt_eq(lit(bounds.x_min))
                    .and(col("x").lt_eq(lit(bounds.x_max)))
                    .and(col("y").gt_eq(lit(bounds.y_min)))
                    .and(col("y").lt_eq(lit(bounds.y_max)));

                params.push(Param::new(
                    format!("{}_{}", self.selection_param, facet_id),
                    ScalarValue::Boolean(Some(true)),  // Placeholder
                ));
            }
        }

        params
    }
}
```

## Usage with Marks

```rust
use avenger_chart::prelude::*;
use datafusion::scalar::ScalarValue;
use datafusion::logical_expr::when;

// Create plot with pan/zoom (future API)
let plot = Plot::<Cartesian>::new()
    .controller(PanZoom::new()  // Future: .controller() method
        .x_channel("x")
        .y_channel("y")
        .wheel_zoom(true)
        .drag_pan(true))
    .mark(Symbol::new()
        .data(df)
        .x(col("gdp"))
        .y(col("life_expectancy")));

// Box selection with conditional encoding (future API)
let selection_param = Param::new("selection", ScalarValue::Boolean(Some(false)));

let plot = Plot::<Cartesian>::new()
    .add_param(selection_param.clone())
    .controller(BoxSelect::new()  // Future: .controller() method
        .channels(vec!["x", "y"])
        .selection_param("selection"))
    .mark(Symbol::new()
        .data(df)
        .x(col("x"))
        .y(col("y"))
        .fill_with(
            when(selection_param.expr(), lit("#4682b4"))
                .otherwise(lit("#cccccc"))
                .unwrap(),
            |c| c
        ));
```

## Implementation Notes

- Controllers modify scales or generate parameters
- State is maintained per-facet or shared based on StateMode
- Event streams are created from controllers during plot compilation
- Integrates with existing avenger-eventstream infrastructure
