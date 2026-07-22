use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
};

use serde_json::{Value, json};

fn write_message(writer: &mut impl Write, value: &Value) {
    let body = serde_json::to_vec(value).expect("serialize LSP message");
    write!(writer, "Content-Length: {}\r\n\r\n", body.len()).expect("write LSP header");
    writer.write_all(&body).expect("write LSP body");
    writer.flush().expect("flush LSP message");
}

fn read_message(reader: &mut impl BufRead) -> Value {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        assert_ne!(reader.read_line(&mut line).expect("read LSP header"), 0);
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            content_length = Some(value.trim().parse::<usize>().expect("content length"));
        }
    }
    let mut body = vec![0; content_length.expect("Content-Length header")];
    reader.read_exact(&mut body).expect("read LSP body");
    serde_json::from_slice(&body).expect("parse LSP response")
}

#[test]
fn repeated_lsp_sessions_frame_stdout_reject_malformed_params_and_exit_cleanly() {
    for restart in 0..3 {
        let mut child = Command::new(env!("CARGO_BIN_EXE_avenger"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn avenger lsp");
        let mut stdin = child.stdin.take().expect("child stdin");
        let mut stdout = BufReader::new(child.stdout.take().expect("child stdout"));

        write_message(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "capabilities": {} }
            }),
        );
        let initialize = read_message(&mut stdout);
        assert_eq!(initialize["id"], 1, "restart {restart}");
        assert_eq!(initialize["result"]["serverInfo"]["name"], "avenger-lsp");

        write_message(
            &mut stdin,
            &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
        );
        write_message(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": 98,
                "method": "textDocument/hover",
                "params": {}
            }),
        );
        let malformed = read_message(&mut stdout);
        assert_eq!(malformed["id"], 98);
        assert!(malformed["error"].is_object());

        write_message(
            &mut stdin,
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" }),
        );
        let shutdown = read_message(&mut stdout);
        assert_eq!(shutdown["id"], 2);
        assert!(shutdown["result"].is_null());
        write_message(&mut stdin, &json!({ "jsonrpc": "2.0", "method": "exit" }));
        drop(stdin);

        let status = child.wait().expect("wait for avenger lsp");
        assert!(status.success(), "restart {restart} exited with {status}");
        let mut trailing = Vec::new();
        stdout
            .read_to_end(&mut trailing)
            .expect("read trailing stdout");
        assert!(
            trailing.is_empty(),
            "restart {restart} emitted non-protocol bytes after shutdown: {:?}",
            String::from_utf8_lossy(&trailing)
        );
    }
}
