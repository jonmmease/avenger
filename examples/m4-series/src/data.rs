//! The series: a seeded random walk, generated once at startup.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch};
use datafusion::common::Result;
use datafusion::datasource::MemTable;
use datafusion::prelude::{DataFrame, SessionConfig, SessionContext};

/// The seed of the walk's generator.
const SEED: u64 = 11;

/// A SplitMix64 generator of uniform values in `[0, 1)`.
struct SplitMix(u64);

impl SplitMix {
    fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A standard normal value, through the Box-Muller transform.
    fn normal(&mut self) -> f64 {
        let radius = (-2.0 * self.uniform().max(f64::MIN_POSITIVE).ln()).sqrt();
        radius * (std::f64::consts::TAU * self.uniform()).cos()
    }
}

/// A random walk of `count` samples, as a DataFrame with each sample's index in `x` and its
/// value in `y`, over a table of `partitions` partitions, so its plans run in parallel and, in
/// the browser, yield between them.
pub fn walk(count: usize, partitions: usize) -> Result<DataFrame> {
    let mut generator = SplitMix(SEED);
    let mut value = 0.0;
    let ys: Vec<f64> = (0..count)
        .map(|_| {
            value += generator.normal();
            value
        })
        .collect();
    let batch = RecordBatch::try_from_iter([
        (
            "x",
            Arc::new(Float64Array::from_iter_values((0..count).map(|x| x as f64))) as ArrayRef,
        ),
        ("y", Arc::new(Float64Array::from(ys)) as ArrayRef),
    ])?;
    let chunk = count.div_ceil(partitions);
    let config = SessionConfig::new().with_target_partitions(partitions);
    let partitions = (0..partitions)
        .map(|partition| {
            let start = (partition * chunk).min(count);
            vec![batch.slice(start, chunk.min(count - start))]
        })
        .collect();
    let table = MemTable::try_new(batch.schema(), partitions)?;
    SessionContext::new_with_config(config).read_table(Arc::new(table))
}
