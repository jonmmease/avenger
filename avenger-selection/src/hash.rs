//! Fingerprints of conditions, clauses, and grids from their content, which a query layer uses to
//! name and compare the calls that carry them. Values with equal fingerprints hold equal content,
//! up to collisions of 64 bits. Equal values in other buffers, such as a slice and a copy of it,
//! can have different fingerprints. A fingerprint is stable within one build of the crate.

use std::hash::{BuildHasher, Hash, Hasher};

use arrow::array::{Array, ArrayData};
use foldhash::fast::FixedState;

use crate::{CellSpan, Clause, Condition, PixelGrid, PolygonCells};

/// The state of the hashers of fingerprints, with a fixed seed.
static STATE: FixedState = FixedState::with_seed(0x9E37_79B9_7F4A_7C15);

/// A hasher of fingerprints.
pub(crate) fn hasher() -> impl Hasher {
    STATE.build_hasher()
}

impl Clause {
    /// A fingerprint of the clause's owner, partition values, and conditions.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = hasher();
        hash_clause(self, &mut hasher);
        hasher.finish()
    }
}

impl PixelGrid {
    /// A fingerprint of what [`same`](Self::same) compares: the type of the grid's scale, its
    /// domain, range, and options, and the grid's origin and cell.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = hasher();
        hash_grid(self, &mut hasher);
        hasher.finish()
    }
}

pub(crate) fn hash_condition<H: Hasher>(condition: &Condition, hasher: &mut H) {
    std::mem::discriminant(condition).hash(hasher);
    match condition {
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
            (column, channel, exclude_end, cells.is_some()).hash(hasher);
            hash_array(ends.as_ref(), hasher);
            if let Some(span) = cells {
                hash_span(span, hasher);
            }
        }
        Condition::Polygon {
            x_column,
            y_column,
            polygons,
            cells,
        } => {
            (x_column, y_column, polygons.len(), cells.is_some()).hash(hasher);
            for (x, y) in polygons {
                hash_array(x.as_ref(), hasher);
                hash_array(y.as_ref(), hasher);
            }
            if let Some(cells) = cells {
                hash_cells(cells, hasher);
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

fn hash_clause<H: Hasher>(clause: &Clause, hasher: &mut H) {
    (&clause.owner, clause.partition.len()).hash(hasher);
    for (column, value) in &clause.partition {
        column.hash(hasher);
        hash_array(value.array().as_ref(), hasher);
    }
    clause.conditions.len().hash(hasher);
    for condition in &clause.conditions {
        hash_condition(condition, hasher);
    }
}

fn hash_grid<H: Hasher>(grid: &PixelGrid, hasher: &mut H) {
    let config = &grid.scale.config;
    grid.scale.scale_impl.scale_type().hash(hasher);
    hash_array(config.domain.as_ref(), hasher);
    hash_array(config.range.as_ref(), hasher);
    let mut options: Vec<_> = config.options.iter().collect();
    options.sort_by_key(|(name, _)| *name);
    options.len().hash(hasher);
    for (name, value) in options {
        name.hash(hasher);
        hash_array(value.array().as_ref(), hasher);
    }
    [grid.origin, grid.cell].map(f64::to_bits).hash(hasher);
}

fn hash_span<H: Hasher>(span: &CellSpan, hasher: &mut H) {
    hash_grid(&span.grid, hasher);
    (span.first, span.last).hash(hasher);
}

fn hash_cells<H: Hasher>(cells: &PolygonCells, hasher: &mut H) {
    for grid in &cells.grids {
        hash_grid(grid, hasher);
    }
    (cells.first_row, &cells.rows).hash(hasher);
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
