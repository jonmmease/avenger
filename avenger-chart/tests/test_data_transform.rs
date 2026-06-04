use async_trait::async_trait;
use avenger_chart::prelude::*;
use avenger_chart_core::{AvengerChartError, CompiledDataTransform, DataTransformExecutionContext};
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
struct AddConstant {
    output_name: String,
    amount: f64,
}

impl AddConstant {
    fn new(output_name: impl Into<String>, amount: f64) -> Self {
        Self {
            output_name: output_name.into(),
            amount,
        }
    }
}

#[derive(Clone)]
struct AddConstantOutput {
    output_name: String,
}

impl AddConstantOutput {
    fn expr(&self) -> Expr {
        col(&self.output_name)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CompiledAddConstantTransform {
    output_name: String,
    amount: f64,
}

impl DataTransform for AddConstant {
    type Output = AddConstantOutput;

    fn into_compiled_and_output(
        self,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        let compiled = CompiledAddConstantTransform {
            output_name: self.output_name.clone(),
            amount: self.amount,
        };
        Ok((
            Box::new(compiled),
            AddConstantOutput {
                output_name: self.output_name,
            },
        ))
    }
}

#[typetag::serde(name = "test_external_add_constant")]
#[async_trait]
impl CompiledDataTransform for CompiledAddConstantTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        _ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataFrame, AvengerChartError> {
        Ok(dataframe.with_column(&self.output_name, col("source_value") + lit(self.amount))?)
    }
}

#[tokio::test]
async fn custom_data_transform_can_live_outside_builtin_transform_crate()
-> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let schema = Arc::new(Schema::new(vec![Field::new(
        "source_value",
        DataType::Float64,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0]))],
    )?;
    let df = ctx.read_batch(batch)?;

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Symbol::new().transform(AddConstant::new("shifted_value", 10.0), |mark, shifted| {
                mark.x(col("source_value"))
                    .y(shifted.expr())
                    .size(64.0)
                    .fill("#2f6fed")
            }),
        );

    let compiled = plot.compile(&ctx).await?;
    let serialized = bincode::serialize(&compiled)?;
    let decoded: avenger_chart::plot::CompiledPlot = bincode::deserialize(&serialized)?;
    let mut session = Arc::new(decoded).instantiate(Arc::new(ctx));
    let evaluated = session.evaluate(EvaluationRequest::new().exact()).await?;

    assert!(evaluated.scene_graph.width > 0.0);
    assert!(evaluated.scene_graph.height > 0.0);
    Ok(())
}
