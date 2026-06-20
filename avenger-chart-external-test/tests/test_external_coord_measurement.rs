use avenger_chart::plot::Plot;
use avenger_chart_external_test::external_coord_system::{MeasuredExternalCoord, MeasuredRect};
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark};
use datafusion::prelude::SessionContext;

fn find_rect<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneRectMark> {
    for mark in marks {
        match mark {
            SceneMark::Rect(rect) if rect.name == name => return Some(rect),
            SceneMark::Group(group) => {
                if let Some(rect) = find_rect(&group.marks, name) {
                    return Some(rect);
                }
            }
            _ => {}
        }
    }
    None
}

#[tokio::test]
async fn external_coordinate_provider_installs_measurement_for_mark_rendering() {
    let ctx = SessionContext::new();
    let compiled = Plot::<MeasuredExternalCoord>::new()
        .mark(MeasuredRect::new())
        .plot_size(400.0, 300.0)
        .compile(&ctx)
        .await
        .expect("compile measured external coordinate plot");

    let evaluated = compiled
        .evaluate(&ctx, None)
        .await
        .expect("evaluate measured external coordinate plot");

    let rect = find_rect(&evaluated.scene_graph.marks, "external_measured_rect")
        .expect("external measured rect should render from custom coord measurement");
    assert_eq!(rect.x.as_vec(1, None), vec![40.0]);
    assert_eq!(rect.y.as_vec(1, None), vec![30.0]);
    assert_eq!(
        rect.width.as_ref().expect("width").as_vec(1, None),
        vec![101.0]
    );
    assert_eq!(
        rect.height.as_ref().expect("height").as_vec(1, None),
        vec![60.0]
    );
}
