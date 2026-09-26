use avenger_chart::{Chart, RenderOptions};
use avenger_chart_definition::{
    dataflow::{
        arrow::{
            array::{Float64Array, StringArray},
            record_batch::RecordBatch,
        },
        *,
    },
    *,
};
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark};
use std::sync::Arc;
fn rect(marks: &[SceneMark]) -> Option<&SceneRectMark> {
    marks.iter().find_map(|m| match m {
        SceneMark::Rect(r) if r.name == "bars" => Some(r),
        SceneMark::Group(g) => rect(&g.marks),
        _ => None,
    })
}
#[tokio::test]
async fn centered_spans_baselines_and_new_descriptor_round_trip() -> anyhow::Result<()> {
    let b = RecordBatch::try_from_iter([
        ("c", Arc::new(StringArray::from(vec!["A", "B"])) as _),
        ("v", Arc::new(Float64Array::from(vec![7., 10.])) as _),
    ])?;
    let mut f = DataflowBuilder::new();
    let t = f.table_snapshot("rows", TableSnapshot::from_batches(b.schema(), vec![b])?)?;
    let t = f.table_output("rows", &t)?;
    let mut d = ChartDefinition::builder(f.finish()?);
    d.background("white");
    d.title("A title\nTwo lines");
    d.plot("plot", |p| {
        p.content_size(100., 100.);
        let x = p.scale(
            "x",
            Scale::band(Domain::column(&t, "c"), Range::Fixed(100., 0.)).include_null(true),
        )?;
        let y = p.scale(
            "y",
            Scale::linear(Domain::numeric(5., 10.), Range::PlotHeightReversed),
        )?;
        p.rect(
            "bars",
            &t,
            RectEncoding::new()
                .xc(x.band_position("c", 0.5))
                .width(6.)
                .y(y.baseline())
                .y2(y.field("v"))
                .y_span(SpanAdjustment {
                    spacing: 1.,
                    minimum: 0.25,
                    offset: 0.5,
                }),
        )?;
        p.axis(
            Axis::bottom(&x)
                .label_angle(30.)
                .title("First line\nSecond line"),
        )?;
        Ok(())
    })?;
    let d = d.finish()?;
    let bytes = d.to_bytes()?;
    let rt = Runtime::new(RuntimeConfig::default())?;
    let d = ChartDefinition::from_bytes(&bytes, &rt)?;
    assert_eq!(d.background(), Some("white"));
    let chart = Chart::prepare(d, Default::default()).await?;
    let f = chart.render(RenderOptions::default()).await?;
    let SceneMark::Group(root) = &f.scenegraph().marks[0] else {
        panic!()
    };
    let header = root
        .marks
        .iter()
        .find_map(|m| match m {
            SceneMark::Text(t) if t.name.ends_with(":header") => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(header.len, 2);
    let r = rect(&f.scenegraph().marks).unwrap();
    assert_eq!(r.x_vec(), vec![72., 22.]);
    assert_eq!(r.y_vec(), vec![61., 1.]);
    assert_eq!(r.height.as_ref().unwrap().as_vec(2, None), vec![39., 99.]);
    Ok(())
}
