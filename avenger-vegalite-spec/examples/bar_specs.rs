use avenger_vegalite_spec::{AggregateOp, UnitSpec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut bars = UnitSpec::from_json(include_str!("../tests/fixtures/aggregate-bars.json"))?;
    bars.encoding
        .as_mut()
        .unwrap()
        .y
        .as_mut()
        .unwrap()
        .aggregate = Some(AggregateOp::Max);
    bars.validate()?;
    println!("Aggregate bars, edited from mean to max:");
    println!("{}\n", serde_json::to_string_pretty(&bars)?);

    let histogram = UnitSpec::from_json(include_str!("../tests/fixtures/histogram.json"))?;
    println!("Histogram with a 10-minute bin width (flights.csv is not accessed):");
    println!("{}\n", serde_json::to_string_pretty(&histogram)?);

    let transformed =
        UnitSpec::from_json(include_str!("../tests/fixtures/explicit-transforms.json"))?;
    println!("Explicit bin and aggregate transforms, using already binned field encodings:");
    println!("{}", serde_json::to_string_pretty(&transformed)?);
    Ok(())
}
