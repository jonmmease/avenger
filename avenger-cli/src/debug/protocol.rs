//! The Debug Adapter Protocol's messages and their framing: each message is a JSON object after a
//! `Content-Length` header and a blank line. Requests come in on standard input; responses and
//! events go out on standard output, which nothing else writes. The adapter's log of every message
//! goes to standard error.

use std::io::{BufRead, Write};

use serde::Deserialize;
use serde_json::{json, Value};

/// A request of the client.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Request {
    pub(crate) seq: i64,
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) arguments: Value,
}

/// Reads the next message of `input`: `None` at the end of the input, and an error for a message
/// whose header or body doesn't parse.
pub(crate) fn read(input: &mut impl BufRead) -> Option<Result<Value, String>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        match input.read_line(&mut line) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(error) => return Some(Err(error.to_string())),
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("Content-Length") {
                length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let Some(length) = length else {
        return Some(Err("a message without a Content-Length header".into()));
    };
    let mut body = vec![0; length];
    if let Err(error) = input.read_exact(&mut body) {
        return Some(Err(error.to_string()));
    }
    Some(serde_json::from_slice(&body).map_err(|error| error.to_string()))
}

/// The writer of the adapter's messages, which numbers them in order.
pub(crate) struct Output<W: Write> {
    out: W,
    seq: i64,
}

impl<W: Write> Output<W> {
    pub(crate) fn new(out: W) -> Self {
        Output { out, seq: 0 }
    }

    /// Writes `message` with the next sequence number, and logs it to standard error.
    fn write(&mut self, mut message: Value) {
        self.seq += 1;
        message["seq"] = json!(self.seq);
        let body = message.to_string();
        eprintln!("avenger dap -> {body}");
        let _ = write!(self.out, "Content-Length: {}\r\n\r\n{body}", body.len());
        let _ = self.out.flush();
    }

    /// The response to `request` that it succeeded, with `body`.
    pub(crate) fn respond(&mut self, request: &Request, body: Value) {
        self.write(json!({
            "type": "response",
            "request_seq": request.seq,
            "success": true,
            "command": request.command,
            "body": body,
        }));
    }

    /// The response to `request` that it failed, with `message`, which the client shows.
    pub(crate) fn fail(&mut self, request: &Request, message: &str) {
        self.write(json!({
            "type": "response",
            "request_seq": request.seq,
            "success": false,
            "command": request.command,
            "message": message,
            "body": {"error": {"id": 1, "format": message, "showUser": true}},
        }));
    }

    /// The event `event`, with `body`.
    pub(crate) fn event(&mut self, event: &str, body: Value) {
        self.write(json!({"type": "event", "event": event, "body": body}));
    }

    /// An `output` event of `text` in the console, as a line.
    pub(crate) fn console(&mut self, text: &str) {
        self.event(
            "output",
            json!({"category": "console", "output": format!("{text}\n")}),
        );
    }
}
