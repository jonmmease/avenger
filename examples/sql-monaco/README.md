# sql-monaco

An editor of an avenger-sql project in the browser, with Monaco and avenger-sql-web. The page opens the Vega project of avenger-sql-project's tests as a project in memory, and Monaco edits its chart scripts, data scripts, module, and stylesheet. avenger-sql-web answers Monaco's requests from avenger-sql-language: completions in their order, hover, signature help, errors and their quick fixes, go to definition and references across files, rename, formatting, folding, the outline, and semantic highlighting. A change of a data script loads it again, with the scripts downstream of it, and the preview draws the charts of the open chart script, or of the last one that was open, with the project's stylesheet.

## Run

Write the fonts, the project's files, and `manifest.json` into `site/`:

```bash
cargo run --release -p sql-monaco --bin build-site
```

Build the library into `site/lib/`:

```bash
avenger-chart-web/scripts/build.sh avenger-sql-web examples/sql-monaco/site/lib
```

Serve the site:

```bash
python3 -m http.server 8782 -d examples/sql-monaco/site
```

Then open `http://localhost:8782/` in a browser with WebGPU. The page loads Monaco from jsDelivr. The `sql-monaco` entry of `.claude/launch.json` serves the same directory.

## Known limitations

- The page runs avenger-sql-web on its main thread, so a slow query or load holds up typing.
- The editor lists the project's scripts, module, and stylesheet, and leaves out its Quarto document, whose cells avenger-sql-web does not edit.
