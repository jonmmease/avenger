//! Stack transform for creating stacked visualizations

use std::marker::PhantomData;
use crate::coords::{CoordinateSystem, Cartesian, Polar};
use crate::error::AvengerChartError;
use super::core::{Transform, DataContext, ChannelInfo};

/// Ordering options for stacking
#[derive(Debug, Clone, Copy)]
pub enum StackOrder {
    /// Order by appearance in data
    Appearance,
    /// Order by sum of values (largest first)
    Sum,
    /// Order by individual values
    Value,
    /// Reverse order
    Reverse,
}

/// Offset algorithms for stacking
#[derive(Debug, Clone, Copy)]
pub enum StackOffset {
    /// Stack from zero baseline
    Zero,
    /// Center stacks around midpoint
    Center,
    /// Normalize to percentage (0-100%)
    Normalize,
}

/// Transform that stacks values based on coordinate system
pub struct Stack<C: CoordinateSystem> {
    /// Channel to stack (e.g., "y" for vertical stacking)
    stack_channel: String,
    
    /// Channel to group by (e.g., "x" for vertical stacking)
    group_channel: String,
    
    /// Optional explicit column for stack values (overrides channel)
    stack_column: Option<String>,
    
    /// Optional explicit column for grouping (overrides channel)
    group_column: Option<String>,
    
    /// Optional explicit column for series/ordering (overrides fill/color)
    series_column: Option<String>,
    
    /// How to order the stack segments
    order: StackOrder,
    
    /// How to calculate stack offsets
    offset: StackOffset,
    
    /// Phantom type for coordinate system
    _phantom: PhantomData<C>,
}

// General implementation
impl<C: CoordinateSystem> Stack<C> {
    /// Create a new stack transform
    fn new(stack_channel: &str, group_channel: &str) -> Self {
        Self {
            stack_channel: stack_channel.to_string(),
            group_channel: group_channel.to_string(),
            stack_column: None,
            group_column: None,
            series_column: None,
            order: StackOrder::Appearance,
            offset: StackOffset::Zero,
            _phantom: PhantomData,
        }
    }
    
    /// Override the column used for stacking
    pub fn stack(mut self, column: impl Into<String>) -> Self {
        self.stack_column = Some(column.into());
        self
    }
    
    /// Override the column used for grouping
    pub fn by(mut self, column: impl Into<String>) -> Self {
        self.group_column = Some(column.into());
        self
    }
    
    /// Specify the series column for ordering
    pub fn series(mut self, column: impl Into<String>) -> Self {
        self.series_column = Some(column.into());
        self
    }
    
    /// Set the ordering method
    pub fn order(mut self, order: StackOrder) -> Self {
        self.order = order;
        self
    }
    
    /// Set the offset algorithm
    pub fn offset(mut self, offset: StackOffset) -> Self {
        self.offset = offset;
        self
    }
}

// Cartesian-specific constructors
impl Stack<Cartesian> {
    /// Create a vertical stack transform (stack y values within x groups)
    pub fn y() -> Self {
        Self::new("y", "x")
    }
    
    /// Create a horizontal stack transform (stack x values within y groups)
    pub fn x() -> Self {
        Self::new("x", "y")
    }
}

// Polar-specific constructors
impl Stack<Polar> {
    /// Create a radial stack transform (stack r values within theta groups)
    pub fn r() -> Self {
        Self::new("r", "theta")
    }
    
    /// Create an angular stack transform (stack theta values within r groups)
    pub fn theta() -> Self {
        Self::new("theta", "r")
    }
}

impl<C: CoordinateSystem> Transform for Stack<C> {
    fn transform(&self, ctx: DataContext) -> Result<DataContext, AvengerChartError> {
        // Resolve columns from encodings or use explicit columns
        let _stack_col = self.stack_column.as_ref()
            .cloned()
            .or_else(|| ctx.encoding(&self.stack_channel));
        
        let group_col = self.group_column.as_ref()
            .cloned()
            .or_else(|| ctx.encoding(&self.group_channel));
        
        // Try to find series column for ordering - check encodings
        let series_col = self.series_column.as_ref()
            .map(|c| c.clone())
            .or_else(|| {
                // Check if fill, color, or stroke encodings exist
                if ctx.encoding("fill").is_some() {
                    ctx.encoding("fill")
                } else if ctx.encoding("color").is_some() {
                    ctx.encoding("color")
                } else if ctx.encoding("stroke").is_some() {
                    ctx.encoding("stroke")
                } else {
                    None
                }
            });
        
        // TODO: Implement actual stacking logic
        // This would:
        // 1. Group by group_col (and series_col if present)
        // 2. Calculate cumulative sums within each group
        // 3. Create new columns for stack boundaries
        
        let df = ctx.dataframe().clone();
        
        // For now, create stub output columns
        let stack_start_col = format!("{}_stack_start", self.stack_channel);
        let stack_end_col = format!("{}_stack_end", self.stack_channel);
        
        // Build result context
        let mut result = DataContext::new(df);
        
        // Preserve existing encodings
        for channel in ctx.channels() {
            if let Some(column) = ctx.encoding(channel) {
                result = result.with_encoding(channel, &column);
            }
        }
        
        // Update stack channel encodings
        result = result
            .with_encoding(&format!("{}1", self.stack_channel), &stack_start_col)
            .with_encoding(&format!("{}2", self.stack_channel), &stack_end_col)
            .with_channel_metadata(&self.stack_channel, serde_json::json!({
                "transform": "stack",
                "order": format!("{:?}", self.order),
                "offset": format!("{:?}", self.offset),
                "group_by": group_col,
                "series": series_col,
            }));
        
        // Also keep midpoint for labels
        result = result.with_encoding(&self.stack_channel, &format!("{}_mid", self.stack_channel));
        
        Ok(result)
    }
    
    fn output_channels(&self) -> Vec<ChannelInfo> {
        vec![
            ChannelInfo {
                name: format!("{}1", self.stack_channel),
                data_type: "quantitative".to_string(),
                required: true,
                description: format!("Stack start values for {}", self.stack_channel),
            },
            ChannelInfo {
                name: format!("{}2", self.stack_channel),
                data_type: "quantitative".to_string(),
                required: true,
                description: format!("Stack end values for {}", self.stack_channel),
            },
            ChannelInfo {
                name: self.stack_channel.clone(),
                data_type: "quantitative".to_string(),
                required: false,
                description: "Stack midpoint (for labels)".to_string(),
            },
        ]
    }
}