# ICU number reference tools

These scripts produce data and test expectations for `avenger-format-number-icu`. Run them from the repository root with Python 3. They download pinned, checksummed files into a temporary directory, so they need network access. Versions and checksums are set in the scripts. File paths below are relative to `avenger-format-number-icu/`.

## When the grid test fails

`tests/reference.rs` checks Avenger against ICU4J's labels for a grid of locales, skeletons, and values in `tests/fixtures/icu4j_grid.json`. Rows where Avenger intentionally differs are recorded in `tests/fixtures/icu4j_grid_differences.json`. A failure lists each changed row with Avenger's label, the expected label, and ICU4J's.

If the change is intended, accept it by copying the file the test writes, then review the diff:

```sh
cp avenger-format-number-icu/tests/output/icu4j_grid_differences.json avenger-format-number-icu/tests/fixtures/
```

## Add a reference case

`tests/fixtures/icu4j.json` pins specific inputs. To add one, append its `locale`, `skeleton`, and `bits`, the input's IEEE 754 bits in hexadecimal, then run the generator:

```sh
python3 -c "import struct; print(struct.pack('>d', 1234.5).hex())"  # bits for 1234.5
python3 tools/icu-number-reference/generate_reference.py
```

The generator runs ICU4J 78.1 on every case and writes its label to `expected`, or its exception to `icu4j_error`. It also regenerates the grid. It needs Java 21; pass `--java /path/to/java` if the `java` on your `PATH` is another version.

When Avenger's result should differ from ICU4J's, add one of these by hand:

- `compatibility_exception`: an intentional difference, with a `reason` and Avenger's `expected` label or `error` category
- `error`: Avenger's error category when ICU4J also rejects the input, with an optional byte `position`

## Update CLDR, ICU, or ICU4J

After changing a version in the scripts, regenerate everything, then run the tests and review the results:

```sh
python3 tools/icu-number-reference/generate_data.py
cargo fmt -p avenger-format-number-icu
python3 tools/icu-number-reference/generate_reference.py
cargo test -p avenger-format-number-icu
```

`generate_data.py` writes `src/generated.rs` and `src/generated_usage.rs`: CLDR data that ICU4X's compiled data lacks. After upgrading ICU4X itself, just run the tests.
