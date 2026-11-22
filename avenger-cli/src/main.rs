//! `avenger`, the command-line program of avenger-sql projects. Each command starts, reads the
//! files, answers, and exits: `check` reports the diagnostics of a project's files, and `fmt`
//! formats its scripts and modules. Results go to standard output, as text or, with `--json`, as
//! JSON, and errors that keep a command from running go to standard error.
//!
//! The exit status is 0 for success, 1 for diagnostics with errors or files that need
//! formatting, and 2 when the command cannot run, such as for a usage error or a manifest that
//! cannot be read.

mod check;
mod files;
mod fmt;
mod report;

use std::path::PathBuf;
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
    /// Format scripts and modules in place: of the given files and directories, or of the
    /// project of the current directory.
    Fmt {
        /// Files or directories, in a project or not. Without one, the current directory.
        paths: Vec<PathBuf>,
        /// Change no file, and list the files that formatting would change.
        #[arg(long)]
        check: bool,
    },
}

/// Whether a command found what fails it: errors, or files that need formatting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Ok,
    Failed,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
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
