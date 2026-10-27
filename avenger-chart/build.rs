//! Generates the prost messages of package `avenger.chart.v1` from the files in
//! `proto/avenger/chart/v1/` with the vendored `protoc`, so that a build needs no system `protoc`.

const PROTOS: [&str; 15] = [
    "attribution",
    "axis",
    "cartesian",
    "chart",
    "coord",
    "encoding",
    "geo",
    "guide",
    "legend",
    "mark",
    "option",
    "resolve",
    "scale",
    "side",
    "style",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto");
    let files = PROTOS.map(|name| format!("proto/avenger/chart/v1/{name}.proto"));
    prost_build::Config::new()
        .protoc_executable(protoc_bin_vendored::protoc_bin_path()?)
        .compile_protos(&files, &["proto"])?;
    Ok(())
}
