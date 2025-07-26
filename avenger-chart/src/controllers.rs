//! Controller types for interactive visualizations

use crate::params::Param;
use crate::scales::ScaleRegistry;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

/// Determines how controller state is shared across facets
#[derive(Debug, Clone, Copy)]
pub enum StateMode {
    /// Single shared state instance
    Shared,
    /// Independent state per facet
    PerFacet,
    /// Shared state per row
    PerRow,
    /// Shared state per column
    PerColumn,
    /// Custom state distribution
    Custom,
}

/// Maps state instances based on facet configuration
#[derive(Debug, Clone)]
pub struct StateMap<T> {
    states: HashMap<String, T>,
}

impl<T> Default for StateMap<T> {
    fn default() -> Self {
        Self {
            states: HashMap::new(),
        }
    }
}

impl<T> StateMap<T> {
    /// Create a new state map
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
        }
    }

    /// Get state for a facet
    pub fn get(&self, facet_id: &str) -> Option<&T> {
        self.states.get(facet_id)
    }
}

/// Configuration for the chart including faceting
#[derive(Debug, Clone)]
pub struct ChartConfig {
    // Stub - would contain faceting and resolve configuration
}

/// A parameter stream that maps events to parameter updates
pub trait ParamStream: Debug + Send + Sync {
    /// Get parameters that this stream updates
    fn output_params(&self) -> Vec<String>;
}

/// Specifies which scales to target
#[derive(Debug, Clone)]
pub enum ScaleTarget {
    /// Target scales by name
    Named(Vec<String>),
    /// Target scales by encoding channel
    Encoding(Vec<String>),
    /// Target all scales
    All,
}

/// Describes how to transform a scale
#[derive(Debug, Clone)]
pub struct ScaleTransform {
    /// Which scale(s) to transform
    pub target: ScaleTarget,
    /// The transformation to apply (stub)
    pub transform: String,
}

/// Modifies scales based on controller state
#[derive(Debug, Clone)]
pub struct ScaleModifier {
    /// Scale transformations to apply
    pub transforms: Vec<ScaleTransform>,
}

/// Main controller trait for organizing interaction logic
pub trait Controller: Debug + Send + Sync + 'static {
    /// The state type this controller uses
    type State: Clone + Default + Send + Sync + 'static;

    /// Get the controller name
    fn name(&self) -> &str;

    /// Determine how state should be distributed across facets
    fn state_mode(&self, chart_config: &ChartConfig) -> StateMode;

    /// Create parameter streams for this controller
    fn create_param_streams(
        &self,
        scale_registry: &ScaleRegistry,
        state_map: &StateMap<Self::State>,
    ) -> Vec<Arc<dyn ParamStream>>;

    /// Generate current parameters from state
    fn generate_params(&self, state_map: &StateMap<Self::State>) -> Vec<Param>;

    /// Generate scale modifiers based on current state
    fn generate_scale_modifiers(
        &self,
        scale_registry: &ScaleRegistry,
        state_map: &StateMap<Self::State>,
    ) -> Vec<ScaleModifier>;
}

/// Pan/zoom controller for 2D navigation
#[derive(Debug, Clone, Default)]
pub struct PanZoom {
    // Configuration fields would go here
}

impl PanZoom {
    /// Create a new pan/zoom controller with defaults
    pub fn new() -> Self {
        Self::default()
    }
}

// Stub implementation for compilation
impl Controller for PanZoom {
    type State = ();

    fn name(&self) -> &str {
        "pan-zoom"
    }

    fn state_mode(&self, _chart_config: &ChartConfig) -> StateMode {
        StateMode::Shared
    }

    fn create_param_streams(
        &self,
        _scale_registry: &ScaleRegistry,
        _state_map: &StateMap<Self::State>,
    ) -> Vec<Arc<dyn ParamStream>> {
        vec![]
    }

    fn generate_params(&self, _state_map: &StateMap<Self::State>) -> Vec<Param> {
        vec![]
    }

    fn generate_scale_modifiers(
        &self,
        _scale_registry: &ScaleRegistry,
        _state_map: &StateMap<Self::State>,
    ) -> Vec<ScaleModifier> {
        vec![]
    }
}

/// Box selection controller
#[derive(Debug, Clone, Default)]
pub struct BoxSelect {
    // Configuration fields would go here
}

impl BoxSelect {
    /// Create a new box selection controller
    pub fn new() -> Self {
        Self::default()
    }
}

// Stub implementation for compilation
impl Controller for BoxSelect {
    type State = ();

    fn name(&self) -> &str {
        "box-select"
    }

    fn state_mode(&self, _chart_config: &ChartConfig) -> StateMode {
        StateMode::Shared
    }

    fn create_param_streams(
        &self,
        _scale_registry: &ScaleRegistry,
        _state_map: &StateMap<Self::State>,
    ) -> Vec<Arc<dyn ParamStream>> {
        vec![]
    }

    fn generate_params(&self, _state_map: &StateMap<Self::State>) -> Vec<Param> {
        vec![]
    }

    fn generate_scale_modifiers(
        &self,
        _scale_registry: &ScaleRegistry,
        _state_map: &StateMap<Self::State>,
    ) -> Vec<ScaleModifier> {
        vec![]
    }
}
