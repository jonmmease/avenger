# Vega-Lite spec fixtures

These small authored examples cover encoding aggregation, encoding binning, and explicit bin-to-aggregate transforms with pre-binned position fields. Tests also vary individual options without duplicating the fixture files.

The fixtures and serialized `bar_specs` example output are checked against the [Vega-Lite v6.4.3 JSON Schema](https://vega.github.io/schema/vega-lite/v6.4.3.json). Schema SHA-256: `4f11cd379b7cac0ddee17eefea84c028bd41619ace28778acf843c009e43abd2`. The schema audit was performed on 2026-09-21 with Python's `jsonschema` validator. Normal Cargo tests use only local files and do not download the schema.
