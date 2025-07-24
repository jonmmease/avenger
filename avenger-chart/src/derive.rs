//! Derive marks from parent marks' scaled data

use crate::adjust::TransformContext;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::Mark;
use datafusion::dataframe::DataFrame;

/// Trait for deriving child marks from parent marks' scaled data
pub trait Derive<C: CoordinateSystem>: Send + Sync {
    /// Generate child marks from parent's scaled data
    ///
    /// # Arguments
    /// * `df` - DataFrame with:
    ///   - Scaled encoding channels (x, y, size, etc.)
    ///   - Bounding box struct column
    ///   - Original data columns
    /// * `context` - Transform context with plot dimensions and session
    ///
    /// # Returns
    /// A mark instance configured with the parent's data
    fn derive(
        &self,
        df: DataFrame,
        context: &TransformContext,
    ) -> Result<Box<dyn Mark<C>>, AvengerChartError>;
}

/// Wrapper for function-based derivations
pub struct DeriveFn<C, F> {
    f: F,
    _phantom: std::marker::PhantomData<C>,
}

impl<C, F> DeriveFn<C, F>
where
    C: CoordinateSystem,
    F: Fn(DataFrame, &TransformContext) -> Result<Box<dyn Mark<C>>, AvengerChartError>
        + Send
        + Sync,
{
    pub fn new(f: F) -> Self {
        Self {
            f,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<C, F> Derive<C> for DeriveFn<C, F>
where
    C: CoordinateSystem,
    F: Fn(DataFrame, &TransformContext) -> Result<Box<dyn Mark<C>>, AvengerChartError>
        + Send
        + Sync,
{
    fn derive(
        &self,
        df: DataFrame,
        context: &TransformContext,
    ) -> Result<Box<dyn Mark<C>>, AvengerChartError> {
        (self.f)(df, context)
    }
}

// Example: Label points implementation
use datafusion::logical_expr::{col, lit};

/// Configuration for labeling points
#[derive(Debug, Clone)]
pub struct LabelPoints {
    /// Column to use for label text
    text_column: String,
    /// Vertical offset from point (negative = above)
    offset_y: f64,
    /// Horizontal alignment
    align: TextAlign,
    /// Font size
    font_size: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

impl LabelPoints {
    pub fn new(text_column: impl Into<String>) -> Self {
        Self {
            text_column: text_column.into(),
            offset_y: -5.0, // 5 pixels above by default
            align: TextAlign::Center,
            font_size: None,
        }
    }

    pub fn offset_y(mut self, offset: f64) -> Self {
        self.offset_y = offset;
        self
    }

    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    pub fn font_size(mut self, size: f64) -> Self {
        self.font_size = Some(size);
        self
    }
}

impl<C: CoordinateSystem> Derive<C> for LabelPoints {
    fn derive(
        &self,
        df: DataFrame,
        _context: &TransformContext,
    ) -> Result<Box<dyn Mark<C>>, AvengerChartError> {
        // Calculate label positions based on parent positions and bounding boxes
        let label_y = if self.offset_y < 0.0 {
            // Above: use top of bbox
            col("bbox.y_min") + lit(self.offset_y)
        } else {
            // Below: use bottom of bbox
            col("bbox.y_max") + lit(self.offset_y)
        };

        // Create derived DataFrame with label positions
        let _label_df = df.with_column("label_y", label_y)?;

        // In a real implementation, this would create a Text mark
        // The context.session can be used for DataFrame operations
        // For now, return a placeholder
        todo!("Create Text mark with label_df")
    }
}
