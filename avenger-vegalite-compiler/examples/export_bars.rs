//! Compile once, then export two numeric-parameter snapshots.
use avenger_chart::{Chart, RenderOptions};
use avenger_datafusion_dataflow::datafusion::common::ScalarValue;
use avenger_vegalite_compiler::{spec::UnitSpec, FromVegaLite, VegaLiteOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let output = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    std::fs::create_dir_all(&output)?;
    let spec_path = std::env::args().nth(2).map(std::path::PathBuf::from);
    let (spec, options, jobs) = if let Some(path) = spec_path {
        let spec = UnitSpec::from_json(&std::fs::read_to_string(&path)?)?;
        let options = VegaLiteOptions {
            base_dir: path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_owned(),
            ..Default::default()
        };
        (spec, options, vec![("chart", RenderOptions::default())])
    } else {
        (
            UnitSpec::from_json(include_str!("bars.json"))?,
            VegaLiteOptions::default(),
            vec![
                ("bars", RenderOptions::default()),
                (
                    "filtered-bars",
                    RenderOptions::default().parameter("minimum", ScalarValue::Float64(Some(15.))),
                ),
            ],
        )
    };
    let chart = Chart::from_vegalite(&spec, &Default::default(), options).await?;
    for (name, request) in jobs {
        let frame = chart.render(request).await?;
        std::fs::write(format!("{output}/{name}.svg"), frame.to_svg()?)?;
        std::fs::write(format!("{output}/{name}.pdf"), frame.to_pdf()?)?;
        std::fs::write(format!("{output}/{name}.png"), frame.to_png(2.).await?)?;
        println!("{name}: {:?}", frame.report().executed_nodes);
    }
    Ok(())
}
