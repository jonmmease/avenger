//! How a kept result replays its batches to the plan above a cut.

use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Int64Array, RecordBatch},
        datatypes::{DataType, Field, Schema},
    },
    catalog::TableProvider,
    physical_plan::ExecutionPlanProperties,
    prelude::{SessionConfig, SessionContext},
};

use super::{Kept, KeptTable};

/// A kept result of four batches of one column, `ordered` or not, as a table.
fn table(ordered: bool) -> KeptTable {
    let schema = Arc::new(Schema::new(vec![Field::new("c0", DataType::Int64, false)]));
    let batches = (0..4)
        .map(|batch| {
            let values = Int64Array::from(vec![batch * 2, batch * 2 + 1]);
            RecordBatch::try_new(schema.clone(), vec![Arc::new(values)]).unwrap()
        })
        .collect();
    KeptTable {
        schema,
        kept: Arc::new(Kept::new(vec![0], batches, ordered, Vec::new())),
    }
}

#[tokio::test]
async fn ordered_batches_replay_in_one_partition_and_others_spread() {
    let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(3));
    let state = ctx.state();
    let ordered = table(true).scan(&state, None, &[], None).await.unwrap();
    assert_eq!(ordered.output_partitioning().partition_count(), 1);
    let unordered = table(false).scan(&state, None, &[], None).await.unwrap();
    assert_eq!(unordered.output_partitioning().partition_count(), 3);
}
