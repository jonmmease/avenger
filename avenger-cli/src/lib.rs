//! `avenger`, the command-line program of avenger-sql projects. Each command starts, reads the
//! files, answers, and exits: `check` reports the diagnostics of a project's files, and `fmt`
//! formats its scripts, modules, and the `{avenger-sql}` cells of its Quarto documents. `lsp` is
//! the exception that an editor starts and stops: the language server of the same files.
//! Results go to standard output, as text or, with `--json`, as JSON, and errors that keep a
//! command from running go to standard error.
//!
//! The exit status is 0 for success, 1 for diagnostics with errors or files that need
//! formatting, and 2 when the command cannot run, such as for a usage error or a manifest that
//! cannot be read.

mod check;
mod files;
mod fmt;
pub mod lsp;
mod render;
mod report;
pub mod view;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::report::Output;

#[derive(Parser)]
#[command(
    name = "avenger",
    version,
    about = "Check and format avenger-sql projects"
)]
struct Cli {
    /// Print the results as JSON, whose shape changes only with its `version` field.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report the errors and warnings of files: of the given files and directories, or of the
    /// project of the current directory.
    Check {
        /// Files or directories, in a project or not. Without one, the current directory.
        paths: Vec<PathBuf>,
        /// Also run each chart script's query, which finds the errors that appear only when it
        /// runs.
        #[arg(long)]
        run: bool,
    },
    /// Serve the Language Server Protocol on standard input and standard output, for an editor.
    Lsp,
    /// Format scripts, modules, and the cells of Quarto documents in place: of the given files
    /// and directories, or of the project of the current directory.
    Fmt {
        /// Files or directories, in a project or not. Without one, the current directory.
        paths: Vec<PathBuf>,
        /// Change no file, and list the files that formatting would change.
        #[arg(long)]
        check: bool,
    },
    /// Open a window on the result of a chart script, or of a cell of a Quarto document, which
    /// stays current as the project's files change.
    View {
        /// A chart script, or a cell as `report.qmd#NUMBER` or `report.qmd#LABEL`.
        target: String,
        /// The color scheme that the window starts in: light or dark. Without it, the system's.
        #[arg(long)]
        color_scheme: Option<String>,
    },
    /// Render the chart of a chart script, or of a cell of a Quarto document, to PNG, SVG, or PDF
    /// by the extension of `-o`, as the editor draws it. A result of several charts renders as a
    /// table.
    Render {
        /// A chart script, or a cell as `report.qmd#NUMBER` or `report.qmd#LABEL`. With `--all`,
        /// a directory of the project, or none for the current directory.
        target: Option<String>,
        /// The image, `.png`, `.svg`, or `.pdf`, or with `--all`, its directory.
        #[arg(short, long)]
        out: PathBuf,
        /// Render every chart script and cell of the project, one file each, into `-o`.
        #[arg(long)]
        all: bool,
        /// The format of the images of `--all`: png, svg, or pdf.
        #[arg(long, default_value = "png")]
        format: String,
        /// The size of a chart, `WxH`, `Wx`, or `xH`, in pixels: of the chart, or of each chart
        /// of a table. Without it, a chart takes its own size or its natural size.
        #[arg(long)]
        size: Option<String>,
        /// The pixel density of PNG.
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
        /// The color scheme that the stylesheets' `prefers-color-scheme` tests, and its background:
        /// light or dark.
        #[arg(long, default_value = "light")]
        color_scheme: String,
        /// A stylesheet after the project's.
        #[arg(long)]
        style: Option<PathBuf>,
        /// A mode of the chart to turn on, which repeats.
        #[arg(long = "mode")]
        modes: Vec<String>,
        /// One chart of a table, `ROW[.COLUMN]`, rows from 1.
        #[arg(long)]
        chart: Option<String>,
        /// The most rows of a table.
        #[arg(long, default_value_t = 20)]
        rows: usize,
        /// The seconds to wait for the images of the charts, such as the tiles of a map.
        #[arg(long, default_value_t = 30.0)]
        timeout: f64,
    },
}

/// The size of `--size`: `WxH`, `Wx`, or `xH`, in pixels.
fn parse_size(size: Option<&str>) -> Result<[Option<f32>; 2], String> {
    let Some(size) = size else {
        return Ok([None, None]);
    };
    let error = || format!("`--size {size}`: a size is `WxH`, `Wx`, or `xH`, such as `640x400`");
    let (width, height) = size.split_once(['x', 'X']).ok_or_else(error)?;
    let side = |text: &str| -> Result<Option<f32>, String> {
        match text.trim() {
            "" => Ok(None),
            text => match text.parse::<f32>() {
                Ok(value) if value.is_finite() && value > 0.0 => Ok(Some(value)),
                _ => Err(error()),
            },
        }
    };
    let size = [side(width)?, side(height)?];
    match size {
        [None, None] => Err(error()),
        size => Ok(size),
    }
}

/// Whether a command found what fails it: errors, or files that need formatting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Ok,
    Failed,
}

/// Runs the command of the program's arguments, and gives its exit status.
pub fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Command::Lsp = cli.command {
        return match lsp::serve_stdio() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(1)
            }
        };
    }
    if let Command::View {
        target,
        color_scheme,
    } = &cli.command
    {
        let scheme = match color_scheme.as_deref() {
            None => None,
            Some("light") => Some(avenger_style::ColorScheme::Light),
            Some("dark") => Some(avenger_style::ColorScheme::Dark),
            Some(other) => {
                eprintln!("error: `--color-scheme {other}`: the color schemes are light and dark");
                return ExitCode::from(2);
            }
        };
        // The window takes this thread, as a native window needs on macOS.
        return match view::run(target.clone(), scheme) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(2)
            }
        };
    }
    let output = Output::new(cli.json);
    // The futures of a loaded project are not `Send`, so they run on this thread.
    let runtime = match tokio::runtime::Builder::new_current_thread().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    let result = match cli.command {
        Command::Check { paths, run } => runtime.block_on(check::run(&paths, run, &output)),
        Command::Fmt { paths, check } => runtime.block_on(fmt::run(&paths, check, &output)),
        Command::Render {
            target,
            out,
            all,
            format,
            size,
            scale,
            color_scheme,
            style,
            modes,
            chart,
            rows,
            timeout,
        } => (|| {
            // The chart apps prepare their charts on a runtime of their own threads.
            let apps = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            let options = render::Options {
                size: parse_size(size.as_deref())?,
                scale,
                color_scheme: match color_scheme.as_str() {
                    "light" => avenger_style::ColorScheme::Light,
                    "dark" => avenger_style::ColorScheme::Dark,
                    _ => {
                        return Err(format!(
                            "`--color-scheme {color_scheme}`: the color schemes are light and dark"
                        ))
                    }
                },
                style,
                modes,
                chart,
                rows,
                timeout: std::time::Duration::from_secs_f64(timeout.max(0.0)),
                runtime: apps.handle().clone(),
            };
            let result = match (all, target) {
                (true, target) => {
                    let path = PathBuf::from(target.unwrap_or_else(|| ".".into()));
                    let format = render::Format::of(Path::new(&format!("image.{format}")))?;
                    runtime.block_on(render::run_all(&path, &out, format, &options, &output))
                }
                (false, Some(target)) => {
                    runtime.block_on(render::run(&target, &out, &options, &output))
                }
                (false, None) => {
                    Err("name a chart script, or render the project with `--all`".into())
                }
            };
            apps.shutdown_background();
            result
        })(),
        Command::Lsp | Command::View { .. } => unreachable!("these run above"),
    };
    match result {
        Ok(Status::Ok) => ExitCode::SUCCESS,
        Ok(Status::Failed) => ExitCode::from(1),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}
