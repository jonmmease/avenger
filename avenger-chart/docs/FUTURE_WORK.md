# Avenger Chart - Future Work Reference

This document consolidates all experimental features and design work for future implementation. It provides complete specifications and implementation guidance for an LLM to rebuild these features.

## Table of Contents
1. [Adjust API - Post-Scale Position Adjustments](#1-adjust-api---post-scale-position-adjustments)
2. [Derive API - Child Mark Generation](#2-derive-api---child-mark-generation)
3. [Transform System - Data Transformations](#3-transform-system---data-transformations)
4. [Controllers and Interactivity](#4-controllers-and-interactivity)
5. [Text Mark Implementation](#5-text-mark-implementation)
6. [Faceting System](#6-faceting-system)
7. [Integration Points](#7-integration-points)
8. [Polar Coordinate System](#8-polar-coordinate-system)

---

## 1. Adjust API - Post-Scale Position Adjustments

### Purpose
Modify mark positions after scales have been applied, operating in visual/pixel space. Enables dodge positioning, jitter for overplotting, and smart label placement.

### Core Design

#### Trait Definition
```rust
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;

/// Context provided to post-scale transforms
#[derive(Clone)]
pub struct TransformContext {
    pub dimensions: PlotDimensions,
    pub session: SessionContext,
}

#[derive(Debug, Clone, Copy)]
pub struct PlotDimensions {
    pub width: f64,
    pub height: f64,
}

/// Trait for post-scale adjustments
pub trait Adjust: Send + Sync {
    /// Adjust mark positions/properties after scaling
    /// 
    /// DataFrame contains:
    /// - Scaled visual coordinates (x, y, size, etc.) in pixels
    /// - bbox struct column with {x_min, y_min, x_max, y_max}
    /// - Original data columns for grouping/filtering
    fn adjust(
        &self,
        df: DataFrame,
        context: &TransformContext,
    ) -> Result<DataFrame, AvengerChartError>;
}
```

#### Function Wrapper for Closures
```rust
pub struct AdjustFn<F> {
    f: F,
}

impl<F> AdjustFn<F>
where
    F: Fn(DataFrame, &TransformContext) -> Result<DataFrame, AvengerChartError> + Send + Sync,
{
    pub fn new(f: F) -> Self {
        Self { f }
    }
}

impl<F> Adjust for AdjustFn<F>
where
    F: Fn(DataFrame, &TransformContext) -> Result<DataFrame, AvengerChartError> + Send + Sync,
{
    fn adjust(&self, df: DataFrame, context: &TransformContext) -> Result<DataFrame, AvengerChartError> {
        (self.f)(df, context)
    }
}
```

### Key Implementations

#### Jitter - Add Random Noise
```rust
pub struct Jitter {
    x_amount: Option<f64>,
    y_amount: Option<f64>,
    seed: Option<u64>,
}

impl Adjust for Jitter {
    fn adjust(&self, df: DataFrame, _context: &TransformContext) -> Result<DataFrame, AvengerChartError> {
        use rand::{Rng, SeedableRng};
        use rand_chacha::ChaCha8Rng;
        
        let mut rng = match self.seed {
            Some(seed) => ChaCha8Rng::seed_from_u64(seed),
            None => ChaCha8Rng::from_entropy(),
        };
        
        let mut result = df;
        
        if let Some(amount) = self.x_amount {
            // Generate random offsets for each row
            let offsets: Vec<f64> = (0..result.count())
                .map(|_| rng.gen_range(-amount..amount))
                .collect();
            
            // Create array from offsets and add to x column
            let offset_array = Float64Array::from(offsets);
            result = result.with_column("x", col("x") + lit_array(offset_array))?;
        }
        
        // Similar for y_amount
        Ok(result)
    }
}
```

#### Dodge - Avoid Overlaps
```rust
pub struct Dodge {
    padding: f64,
    group_by: Option<String>,
}

impl Adjust for Dodge {
    fn adjust(&self, df: DataFrame, _context: &TransformContext) -> Result<DataFrame, AvengerChartError> {
        // Group marks by x position and optional grouping column
        let groups = if let Some(ref group_col) = self.group_by {
            df.group_by(vec![col("x"), col(group_col)])?
        } else {
            df.group_by(vec![col("x")])?
        };
        
        // For each group, calculate positions to avoid overlap
        let adjusted = groups.apply(|group_df| {
            let count = group_df.count();
            let total_width = count as f64 * (/* mark width */ + self.padding);
            
            // Create position offsets
            let offsets: Vec<f64> = (0..count)
                .map(|i| {
                    let position = i as f64 - (count as f64 - 1.0) / 2.0;
                    position * (/* mark width */ + self.padding)
                })
                .collect();
            
            // Apply offsets to x positions
            group_df.with_column("x", col("x") + lit_array(offsets))
        })?;
        
        Ok(adjusted)
    }
}
```

### Usage Examples
```rust
// Built-in adjustments
Symbol::new()
    .data(df)
    .x("category")
    .y("value")
    .adjust(Jitter::new().x(10.0).seed(42))
    .adjust(Dodge::new().padding(2.0));

// Custom adjustment with closure
Symbol::new()
    .adjust(AdjustFn::new(|df, context| {
        // Center points in left half of viewport
        let condition = col("x").lt(lit(context.width() / 2.0));
        let new_x = when(condition, col("x") + lit(context.width() / 4.0))
            .otherwise(col("x"))?;
        Ok(df.with_column("x", new_x)?)
    }));
```

---

## 2. Derive API - Child Mark Generation

### Purpose
Generate child marks from parent marks' scaled data. Enables automatic label placement, error bars, connectors, and annotations.

### Core Design

#### Trait Definition
```rust
use crate::marks::Mark;

/// Trait for deriving child marks from parent marks
pub trait Derive<C: CoordinateSystem>: Send + Sync {
    /// Generate child marks from parent's scaled data
    fn derive(
        &self,
        df: DataFrame,
        context: &TransformContext,
    ) -> Result<Box<dyn Mark<C>>, AvengerChartError>;
}

/// Function wrapper for closures
pub struct DeriveFn<C, F> {
    f: F,
    _phantom: std::marker::PhantomData<C>,
}
```

### Key Implementation: Label Points

```rust
pub struct LabelPoints {
    text_column: String,
    offset_y: f64,
    align: TextAlign,
    font_size: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

impl<C: CoordinateSystem> Derive<C> for LabelPoints {
    fn derive(
        &self,
        df: DataFrame,
        context: &TransformContext,
    ) -> Result<Box<dyn Mark<C>>, AvengerChartError> {
        // Calculate label positions based on parent marks
        let label_y = if self.offset_y < 0.0 {
            // Above: use top of bbox
            col("bbox.y_min") + lit(self.offset_y)
        } else {
            // Below: use bottom of bbox
            col("bbox.y_max") + lit(self.offset_y)
        };
        
        let label_df = df
            .with_column("label_y", label_y)?
            .with_column("label_text", col(&self.text_column))?;
        
        // Create Text mark with calculated positions
        let text_mark = Text::<C>::new()
            .data(label_df)
            .x(col("x"))
            .y(col("label_y"))
            .text(col("label_text"))
            .align(self.align)
            .font_size(self.font_size.unwrap_or(12.0));
        
        Ok(Box::new(text_mark))
    }
}
```

### Usage Examples
```rust
// Automatic labels above points
Symbol::new()
    .data(df)
    .x("gdp_per_capita")
    .y("life_expectancy")
    .derive(LabelPoints::new("country")
        .offset_y(-8.0)
        .align(TextAlign::Center));

// Error bars using derive
Symbol::new()
    .derive(DeriveFn::new(|scaled_df, _context| {
        let error_df = scaled_df
            .with_column("y_low", col("y") - col("stderr") * lit(1.96))?
            .with_column("y_high", col("y") + col("stderr") * lit(1.96))?;
        
        Ok(Box::new(Line::new()
            .data(error_df)
            .y(col("y_low"))
            .y2(col("y_high"))))
    }));

// Smart label placement with adjustment
Symbol::new()
    .derive(DeriveFn::new(|scaled_df, context| {
        let labels = Text::new()
            .data(scaled_df)
            .adjust(SmartLabelPlacement::new());
        Ok(Box::new(labels))
    }));
```

---

## 3. Transform System - Data Transformations

### Purpose
Pre-scale data transformations that are coordinate-system aware. Includes binning, grouping, and stacking operations.

### Core Architecture

#### Transform Trait
```rust
pub trait Transform {
    fn transform(&self, ctx: DataContext) -> Result<DataContext, AvengerChartError>;
    fn output_channels(&self) -> Vec<ChannelInfo>;
}

#[derive(Clone)]
pub struct DataContext {
    dataframe: DataFrame,
    encodings: HashMap<String, String>,  // channel -> column mapping
    metadata: HashMap<String, serde_json::Value>,
}

pub struct ChannelInfo {
    pub name: String,
    pub data_type: String,
    pub required: bool,
    pub description: String,
}
```

### Bin Transform Implementation

```rust
pub struct Bin<C: CoordinateSystem> {
    inner: BinNd<C>,
}

pub struct BinNd<C: CoordinateSystem> {
    configs: Vec<BinConfig>,
    agg: Option<Expr>,
    extra_aggs: Vec<(&'static str, Expr)>,
    _phantom: PhantomData<C>,
}

struct BinConfig {
    field: String,
    channel_start: String,
    channel_end: String,
    width: Option<f64>,
    bins: Option<usize>,
    nice: bool,
    domain: Option<(f64, f64)>,
}

// Cartesian-specific constructors
impl Bin<Cartesian> {
    pub fn x(field: impl Into<String>) -> Self {
        Self { inner: BinNd::new(vec![(field, "x")]) }
    }
    
    pub fn y(field: impl Into<String>) -> Self {
        Self { inner: BinNd::new(vec![(field, "y")]) }
    }
}

// Configuration methods
impl<C: CoordinateSystem> Bin<C> {
    pub fn width(self, width: f64) -> Self {
        Self { inner: self.inner.width_for(0, width) }
    }
    
    pub fn bins(self, bins: usize) -> Self {
        Self { inner: self.inner.bins_for(0, bins) }
    }
    
    pub fn aggregate(self, agg: Expr) -> Self {
        Self { inner: self.inner.aggregate(agg) }
    }
}

impl<C: CoordinateSystem> Transform for Bin<C> {
    fn transform(&self, ctx: DataContext) -> Result<DataContext, AvengerChartError> {
        let df = ctx.dataframe().clone();
        
        // Calculate bin boundaries
        for config in &self.configs {
            let column_data = df.column(&config.field)?;
            let (min, max) = calculate_extent(column_data)?;
            
            let (bin_edges, bin_width) = if let Some(width) = config.width {
                calculate_bins_by_width(min, max, width, config.nice)
            } else if let Some(bins) = config.bins {
                calculate_bins_by_count(min, max, bins, config.nice)
            } else {
                return Err(AvengerChartError::InvalidArgument(
                    "Bin requires either width or bins".to_string()
                ));
            };
            
            // Create binned DataFrame
            let binned = df
                .with_column(
                    &format!("{}_bin_start", config.field),
                    floor(col(&config.field) / lit(bin_width)) * lit(bin_width)
                )?
                .with_column(
                    &format!("{}_bin_end", config.field),
                    col(&format!("{}_bin_start", config.field)) + lit(bin_width)
                )?;
            
            // Apply aggregation if specified
            if let Some(ref agg_expr) = self.agg {
                let grouped = binned.group_by(vec![
                    col(&format!("{}_bin_start", config.field)),
                    col(&format!("{}_bin_end", config.field)),
                ])?;
                
                let aggregated = grouped.aggregate(vec![agg_expr.clone()])?;
                return Ok(DataContext::new(aggregated)
                    .with_encoding(&config.channel_start, &format!("{}_bin_start", config.field))
                    .with_encoding(&config.channel_end, &format!("{}_bin_end", config.field)));
            }
        }
        
        Ok(result)
    }
}
```

### Group Transform Implementation

```rust
pub struct Group<C: CoordinateSystem> {
    group_fields: Vec<(String, String)>,  // (field, channel)
    default_agg_channel: String,
    primary_agg: Option<Expr>,
    extra_aggs: Vec<(&'static str, Expr)>,
    _phantom: PhantomData<C>,
}

impl Group<Cartesian> {
    pub fn x(field: impl Into<String>) -> Self {
        Self::new(vec![(field, "x")], "y")
    }
    
    pub fn xfill(x_field: impl Into<String>, fill_field: impl Into<String>) -> Self {
        Self::new(vec![(x_field, "x"), (fill_field, "fill")], "y")
    }
}

impl<C: CoordinateSystem> Transform for Group<C> {
    fn transform(&self, ctx: DataContext) -> Result<DataContext, AvengerChartError> {
        let df = ctx.dataframe().clone();
        
        // Create group by expressions
        let group_cols: Vec<Expr> = self.group_fields
            .iter()
            .map(|(field, _)| col(field))
            .collect();
        
        // Create aggregation expressions
        let mut agg_exprs = vec![];
        if let Some(ref primary) = self.primary_agg {
            agg_exprs.push(primary.clone());
        }
        for (_, expr) in &self.extra_aggs {
            agg_exprs.push(expr.clone());
        }
        
        // Perform grouping and aggregation
        let grouped = df.group_by(group_cols)?;
        let aggregated = grouped.aggregate(agg_exprs)?;
        
        // Build result context with channel mappings
        let mut result = DataContext::new(aggregated);
        for (field, channel) in &self.group_fields {
            result = result.with_encoding(channel, field);
        }
        
        Ok(result)
    }
}
```

### Stack Transform Implementation

```rust
pub struct Stack<C: CoordinateSystem> {
    stack_channel: String,
    group_channel: String,
    order: StackOrder,
    offset: StackOffset,
    _phantom: PhantomData<C>,
}

#[derive(Debug, Clone, Copy)]
pub enum StackOrder {
    Appearance,
    Sum,
    Value,
    Reverse,
}

#[derive(Debug, Clone, Copy)]
pub enum StackOffset {
    Zero,
    Center,
    Normalize,
}

impl Stack<Cartesian> {
    pub fn y() -> Self {
        Self::new("y", "x")
    }
}

impl<C: CoordinateSystem> Transform for Stack<C> {
    fn transform(&self, ctx: DataContext) -> Result<DataContext, AvengerChartError> {
        let df = ctx.dataframe().clone();
        
        // Get grouping and stacking columns
        let group_col = ctx.encoding(&self.group_channel)
            .ok_or_else(|| AvengerChartError::MissingChannel(self.group_channel.clone()))?;
        let stack_col = ctx.encoding(&self.stack_channel)
            .ok_or_else(|| AvengerChartError::MissingChannel(self.stack_channel.clone()))?;
        
        // Sort according to order
        let sorted = match self.order {
            StackOrder::Appearance => df,
            StackOrder::Sum => df.sort(vec![col(&stack_col).sort(false)])?,
            StackOrder::Value => df.sort(vec![col(&stack_col)])?,
            StackOrder::Reverse => df.sort(vec![col("_row_id").sort(false)])?,
        };
        
        // Calculate cumulative sums within groups
        let window_spec = WindowSpec::new()
            .partition_by(vec![col(&group_col)])
            .order_by(vec![col("_row_id")]);
        
        let stacked = sorted
            .with_column(
                &format!("{}_stack_start", self.stack_channel),
                lag(sum(col(&stack_col)).over(window_spec.clone()), 1)
                    .fill_null(lit(0.0))
            )?
            .with_column(
                &format!("{}_stack_end", self.stack_channel),
                sum(col(&stack_col)).over(window_spec)
            )?;
        
        // Apply offset
        let final_df = match self.offset {
            StackOffset::Zero => stacked,
            StackOffset::Center => {
                // Calculate total per group and center
                let totals = stacked.group_by(vec![col(&group_col)])?
                    .aggregate(vec![sum(col(&stack_col)).alias("_total")])?;
                
                stacked.join(totals, JoinType::Inner, vec![&group_col])?
                    .with_column(
                        &format!("{}_stack_start", self.stack_channel),
                        col(&format!("{}_stack_start", self.stack_channel)) - col("_total") / lit(2.0)
                    )?
                    .with_column(
                        &format!("{}_stack_end", self.stack_channel),
                        col(&format!("{}_stack_end", self.stack_channel)) - col("_total") / lit(2.0)
                    )?
            }
            StackOffset::Normalize => {
                // Normalize to 0-100%
                let totals = stacked.group_by(vec![col(&group_col)])?
                    .aggregate(vec![sum(col(&stack_col)).alias("_total")])?;
                
                stacked.join(totals, JoinType::Inner, vec![&group_col])?
                    .with_column(
                        &format!("{}_stack_start", self.stack_channel),
                        col(&format!("{}_stack_start", self.stack_channel)) / col("_total") * lit(100.0)
                    )?
                    .with_column(
                        &format!("{}_stack_end", self.stack_channel),
                        col(&format!("{}_stack_end", self.stack_channel)) / col("_total") * lit(100.0)
                    )?
            }
        };
        
        Ok(DataContext::new(final_df)
            .with_encoding(&format!("{}1", self.stack_channel), &format!("{}_stack_start", self.stack_channel))
            .with_encoding(&format!("{}2", self.stack_channel), &format!("{}_stack_end", self.stack_channel)))
    }
}
```

### Usage Examples

```rust
// Histogram with binning
Rect::new()
    .data(df)
    .transform(Bin::x("price")
        .bins(10)
        .aggregate(count(lit(1))))
    .y(lit(0));  // x, x2, y2 set automatically by transform

// 2D histogram
Rect::new()
    .transform(BinNd::xy("price", "weight")
        .width_x(500.0)
        .bins_y(5)
        .aggregate(count(lit(1))))
    .fill(col("count"));

// Grouped aggregation
Rect::new()
    .transform(Group::x("category")
        .aggregate(sum(col("sales"))));

// Stacked bar chart
Rect::new()
    .transform(Group::xfill("month", "product")
        .aggregate(sum(col("sales"))))
    .transform(Stack::y());
```

---

## 4. Controllers and Interactivity

### Purpose
Manage interactive behaviors like pan/zoom, selection, and brushing through a controller abstraction with state management.

### Core Architecture

```rust
use datafusion::scalar::ScalarValue;

/// A parameter that can be updated by controllers
#[derive(Debug, Clone)]
pub struct Param {
    name: String,
    value: ScalarValue,
}

impl Param {
    pub fn expr(&self) -> datafusion::logical_expr::Expr {
        use datafusion::logical_expr::{Expr, expr::Placeholder};
        Expr::Placeholder(Placeholder {
            id: self.name.clone(),
            data_type: None,
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

### Pan/Zoom Controller Implementation

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

### Box Selection Controller

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
        let mut params = vec![];
        
        for (facet_id, state) in state_map.iter() {
            if let Some(ref bounds) = state.selection_bounds {
                // Create selection filter expression
                let filter = and(
                    and(
                        col("x").gt_eq(lit(bounds.x_min)),
                        col("x").lt_eq(lit(bounds.x_max))
                    ),
                    and(
                        col("y").gt_eq(lit(bounds.y_min)),
                        col("y").lt_eq(lit(bounds.y_max))
                    )
                );
                
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

### Usage with Marks

```rust
// Create plot with pan/zoom
let plot = Plot::new(Cartesian)
    .controller(PanZoom::new()
        .x_channel("x")
        .y_channel("y")
        .wheel_zoom(true)
        .drag_pan(true))
    .mark(Symbol::new()
        .data(df)
        .x("gdp")
        .y("life_expectancy"));

// Box selection with conditional encoding
let selection_param = Param::new("selection", ScalarValue::Boolean(Some(false)));

let plot = Plot::new(Cartesian)
    .param(selection_param.clone())
    .controller(BoxSelect::new()
        .channels(vec!["x", "y"])
        .selection_param("selection"))
    .mark(Symbol::new()
        .data(df)
        .x("x")
        .y("y")
        .fill(when(selection_param.expr(), lit("#4682b4"))
            .otherwise(lit("#cccccc"))));
```

---

## 5. Text Mark Implementation

### Purpose
Render text labels on visualizations, needed for annotations, labels, and titles.

### Implementation

```rust
use avenger_scenegraph::marks::text::{
    TextMark as SceneTextMark,
    TextAlign as SceneTextAlign,
    TextBaseline as SceneTextBaseline,
    TextWeight,
};

pub struct Text<C: CoordinateSystem> {
    state: MarkState<C>,
}

impl<C: CoordinateSystem> Text<C> {
    pub fn new() -> Self {
        Self {
            state: MarkState::default(),
        }
    }
    
    // Channel methods
    channel_methods! {
        text: String,
        font: String,
        font_size: f64,
        font_weight: TextWeight,
        align: TextAlign,
        baseline: TextBaseline,
        angle: f64,
        dx: f64,  // Horizontal offset
        dy: f64,  // Vertical offset
        limit: f64,  // Text truncation limit
    }
}

impl<C: CoordinateSystem> Mark<C> for Text<C> {
    fn to_scene_graph(&self, scales: &ScaleSet) -> Result<Vec<SceneMark>, AvengerChartError> {
        let df = self.resolve_data(scales)?;
        
        let mut text_marks = vec![];
        
        for row in df.iter() {
            let text_mark = SceneTextMark {
                text: row.get_string("text")?,
                x: row.get_f64("x")?,
                y: row.get_f64("y")?,
                font: row.get_string_or("font", "Atkinson Hyperlegible Next")?,
                font_size: row.get_f64_or("font_size", 12.0)?,
                font_weight: row.get_enum_or("font_weight", TextWeight::Normal)?,
                align: row.get_enum_or("align", TextAlign::Left)?,
                baseline: row.get_enum_or("baseline", TextBaseline::Alphabetic)?,
                angle: row.get_f64_or("angle", 0.0)?,
                dx: row.get_f64_or("dx", 0.0)?,
                dy: row.get_f64_or("dy", 0.0)?,
                fill: row.get_color_or("fill", Color::BLACK)?,
                opacity: row.get_f64_or("opacity", 1.0)?,
                limit: row.get_f64_or("limit", f64::INFINITY)?,
            };
            
            text_marks.push(SceneMark::Text(text_mark));
        }
        
        Ok(text_marks)
    }
}
```

### Smart Label Placement

```rust
pub struct SmartLabelPlacement {
    avoid_overlap: bool,
    avoid_marks: bool,
    padding: f64,
    max_iterations: usize,
}

impl Adjust for SmartLabelPlacement {
    fn adjust(&self, df: DataFrame, context: &TransformContext) -> Result<DataFrame, AvengerChartError> {
        if !self.avoid_overlap {
            return Ok(df);
        }
        
        // Build spatial index of existing labels
        let mut rtree = RTree::new();
        let mut label_bounds = vec![];
        
        for (i, row) in df.iter().enumerate() {
            let bounds = calculate_text_bounds(
                row.get_string("text")?,
                row.get_f64("font_size")?,
                row.get_f64("x")?,
                row.get_f64("y")?,
            );
            
            rtree.insert(bounds.clone());
            label_bounds.push(bounds);
        }
        
        // Iteratively adjust overlapping labels
        for _ in 0..self.max_iterations {
            let mut adjusted = false;
            
            for i in 0..label_bounds.len() {
                let bounds = &label_bounds[i];
                
                // Find overlapping labels
                let overlaps: Vec<_> = rtree.locate_in_envelope_intersecting(bounds)
                    .filter(|other| !std::ptr::eq(*other, bounds))
                    .collect();
                
                if !overlaps.is_empty() {
                    // Calculate repulsion vector
                    let mut dx = 0.0;
                    let mut dy = 0.0;
                    
                    for other in overlaps {
                        let overlap_x = bounds.center_x() - other.center_x();
                        let overlap_y = bounds.center_y() - other.center_y();
                        let distance = (overlap_x * overlap_x + overlap_y * overlap_y).sqrt();
                        
                        if distance > 0.0 {
                            dx += overlap_x / distance * self.padding;
                            dy += overlap_y / distance * self.padding;
                        }
                    }
                    
                    // Update position
                    label_bounds[i].translate(dx, dy);
                    adjusted = true;
                }
            }
            
            if !adjusted {
                break;
            }
        }
        
        // Apply adjusted positions to DataFrame
        let mut result = df;
        for (i, bounds) in label_bounds.iter().enumerate() {
            result = result.with_column_at_index(
                i,
                "x",
                lit(bounds.center_x())
            )?;
            result = result.with_column_at_index(
                i,
                "y",
                lit(bounds.center_y())
            )?;
        }
        
        Ok(result)
    }
}
```

---

## 6. Faceting System

### Purpose
Create small multiples with sophisticated control over scale, axis, and legend sharing.

### Core Types

```rust
/// Enhanced resolution options with row/column specificity
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Shared,       // Completely shared across all facets
    Independent,  // Independent per facet
    SharedRows,   // Shared within rows, independent across rows
    SharedCols,   // Shared within columns, independent across columns
}

/// Fine-grained resolution control for faceted plots
#[derive(Debug, Clone)]
pub struct FacetResolve {
    scales: HashMap<String, Resolution>,
    axes: HashMap<String, Resolution>,
    legends: HashMap<String, Resolution>,
}

/// Faceting specification
#[derive(Debug, Clone)]
pub enum FacetSpec {
    Wrap {
        column: String,
        columns: Option<usize>,
        resolve: FacetResolve,
        spacing: Option<f64>,
        strip: Option<StripConfig>,
    },
    Grid {
        row: Option<String>,
        column: Option<String>,
        resolve: FacetResolve,
        spacing: Option<(f64, f64)>,
        strip: Option<StripConfig>,
    },
}
```

### Builder API

```rust
impl Plot {
    pub fn facet_wrap<S: Into<String>>(self, column: S) -> FacetWrapBuilder {
        FacetWrapBuilder {
            plot: self,
            column: column.into(),
            columns: None,
            resolve: FacetResolve::new(),
            spacing: None,
            strip: None,
        }
    }
    
    pub fn facet_grid(self) -> FacetGridBuilder {
        FacetGridBuilder {
            plot: self,
            row: None,
            column: None,
            resolve: FacetResolve::new(),
            spacing: None,
            strip: None,
        }
    }
}

impl FacetWrapBuilder {
    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = Some(columns);
        self
    }
    
    pub fn scale<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve.scales.insert(channel.into(), resolution);
        self
    }
    
    pub fn axis<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve.axes.insert(channel.into(), resolution);
        self
    }
}
```

### Usage Examples

```rust
// Basic facet wrap with shared scales
let plot = Plot::new(Cartesian)
    .data(df)
    .facet_wrap("continent")
    .mark(Symbol::new().x("gdp").y("life"));

// Independent scales per facet
let plot = Plot::new(Cartesian)
    .data(df)
    .facet_wrap("metric")
        .scale("y", Resolution::Independent)
    .mark(Line::new().x("date").y("value"));

// Grid with row/column specific sharing
let plot = Plot::new(Cartesian)
    .data(df)
    .facet_grid()
        .row("region")
        .column("metric")
        .scale("y", Resolution::SharedCols)  // Same scale per metric
        .scale("x", Resolution::Shared)      // Same time axis everywhere
    .mark(Line::new().x("year").y("value"));
```

---

## 7. Integration Points

### Mark State Extension

Add to `MarkState` in `src/marks/mod.rs`:

```rust
pub struct MarkState<C: CoordinateSystem> {
    pub data: DataContext,
    pub channels: HashMap<String, ChannelValue>,
    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,
    pub shapes: Option<Vec<SymbolShape>>,
    
    // Add these fields:
    pub adjustments: Vec<Box<dyn Adjust>>,
    pub derived_marks: Vec<Box<dyn Derive<C>>>,
}
```

### Mark Builder Methods

Add to mark implementations via macro:

```rust
impl<C: CoordinateSystem> $mark_name<C> {
    pub fn transform(mut self, transform: impl Transform) -> Result<Self, AvengerChartError> {
        self.state.data = transform.transform(self.state.data)?;
        Ok(self)
    }
    
    pub fn adjust(mut self, adjustment: impl Adjust + 'static) -> Self {
        self.state.adjustments.push(Box::new(adjustment));
        self
    }
    
    pub fn derive(mut self, deriver: impl Derive<C> + 'static) -> Self {
        self.state.derived_marks.push(Box::new(deriver));
        self
    }
}
```

### Rendering Pipeline

Modify the plot rendering to apply adjustments and generate derived marks:

```rust
impl Plot {
    fn render_marks(&self) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut all_marks = vec![];
        
        for mark in &self.marks {
            // 1. Apply data transforms (already happens)
            let transformed_data = mark.apply_transforms()?;
            
            // 2. Apply scales to get visual coordinates
            let scaled_data = self.apply_scales(transformed_data)?;
            
            // 3. Apply adjustments
            let mut adjusted_data = scaled_data;
            for adjustment in &mark.adjustments {
                adjusted_data = adjustment.adjust(
                    adjusted_data,
                    &self.transform_context()
                )?;
            }
            
            // 4. Create scene marks
            let scene_marks = mark.to_scene_graph(adjusted_data)?;
            all_marks.extend(scene_marks);
            
            // 5. Generate derived marks
            for deriver in &mark.derived_marks {
                let derived_mark = deriver.derive(
                    adjusted_data.clone(),
                    &self.transform_context()
                )?;
                let derived_scene = derived_mark.to_scene_graph(adjusted_data)?;
                all_marks.extend(derived_scene);
            }
        }
        
        Ok(all_marks)
    }
}
```

### Testing Strategy

1. **Unit Tests**: Test each transform/adjust/derive in isolation
2. **Integration Tests**: Test combinations of features
3. **Visual Tests**: Generate baseline images for complex scenarios
4. **Performance Tests**: Ensure transforms scale with data size

### Performance Considerations

1. **Lazy Evaluation**: Transforms should compose without immediate execution
2. **Pushdown**: Where possible, push operations to DataFusion
3. **Caching**: Cache transform results when parameters haven't changed
4. **Parallel Execution**: Use DataFusion's parallel execution capabilities
5. **GPU Acceleration**: Some adjustments (force layout) could use GPU

## 8. Polar Coordinate System

### Purpose
Implement proper polar coordinate support for radial visualizations like pie charts, radial bar charts, and radar plots.

### Current State
- Basic Polar struct exists in `coords.rs`
- Transform from polar to cartesian coordinates is implemented
- Axes not yet implemented (currently returns empty axes)

### Implementation Needed

#### PolarAxis Type
```rust
pub struct PolarAxis {
    // Radial axis properties
    radial_grid: bool,
    radial_labels: bool,
    radial_domain: Option<(f64, f64)>,
    
    // Angular axis properties
    angular_grid: bool,
    angular_labels: bool,
    angular_start: f64,  // Starting angle in radians
    angular_direction: AngularDirection,  // Clockwise or CounterClockwise
    
    // Common properties
    title: Option<String>,
    format: Option<String>,
    visible: bool,
}

pub enum AngularDirection {
    Clockwise,
    CounterClockwise,
}
```

#### Rendering Implementation
- Circular grid lines for angular divisions
- Radial lines from center for radius divisions
- Labels around the circumference for angular values
- Labels along radius for radial values

### Usage Examples
```rust
// Pie chart
Plot::new(Polar)
    .mark(Arc::new()
        .theta("value")  // Maps to angle
        .r(1.0)          // Constant radius
        .fill("category"));

// Radial bar chart
Plot::new(Polar)
    .mark(Arc::new()
        .theta("category")
        .r("value")
        .r2(0.0));
```

## Implementation Order

Suggested order for implementing these features:

1. **Text Mark** - Needed by several other features
2. **Transform System** - Foundation for data manipulation
3. **Adjust API** - Builds on transform output
4. **Derive API** - Needs Text mark and Adjust
5. **Controllers** - Can be developed in parallel
6. **Faceting** - Most complex, builds on everything else
7. **Polar Coordinate System** - Independent, can be developed in parallel

## Key Dependencies

- `datafusion`: For DataFrame operations
- `rand` / `rand_chacha`: For jitter randomization
- `rstar`: For spatial indexing in smart labels
- `cosmic-text`: For text rendering (already in use)

## Notes for Implementation

1. All traits must be `Send + Sync` for parallel execution
2. Use `Box<dyn Trait>` for runtime polymorphism
3. Leverage DataFusion's expression system for efficient operations
4. Consider WASM compatibility for all features
5. Maintain separation between data space and visual space operations
6. Document panic conditions and error cases thoroughly
7. Include examples for all major features