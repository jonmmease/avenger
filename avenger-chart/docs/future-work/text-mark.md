# Text Mark Implementation

## Purpose

Render text labels on visualizations, needed for annotations, labels, and titles.

## Implementation

```rust
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::MarkState;
use avenger_scenegraph::marks::text::{
    TextMark as SceneTextMark,
    TextAlign as SceneTextAlign,
    TextBaseline as SceneTextBaseline,
    FontWeight,
};

pub struct Text<C: CoordinateSystem> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
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

## Smart Label Placement

```rust
pub struct SmartLabelPlacement {
    avoid_overlap: bool,
    avoid_marks: bool,
    padding: f64,
    max_iterations: usize,
}

impl Adjust for SmartLabelPlacement {
    fn adjust(&self, df: DataFrame, _context: &TransformContext) -> Result<DataFrame, AvengerChartError> {
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

## Usage Examples

```rust
use datafusion::prelude::*;

// Basic text mark
Text::new()
    .data(df)
    .x(col("x_position"))
    .y(col("y_position"))
    .text(col("label"))
    .font_size(14.0)  // Visual property: unscaled literal
    .align(TextAlign::Center);

// Annotations with offsets
Text::new()
    .data(df)
    .x(col("x"))
    .y(col("y"))
    .text(col("country"))
    .dy(-10.0)  // Visual offset: unscaled literal
    .align(TextAlign::Center);

// Rotated labels
Text::new()
    .data(df)
    .x(col("category"))
    .y(lit(0.0))  // Data value: scaled through y scale
    .text(col("category"))
    .angle(-45.0)  // Visual property: unscaled literal
    .align(TextAlign::Right);
```

## Dependencies

- Uses existing `avenger-scenegraph::marks::text` and `cosmic-text` (already available)
- Smart label placement needs `rstar = "0.12"` for spatial indexing
- Required by [Derive API](derive-api.md) for label generation

## Notes

- Text rendering already implemented in avenger-scenegraph
- This adds the high-level mark API
- `SmartLabelPlacement` uses R-tree for overlap detection
