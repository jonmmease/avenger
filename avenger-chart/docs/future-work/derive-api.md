# Derive API - Child Mark Generation

## Purpose

Generate child marks from parent marks' scaled data. Enables automatic label placement, error bars, connectors, and annotations.

## Core Design

### Trait Definition

```rust
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::Mark;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::{col, lit};

/// Context provided to derive transforms (same as Adjust API)
pub struct TransformContext {
    pub dimensions: PlotDimensions,
    pub session: SessionContext,
}

pub struct PlotDimensions {
    pub width: f64,
    pub height: f64,
}

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

## Key Implementation: Label Points

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

## Usage Examples

```rust
use datafusion::prelude::*;

// Automatic labels above points
Symbol::new()
    .data(df)
    .x(col("gdp_per_capita"))
    .y(col("life_expectancy"))
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
    .derive(DeriveFn::new(|scaled_df, _context| {
        let labels = Text::new()
            .data(scaled_df)
            .adjust(SmartLabelPlacement::new());
        Ok(Box::new(labels))
    }));
```

## Dependencies

- Requires [Text Mark Implementation](text-mark.md)
- Can use [Adjust API](adjust-api.md) for label placement
