use anyhow::{Result, bail};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub data: PathBuf,
    pub preaggregate: bool,
    pub exact: bool,
    pub diagnostics: bool,
    pub sql: bool,
    pub headless: bool,
    pub no_cache: bool,
    pub cache_mib: usize,
    pub materialization_mib: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            data: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/flights.parquet"),
            preaggregate: true,
            exact: false,
            diagnostics: false,
            sql: false,
            headless: false,
            no_cache: false,
            cache_mib: 256,
            materialization_mib: 512,
        }
    }
}
impl Config {
    pub fn parse() -> Result<Self> {
        let mut c = Self::default();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--data" => {
                    c.data = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--data requires a path"))?
                        .into()
                }
                "--preaggregate" => {
                    c.preaggregate = match args.next().as_deref() {
                        Some("auto") => true,
                        Some("off") => false,
                        _ => bail!("--preaggregate requires auto or off"),
                    }
                }
                "--exact-selection" => c.exact = true,
                "--diagnostics" => c.diagnostics = true,
                "--sql" => c.sql = true,
                "--headless" => c.headless = true,
                "--no-cache" => c.no_cache = true,
                "--cache-mib" => {
                    c.cache_mib = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("missing cache size"))?
                        .parse()?
                }
                "--materialization-mib" => {
                    c.materialization_mib = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("missing materialization size"))?
                        .parse()?
                }
                "--help" | "-h" => {
                    println!(
                        "Flight Delay Explorer\n  --data PATH\n  --preaggregate auto|off (default: auto)\n  --exact-selection\n  --diagnostics\n  --sql\n  --headless\n  --no-cache\n  --cache-mib N (default: 256)\n  --materialization-mib N (default: 512)"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown option: {arg}"),
            }
        }
        Ok(c)
    }
    pub fn mode(&self) -> &'static str {
        if self.preaggregate { "auto" } else { "off" }
    }
}
