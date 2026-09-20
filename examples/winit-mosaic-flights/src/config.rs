use anyhow::{Context, Result, bail};
use std::path::PathBuf;

pub struct Config {
    pub data: PathBuf,
    pub preaggregate: bool,
    pub diagnostics: bool,
}
impl Config {
    pub fn parse() -> Result<Self> {
        let mut data = None;
        let mut preaggregate = true;
        let mut diagnostics = false;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--data" => data = Some(args.next().context("--data requires a path")?.into()),
                "--preaggregate" => {
                    preaggregate = match args.next().as_deref() {
                        Some("auto") => true,
                        Some("off") => false,
                        _ => bail!("--preaggregate requires auto or off"),
                    };
                }
                "--diagnostics" => diagnostics = true,
                "--help" | "-h" => {
                    println!(
                        "Mosaic Flights · 10M\n  --data PATH\n  --preaggregate auto|off (default: auto)\n  --diagnostics"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown option: {arg}"),
            }
        }
        Ok(Self {
            data: data.context(
                "--data is required: provide a Mosaic flights Parquet file (see README)",
            )?,
            preaggregate,
            diagnostics,
        })
    }
}
