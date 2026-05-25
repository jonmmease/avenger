pub use avenger_chart_marks::Subplot;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::{
        arrow::{array::Float32Array, record_batch::RecordBatch},
        prelude::{SessionContext, col},
    };

    use super::*;
    use crate::{
        concat::{HConcat, compiled_subplot},
        plot::Plot,
        zerod::ZeroDCoord,
    };
    use avenger_chart_core::{
        AvengerChartError, CompiledMarkCore, CompiledMarkState, FacetDimensionConfig, Mark,
        RowDimensionConfig, SubplotDataSource,
    };

    fn single_column_df(ctx: &SessionContext, value: f32) -> datafusion::dataframe::DataFrame {
        let batch = RecordBatch::try_from_iter(vec![(
            "x",
            Arc::new(Float32Array::from(vec![value])) as Arc<dyn datafusion::arrow::array::Array>,
        )])
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    #[tokio::test]
    async fn subplot_compilation_preserves_label_key_and_child_plot() {
        let ctx = SessionContext::new();
        let subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new())
            .label("overview")
            .key("overview-key");

        let compiled_state = CompiledMarkState::from_mark_state(subplot.state(), None);
        let compiled = <Subplot<HConcat> as Mark<HConcat>>::compile(&subplot, compiled_state, &ctx)
            .await
            .unwrap();
        let compiled = compiled_subplot(compiled.as_ref()).unwrap();

        assert_eq!(compiled.mark_type(), "subplot");
        assert_eq!(compiled.label(), Some("overview"));
        assert_eq!(compiled.key(), Some("overview-key"));
        assert_eq!(compiled.data_source(), SubplotDataSource::InheritParent);
        assert!(compiled.inherits_parent_data());
        assert_eq!(compiled.compiled_subplot().marks().len(), 0);
    }

    #[tokio::test]
    async fn subplot_compilation_keeps_explicit_child_plot_data() {
        let ctx = SessionContext::new();
        let child_data = single_column_df(&ctx, 1.0);
        let subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new().data(child_data));

        let compiled_state = CompiledMarkState::from_mark_state(subplot.state(), None);
        let compiled = <Subplot<HConcat> as Mark<HConcat>>::compile(&subplot, compiled_state, &ctx)
            .await
            .unwrap();
        let compiled = compiled_subplot(compiled.as_ref()).unwrap();

        assert_eq!(compiled.data_source(), SubplotDataSource::ExplicitChild);
        assert!(compiled.has_explicit_child_data());
        assert!(compiled.compiled_subplot().data.is_some());
    }

    #[tokio::test]
    async fn concat_subplot_rejects_facet_channels() {
        let ctx = SessionContext::new();
        let subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new())
            .with_channel_value(RowDimensionConfig::channel_name(), col("group").into());

        let compiled_state = CompiledMarkState::from_mark_state(subplot.state(), None);
        let result =
            <Subplot<HConcat> as Mark<HConcat>>::compile(&subplot, compiled_state, &ctx).await;

        assert!(matches!(result, Err(AvengerChartError::InvalidArgument(_))));
    }

    #[tokio::test]
    async fn plot_compile_passes_parent_data_to_subplot_mark_state() {
        let ctx = SessionContext::new();
        let parent_data = single_column_df(&ctx, 2.0);
        let subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new());

        let compiled_plot = Plot::<HConcat>::new()
            .data(parent_data)
            .mark(subplot)
            .compile(&ctx)
            .await
            .unwrap();
        let compiled = compiled_subplot(compiled_plot.marks()[0].as_ref()).unwrap();

        assert_eq!(compiled.data_source(), SubplotDataSource::InheritParent);
        assert!(
            compiled
                .compiled_state()
                .data
                .dataframe_with_context(&ctx)
                .is_some()
        );
        assert!(compiled.compiled_subplot().data.is_none());
    }

    #[tokio::test]
    async fn repeated_subplot_marks_receive_stable_child_indexes() {
        let ctx = SessionContext::new();
        let compiled_plot = Plot::<HConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("first"))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("second"))
            .compile(&ctx)
            .await
            .unwrap();

        let first = compiled_subplot(compiled_plot.marks()[0].as_ref()).unwrap();
        let second = compiled_subplot(compiled_plot.marks()[1].as_ref()).unwrap();

        assert_eq!(first.child_index(), 0);
        assert_eq!(second.child_index(), 1);
        assert_eq!(first.key(), Some("first"));
        assert_eq!(second.key(), Some("second"));
    }
}
