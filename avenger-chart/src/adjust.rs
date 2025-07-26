//! Post-scale adjustment transforms for marks

use crate::error::AvengerChartError;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;

/// Dimensions of the plot area
#[derive(Debug, Clone, Copy)]
pub struct PlotDimensions {
    pub width: f64,
    pub height: f64,
}

/// Context provided to post-scale transforms (adjust and derive)
#[derive(Clone)]
pub struct TransformContext {
    /// Plot area dimensions
    pub dimensions: PlotDimensions,
    /// DataFusion session context for DataFrame operations
    pub session: SessionContext,
}

impl TransformContext {
    pub fn new(dimensions: PlotDimensions, session: SessionContext) -> Self {
        Self {
            dimensions,
            session,
        }
    }

    /// Convenience accessor for width
    pub fn width(&self) -> f64 {
        self.dimensions.width
    }

    /// Convenience accessor for height
    pub fn height(&self) -> f64 {
        self.dimensions.height
    }
}

/// Trait for post-scale adjustments that operate on scaled mark data
pub trait Adjust: Send + Sync {
    /// Adjust mark positions/properties after scaling
    ///
    /// # Arguments
    /// * `df` - DataFrame with columns:
    ///   - Encoding channels (x, y, size, etc.) with scaled visual values
    ///   - "bbox" - Struct column with {x_min, y_min, x_max, y_max}
    /// * `context` - Transform context with plot dimensions and session
    ///
    /// # Returns
    /// Modified DataFrame with same schema but adjusted values
    fn adjust(
        &self,
        df: DataFrame,
        context: &TransformContext,
    ) -> Result<DataFrame, AvengerChartError>;
}

/// Wrapper for function-based adjustments
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
    fn adjust(
        &self,
        df: DataFrame,
        context: &TransformContext,
    ) -> Result<DataFrame, AvengerChartError> {
        (self.f)(df, context)
    }
}

// Example implementations

/// Jitter adjustment - adds random noise to positions
#[derive(Debug, Clone)]
pub struct Jitter {
    x_amount: Option<f64>,
    y_amount: Option<f64>,
    seed: Option<u64>,
}

impl Jitter {
    pub fn new() -> Self {
        Self {
            x_amount: None,
            y_amount: None,
            seed: None,
        }
    }

    pub fn x(mut self, amount: f64) -> Self {
        self.x_amount = Some(amount);
        self
    }

    pub fn y(mut self, amount: f64) -> Self {
        self.y_amount = Some(amount);
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }
}

impl Default for Jitter {
    fn default() -> Self {
        Self::new()
    }
}

impl Adjust for Jitter {
    fn adjust(
        &self,
        df: DataFrame,
        _context: &TransformContext,
    ) -> Result<DataFrame, AvengerChartError> {
        use datafusion::logical_expr::{col, lit};

        let mut result = df;

        // Add jitter to x if specified
        if let Some(amount) = self.x_amount {
            // For now, use a simple offset instead of random
            // In a real implementation, this would use random values
            let jitter_expr = col("x") + lit(amount * 0.5);
            result = result.with_column("x", jitter_expr)?;
        }

        // Add jitter to y if specified
        if let Some(amount) = self.y_amount {
            let jitter_expr = col("y") + lit(amount * 0.5);
            result = result.with_column("y", jitter_expr)?;
        }

        Ok(result)
    }
}

/// Dodge adjustment - shifts marks to avoid overlap
#[derive(Debug, Clone)]
pub struct Dodge {
    padding: f64,
    group_by: Option<String>,
}

impl Dodge {
    pub fn new() -> Self {
        Self {
            padding: 2.0,
            group_by: None,
        }
    }

    pub fn padding(mut self, padding: f64) -> Self {
        self.padding = padding;
        self
    }

    pub fn by(mut self, column: impl Into<String>) -> Self {
        self.group_by = Some(column.into());
        self
    }
}

impl Default for Dodge {
    fn default() -> Self {
        Self::new()
    }
}

impl Adjust for Dodge {
    fn adjust(
        &self,
        df: DataFrame,
        _context: &TransformContext,
    ) -> Result<DataFrame, AvengerChartError> {
        // Simplified implementation for now
        // In a real implementation, this would:
        // 1. Group by x position (and optional grouping column)
        // 2. Calculate positions within each group
        // 3. Offset marks to avoid overlap
        // The context.session can be used for DataFrame operations

        Ok(df)
    }
}
