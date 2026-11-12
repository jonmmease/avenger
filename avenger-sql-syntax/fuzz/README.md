# Fuzzing avenger-sql-syntax

The `parse` target gives arbitrary text to the parser and checks the guarantees of the crate. It skips input that is not valid UTF-8. For each text, it parses the text as a script and as a module, checks that each tree holds the text exactly, and calls `expected_at` at the end of the text. A panic, a failed assertion, or a stack overflow is a crash.

The target uses [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz), which runs libFuzzer and requires a nightly toolchain. This directory is a Cargo workspace of its own, so its build goes to `fuzz/target/` and does not change the lock file of the avenger workspace.

## Install

```bash
rustup toolchain install nightly
```

```bash
cargo install cargo-fuzz
```

## Run

Run the target from the `avenger-sql-syntax` directory. cargo-fuzz builds the target with optimizations. The arguments after `--` go to libFuzzer, and `-max_total_time` is the run time in seconds:

```bash
cargo +nightly fuzz run parse -- -max_total_time=300
```

Without `-max_total_time`, the run continues until you stop it with Ctrl-C. The status lines show the number of runs, `cov` (the code locations that the inputs reach), and `corp` (the number of inputs that libFuzzer keeps). A run of five minutes makes more than a million runs.

libFuzzer keeps each input that reaches new code in `fuzz/corpus/parse/`. A later run starts from these inputs, so the coverage grows from run to run. To guide a run toward a part of the grammar, put text files of SQL in that directory before the run.

## Fix a crash

When a run finds a crash, libFuzzer stops and writes the input to `fuzz/artifacts/parse/crash-<HASH>`, where `<HASH>` is the hash in the file name that the run prints. Run that input again with this command:

```bash
cargo +nightly fuzz run parse fuzz/artifacts/parse/crash-<HASH>
```

Make the input as small as possible:

```bash
cargo +nightly fuzz tmin parse fuzz/artifacts/parse/crash-<HASH>
```

Add the smallest text to the inputs of `tests/lossless.rs`, which make the same checks as the target. Then fix the parser and confirm that the test passes:

```bash
cargo test --release -p avenger-sql-syntax --test lossless
```

Git ignores `target/`, `corpus/`, `artifacts/`, and `coverage/` in this directory.
