//! Hashes of conditions, clauses, and grids from their content, which a query layer uses to name
//! and compare the calls that carry them. Values that hash equally hold equal content, up to
//! collisions. Equal values in other buffers, such as a slice and a copy of it, can hash
//! differently.

use std::hash::{Hash, Hasher};

use arrow::array::{Array, ArrayData};

use crate::{CellSpan, Clause, Condition, PixelGrid, PolygonCells};

impl Hash for Condition {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        std::mem::discriminant(self).hash(hasher);
        match self {
            Condition::Set {
                columns,
                channels,
                values,
            } => {
                (columns, channels, values.len()).hash(hasher);
                for list in values {
                    hash_array(list.as_ref(), hasher);
                }
            }
            Condition::Range {
                column,
                channel,
                ends,
                exclude_end,
                cells,
            } => {
                (column, channel, exclude_end, cells).hash(hasher);
                hash_array(ends.as_ref(), hasher);
            }
            Condition::Polygon {
                x_column,
                y_column,
                polygons,
                cells,
            } => {
                (x_column, y_column, polygons.len(), cells).hash(hasher);
                for (x, y) in polygons {
                    hash_array(x.as_ref(), hasher);
                    hash_array(y.as_ref(), hasher);
                }
            }
            Condition::CyclicRange {
                column,
                channel,
                start,
                end,
                period,
            } => (column, channel, [start, end, period].map(|v| v.to_bits())).hash(hasher),
        }
    }
}

impl Hash for Clause {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        (&self.owner, self.partition.len()).hash(hasher);
        for (column, value) in &self.partition {
            column.hash(hasher);
            hash_array(value.array().as_ref(), hasher);
        }
        self.conditions.hash(hasher);
    }
}

/// Hashes what [`PixelGrid::same`] compares: the type of the grid's scale, its domain, range, and
/// options, and the grid's origin and cell.
impl Hash for PixelGrid {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        let config = &self.scale.config;
        self.scale.scale_impl.scale_type().hash(hasher);
        hash_array(config.domain.as_ref(), hasher);
        hash_array(config.range.as_ref(), hasher);
        let mut options: Vec<_> = config.options.iter().collect();
        options.sort_by_key(|(name, _)| *name);
        options.len().hash(hasher);
        for (name, value) in options {
            name.hash(hasher);
            hash_array(value.array().as_ref(), hasher);
        }
        [self.origin, self.cell].map(f64::to_bits).hash(hasher);
    }
}

impl Hash for CellSpan {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        (&self.grid, self.first, self.last).hash(hasher);
    }
}

impl Hash for PolygonCells {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        (&self.grids, self.first_row, &self.rows).hash(hasher);
    }
}

/// Hashes the type, the length, and the buffers of `array`.
pub(crate) fn hash_array<H: Hasher>(array: &dyn Array, hasher: &mut H) {
    hash_data(&array.to_data(), hasher);
}

fn hash_data<H: Hasher>(data: &ArrayData, hasher: &mut H) {
    (data.data_type(), data.len(), data.offset()).hash(hasher);
    match data.nulls() {
        Some(nulls) => {
            let bits = nulls.inner();
            (true, bits.offset(), bits.len(), bits.inner().as_slice()).hash(hasher);
        }
        None => false.hash(hasher),
    }
    data.buffers().len().hash(hasher);
    for buffer in data.buffers() {
        buffer.as_slice().hash(hasher);
    }
    data.child_data().len().hash(hasher);
    for child in data.child_data() {
        hash_data(child, hasher);
    }
}
