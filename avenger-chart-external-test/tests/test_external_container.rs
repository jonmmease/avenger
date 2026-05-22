use avenger_chart::{
    marks::{Mark, Subplot},
    plot::Plot,
    zerod::ZeroDCoord,
};
use avenger_chart_external_test::external_container::{
    CompiledExternalStackSubplot, ExternalStack,
};
use datafusion::prelude::SessionContext;

#[tokio::test]
async fn external_container_can_compile_subplot_mark() {
    let ctx = SessionContext::new();
    let child = Plot::<ZeroDCoord>::new();
    let subplot = Subplot::<ExternalStack>::new(child)
        .label("child label")
        .key("child-key");

    let compiled = subplot.compile_untransformed(&ctx).await.unwrap();

    assert_eq!(compiled.mark_type(), "external_stack_subplot");
    let compiled = compiled
        .as_any()
        .downcast_ref::<CompiledExternalStackSubplot>()
        .unwrap();
    assert_eq!(compiled.payload().label(), Some("child label"));
    assert_eq!(compiled.payload().key(), Some("child-key"));
    assert!(compiled.payload().inherits_parent_data());
    assert_eq!(compiled.payload().compiled_subplot().marks().len(), 0);
}

#[tokio::test]
async fn external_container_subplot_can_be_added_to_plot() {
    let ctx = SessionContext::new();
    let plot = Plot::<ExternalStack>::new().mark(Subplot::new(Plot::<ZeroDCoord>::new()));

    let compiled = plot.compile(&ctx).await.unwrap();

    assert_eq!(compiled.marks().len(), 1);
    assert_eq!(compiled.marks()[0].mark_type(), "external_stack_subplot");
}
