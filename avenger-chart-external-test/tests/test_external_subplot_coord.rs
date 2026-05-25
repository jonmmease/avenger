use avenger_chart::{
    marks::{Mark, Subplot},
    plot::Plot,
};
use avenger_chart_core::ZeroDCoord;
use avenger_chart_external_test::external_subplot_coord::{
    CompiledExternalCoordSubplot, ExternalSubplotCoord,
};
use datafusion::prelude::SessionContext;

#[tokio::test]
async fn external_coordinate_can_compile_subplot_mark() {
    let ctx = SessionContext::new();
    let child = Plot::<ZeroDCoord>::new();
    let subplot = Subplot::<ExternalSubplotCoord>::new(child)
        .label("child label")
        .key("child-key");

    let compiled = subplot.compile_untransformed(&ctx).await.unwrap();

    assert_eq!(compiled.mark_type(), "external_coord_subplot");
    let compiled = compiled
        .as_any()
        .downcast_ref::<CompiledExternalCoordSubplot>()
        .unwrap();
    assert_eq!(compiled.payload().label(), Some("child label"));
    assert_eq!(compiled.payload().key(), Some("child-key"));
    assert!(compiled.payload().inherits_parent_data());
    assert_eq!(compiled.payload().compiled_subplot().marks().len(), 0);
}

#[tokio::test]
async fn external_coordinate_subplot_can_be_added_to_plot() {
    let ctx = SessionContext::new();
    let plot = Plot::<ExternalSubplotCoord>::new().mark(Subplot::new(Plot::<ZeroDCoord>::new()));

    let compiled = plot.compile(&ctx).await.unwrap();

    assert_eq!(compiled.marks().len(), 1);
    assert_eq!(compiled.marks()[0].mark_type(), "external_coord_subplot");
}
