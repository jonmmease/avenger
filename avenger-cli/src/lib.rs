//! `avenger`, the command-line program of avenger-sql projects. Each command starts, reads the
//! files, answers, and exits: `check` reports the diagnostics of a project's files, and `fmt`
//! formats its scripts, modules, and the `{avenger-sql}` cells of its Quarto documents. `tables`,
//! `schema`, and `query` read the project's data, `charts` lists its charts, `render` draws them,
//! and `view` opens a window on one. `docs` prints the reference of the language, and `complete`
//! and `signature` the completions and the signature help at a position. `lsp` is
//! the exception that an editor starts and stops: the language server of the same files.
//! Results go to standard output, as text or, with `--json`, as JSON, and errors that keep a
//! command from running go to standard error.
//!
//! The exit status is 0 for success, 1 for diagnostics with errors or files that need
//! formatting, and 2 when the command cannot run, such as for a usage error or a manifest that
//! cannot be read.

mod assist;
mod at;
mod charts;
mod check;
mod data;
mod docs;
mod files;
mod fmt;
pub mod lsp;
mod project;
mod query;
mod render;
mod report;
pub mod statement;
pub mod view;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::report::Output;

#[derive(Parser)]
#[command(
    name = "avenger",
    version,
    about = "Check, format, query, render, and look up avenger-sql projects"
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
    /// Print the reference of a function, a constructor, a parameter path such as
    /// `mark_bar.x.scale`, a style property or element as `style:NAME`, or a syntax form, or search
    /// the reference with `docs search WORDS`.
    Docs {
        /// A name, a dotted path, `style:NAME`, a syntax form such as `LOAD DATA`, or `search` and
        /// its words.
        words: Vec<String>,
        /// The hover at a position, `PATH:LINE:COLUMN`.
        #[arg(long, conflicts_with_all = ["words", "all"])]
        at: Option<String>,
        /// The whole reference, as one Markdown file.
        #[arg(long, conflicts_with = "words")]
        all: bool,
        /// The forms only.
        #[arg(long, conflicts_with = "examples")]
        brief: bool,
        /// The examples only.
        #[arg(long)]
        examples: bool,
        /// Keep the functions of a kind.
        #[arg(long, value_enum)]
        kind: Option<docs::Kind>,
        /// Keep the entries of a family.
        #[arg(long, value_enum)]
        family: Option<docs::FamilyName>,
        /// Keep the functions that return a type, such as `INT64`.
        #[arg(long)]
        returns: Option<String>,
        /// Keep the functions that take a type, such as `TIMESTAMP`.
        #[arg(long)]
        takes: Option<String>,
    },
    /// Print the completions at a position, `PATH:LINE:COLUMN`, as an editor shows them.
    Complete {
        /// The position: lines and columns from 1, columns in characters.
        at: String,
        /// The most completions.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Print the signature help at a position, `PATH:LINE:COLUMN`: the forms of the call there,
    /// the form that it takes, and its argument at the position.
    Signature {
        /// The position: lines and columns from 1, columns in characters.
        at: String,
    },
    /// List the names of the tables and the views that the project's data scripts register, with
    /// their data scripts.
    Tables {
        /// A directory of the project. Without it, the current directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Print the columns, the types, and the row counts of tables of the project: of the tables
    /// named, or of every table.
    Schema {
        /// The tables, as a data script names them, such as `cars` or `datasets.cars`. Without
        /// one, every table.
        tables: Vec<String>,
        /// A directory of the project. Without it, the current directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Run a query in the project's session, as a chart script runs, and print its rows.
    Query {
        /// The query, avenger-sql. Without it, `-f`.
        #[arg(required_unless_present = "file", conflicts_with = "file")]
        sql: Option<String>,
        /// A file of the query: a chart script, a cell as `report.qmd#NUMBER` or
        /// `report.qmd#LABEL`, a statement at a line as `FILE:LINE`, or another file of a project.
        #[arg(short, long)]
        file: Option<String>,
        /// A directory of the project of the query. Without it, the current directory.
        #[arg(long, conflicts_with = "file")]
        project: Option<PathBuf>,
        /// The most rows, or `all`. Without it, 10 rows that print, and every row that `-o`
        /// writes to a file.
        #[arg(long, value_parser = query::Limit::parse)]
        limit: Option<query::Limit>,
        /// The format of the rows: table, csv, json, parquet, or arrow. Without it, the extension
        /// of `-o`, or a table.
        #[arg(long = "output")]
        format: Option<String>,
        /// Print the logical and the physical plans in place of the rows.
        #[arg(long)]
        explain: bool,
        /// A file to write the rows to, in place of standard output.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// List the chart scripts and the `{avenger-sql}` cells of the project, as `render` and
    /// `view` name them, each with the columns of charts of its result.
    Charts {
        /// A directory of the project. Without it, the current directory.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Open a window on the result of a chart script, or of a cell of a Quarto document, which
    /// stays current as the project's files change.
    View {
        /// A chart script, a cell as `report.qmd#NUMBER` or `report.qmd#LABEL`, or the statement
        /// at a line of either as `FILE:LINE`.
        target: String,
        /// The color scheme that the window starts in: light or dark. Without it, the system's.
        #[arg(long)]
        color_scheme: Option<String>,
    },
    /// Render the chart of a chart script, or of a cell of a Quarto document, to PNG, SVG, or PDF
    /// by the extension of `-o`, as `avenger view` draws it. A result of several charts renders as
    /// a table.
    Render {
        /// A chart script, a cell as `report.qmd#NUMBER` or `report.qmd#LABEL`, or the statement
        /// at a line of either as `FILE:LINE`. With `--all`, a directory of the project, or none
        /// for the current directory.
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
        Command::Docs {
            words,
            at,
            all,
            brief,
            examples,
            kind,
            family,
            returns,
            takes,
        } => {
            let options = docs::Options {
                words,
                at,
                all,
                brief,
                examples,
                kind,
                family,
                returns,
                takes,
            };
            runtime.block_on(docs::run(&options, &output))
        }
        Command::Complete { at, limit } => runtime.block_on(assist::complete(&at, limit, &output)),
        Command::Signature { at } => runtime.block_on(assist::signature(&at, &output)),
        Command::Tables { project } => runtime.block_on(data::tables(project.as_deref(), &output)),
        Command::Schema { tables, project } => {
            runtime.block_on(data::schema(&tables, project.as_deref(), &output))
        }
        Command::Query {
            sql,
            file,
            project,
            limit,
            format,
            explain,
            out,
        } => {
            let source = match (sql, file) {
                (Some(sql), _) => query::Source::Text(sql),
                (None, Some(file)) => query::Source::File(file),
                (None, None) => unreachable!("clap requires one"),
            };
            let options = query::Options {
                project,
                limit,
                format,
                explain,
                out,
            };
            runtime.block_on(query::run(source, &options, &output))
        }
        Command::Charts { project } => runtime.block_on(charts::run(project.as_deref(), &output)),
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
