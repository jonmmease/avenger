# chart-server

A server that builds a chart on each request, and a page that draws it with avenger-chart-web. The page sends a genre and a range of years. The server selects the movies of `movie-search` with a DataFusion query, builds a scatter chart of their ratings with a navigation and a tooltip, and answers with the bytes of its message. DataFusion and the data stay on the server, and the page only draws.

## Run

Build the npm package of avenger-chart-web in `avenger-chart-web/js`:

```bash
cd avenger-chart-web/js && npm ci && npm run build
```

Start the server, which serves the page, the package, and the charts:

```bash
cargo run --release -p chart-server
```

Then open `http://127.0.0.1:8779/` in a browser with WebGPU. `chart-server <PORT> <PACKAGE_DIR>` serves another port or package.

## Routes

- `GET /`: the page.
- `GET /pkg/<FILE>`: a file of the package's `dist/` or `wasm/`.
- `GET /genres`: the genres of the movies, as JSON.
- `POST /chart`: the bytes of the chart of the choices in the body, such as `{"genre": "Drama", "from": 1990, "to": 2000}`, or status 400 and the reason. A `null` genre selects every genre.
