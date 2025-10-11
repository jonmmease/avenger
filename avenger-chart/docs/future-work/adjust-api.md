# Adjust API - Post-Scale Position Adjustments

## Purpose

Modify mark positions after scales have been applied, operating in visual/pixel space. Enables dodge positioning, jitter for overplotting, and smart label placement.

**Dependencies**: Add `rand = "0.8"` and `rand_chacha = "0.3"` for deterministic jitter.

## Core Design

### Trait Definition

```rust
use datafusion::dataframe::DataFrame;
use datafusion::prelude::{SessionContext, Expr, col, lit};

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

### Function Wrapper for Closures

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

## Key Implementations

### Jitter - Add Random Noise

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
            // Create UDF that generates random jitter for each row
            let jitter_udf = create_jitter_udf(amount, &mut rng);

            // Apply jitter to x column
            result = result.with_column("x", col("x") + jitter_udf)?;
        }

        // Similar for y_amount
        Ok(result)
    }
}
```

### Dodge - Avoid Overlaps

```rust
pub struct Dodge {
    padding: f64,
    group_by: Option<String>,
}

impl Adjust for Dodge {
    fn adjust(&self, df: DataFrame, _context: &TransformContext) -> Result<DataFrame, AvengerChartError> {
        // Strategy:
        // 1. Window function to count items per x position (and optional group)
        // 2. Window function to assign index within each x position
        // 3. Calculate offset: (index - (count-1)/2) * (width + padding)
        // 4. Apply offset to x column

        let partition_cols = if let Some(ref group_col) = self.group_by {
            vec![col("x"), col(group_col)]
        } else {
            vec![col("x")]
        };

        // Use window functions to calculate per-group offsets
        let result = df
            .with_column("_dodge_count", count(col("x")).over(partition_cols.clone()))?
            .with_column("_dodge_index", row_number().over(partition_cols))?
            .with_column(
                "x",
                col("x") + (col("_dodge_index") - (col("_dodge_count") - lit(1)) / lit(2))
                    * lit(self.padding)
            )?
            .drop_columns(&["_dodge_count", "_dodge_index"])?;

        Ok(result)
    }
}
```

## Usage Examples

```rust
use datafusion::prelude::*;

// Built-in adjustments
Symbol::new()
    .data(df)
    .x(col("category"))
    .y(col("value"))
    .adjust(Jitter::new().x(10.0).seed(42))
    .adjust(Dodge::new().padding(2.0));

// Custom adjustment with closure
Symbol::new()
    .adjust(AdjustFn::new(|df, context| {
        // Center points in left half of viewport
        let condition = col("x").lt(lit(context.dimensions.width / 2.0));
        let new_x = when(condition, col("x") + lit(context.dimensions.width / 4.0))
            .otherwise(col("x"))?;
        Ok(df.with_column("x", new_x)?)
    }));
```
