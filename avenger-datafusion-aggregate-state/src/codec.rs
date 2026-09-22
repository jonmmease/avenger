use crate::{
    aggregate::{Operation, StateAggregate},
    families::Family,
    finalize::Finalize,
};

use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::TableProvider,
    common::{plan_err, Result, TableReference},
    datasource::file_format::FileFormatFactory,
    execution::TaskContext,
    logical_expr::{AggregateUDF, Extension, HigherOrderUDF, LogicalPlan, ScalarUDF, WindowUDF},
};
use datafusion_proto::logical_plan::{DefaultLogicalExtensionCodec, LogicalExtensionCodec};
use std::{collections::BTreeMap, sync::Arc};

/// Semantic version of the functions and their pinned native state representation.
pub const AGGREGATE_STATE_FUNCTION_VERSION: &str = "avenger-aggregate-state/1/datafusion-54.1.0";

/// Function-version requirements for dataflows using aggregate-state functions.
pub fn function_versions() -> BTreeMap<String, String> {
    Family::ALL
        .into_iter()
        .flat_map(|family| {
            let mut names = [Operation::State, Operation::Merge, Operation::MergeState]
                .map(|op| {
                    format!(
                        "aggregate:{}",
                        AggregateUDF::from(StateAggregate::new(family, op)).name()
                    )
                })
                .to_vec();
            names.push(format!(
                "scalar:{}",
                ScalarUDF::from(Finalize::new(family)).name()
            ));
            names
                .into_iter()
                .map(|name| (name, AGGREGATE_STATE_FUNCTION_VERSION.into()))
        })
        .collect()
}

const MAGIC: &[u8] = b"AVENGER_AGGREGATE_STATE\0";

/// Encode aggregate-state function identities and delegate unrelated functions and extensions.
#[derive(Debug)]
pub struct AggregateStateExtensionCodec {
    fallback: Arc<dyn LogicalExtensionCodec>,
}
impl Default for AggregateStateExtensionCodec {
    fn default() -> Self {
        Self::with_fallback(Arc::new(DefaultLogicalExtensionCodec {}))
    }
}
impl AggregateStateExtensionCodec {
    /// Wrap another codec for graphs that also contain other libraries' functions.
    pub fn with_fallback(fallback: Arc<dyn LogicalExtensionCodec>) -> Self {
        Self { fallback }
    }
}
impl LogicalExtensionCodec for AggregateStateExtensionCodec {
    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> Result<()> {
        if node.inner().is::<Finalize>() {
            buf.extend_from_slice(MAGIC);
            buf.extend_from_slice(AGGREGATE_STATE_FUNCTION_VERSION.as_bytes());
            buf.push(0);
            buf.extend_from_slice(node.name().as_bytes());
            Ok(())
        } else {
            self.fallback.try_encode_udf(node, buf)
        }
    }
    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> Result<Arc<ScalarUDF>> {
        let Some(payload) = buf.strip_prefix(MAGIC) else {
            return self.fallback.try_decode_udf(name, buf);
        };
        let expected = format!("{AGGREGATE_STATE_FUNCTION_VERSION}\0{name}");
        if payload != expected.as_bytes() {
            return plan_err!(
                "Unsupported aggregate-state function version or mismatched name for {name}"
            );
        }
        Family::ALL
            .into_iter()
            .map(|family| Arc::new(ScalarUDF::from(Finalize::new(family))))
            .find(|f| f.name() == name)
            .ok_or_else(|| {
                datafusion::common::DataFusionError::Plan(format!(
                    "Unknown aggregate-state scalar function {name}"
                ))
            })
    }
    fn try_decode(
        &self,
        buf: &[u8],
        inputs: &[LogicalPlan],
        ctx: &TaskContext,
    ) -> Result<Extension> {
        self.fallback.try_decode(buf, inputs, ctx)
    }
    fn try_encode(&self, node: &Extension, buf: &mut Vec<u8>) -> Result<()> {
        self.fallback.try_encode(node, buf)
    }
    fn try_decode_table_provider(
        &self,
        buf: &[u8],
        table_ref: &TableReference,
        schema: SchemaRef,
        ctx: &TaskContext,
    ) -> Result<Arc<dyn TableProvider>> {
        self.fallback
            .try_decode_table_provider(buf, table_ref, schema, ctx)
    }
    fn try_encode_table_provider(
        &self,
        table_ref: &TableReference,
        node: Arc<dyn TableProvider>,
        buf: &mut Vec<u8>,
    ) -> Result<()> {
        self.fallback
            .try_encode_table_provider(table_ref, node, buf)
    }
    fn try_decode_file_format(
        &self,
        buf: &[u8],
        ctx: &TaskContext,
    ) -> Result<Arc<dyn FileFormatFactory>> {
        self.fallback.try_decode_file_format(buf, ctx)
    }
    fn try_encode_file_format(
        &self,
        buf: &mut Vec<u8>,
        node: Arc<dyn FileFormatFactory>,
    ) -> Result<()> {
        self.fallback.try_encode_file_format(buf, node)
    }
    fn try_decode_udaf(&self, name: &str, buf: &[u8]) -> Result<Arc<AggregateUDF>> {
        let Some(payload) = buf.strip_prefix(MAGIC) else {
            return self.fallback.try_decode_udaf(name, buf);
        };
        let expected = format!("{AGGREGATE_STATE_FUNCTION_VERSION}\0{name}");
        if payload != expected.as_bytes() {
            return plan_err!(
                "Unsupported aggregate-state function version or mismatched name for {name}"
            );
        }
        Family::ALL
            .into_iter()
            .flat_map(|family| {
                [Operation::State, Operation::Merge, Operation::MergeState]
                    .map(|op| Arc::new(AggregateUDF::from(StateAggregate::new(family, op))))
            })
            .find(|f| f.name() == name)
            .ok_or_else(|| {
                datafusion::common::DataFusionError::Plan(format!(
                    "Unknown aggregate-state aggregate function {name}"
                ))
            })
    }
    fn try_encode_udaf(&self, node: &AggregateUDF, buf: &mut Vec<u8>) -> Result<()> {
        if node.inner().is::<StateAggregate>() {
            buf.extend_from_slice(MAGIC);
            buf.extend_from_slice(AGGREGATE_STATE_FUNCTION_VERSION.as_bytes());
            buf.push(0);
            buf.extend_from_slice(node.name().as_bytes());
            Ok(())
        } else {
            self.fallback.try_encode_udaf(node, buf)
        }
    }
    fn try_decode_udwf(&self, name: &str, buf: &[u8]) -> Result<Arc<WindowUDF>> {
        self.fallback.try_decode_udwf(name, buf)
    }
    fn try_encode_udwf(&self, node: &WindowUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.fallback.try_encode_udwf(node, buf)
    }
    fn try_decode_higher_order_function(
        &self,
        name: &str,
        buf: &[u8],
    ) -> Result<Arc<HigherOrderUDF>> {
        self.fallback.try_decode_higher_order_function(name, buf)
    }
    fn try_encode_higher_order_function(
        &self,
        node: &HigherOrderUDF,
        buf: &mut Vec<u8>,
    ) -> Result<()> {
        self.fallback.try_encode_higher_order_function(node, buf)
    }
}
