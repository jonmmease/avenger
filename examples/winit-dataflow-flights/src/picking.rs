use crate::dataflow::{Metadata, batch};
use anyhow::{Context, Result};
use arrow::array::{Float32Array, Int32Array, StringArray};
use avenger_color::ColorOrGradient;
use avenger_datafusion_dataflow::{SnapshotId, TableSnapshot};
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use rstar::{AABB, PointDistance, RTree, RTreeObject};
use std::sync::Arc;

pub const PALETTE: [[f32; 4]; 8] = [
    [0.12, 0.43, 0.62, 0.46],
    [0.90, 0.39, 0.20, 0.46],
    [0.19, 0.59, 0.50, 0.46],
    [0.53, 0.39, 0.69, 0.46],
    [0.83, 0.63, 0.19, 0.46],
    [0.34, 0.58, 0.72, 0.46],
    [0.75, 0.36, 0.53, 0.46],
    [0.41, 0.51, 0.36, 0.46],
];
#[derive(Clone)]
struct Point {
    xy: [f32; 2],
    row: usize,
    id: i32,
}
impl RTreeObject for Point {
    type Envelope = AABB<[f32; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.xy)
    }
}
impl PointDistance for Point {
    fn distance_2(&self, p: &[f32; 2]) -> f32 {
        (p[0] - self.xy[0]).powi(2) + (p[1] - self.xy[1]).powi(2)
    }
}
pub struct Points {
    pub id: SnapshotId,
    pub marks: Vec<SceneSymbolMark>,
    pub count: usize,
    index: RTree<Point>,
    table: arrow::record_batch::RecordBatch,
}
impl Points {
    pub fn new(table: &TableSnapshot, metadata: &Metadata) -> Result<Arc<Self>> {
        let b = batch(table)?;
        let x = b
            .column_by_name("x")
            .context("x")?
            .as_any()
            .downcast_ref::<Float32Array>()
            .context("Float32 x")?;
        let y = b
            .column_by_name("y")
            .context("y")?
            .as_any()
            .downcast_ref::<Float32Array>()
            .context("Float32 y")?;
        let ids = b
            .column_by_name("flight_id")
            .context("ids")?
            .as_any()
            .downcast_ref::<Int32Array>()
            .context("Int32 ids")?;
        let cs = arrow::compute::cast(
            b.column_by_name("carrier").context("carriers")?,
            &arrow::datatypes::DataType::Utf8,
        )?;
        let carriers = cs.as_any().downcast_ref::<StringArray>().unwrap();
        let mut centers = Vec::with_capacity(b.num_rows());
        let mut xs = vec![Vec::new(); metadata.carriers.len()];
        let mut ys = xs.clone();
        for row in 0..b.num_rows() {
            let c = metadata
                .carriers
                .binary_search_by(|s| s.as_str().cmp(carriers.value(row)))
                .expect("catalog carrier");
            xs[c].push(x.value(row));
            ys[c].push(y.value(row));
            centers.push(Point {
                xy: [x.value(row), y.value(row)],
                row,
                id: ids.value(row),
            });
        }
        let marks = xs
            .into_iter()
            .zip(ys)
            .enumerate()
            .filter(|(_, (x, _))| !x.is_empty())
            .map(|(i, (x, y))| SceneSymbolMark {
                name: format!("points-{}", metadata.carriers[i]),
                interactive: false,
                clip: true,
                len: x.len() as u32,
                x: x.into(),
                y: y.into(),
                size: 7_f32.into(),
                fill: ColorOrGradient::Color(PALETTE[i % PALETTE.len()]).into(),
                stroke_width: None,
                ..Default::default()
            })
            .collect();
        Ok(Arc::new(Self {
            id: table.id(),
            marks,
            count: b.num_rows(),
            index: RTree::bulk_load(centers),
            table: b,
        }))
    }
    pub fn tooltip(&self, position: [f32; 2]) -> Option<Vec<(String, String)>> {
        let p = self
            .index
            .locate_within_distance(position, 16.)
            .min_by(|a, b| {
                a.distance_2(&position)
                    .total_cmp(&b.distance_2(&position))
                    .then(a.id.cmp(&b.id))
            })?;
        [
            ("Flight row", "flight_id"),
            ("Airline", "carrier"),
            ("Destination", "dest"),
            ("Departure delay", "dep_delay"),
            ("Arrival delay", "arr_delay"),
            ("Scheduled minute", "scheduled_minute"),
        ]
        .into_iter()
        .map(|(label, field)| {
            let array = self.table.column_by_name(field)?;
            let value = datafusion::common::ScalarValue::try_from_array(array, p.row).ok()?;
            Some((label.into(), value.to_string()))
        })
        .collect()
    }
}
