use anyhow::{Context, Result, bail, ensure};
use std::path::PathBuf;

pub struct Config {
    pub data: PathBuf,
    pub batch_rows: usize,
    pub interval_ms: u64,
    pub max_batches: Option<usize>,
    pub paused: bool,
    pub diagnostics: bool,
    pub headless: bool,
}

impl Config {
    pub fn parse() -> Result<Self> {
        let mut data = None;
        let mut batch_rows = 100_000;
        let mut interval_ms = 1_000;
        let mut max_batches = None;
        let mut paused = false;
        let mut diagnostics = false;
        let mut headless = false;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--data" => {
                    data = Some(PathBuf::from(
                        args.next().context("--data requires a path")?,
                    ))
                }
                "--batch-rows" => {
                    batch_rows = args
                        .next()
                        .context("--batch-rows requires a number")?
                        .parse()?
                }
                "--interval-ms" => {
                    interval_ms = args
                        .next()
                        .context("--interval-ms requires a number")?
                        .parse()?
                }
                "--max-batches" => {
                    max_batches = Some(
                        args.next()
                            .context("--max-batches requires a number")?
                            .parse()?,
                    )
                }
                "--paused" => paused = true,
                "--headless" => headless = true,
                "--diagnostics" => diagnostics = true,
                "--help" | "-h" => {
                    println!(
                        "Streaming Mosaic Flights · historical replay\n  --data PATH (BTS Parquet file or directory)\n  --batch-rows N (default: 100000)\n  --interval-ms N (default: 1000)\n  --max-batches N\n  --paused\n  --diagnostics\n  --headless (JSON baseline measurements, no replay delay)\n\nSpace: play/pause · N: next batch · R: restart · +/-: speed · W: retry warming"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown option: {arg}"),
            }
        }
        ensure!(batch_rows > 0, "--batch-rows must be positive");
        ensure!(interval_ms > 0, "--interval-ms must be positive");
        ensure!(max_batches != Some(0), "--max-batches must be positive");
        Ok(Self {
            data: data.context("--data is required: provide the BTS dataset path (see README)")?,
            batch_rows,
            interval_ms,
            max_batches,
            paused,
            diagnostics,
            headless,
        })
    }
}
