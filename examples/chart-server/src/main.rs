//! Serves the chart server's page and the avenger-chart-web package, and answers each request for a
//! chart with the bytes of a chart that it builds.
//!
//! Usage: `chart-server [PORT] [PACKAGE_DIR]`. `PORT` is 8779 by default, and `PACKAGE_DIR` is the
//! npm package that `npm run build` writes in `avenger-chart-web/js`, by default.
//!
//! Routes:
//! - `GET /`: the page.
//! - `GET /pkg/<FILE>`: a file of the package, such as `dist/index.js` and the module.
//! - `GET /genres`: the genres of the movies, as JSON.
//! - `POST /chart`: the bytes of the chart of the choices in the body, or status 400 and the reason.

use std::path::{Component, Path, PathBuf};

use chart_server::Movies;
use tiny_http::{Header, Method, Response, Server};

const PAGE: &str = include_str!("../site/index.html");

fn main() {
    let mut args = std::env::args().skip(1);
    let port: u16 = args
        .next()
        .map(|port| port.parse().expect("the port is a number"))
        .unwrap_or(8779);
    let package = args.next().map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../avenger-chart-web/js")
    });
    let runtime = tokio::runtime::Runtime::new().expect("the runtime starts");
    let movies = Movies::new();
    let server = Server::http(("127.0.0.1", port)).expect("the server binds its port");
    println!("chart-server: http://127.0.0.1:{port}/");
    for mut request in server.incoming_requests() {
        let url = request.url().to_string();
        let response = match (request.method(), url.as_str()) {
            (Method::Get, "/") => text(200, PAGE, "text/html; charset=utf-8"),
            (Method::Get, "/genres") => match runtime.block_on(movies.genres()) {
                Ok(genres) => text(
                    200,
                    &serde_json::to_string(&genres).expect("the genres encode"),
                    "application/json",
                ),
                Err(rejection) => text(rejection.status, &rejection.message, "text/plain"),
            },
            (Method::Post, "/chart") => {
                let mut body = Vec::new();
                match std::io::Read::read_to_end(request.as_reader(), &mut body) {
                    Ok(_) => match runtime.block_on(movies.chart(&body)) {
                        Ok(bytes) => bytes_response(bytes, "application/octet-stream"),
                        Err(rejection) => text(rejection.status, &rejection.message, "text/plain"),
                    },
                    Err(error) => text(400, &error.to_string(), "text/plain"),
                }
            }
            (Method::Get, path) if path.starts_with("/pkg/") => file(&package, &path[5..]),
            _ => text(404, "not found", "text/plain"),
        };
        let _ = request.respond(response);
    }
}

fn header(value: &str) -> Header {
    Header::from_bytes("Content-Type", value).expect("the header is valid")
}

fn text(status: u16, body: &str, kind: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(status)
        .with_header(header(kind))
}

fn bytes_response(bytes: Vec<u8>, kind: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(bytes).with_header(header(kind))
}

/// The file `name` of the package's `dist/` or `wasm/`, which names no parent directory.
fn file(package: &Path, name: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let relative = Path::new(name);
    let served = relative.starts_with("dist") || relative.starts_with("wasm");
    if !served
        || !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return text(404, "not found", "text/plain");
    }
    let kind = match relative
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("js") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    };
    match std::fs::read(package.join(relative)) {
        Ok(bytes) => bytes_response(bytes, kind),
        Err(_) => text(404, "not found", "text/plain"),
    }
}
