use async_trait::async_trait;
use datafusion::{dataframe::DataFrame, prelude::SessionContext};
use serde::{Deserialize, Serialize};

use crate::{AvengerChartError, DerivedScalarMap, Sharing};

pub struct DataTransformExecutionContext<'a> {
    pub session_context: &'a SessionContext,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataTransformCompileContext {
    pub scope: Sharing,
}

impl DataTransformCompileContext {
    pub fn new(scope: Sharing) -> Self {
        Self {
            scope: scope.to_normalized(),
        }
    }
}

#[typetag::serde(tag = "type")]
#[async_trait]
pub trait CompiledDataTransform: Send + Sync {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform>;

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError>;
}

impl Clone for Box<dyn CompiledDataTransform> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DataTransformStage {
    pub scope: Sharing,
    pub transform: Box<dyn CompiledDataTransform>,
}

impl DataTransformStage {
    pub fn new(scope: Sharing, transform: Box<dyn CompiledDataTransform>) -> Self {
        Self {
            scope: scope.to_normalized(),
            transform,
        }
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
        ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError>;
}

/// Result of applying a compiled data transform.
pub struct DataTransformResult {
    pub dataframe: DataFrame,
    pub derived_scalars: DerivedScalarMap,
}

impl DataTransformResult {
    pub fn dataframe(dataframe: DataFrame) -> Self {
        Self {
            dataframe,
            derived_scalars: DerivedScalarMap::new(),
        }
    }
}

pub async fn apply_compiled_data_transforms(
    mut dataframe: DataFrame,
    transforms: &[DataTransformStage],
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<DataTransformResult, AvengerChartError> {
    let mut derived_scalars = DerivedScalarMap::new();
    for stage in transforms {
        let result = stage.transform.apply(dataframe, ctx).await?;
        dataframe = result.dataframe;
        for (id, expr) in result.derived_scalars {
            if derived_scalars.insert(id.clone(), expr).is_some() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Derived scalar '{id}' was produced more than once in the same data scope"
                )));
            }
        }
    }
    Ok(DataTransformResult {
        dataframe,
        derived_scalars,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::{
        arrow::{datatypes::Schema, record_batch::RecordBatch},
        logical_expr::lit,
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
        ) -> Result<DataTransformResult, AvengerChartError> {
            Ok(DataTransformResult::dataframe(dataframe))
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct DerivedScalarTransform {
        id: String,
        value: f64,
    }

    #[typetag::serde(name = "test_derived_scalar")]
    #[async_trait]
    impl CompiledDataTransform for DerivedScalarTransform {
        fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
            Box::new(self.clone())
        }

        async fn apply(
            &self,
            dataframe: DataFrame,
            _ctx: &DataTransformExecutionContext<'_>,
        ) -> Result<DataTransformResult, AvengerChartError> {
            let mut derived_scalars = DerivedScalarMap::new();
            derived_scalars.insert(self.id.clone(), lit(self.value));
            Ok(DataTransformResult {
                dataframe,
                derived_scalars,
            })
        }
    }

    fn empty_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.read_batch(RecordBatch::new_empty(Arc::new(Schema::empty())))
            .unwrap()
    }

    #[tokio::test]
    async fn boxed_compiled_transform_clones_and_applies() {
        let ctx = SessionContext::new();
        let dataframe = empty_dataframe(&ctx);
        let transforms = vec![DataTransformStage::new(
            Sharing::Free,
            Box::new(IdentityTransform),
        )];
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

    #[tokio::test]
    async fn transform_results_accumulate_derived_scalars() {
        let ctx = SessionContext::new();
        let transforms = vec![
            DataTransformStage::new(
                Sharing::Free,
                Box::new(DerivedScalarTransform {
                    id: "first".to_string(),
                    value: 1.0,
                }),
            ),
            DataTransformStage::new(
                Sharing::Free,
                Box::new(DerivedScalarTransform {
                    id: "second".to_string(),
                    value: 2.0,
                }),
            ),
        ];

        let result = apply_compiled_data_transforms(
            empty_dataframe(&ctx),
            &transforms,
            &DataTransformExecutionContext {
                session_context: &ctx,
            },
        )
        .await
        .expect("apply transforms");

        assert!(result.derived_scalars.contains_key("first"));
        assert!(result.derived_scalars.contains_key("second"));
        assert_eq!(result.derived_scalars.len(), 2);
    }

    #[tokio::test]
    async fn duplicate_derived_scalar_ids_error() {
        let ctx = SessionContext::new();
        let transforms = vec![
            DataTransformStage::new(
                Sharing::Free,
                Box::new(DerivedScalarTransform {
                    id: "duplicate".to_string(),
                    value: 1.0,
                }),
            ),
            DataTransformStage::new(
                Sharing::Free,
                Box::new(DerivedScalarTransform {
                    id: "duplicate".to_string(),
                    value: 2.0,
                }),
            ),
        ];

        let result = apply_compiled_data_transforms(
            empty_dataframe(&ctx),
            &transforms,
            &DataTransformExecutionContext {
                session_context: &ctx,
            },
        )
        .await;
        let err = match result {
            Ok(_) => panic!("duplicate derived scalar should error"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("Derived scalar 'duplicate' was produced more than once"),
            "{err}"
        );
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

    #[test]
    fn transform_stage_serializes_scope_and_transform() {
        let stage = DataTransformStage::new(Sharing::Level(2), Box::new(IdentityTransform));
        let bytes = bincode::serialize(&stage).expect("serialize transform stage");
        let decoded: DataTransformStage =
            bincode::deserialize(&bytes).expect("deserialize transform stage");
        assert_eq!(decoded.scope, Sharing::Level(2));

        let json = serde_json::to_string(&decoded).expect("serialize decoded stage as json");
        assert!(json.contains("test_identity"));
    }
}
