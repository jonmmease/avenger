use async_trait::async_trait;
use datafusion::{
    common::ScalarValue, dataframe::DataFrame, logical_expr::Expr, prelude::SessionContext,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    AvengerChartError, CoordinationScope, DerivedScalarMap, MaterializationIdentity,
    MaterializationPolicy, MaterializationRequest, SharingLevel, TimeContext,
};

#[derive(Clone, Debug)]
pub struct DataTransformFacetContext {
    pub transform_level: SharingLevel,
    pub final_mark_level: SharingLevel,
    /// Facet expressions needed to distinguish the final mark cells below this
    /// transform's current sharing scope.
    pub partition_exprs: Vec<Expr>,
}

pub struct DataTransformExecutionContext<'a> {
    pub session_context: &'a SessionContext,
    pub params: &'a IndexMap<String, ScalarValue>,
    pub time_context: TimeContext,
    pub facet_context: Option<DataTransformFacetContext>,
}

pub struct ViewMaterializationContext<'a> {
    pub session_context: &'a SessionContext,
    pub params: &'a IndexMap<String, ScalarValue>,
    pub time_context: TimeContext,
    pub facet_context: Option<&'a DataTransformFacetContext>,
    pub policy: MaterializationPolicy,
    pub priority: f32,
}

pub struct ViewMaterializationRequest {
    pub request: MaterializationRequest,
    pub empty_dataframe: Option<DataFrame>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataTransformCompileContext {
    pub scope: CoordinationScope,
}

impl DataTransformCompileContext {
    pub fn new(scope: CoordinationScope) -> Self {
        Self {
            scope: scope.to_normalized(),
        }
    }
}

#[typetag::serde(tag = "type")]
#[async_trait]
pub trait CompiledDataTransform: Send + Sync {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform>;

    fn map_exprs(
        &self,
        _f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(self.clone_box())
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError>;

    fn view_materialization_request(
        &self,
        _dataframe: &DataFrame,
        _ctx: &ViewMaterializationContext<'_>,
    ) -> Result<Option<ViewMaterializationRequest>, AvengerChartError> {
        Ok(None)
    }

    /// Materialization identity computed from THIS transform instance.
    ///
    /// The chain executor calls this on the UNRESOLVED stage transform
    /// (derived-scalar placeholders intact) to override the identity that
    /// [`Self::view_materialization_request`] computed from the resolved
    /// copy. Resolved derived scalars are runtime-varying inputs exactly
    /// like params: baked into the identity they make it churn with every
    /// value change, so the stale-result fallback never finds a
    /// same-lineage result to re-display.
    fn view_materialization_identity(
        &self,
        _dataframe: &DataFrame,
        _ctx: &ViewMaterializationContext<'_>,
    ) -> Result<Option<MaterializationIdentity>, AvengerChartError> {
        Ok(None)
    }
}

impl Clone for Box<dyn CompiledDataTransform> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DataTransformStage {
    pub scope: CoordinationScope,
    pub transform: Box<dyn CompiledDataTransform>,
}

impl DataTransformStage {
    pub fn new(scope: CoordinationScope, transform: Box<dyn CompiledDataTransform>) -> Self {
        Self {
            scope: scope.to_normalized(),
            transform,
        }
    }

    pub fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(Self {
            scope: self.scope,
            transform: self.transform.map_exprs(f)?,
        })
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

/// Apply a transform chain, accumulating derived scalars with duplicate
/// detection and resolving already-produced scalars into later stage
/// expressions.
///
/// This core helper resolves within-chain scalars only. Inherited-scalar
/// seeding (scalars produced by prepared base / mark-group data) is a
/// facade-runtime concept handled by the scoped chain executor in
/// `avenger-chart`'s `plot::compiled::mark_data_runtime`; this helper's
/// callers have no inheritance source.
pub async fn apply_compiled_data_transforms(
    mut dataframe: DataFrame,
    transforms: &[DataTransformStage],
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<DataTransformResult, AvengerChartError> {
    let mut derived_scalars = DerivedScalarMap::new();
    for stage in transforms {
        let transform = stage.transform.map_exprs(&mut |expr| {
            let expr = crate::resolve_known_derived_scalars(expr, &derived_scalars)?;
            // Stages run in order: a still-unresolved derived-scalar
            // reference can never be satisfied by a later stage.
            if let Some(id) = crate::collect_derived_scalar_ids(&expr)?.into_iter().next() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Derived scalar '{id}' was referenced but not produced in this data scope"
                )));
            }
            Ok(expr)
        })?;
        let result = transform.apply(dataframe, ctx).await?;
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
            CoordinationScope::Free,
            Box::new(IdentityTransform),
        )];
        let cloned = transforms.clone();
        let result = apply_compiled_data_transforms(
            dataframe,
            &cloned,
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
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
                CoordinationScope::Free,
                Box::new(DerivedScalarTransform {
                    id: "first".to_string(),
                    value: 1.0,
                }),
            ),
            DataTransformStage::new(
                CoordinationScope::Free,
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
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
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
                CoordinationScope::Free,
                Box::new(DerivedScalarTransform {
                    id: "duplicate".to_string(),
                    value: 1.0,
                }),
            ),
            DataTransformStage::new(
                CoordinationScope::Free,
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
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
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
        let stage =
            DataTransformStage::new(CoordinationScope::Level(2), Box::new(IdentityTransform));
        let bytes = bincode::serialize(&stage).expect("serialize transform stage");
        let decoded: DataTransformStage =
            bincode::deserialize(&bytes).expect("deserialize transform stage");
        assert_eq!(decoded.scope, CoordinationScope::Level(2));

        let json = serde_json::to_string(&decoded).expect("serialize decoded stage as json");
        assert!(json.contains("test_identity"));
    }
}
