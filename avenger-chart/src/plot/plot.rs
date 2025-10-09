//! Plot builder for creating visualizations

use std::collections::HashMap;
use std::sync::Arc;

use datafusion::dataframe::DataFrame;
use datafusion::prelude::{Expr, lit};
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;

/// Trait for types that can be converted to Expr (for dimensions)
pub trait IntoExpr {
    fn into_expr(self) -> Expr;
}

impl IntoExpr for Expr {
    fn into_expr(self) -> Expr {
        self
    }
}

impl IntoExpr for f32 {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for f64 {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for i32 {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for i64 {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for String {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for &str {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for bool {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for usize {
    fn into_expr(self) -> Expr {
        lit(self as i64)
    }
}

impl IntoExpr for crate::cartesian::axis::AxisPosition {
    fn into_expr(self) -> Expr {
        use crate::cartesian::axis::AxisPosition;
        let s = match self {
            AxisPosition::Top => "top",
            AxisPosition::Bottom => "bottom",
            AxisPosition::Left => "left",
            AxisPosition::Right => "right",
        };
        lit(s)
    }
}

impl IntoExpr for crate::param::Param {
    fn into_expr(self) -> Expr {
        self.expr()
    }
}

impl IntoExpr for &crate::param::Param {
    fn into_expr(self) -> Expr {
        self.expr()
    }
}

impl IntoExpr for crate::polar::axis::PolarAxisType {
    fn into_expr(self) -> Expr {
        use crate::polar::axis::PolarAxisType;
        let s = match self {
            PolarAxisType::Radial => "radial",
            PolarAxisType::Angular => "angular",
        };
        lit(s)
    }
}

impl IntoExpr for crate::polar::axis::PolarDirection {
    fn into_expr(self) -> Expr {
        use crate::polar::axis::PolarDirection;
        let s = match self {
            PolarDirection::Clockwise => "clockwise",
            PolarDirection::CounterClockwise => "counterclockwise",
        };
        lit(s)
    }
}

impl IntoExpr for crate::legend::LegendPosition {
    fn into_expr(self) -> Expr {
        use crate::legend::LegendPosition;
        let s = match self {
            LegendPosition::Top => "top",
            LegendPosition::Right => "right",
            LegendPosition::Bottom => "bottom",
            LegendPosition::Left => "left",
        };
        lit(s)
    }
}

impl IntoExpr for crate::legend::LegendOrientation {
    fn into_expr(self) -> Expr {
        use crate::legend::LegendOrientation;
        let s = match self {
            LegendOrientation::Horizontal => "horizontal",
            LegendOrientation::Vertical => "vertical",
        };
        lit(s)
    }
}

impl IntoExpr for crate::plot::title::TitleSpan {
    fn into_expr(self) -> Expr {
        lit(self.to_str())
    }
}

impl IntoExpr for crate::plot::title::TitleAlign {
    fn into_expr(self) -> Expr {
        lit(self.to_str())
    }
}

use super::compiled::CompiledPlot;
use super::specs::{AxisSpec, ScaleSpec};
use super::title::{PlotSubtitle, PlotTitle};
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::guide::CoordinateGuide;
use crate::layout::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint};
use crate::legend::Legend;
use crate::marks::{CompiledMark, CompiledMarkState, Mark};
use crate::serialization::LogicalPlanNodeExt;
use crate::theme::Theme;

#[derive(Clone)]
pub struct Plot<C: CoordinateSystem> {
    coord_system: C,

    /// Marks stored until compilation
    marks: Vec<Arc<dyn Mark<C>>>,

    /// Plot-level data for mark inheritance
    pub(crate) data: Option<DataFrame>,

    /// Plot-level scale configurations (set via .scale())
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    /// Plot-level legend configurations (set via .legend())
    pub(crate) legends: IndexMap<String, Legend>,

    /// Layout specification for sizing and margins
    pub(crate) layout_spec: LayoutSpec,

    /// Optional plot title rendered by the layout system
    pub(crate) title: Option<PlotTitle>,

    /// Optional plot subtitle rendered by the layout system
    pub(crate) subtitle: Option<PlotSubtitle>,

    /// Theme for visual styling
    pub(crate) theme: Option<Arc<Theme>>,

    /// Guide configuration
    pub(crate) guide_config: Option<C::Guide>,

    /// Parameters that can be used in expressions
    pub(crate) params: Vec<crate::param::Param>,
}

impl<C: CoordinateSystem> Plot<C> {
    pub fn with_coord(coord_system: C) -> Self {
        Plot {
            coord_system,
            marks: Vec::new(),
            data: None,
            scale_specs: HashMap::new(),
            legends: IndexMap::new(),
            layout_spec: LayoutSpec::default(),
            title: None,
            subtitle: None,
            theme: None,
            guide_config: None,
            params: Vec::new(),
        }
    }
}

impl<C: CoordinateSystem + Default> Default for Plot<C> {
    fn default() -> Self {
        Self::with_coord(C::default())
    }
}

impl<C: CoordinateSystem + Default> Plot<C> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<C: CoordinateSystem> Plot<C> {
    /// Compile this plot into a renderable form (consuming self)
    pub async fn compile(
        self,
        session_context: &datafusion::prelude::SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        // Start with plot-level configurations
        let mut axis_specs: HashMap<String, AxisSpec> = HashMap::new();
        let mut legends: IndexMap<String, Legend> = self.legends.clone();
        let mut scale_specs: HashMap<String, ScaleSpec> = self.scale_specs.clone();
        let mut scale_to_coord_channel: HashMap<String, String> = HashMap::new();

        // 1. Extract and merge channel configs from all marks with proper SessionContext
        for mark in &self.marks {
            crate::plot::channel::extract_channel_configs(
                mark.as_ref(),
                session_context,
                &mut axis_specs,
                &mut legends,
                &mut scale_specs,
                &mut scale_to_coord_channel,
            );
        }

        // 2. Compile all marks, applying aggregation if needed
        let compiled_marks: Vec<Arc<dyn CompiledMark>> = self
            .marks
            .iter()
            .map(|m| {
                let mark_state = m.state();
                // Get the DataFrame (or use plot-level data)
                let df = mark_state
                    .data
                    .dataframe()
                    .cloned()
                    .or_else(|| self.data.clone())
                    .unwrap_or_else(|| {
                        DataFrame::new(
                            session_context.state().clone(),
                            datafusion::logical_expr::LogicalPlan::EmptyRelation(
                                datafusion::logical_expr::EmptyRelation {
                                    produce_one_row: false,
                                    schema: Arc::new(datafusion::common::DFSchema::empty()),
                                },
                            ),
                        )
                    });

                // Check if any channel uses aggregate functions
                let needs_aggregation = mark_state
                    .data
                    .channels()
                    .values()
                    .filter_map(|value| value.expr(session_context))
                    .any(|expr| crate::utils::contains_aggregate(&expr));

                if needs_aggregation {
                    // Apply aggregation and update channel expressions
                    self.compile_mark_with_aggregation(m, mark_state, df, session_context)
                } else {
                    // No aggregation needed - compile as-is
                    let compiled_state = CompiledMarkState::from_mark_state(mark_state, df);
                    Ok(m.compile(compiled_state))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        // 3. Build guide renderer - either from config or default
        let mut guide = if let Some(config) = &self.guide_config {
            config.clone()
        } else {
            // Create default guide for the coordinate system
            C::Guide::default()
        };

        // We need to populate axes from axis_specs before building
        // Create axes map for the guide
        let mut guide_axes = HashMap::new();

        // Add user-specified axes from axis_specs
        for (channel, axis_spec) in &axis_specs {
            let crate::plot::AxisSpec::Local(axis_config) = axis_spec;
            // The axis_config is already the correct type for this coordinate system
            // We need to downcast it to the specific axis type for the guide
            // This is safe because the axis type matches the coordinate system
            if let Some(typed_axis) = axis_config
                .as_any()
                .downcast_ref::<<C::Guide as CoordinateGuide>::Axis>()
            {
                guide_axes.insert(channel.clone(), typed_axis.clone());
            }
        }

        // Set the axes on the guide
        guide.set_axes(guide_axes);

        // Pass compiled marks to the guide so it can extract titles at render time
        guide.set_compiled_marks(compiled_marks.clone(), session_context);

        let compiled_guide = Arc::from(guide.build());

        // 4. Build CompiledPlot
        Ok(CompiledPlot {
            coord_transform: self.coord_system.create_transform(),
            compiled_guide: Some(compiled_guide),
            marks: compiled_marks,
            axis_specs,
            legends,
            layout_spec: self.layout_spec,
            title: self.title,
            subtitle: self.subtitle,
            theme: self.theme,
            scale_to_coord_channel,
            scale_specs,
            data: match self.data {
                Some(df) => {
                    let plan = df.logical_plan().clone();
                    Some(LogicalPlanNode::from_logical_plan(&plan).map_err(|e| {
                        AvengerChartError::InternalError(format!(
                            "Failed to serialize logical plan: {}",
                            e
                        ))
                    })?)
                }
                None => None,
            },
            default_params: self
                .params
                .iter()
                .map(|p| (p.name.clone(), p.default.clone()))
                .collect(),
        })
    }

    /// Helper method to compile a mark with aggregation
    ///
    /// This detects aggregate functions in channels, applies DataFrame aggregation,
    /// and updates channel expressions to reference the aggregated output columns.
    fn compile_mark_with_aggregation(
        &self,
        mark: &Arc<dyn Mark<C>>,
        mark_state: &crate::marks::MarkState,
        df: DataFrame,
        session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        use datafusion::prelude::col;
        use datafusion_proto::protobuf::LogicalExprNode;
        use crate::serialization::LogicalExprNodeExt;

        // Collect all channel expressions and deduplicate group expressions
        // IndexMap preserves insertion order which matches schema field order
        let mut unique_group_exprs = indexmap::IndexMap::new(); // expr -> insertion_index
        let mut unique_agg_exprs = indexmap::IndexMap::new(); // expr -> insertion_index
        let mut channel_info: Vec<(String, Expr, bool, bool, crate::marks::ChannelValue)> = Vec::new(); // (name, expr, is_aggregate, is_literal, original_channel_value)

        for (channel_name, channel_value) in mark_state.data.channels() {
            // Skip channels without expressions (e.g., conditional channels)
            if let Some(expr) = channel_value.expr(session_context) {
                let is_aggregate = crate::utils::contains_aggregate(&expr);
                let is_literal = matches!(expr, Expr::Literal(_, _));

                if is_aggregate {
                    // Track unique aggregate expressions
                    if !unique_agg_exprs.contains_key(&expr) {
                        unique_agg_exprs.insert(expr.clone(), unique_agg_exprs.len());
                    }
                } else if !is_literal {
                    // Track unique group expressions (excluding literals)
                    if !unique_group_exprs.contains_key(&expr) {
                        unique_group_exprs.insert(expr.clone(), unique_group_exprs.len());
                    }
                }

                channel_info.push((channel_name.clone(), expr, is_aggregate, is_literal, channel_value.clone()));
            }
        }

        // Extract deduplicated expression lists for aggregation
        let group_by_exprs: Vec<Expr> = unique_group_exprs.keys().cloned().collect();
        let agg_exprs: Vec<Expr> = unique_agg_exprs.keys().cloned().collect();

        // Apply aggregation
        let agg_df = df
            .aggregate(group_by_exprs.clone(), agg_exprs.clone())?;

        // Get the schema to discover DataFusion's chosen column names
        let schema = agg_df.schema();

        // Build updated channels by matching expressions to schema fields
        let mut updated_channels = indexmap::IndexMap::new();

        for (channel_name, original_expr, is_aggregate, is_literal, original_channel_value) in channel_info {
            if is_literal {
                // Literals stay as-is - keep original channel value unchanged
                updated_channels.insert(channel_name, original_channel_value);
            } else if is_aggregate {
                // Look up which aggregate expression this is
                let agg_index = unique_agg_exprs.get(&original_expr).unwrap();
                // Aggregate expressions come after grouping expressions in schema
                let field_index = group_by_exprs.len() + agg_index;
                let field_name = schema.field(field_index).name().clone();
                // Update expression while preserving channel configuration (band, scale, etc.)
                let new_expr = LogicalExprNode::from_expr(col(&field_name))?;
                updated_channels.insert(
                    channel_name,
                    original_channel_value.with_expr(new_expr),
                );
            } else {
                // Look up which group expression this is
                let group_index = unique_group_exprs.get(&original_expr).unwrap();
                let field_name = schema.field(*group_index).name().clone();
                // Update expression while preserving channel configuration (band, scale, etc.)
                let new_expr = LogicalExprNode::from_expr(col(&field_name))?;
                updated_channels.insert(
                    channel_name,
                    original_channel_value.with_expr(new_expr),
                );
            }
        }

        // Create CompiledMarkState with aggregated DataFrame and updated channels
        let compiled_state = CompiledMarkState::from_mark_state_with_channels(
            mark_state,
            agg_df,
            updated_channels,
        );

        Ok(mark.compile(compiled_state))
    }

    /// Get a reference to the coordinate system
    pub fn coord_system(&self) -> &C {
        &self.coord_system
    }

    pub fn mark<M: Mark<C> + 'static>(mut self, mark: M) -> Self {
        // Just store the mark - config extraction happens during compile()
        self.marks.push(Arc::new(mark));
        self
    }

    /// Set plot-level data that can be inherited by marks
    pub fn data(mut self, data: DataFrame) -> Self {
        self.data = Some(data);
        self
    }

    /// Add a parameter that can be used in plot expressions
    pub fn add_param(mut self, param: crate::param::Param) -> Self {
        self.params.push(param);
        self
    }

    /// Add multiple parameters at once
    pub fn add_params(mut self, params: impl IntoIterator<Item = crate::param::Param>) -> Self {
        self.params.extend(params);
        self
    }

    /// Get the layout specification
    pub fn get_layout_spec(&self) -> &LayoutSpec {
        &self.layout_spec
    }

    // ====== Layout API ======

    /// Set fixed canvas dimensions (traditional mode)
    /// The plot area will fill the available space within the canvas
    ///
    /// Accepts numeric literals (e.g., `800.0`), `Expr` values, or column references via `col()`
    pub fn canvas_size<W, H>(mut self, width: W, height: H) -> Self
    where
        W: IntoExpr,
        H: IntoExpr,
    {
        let width_expr = width.into_expr();
        let height_expr = height.into_expr();

        self.layout_spec.canvas = crate::layout::SizeMode::Fixed {
            width: width_expr.into(),
            height: height_expr.into(),
        };
        self
    }

    /// Set canvas sizing constraint for responsive layouts
    pub fn canvas_constraint(mut self, constraint: CanvasConstraint) -> Self {
        self.layout_spec.canvas = constraint.into();
        self
    }

    /// Set fixed plot area dimensions (data-first mode)
    /// The canvas will expand to accommodate the plot area plus margins, axes, and legends
    ///
    /// Accepts numeric literals (e.g., `400.0`), `Expr` values, or column references via `col()`
    pub fn plot_size<W, H>(mut self, width: W, height: H) -> Self
    where
        W: IntoExpr,
        H: IntoExpr,
    {
        let width_expr = width.into_expr();
        let height_expr = height.into_expr();

        self.layout_spec.plot_area = crate::layout::SizeMode::Fixed {
            width: width_expr.into(),
            height: height_expr.into(),
        };
        self
    }

    /// Set plot area sizing constraint for responsive layouts
    pub fn plot_constraint(mut self, constraint: PlotConstraint) -> Self {
        self.layout_spec.plot_area = match constraint {
            PlotConstraint::Auto => crate::layout::SizeMode::Auto,
            PlotConstraint::Width(w) => crate::layout::SizeMode::Width(w.into()),
            PlotConstraint::Height(h) => crate::layout::SizeMode::Height(h.into()),
        };
        self
    }

    /// Set margins
    pub fn margins(mut self, margins: Margins) -> Self {
        self.layout_spec.margins = margins;
        self
    }

    /// Set the theme for the plot
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = Some(Arc::new(theme));
        self
    }

    /// Configure the guide (coordinate system visual elements like axes and background)
    pub fn configure_guide(mut self, guide: C::Guide) -> Self {
        self.guide_config = match self.guide_config {
            Some(existing) => {
                let mut updated = existing.clone();
                updated.update(guide);
                Some(updated)
            }
            None => Some(guide),
        };
        self
    }

    /// Access the configured theme (or default if not set)
    pub fn get_theme(&self) -> Arc<Theme> {
        self.theme
            .clone()
            .unwrap_or_else(|| Arc::new(Theme::light()))
    }
}
