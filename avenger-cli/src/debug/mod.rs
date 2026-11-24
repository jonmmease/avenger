//! `avenger dap`: a debug adapter of a query's stages, which speaks the Debug Adapter Protocol on
//! standard input and output and shows each stop in a window as `avenger view` shows a result.
//!
//! In pipe syntax every prefix of a query is a query, so a stop at a `|>` shows the rows that
//! enter its operator by running the script's text cut before the `|>`, and a stop at a
//! statement's result shows its rows, its table, or its chart. A breakpoint on a `|>` line stops
//! there, and the debugger's steps move to the next stage, the stage before, or the statement's
//! result.
//!
//! Three threads share the work. One reads the protocol's messages from standard input. The
//! adapter's thread answers them in order, runs the stops, and writes responses and events to
//! standard output, which nothing else writes. The window, a native window that macOS keeps on the
//! process's first thread, opens when `launch` loads its target, and takes each stop's rows over
//! a channel. Without a window, `--headless` answers the same requests, so the stops show only
//! in the debugger's panes.

mod adapter;
mod debuggee;
mod protocol;
mod stops;
#[cfg(test)]
mod tests;

use std::io::BufReader;
use std::sync::mpsc::channel;

use adapter::{Adapter, Open};

/// Serves a debugging session on standard input and output until the client disconnects, with a
/// window on each stop, or without one when `headless`.
pub fn run(headless: bool) -> Result<(), String> {
    let (messages, received) = channel();
    std::thread::Builder::new()
        .name("avenger-dap-input".into())
        .spawn(move || {
            let mut input = BufReader::new(std::io::stdin().lock());
            loop {
                let message = protocol::read(&mut input);
                let end = message.is_none();
                if messages.send(message).is_err() || end {
                    return;
                }
            }
        })
        .map_err(|error| error.to_string())?;
    let (window, opens) = channel::<Open>();
    let window = (!headless).then_some(window);
    let adapter = Adapter::new(std::io::stdout(), window)?;
    let session = std::thread::Builder::new()
        .name("avenger-dap".into())
        .spawn(move || adapter.serve(received))
        .map_err(|error| error.to_string())?;
    // The window takes this thread, as a native window needs on macOS, once the target loads.
    if let Ok(Open { title, sink }) = opens.recv() {
        crate::view::window(title, None, move |window| {
            let _ = sink.send(window);
        })?;
    }
    session
        .join()
        .map_err(|_| "the session's thread panicked".to_string())
}
