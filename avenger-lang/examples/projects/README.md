# Example projects

`hello_scatter/chart.avenger` is the first complete language/compiler vertical
slice. Run it through the real file compiler with:

```sh
cargo run --release -p avenger-lang --example compile_module -- \
  avenger-lang/examples/projects/hello_scatter/chart.avenger
```

`multi_chart/` demonstrates deterministic project compilation with shared
catalog data and definition imports:

```sh
cargo run --release -p avenger-lang --example compile_project -- \
  avenger-lang/examples/projects/multi_chart
cargo run --release -p avenger-lang --example inspect_interface -- \
  avenger-lang/examples/projects/multi_chart
```
