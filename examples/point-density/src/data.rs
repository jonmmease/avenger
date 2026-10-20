//! The points: a seeded mixture of Gaussians, generated once at startup.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch};
use datafusion::common::Result;
use datafusion::datasource::MemTable;
use datafusion::prelude::{DataFrame, SessionConfig, SessionContext};

/// The clusters' weights, centers, and standard deviations along x and y.
const CLUSTERS: [(f64, [f64; 2], [f64; 2]); 5] = [
    (0.35, [-1.5, -0.5], [0.9, 0.6]),
    (0.25, [1.8, 1.0], [0.4, 0.8]),
    (0.2, [0.4, -1.6], [1.6, 0.25]),
    (0.15, [2.6, -1.2], [0.15, 0.15]),
    (0.05, [-2.8, 1.9], [0.05, 0.4]),
];

/// The seed of the points' generator.
const SEED: u64 = 7;

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

    /// Two independent standard normal values, through the Box-Muller transform.
    fn normals(&mut self) -> [f64; 2] {
        let radius = (-2.0 * self.uniform().max(f64::MIN_POSITIVE).ln()).sqrt();
        let angle = std::f64::consts::TAU * self.uniform();
        [radius * angle.cos(), radius * angle.sin()]
    }
}

/// `count` points from the clusters, as a DataFrame with columns `x` and `y` over a table of
/// `partitions` partitions, so its plans run in parallel and, in the browser, yield between
/// them.
pub fn points(count: usize, partitions: usize) -> Result<DataFrame> {
    let mut generator = SplitMix(SEED);
    let (mut xs, mut ys) = (Vec::with_capacity(count), Vec::with_capacity(count));
    for _ in 0..count {
        let pick = generator.uniform();
        let mut total = 0.0;
        let (_, center, spread) = CLUSTERS
            .iter()
            .find(|(weight, _, _)| {
                total += weight;
                pick < total
            })
            .unwrap_or(&CLUSTERS[CLUSTERS.len() - 1]);
        let [dx, dy] = generator.normals();
        xs.push(center[0] + spread[0] * dx);
        ys.push(center[1] + spread[1] * dy);
    }
    let batch = RecordBatch::try_from_iter([
        ("x", Arc::new(Float64Array::from(xs)) as ArrayRef),
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
