use avenger_datafusion_aggregate_state::{
    functions::{avg_finalize_udf, avg_state_udaf},
    register_all, AggregateStateExtensionCodec,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::{Result, ScalarValue},
    logical_expr::{create_udf, ColumnarValue, Volatility},
    prelude::SessionContext,
};
use datafusion_proto::{
    bytes::{
        logical_plan_from_bytes_with_extension_codec, logical_plan_to_bytes_with_extension_codec,
    },
    logical_plan::LogicalExtensionCodec,
};
use std::sync::Arc;

#[tokio::test]
async fn all_operations_decode_and_execute_without_function_registration() -> Result<()> {
    let mut source = SessionContext::new();
    register_all(&mut source)?;
    let codec = AggregateStateExtensionCodec::default();
    for family in [
        "count",
        "sum",
        "min",
        "max",
        "avg",
        "varSamp",
        "varPop",
        "stddevSamp",
        "stddevPop",
    ] {
        for predicate in ["true", "false"] {
            let plan = source
                .sql(&format!(
                    "SELECT {family}Finalize({family}MergeState(s)) AS finalized, \
                 {family}Merge(s) AS merged FROM \
                 (SELECT {family}State(x) AS s FROM \
                 (VALUES (0, CAST(2 AS DOUBLE)), (0, NULL), (1, 4), (1, 6)) AS raw(g, x) \
                 WHERE {predicate} GROUP BY g) AS states"
                ))
                .await?
                .into_unoptimized_plan();
            let bytes = logical_plan_to_bytes_with_extension_codec(&plan, &codec)?;
            let fresh = SessionContext::new();
            let decoded =
                logical_plan_from_bytes_with_extension_codec(&bytes, &fresh.task_ctx(), &codec)?;
            assert_eq!(plan.schema(), decoded.schema());
            let expected = source.execute_logical_plan(plan).await?.collect().await?;
            let actual = fresh.execute_logical_plan(decoded).await?.collect().await?;
            assert_eq!(expected, actual, "{family}, {predicate}");
        }
    }
    Ok(())
}

#[test]
fn rejects_wrong_names_versions_kinds_and_truncated_payloads() -> Result<()> {
    let codec = AggregateStateExtensionCodec::default();
    let mut aggregate = vec![];
    codec.try_encode_udaf(&avg_state_udaf(), &mut aggregate)?;
    let mut scalar = vec![];
    codec.try_encode_udf(&avg_finalize_udf(), &mut scalar)?;
    assert!(codec.try_decode_udaf("sumState", &aggregate).is_err());
    assert!(codec.try_decode_udf("sumFinalize", &scalar).is_err());
    assert!(codec.try_decode_udf("avgState", &aggregate).is_err());
    assert!(codec.try_decode_udaf("avgFinalize", &scalar).is_err());
    for end in 0..aggregate.len() {
        assert!(codec
            .try_decode_udaf("avgState", &aggregate[..end])
            .is_err());
    }
    for end in 0..scalar.len() {
        assert!(codec.try_decode_udf("avgFinalize", &scalar[..end]).is_err());
    }
    for bytes in [&mut aggregate, &mut scalar] {
        let version = bytes.iter().position(|b| *b == b'/').unwrap() + 1;
        bytes[version] = b'9';
    }
    assert!(codec.try_decode_udaf("avgState", &aggregate).is_err());
    assert!(codec.try_decode_udf("avgFinalize", &scalar).is_err());
    Ok(())
}

#[test]
fn does_not_claim_foreign_functions_with_matching_names() -> Result<()> {
    let codec = AggregateStateExtensionCodec::default();
    let foreign = create_udf(
        "avgFinalize",
        vec![],
        DataType::Int64,
        Volatility::Immutable,
        Arc::new(|_| Ok(ColumnarValue::Scalar(ScalarValue::Int64(Some(7))))),
    );
    let mut bytes = vec![];
    codec.try_encode_udf(&foreign, &mut bytes)?;
    assert!(bytes.is_empty());
    codec.try_encode_udaf(
        &datafusion::functions_aggregate::sum::sum_udaf(),
        &mut bytes,
    )?;
    assert!(bytes.is_empty());
    Ok(())
}
