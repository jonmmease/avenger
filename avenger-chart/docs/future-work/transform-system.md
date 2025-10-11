# Transform System - Data Transformations

## Purpose

Pre-scale data transformations that are coordinate-system aware. Includes binning, grouping, and stacking operations.

## Core Architecture

### Transform Trait

```rust
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::DataContext;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::{col, lit, Expr};
use std::marker::PhantomData;

/// Trait for data transformations that modify the DataContext
pub trait Transform {
    fn transform(&self, ctx: DataContext) -> Result<DataContext, AvengerChartError>;
    fn output_channels(&self) -> Vec<ChannelInfo>;
}

/// DataContext already exists in avenger-chart/src/marks/data_context.rs
/// It stores DataFrame and channel-to-expression mappings

pub struct ChannelInfo {
    pub name: String,
    pub data_type: String,
    pub required: bool,
    pub description: String,
}
```

## Bin Transform Implementation

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
    fn transform(&self, mut ctx: DataContext) -> Result<DataContext, AvengerChartError> {
        let df = ctx.dataframe()
            .ok_or_else(|| AvengerChartError::InternalError("No dataframe in context".to_string()))?
            .clone();

        // Calculate bin boundaries
        for config in &self.inner.configs {
            let (bin_edges, bin_width) = if let Some(width) = config.width {
                calculate_bins_by_width(config.domain, width, config.nice)
            } else if let Some(bins) = config.bins {
                calculate_bins_by_count(config.domain, bins, config.nice)
            } else {
                return Err(AvengerChartError::InvalidArgument(
                    "Bin requires either width or bins".to_string()
                ));
            };

            // Create binned DataFrame using floor function
            let binned = df
                .with_column(
                    &format!("{}_bin_start", config.field),
                    (col(&config.field) / lit(bin_width)).floor() * lit(bin_width)
                )?
                .with_column(
                    &format!("{}_bin_end", config.field),
                    col(&format!("{}_bin_start", config.field)) + lit(bin_width)
                )?;

            // Apply aggregation if specified
            if let Some(ref agg_expr) = self.inner.agg {
                use datafusion::functions_aggregate::count::count;

                let grouped = binned.aggregate(
                    vec![
                        col(&format!("{}_bin_start", config.field)),
                        col(&format!("{}_bin_end", config.field)),
                    ],
                    vec![agg_expr.clone()]
                )?;

                return Ok(DataContext::new(grouped));
            }
        }

        Ok(ctx)
    }
}
```

## Group Transform Implementation

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
        let df = ctx.dataframe()
            .ok_or_else(|| AvengerChartError::InternalError("No dataframe in context".to_string()))?
            .clone();

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
        let aggregated = df.aggregate(group_cols, agg_exprs)?;

        // Build result context
        Ok(DataContext::new(aggregated))
    }
}
```

## Stack Transform Implementation

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
        use datafusion::functions_aggregate::sum::sum;
        use datafusion::prelude::JoinType;

        let df = ctx.dataframe()
            .ok_or_else(|| AvengerChartError::InternalError("No dataframe in context".to_string()))?
            .clone();

        let group_col = &self.group_channel;
        let stack_col = &self.stack_channel;

        // Sort according to order
        let sorted = match self.order {
            StackOrder::Appearance => df,
            StackOrder::Sum => df.sort(vec![col(stack_col).sort(false, false)])?,
            StackOrder::Value => df.sort(vec![col(stack_col).sort(true, false)])?,
            StackOrder::Reverse => df,  // Reverse after processing
        };

        // Calculate cumulative sums within groups using window functions
        // Note: Window spec syntax may vary in DataFusion versions
        let window_expr = sum(col(stack_col))
            .partition_by(vec![col(group_col)])
            .order_by(vec![col(stack_col).sort(true, false)])
            .build()?;

        let stacked = sorted
            .with_column(
                &format!("{}_stack_end", self.stack_channel),
                window_expr
            )?
            .with_column(
                &format!("{}_stack_start", self.stack_channel),
                col(&format!("{}_stack_end", self.stack_channel)) - col(stack_col)
            )?;

        // Apply offset
        let final_df = match self.offset {
            StackOffset::Zero => stacked,
            StackOffset::Center => {
                // Calculate total per group and center
                let totals = stacked.aggregate(
                    vec![col(group_col)],
                    vec![sum(col(stack_col)).alias("_total")]
                )?;

                stacked.join(totals, JoinType::Inner, &[group_col.as_str()], &[group_col.as_str()], None)?
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
                let totals = stacked.aggregate(
                    vec![col(group_col)],
                    vec![sum(col(stack_col)).alias("_total")]
                )?;

                stacked.join(totals, JoinType::Inner, &[group_col.as_str()], &[group_col.as_str()], None)?
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

        Ok(DataContext::new(final_df))
    }
}
```

## Usage Examples

```rust
use datafusion::functions_aggregate::count::count;
use datafusion::functions_aggregate::sum::sum;
use datafusion::prelude::*;

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
    .fill_with(col("count"), |c| c);

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
