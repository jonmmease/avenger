//! Compatibility re-export for core DataFrame serialization.

pub use avenger_chart_core::serialization::SerializableDataFrame;

#[cfg(test)]
mod tests {
    use datafusion::prelude::SessionContext;

    use super::*;

    #[tokio::test]
    async fn test_serializable_dataframe_roundtrip() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT * FROM (VALUES (1, 'a'), (2, 'b'), (3, 'c')) AS t(id, name)")
            .await
            .unwrap();

        let serializable = SerializableDataFrame::from_dataframe(df).unwrap();
        let json = serde_json::to_string(&serializable).unwrap();
        let deserialized: SerializableDataFrame = serde_json::from_str(&json).unwrap();

        let new_ctx = SessionContext::new();
        let restored_df = deserialized.to_dataframe(&new_ctx);

        assert_eq!(
            restored_df.unwrap().logical_plan().schema().fields().len(),
            2
        );
    }

    #[tokio::test]
    async fn test_option_serializable_dataframe() {
        let none_df: Option<SerializableDataFrame> = None;
        let json = serde_json::to_string(&none_df).unwrap();
        assert_eq!(json, "null");

        let deserialized: Option<SerializableDataFrame> = serde_json::from_str(&json).unwrap();
        assert!(deserialized.is_none());

        let ctx = SessionContext::new();
        let df = ctx.sql("SELECT 1 as id").await.unwrap();
        let some_df = Some(SerializableDataFrame::from_dataframe(df).unwrap());

        let json = serde_json::to_string(&some_df).unwrap();
        let deserialized: Option<SerializableDataFrame> = serde_json::from_str(&json).unwrap();
        assert!(deserialized.is_some());
    }
}
