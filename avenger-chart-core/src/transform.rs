use async_trait::async_trait;
use datafusion::{dataframe::DataFrame, prelude::SessionContext};

use crate::AvengerChartError;

pub struct DataTransformExecutionContext<'a> {
    pub session_context: &'a SessionContext,
}

#[typetag::serde(tag = "type")]
#[async_trait]
pub trait CompiledDataTransform: Send + Sync {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform>;

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataFrame, AvengerChartError>;
}

impl Clone for Box<dyn CompiledDataTransform> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// Authoring-side transform builder contract.
///
/// Implemented by built-in and external transform crates. The output handle is
/// returned to the mark transform closure so authors can reference generated
/// columns without spelling generated names directly.
pub trait DataTransform: Clone + Send + Sync + 'static {
    type Output;

    fn into_compiled_and_output(
        self,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError>;
}

pub async fn apply_compiled_data_transforms(
    mut dataframe: DataFrame,
    transforms: &[Box<dyn CompiledDataTransform>],
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<DataFrame, AvengerChartError> {
    for transform in transforms {
        dataframe = transform.apply(dataframe, ctx).await?;
    }
    Ok(dataframe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::{
        arrow::{datatypes::Schema, record_batch::RecordBatch},
        prelude::SessionContext,
    };
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct IdentityTransform;

    #[typetag::serde(name = "test_identity")]
    #[async_trait]
    impl CompiledDataTransform for IdentityTransform {
        fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
            Box::new(self.clone())
        }

        async fn apply(
            &self,
            dataframe: DataFrame,
            _ctx: &DataTransformExecutionContext<'_>,
        ) -> Result<DataFrame, AvengerChartError> {
            Ok(dataframe)
        }
    }

    #[tokio::test]
    async fn boxed_compiled_transform_clones_and_applies() {
        let ctx = SessionContext::new();
        let dataframe = ctx
            .read_batch(RecordBatch::new_empty(Arc::new(Schema::empty())))
            .unwrap();
        let transforms: Vec<Box<dyn CompiledDataTransform>> = vec![Box::new(IdentityTransform)];
        let cloned = transforms.clone();
        let result = apply_compiled_data_transforms(
            dataframe,
            &cloned,
            &DataTransformExecutionContext {
                session_context: &ctx,
            },
        )
        .await;
        assert!(result.is_ok());
    }

    #[test]
    fn compiled_transform_serializes_with_typetag() {
        let transform: Box<dyn CompiledDataTransform> = Box::new(IdentityTransform);
        let bytes = bincode::serialize(&transform).expect("serialize transform");
        let decoded: Box<dyn CompiledDataTransform> =
            bincode::deserialize(&bytes).expect("deserialize transform");
        let json = serde_json::to_string(&decoded).expect("serialize decoded transform as json");
        assert!(json.contains("test_identity"));
    }
}
