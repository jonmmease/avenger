use datafusion::arrow::{datatypes::DataType, record_batch::RecordBatch};
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::SerializableDataType;

/// Data type declaration for a logical event datum field emitted by a mark or guide.
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventDatumFieldSpec {
    pub name: String,
    #[serde_as(as = "FromInto<SerializableDataType>")]
    pub data_type: DataType,
}

/// Logical datum rows associated with one rendered guide scene mark.
#[derive(Clone, Debug)]
pub struct GuideEventDatumRows {
    /// Index into the flat guide mark list returned by `CompiledGuide::evaluate`.
    pub guide_mark_index: usize,
    /// Logical rows in the same order as rendered guide mark instances.
    pub rows: RecordBatch,
}
